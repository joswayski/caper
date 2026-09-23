use super::*;
use futures_util::{SinkExt, StreamExt};
use sqlx::{
    Connection, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{future::IntoFuture, str::FromStr};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

// Only provider provisioning is mocked; sockets, authorization and shared state
// below use the real implementations and disposable Postgres/Valkey.
struct Provider {
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
}
#[async_trait::async_trait]
impl crate::Provider for Provider {
    async fn create_session(&self, _: &Config) -> Result<String, crate::ProviderError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(Uuid::new_v4().to_string())
    }
    async fn turn(&self, _: &Config) -> Result<Vec<crate::IceServer>, crate::ProviderError> {
        Ok(vec![])
    }
    async fn revoke_turn(&self, _: &Config, _: &str) -> Result<(), crate::ProviderError> {
        Ok(())
    }
    async fn session_tracks(&self, _: &Config, _: &str) -> Result<Value, crate::ProviderError> {
        Ok(json!({"tracks":[]}))
    }
    async fn tracks_new(
        &self,
        _: &Config,
        _: &str,
        _: Value,
    ) -> Result<Value, crate::ProviderError> {
        unreachable!("no audio negotiation in gateway test")
    }
    async fn negotiate(
        &self,
        _: &Config,
        _: &str,
        _: Value,
    ) -> Result<Value, crate::ProviderError> {
        unreachable!("no audio negotiation in gateway test")
    }
    async fn close(&self, _: &Config, _: &str, _: &str) -> Result<Value, crate::ProviderError> {
        unreachable!("no audio tracks in gateway test")
    }
}

#[test]
fn protocol_limits_and_path_validation_are_explicit() {
    assert!(valid_id("Ab0123456789"));
    assert!(!valid_id("../secret/id"));
    assert!(!valid_id(""));
    assert!(serde_json::from_value::<Frame>(json!({"type":"subscribe","id":"a","kind":"presence","userIds":["user"],"arbitrary":true})).is_err());
    assert!(
        serde_json::from_value::<Frame>(json!({"type":"activity","activityAgeMs":-1})).is_err()
    );
    assert!(serde_json::from_value::<Frame>(json!({"type":"subscribe","id":"a","kind":"chat","channelId":"Ab0123456789","after":"9007199254740993"})).is_ok());
}

#[tokio::test]
async fn local_fanout_does_not_wake_unrelated_channels() {
    let app = Application::default();
    let left = app.channel("left");
    let right = app.channel("right");
    let mut a = left.events.subscribe();
    let mut b = right.events.subscribe();
    let payload = json!({"type":"typing.updated","channelId":"right"});
    app.dispatch(&payload);
    assert_eq!(b.recv().await.unwrap(), payload);
    assert!(matches!(
        a.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    ));
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next(socket: &mut Socket, kind: &str, id: Option<&str>) -> Value {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let text = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let value: Value = serde_json::from_str(&text).unwrap();
            if value["type"] == kind && id.is_none_or(|id| value["id"] == id) {
                return value;
            }
        }
    })
    .await
    .expect("gateway event timeout")
}

async fn transmit(socket: &mut Socket, value: Value) {
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            value.to_string().into(),
        ))
        .await
        .unwrap();
}

async fn connect(address: std::net::SocketAddr, token: &str) -> Socket {
    let mut request = format!("ws://{address}/api/chat/events")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("cookie", format!("caper_session={token}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(
        next(&mut socket, "hello", None).await["idleTimeoutSeconds"],
        600
    );
    transmit(&mut socket, json!({"type":"heartbeat","activityAgeMs":0})).await;
    socket
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn multiplexed_presence_commands_and_cross_gateway_handoff() {
    let options =
        PgConnectOptions::from_str(&std::env::var("CHAT_TEST_DATABASE_URL").unwrap()).unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("gateway_test_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE DATABASE {database}").as_str())
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(12)
        .connect_with(options.database(&database))
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    chat::seed(&pool).await.unwrap();
    let user_external = crate::auth::random_id(12);
    let user: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,'owner','Owner') RETURNING id")
        .bind(&user_external).fetch_one(&pool).await.unwrap();
    let token = Uuid::new_v4().to_string();
    let hash = Sha256::digest(token.as_bytes()).to_vec();
    sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')")
        .bind(&hash).bind(user).execute(&pool).await.unwrap();
    let space = crate::auth::random_id(12);
    let space_id: i64 = sqlx::query_scalar(
        "INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,'Space',$2) RETURNING id",
    )
    .bind(&space)
    .bind(user)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
        .bind(space_id)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    let channel = crate::auth::random_id(12);
    sqlx::query("INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,'general')")
        .bind(&channel)
        .bind(space_id)
        .execute(&pool)
        .await
        .unwrap();
    let redis_url = std::env::var("CHAT_TEST_VALKEY_URL").unwrap();
    assert!(
        redis_url.starts_with("redis://127.0.0.1:") || redis_url.starts_with("redis://localhost:")
    );
    let broker = redis::Client::open(redis_url).unwrap();
    let chat = Chat {
        pool: pool.clone(),
        broker: broker.clone(),
        wake: Arc::new(tokio::sync::Notify::new()),
    };
    let chat_token = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,user_id,name,account_session_hash) VALUES($1,$2,$3,'Owner',$4)")
        .bind(crate::auth::random_id(12)).bind(Sha256::digest(chat_token.as_bytes()).to_vec())
        .bind(user).bind(&hash).execute(&pool).await.unwrap();
    let mut servers = Vec::new();
    let mut gateways = Vec::new();
    let media_key = format!("caper:{{gateway-test-{}}}:state", Uuid::new_v4());
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    for _ in 0..2 {
        let mut app = AppState::with_database(
            Config::test(true),
            Arc::new(Provider {
                entered: entered.clone(),
                release: release.clone(),
            }),
            Some(pool.clone()),
        );
        app.auth = crate::auth::AuthVerifier::new();
        app.chat = Some(chat.clone());
        app.connect_media_store(&std::env::var("CHAT_TEST_VALKEY_URL").unwrap(), &media_key)
            .await
            .unwrap();
        let application = Application {
            state: Some(app),
            presence: Some(
                Presence::new(&broker, Duration::from_secs(600))
                    .await
                    .unwrap(),
            ),
            broker: Mutex::new(Some(Application::connect(&chat).await.unwrap())),
            idle_seconds: 600,
            ..Application::default()
        };
        let mut gateway = Gateway::new(chat.clone());
        gateway.application = Arc::new(application);
        gateway.start();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        servers.push(tokio::spawn(
            axum::serve(listener, super::super::router(gateway.clone())).into_future(),
        ));
        gateways.push((gateway, address));
    }
    let mut old = connect(gateways[0].1, &token).await;
    transmit(
        &mut old,
        json!({"type":"subscribe","kind":"chat","id":"chat","channelId":channel,"after":"0"}),
    )
    .await;
    assert_eq!(
        next(&mut old, "event", Some("chat")).await["event"]["cursor"],
        "0"
    );
    next(&mut old, "subscribed", Some("chat")).await;
    transmit(&mut old, json!({"type":"subscribe","kind":"presence","id":"people","spaceId":space,"userIds":[user_external]})).await;
    let snapshot = next(&mut old, "event", Some("people")).await;
    assert_eq!(snapshot["event"]["members"][0]["status"], "online");
    next(&mut old, "subscribed", Some("people")).await;

    // Commit two events but publish only the second: replay must repair the gap
    // and preserve order, rather than treating the broker as an ordered log.
    let mut tx = pool.begin().await.unwrap();
    for seq in 1..=2i64 {
        sqlx::query("INSERT INTO public.channel_events(channel_id,seq,payload) SELECT id,$2,$3 FROM public.channels WHERE external_id=$1")
            .bind(&channel).bind(seq).bind(json!({"type":"message.created","channelId":channel,"seq":seq.to_string()}))
            .execute(&mut *tx).await.unwrap();
    }
    sqlx::query("UPDATE public.channels SET last_seq=2 WHERE external_id=$1")
        .bind(&channel)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let mut connection = broker.get_multiplexed_async_connection().await.unwrap();
    let _: i64 = redis::cmd("PUBLISH")
        .arg(format!("{}:{channel}", chat::TOPIC))
        .arg(json!({"type":"message.created","channelId":channel,"seq":"2"}).to_string())
        .query_async(&mut connection)
        .await
        .unwrap();
    assert_eq!(
        next(&mut old, "event", Some("chat")).await["event"]["seq"],
        "1"
    );
    assert_eq!(
        next(&mut old, "event", Some("chat")).await["event"]["seq"],
        "2"
    );

    let mut typing_feed = broker.get_async_pubsub().await.unwrap();
    typing_feed
        .subscribe(format!("{}:{channel}", chat::TYPING_TOPIC))
        .await
        .unwrap();
    let mut pulses = typing_feed.on_message();
    let typing_id = Uuid::new_v4().to_string();
    // Reusing an ID with a different typing state must publish the new pulse,
    // rather than replaying a cached result or rejecting a receipt conflict.
    for active in [true, false] {
        transmit(&mut old, json!({"type":"command","id":typing_id,"issuedAt":chrono::Utc::now().timestamp_millis(),"method":"typing","channelId":channel,"chatToken":chat_token,"body":{"typing":active}})).await;
        assert_eq!(
            next(&mut old, "result", Some(&typing_id)).await["status"],
            204
        );
        let pulse = tokio::time::timeout(Duration::from_secs(2), pulses.next())
            .await
            .unwrap()
            .unwrap();
        let event: Value = serde_json::from_str(&pulse.get_payload::<String>().unwrap()).unwrap();
        assert_eq!(event["typing"], active);
        assert_eq!(event["author"]["id"], user_external);
        assert_eq!(event["channelId"], channel);
    }
    let receipts: Vec<String> = redis::cmd("KEYS")
        .arg(format!("*{typing_id}"))
        .query_async(&mut connection)
        .await
        .unwrap();
    assert!(
        receipts.is_empty(),
        "typing must not retain command receipts"
    );
    transmit(&mut old, json!({"type":"command","id":typing_id,"issuedAt":chrono::Utc::now().timestamp_millis(),"method":"typing","channelId":channel,"chatToken":"invalid-token","body":{"typing":true}})).await;
    assert_eq!(
        next(&mut old, "result", Some(&typing_id)).await["status"],
        401
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), pulses.next())
            .await
            .is_err(),
        "no receipt must not mean no authorization"
    );

    let id = Uuid::new_v4().to_string();
    let command = json!({"type":"command","id":id,"issuedAt":chrono::Utc::now().timestamp_millis(),"method":"media.join","channelId":channel,"body":{"name":"Owner"}});
    transmit(&mut old, command.clone()).await;
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    gateways[0].0.begin_shutdown();
    next(&mut old, "migrating", None).await;
    // Lose the original socket while the provider is still accepting Join.
    // Its detached command must finish; another gateway must not execute it.
    drop(old);
    let mut replacement = connect(gateways[1].1, &token).await;
    transmit(&mut replacement, command.clone()).await;
    let pending = next(&mut replacement, "result", Some(&id)).await;
    assert_eq!(pending["status"], 409);
    assert_eq!(pending["body"]["code"], "command_pending");
    release.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            transmit(&mut replacement, command.clone()).await;
            let response = next(&mut replacement, "result", Some(&id)).await;
            if response["status"] != 409 {
                break response;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("detached Join never completed");
    assert_eq!(result["status"], 200, "{result}");
    let media_token = result["body"]["token"].as_str().unwrap();
    transmit(&mut replacement, command.clone()).await;
    assert_eq!(next(&mut replacement, "result", Some(&id)).await, result);
    let mut conflict = command;
    conflict["body"] = json!({"different":true});
    transmit(&mut replacement, conflict).await;
    assert_eq!(
        next(&mut replacement, "result", Some(&id)).await["status"],
        409
    );
    let statuses = gateways[1]
        .0
        .application
        .presence
        .as_ref()
        .unwrap()
        .statuses(std::slice::from_ref(&user_external))
        .await
        .unwrap();
    assert_eq!(
        statuses[0]["status"], "online",
        "old socket disappearing cannot clear its replacement"
    );

    transmit(&mut replacement, json!({"type":"subscribe","kind":"media","id":"room","channelId":channel,"token":media_token})).await;
    let roster = next(&mut replacement, "event", Some("room")).await;
    assert_eq!(
        roster["event"]["participants"].as_array().unwrap().len(),
        1,
        "retry Join cannot create another participant"
    );
    assert_eq!(roster["event"]["participants"][0]["name"], "Owner");
    assert_eq!(roster["event"]["participants"][0]["muted"], false);
    next(&mut replacement, "subscribed", Some("room")).await;
    let mute = Uuid::new_v4().to_string();
    transmit(&mut replacement, json!({"type":"command","id":mute,"issuedAt":chrono::Utc::now().timestamp_millis(),"method":"media.state","channelId":channel,"token":media_token,"body":{"muted":true,"deafened":false,"sequence":1}})).await;
    // Result and pushed state can arrive in either order; neither is discarded.
    let (mut accepted, mut pushed) = (false, false);
    tokio::time::timeout(Duration::from_secs(8), async {
        while !accepted || !pushed {
            let value: Value = serde_json::from_str(
                &replacement
                    .next()
                    .await
                    .unwrap()
                    .unwrap()
                    .into_text()
                    .unwrap(),
            )
            .unwrap();
            if value["type"] == "result" && value["id"] == mute {
                assert_eq!(value["status"], 204);
                accepted = true;
            }
            if value["type"] == "event" && value["id"] == "room" {
                assert_eq!(value["event"]["participants"][0]["muted"], true);
                pushed = true;
            }
        }
    })
    .await
    .unwrap();

    transmit(&mut replacement, json!({"type":"subscribe","kind":"presence","id":"denied","spaceId":space,"userIds":["NotAMember12"]})).await;
    assert_eq!(
        next(&mut replacement, "error", Some("denied")).await["status"],
        403
    );
    sqlx::query("DELETE FROM public.space_members WHERE space_id=$1 AND user_id=$2")
        .bind(space_id)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    transmit(
        &mut replacement,
        json!({"type":"subscribe","kind":"chat","id":"revoked","channelId":channel,"after":"0"}),
    )
    .await;
    let denied = next(&mut replacement, "error", Some("revoked")).await;
    assert!(matches!(denied["status"].as_u64(), Some(403 | 404)));
    drop(replacement);
    for server in servers {
        server.abort();
    }
    pool.close().await;
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}
