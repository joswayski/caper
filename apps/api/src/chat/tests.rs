use super::*;
use futures_util::{SinkExt, StreamExt};
use sqlx::{
    Connection, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{future::IntoFuture, str::FromStr};
use tower::ServiceExt;

#[test]
fn text_limits_count_unicode_and_preserve_literal_text() {
    let text = " BO2 <script>alert(1)</script>\n🙂\t";
    assert_eq!(prepare_text(text).unwrap()["text"], text);
    assert!(prepare_text(&"🙂".repeat(4000)).is_ok());
    assert!(prepare_text(&"🙂".repeat(4001)).is_err());
    assert!(prepare_text(" \n\t").is_err());
    assert!(prepare_text("a\0b").is_err());
}

#[test]
fn cursors_do_not_round_at_javascript_integer_limit() {
    assert_eq!(cursor("9007199254740993").unwrap(), 9_007_199_254_740_993);
    assert!(cursor("-1").is_err());
    assert!(cursor("9223372036854775808").is_err());
    assert!(cursor("1.5").is_err());
}

#[test]
fn external_ids_match_existing_alphabet_and_lengths() {
    for length in [12, 15] {
        let id = random_id(length);
        assert_eq!(id.len(), length);
        assert!(id.bytes().all(|v| v.is_ascii_alphanumeric()));
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn event(socket: &mut Socket) -> Value {
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                tokio_tungstenite::tungstenite::Message::Text(text) => {
                    return serde_json::from_str(&text).unwrap();
                }
                tokio_tungstenite::tungstenite::Message::Ping(bytes) => {
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Pong(bytes))
                        .await
                        .unwrap();
                }
                frame => panic!("unexpected frame {frame:?}"),
            }
        }
    })
    .await
    .expect("event timeout")
}

async fn typing_command(
    app: &Router,
    channel: &str,
    token: Option<&str>,
    body: Value,
) -> StatusCode {
    let mut request = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/api/chat/channels/{channel}/typing"))
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("x-caper-chat-token", token);
    }
    app.clone()
        .oneshot(
            request
                .body(axum::body::Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn durable_guest_delivery_replay_and_handoff() {
    let url = std::env::var("CHAT_TEST_DATABASE_URL").expect("disposable test database required");
    let options = PgConnectOptions::from_str(&url).unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("chat_test_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE DATABASE {database}").as_str())
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options.database(&database))
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    seed(&pool).await.unwrap();
    seed(&pool).await.unwrap();
    let channel: String = sqlx::query_scalar("SELECT external_id FROM public.channels")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(channel.len(), 12);
    let token = "test-guest-capability";
    sqlx::query("INSERT INTO public.chat_sessions (external_id, token_hash, name) VALUES ($1,$2,'Guest One')").bind(random_id(12)).bind(Sha256::digest(token.as_bytes()).as_slice()).execute(&pool).await.unwrap();
    assert_eq!(
        persist(&pool, &channel, "wrong", Uuid::new_v4(), "no")
            .await
            .unwrap_err()
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        persist(&pool, "unknown", token, Uuid::new_v4(), "no")
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let id = Uuid::new_v4();
    let (one, retry) = tokio::join!(
        persist(&pool, &channel, token, id, "one"),
        persist(&pool, &channel, token, id, "one")
    );
    let one = one.unwrap();
    assert_eq!(one, retry.unwrap());
    assert_eq!(one["seq"], "1");
    assert_eq!(one["id"].as_str().unwrap().len(), 15);
    assert_eq!(
        persist(&pool, &channel, token, id, "changed")
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    // An outbox failure must roll back both the message and sequence.
    pool.execute("ALTER TABLE public.channel_events ADD CONSTRAINT reject_next CHECK (seq < 2)")
        .await
        .unwrap();
    assert!(
        persist(&pool, &channel, token, Uuid::new_v4(), "rollback")
            .await
            .is_err()
    );
    pool.execute("ALTER TABLE public.channel_events DROP CONSTRAINT reject_next")
        .await
        .unwrap();
    assert_eq!(
        history_page(&pool, &channel, None).await.unwrap()["cursor"],
        "1"
    );
    let (two, three) = tokio::join!(
        persist(&pool, &channel, token, Uuid::new_v4(), "two"),
        persist(&pool, &channel, token, Uuid::new_v4(), "three")
    );
    let mut positions = [
        two.unwrap()["seq"].as_str().unwrap().to_owned(),
        three.unwrap()["seq"].as_str().unwrap().to_owned(),
    ];
    positions.sort();
    assert_eq!(positions, ["2", "3"]);
    let history = history_page(&pool, &channel, Some(3)).await.unwrap();
    assert_eq!(history["messages"].as_array().unwrap().len(), 2);
    assert_eq!(history["messages"][0]["seq"], "1");
    assert_eq!(history["messages"][1]["seq"], "2");

    let broker_url = std::env::var("CHAT_TEST_VALKEY_URL").expect("disposable broker required");
    let broker = redis::Client::open(broker_url).unwrap();
    let chat = Chat {
        pool: pool.clone(),
        broker,
        wake: Arc::new(Notify::new()),
    };
    let mut state = AppState::new(
        crate::Config::test(false),
        Arc::new(crate::Cloudflare::new()),
    );
    state.chat = Some(chat.clone());
    let app = crate::app(state);
    assert_eq!(
        typing_command(&app, &channel, None, json!({"typing":true})).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        typing_command(&app, &channel, Some("wrong"), json!({"typing":true})).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        typing_command(
            &app,
            &channel,
            Some(token),
            json!({"typing":true,"text":"never accept drafts"})
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    // Simulate broker outage: durable send already succeeded; outbox remains pending.
    let down = Chat {
        broker: redis::Client::open("redis://127.0.0.1:1").unwrap(),
        ..chat.clone()
    };
    assert!(publish_pending(&down).await.is_err());
    let pending: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.channel_events WHERE published_at IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(pending, 3);
    let old = crate::gateway::Gateway::new(chat.clone());
    let replacement = crate::gateway::Gateway::new(chat.clone());
    old.start();
    replacement.start();
    let old_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let new_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let old_address = old_listener.local_addr().unwrap();
    let new_address = new_listener.local_addr().unwrap();
    let old_server =
        tokio::spawn(axum::serve(old_listener, crate::gateway::router(old.clone())).into_future());
    let new_server = tokio::spawn(
        axum::serve(new_listener, crate::gateway::router(replacement.clone())).into_future(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(format!(
        "ws://{old_address}/api/chat/events?channelId={channel}&after=0"
    ))
    .await
    .unwrap();
    for seq in ["1", "2", "3"] {
        assert_eq!(event(&mut socket).await["seq"], seq);
    }
    assert_eq!(
        event(&mut socket).await,
        json!({"type":"ready","cursor":"3"})
    );
    // Real broker fanout reaches both gateways, without persisting presence or
    // sending new event types to clients that did not opt in.
    tokio::time::timeout(Duration::from_secs(5), async {
        for address in [old_address, new_address] {
            while reqwest::get(format!("http://{address}/readyz"))
                .await
                .unwrap()
                .status()
                != StatusCode::NO_CONTENT
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    })
    .await
    .unwrap();
    let (mut typing_old, _) = tokio_tungstenite::connect_async(format!(
        "ws://{old_address}/api/chat/events?channelId={channel}&after=3&typing=true"
    ))
    .await
    .unwrap();
    let (mut typing_new, _) = tokio_tungstenite::connect_async(format!(
        "ws://{new_address}/api/chat/events?channelId={channel}&after=3&typing=true"
    ))
    .await
    .unwrap();
    assert_eq!(event(&mut typing_old).await["cursor"], "3");
    assert_eq!(event(&mut typing_new).await["cursor"], "3");
    assert_eq!(
        typing_command(&app, &channel, Some(token), json!({"typing":true})).await,
        StatusCode::NO_CONTENT
    );
    let started = event(&mut typing_old).await;
    assert_eq!(event(&mut typing_new).await, started);
    assert_eq!(started["type"], "typing.updated");
    assert_eq!(started["typing"], true);
    assert_eq!(started["author"], one["author"]);
    assert!(started.get("seq").is_none());
    assert_eq!(
        typing_command(&app, &channel, Some(token), json!({"typing":false})).await,
        StatusCode::NO_CONTENT
    );
    let stopped = event(&mut typing_new).await;
    assert_eq!(event(&mut typing_old).await, stopped);
    assert_eq!(stopped["typing"], false);
    assert!(
        cursor(stopped["revision"].as_str().unwrap()).unwrap()
            > cursor(started["revision"].as_str().unwrap()).unwrap()
    );
    assert_eq!(
        typing_command(&app, &channel, Some(token), json!({"typing":true})).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        history_page(&pool, &channel, None).await.unwrap()["cursor"],
        "3"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM public.channel_events")
            .fetch_one(&pool)
            .await
            .unwrap(),
        3
    );
    typing_old.close(None).await.unwrap();
    typing_new.close(None).await.unwrap();
    // Mark-published state may be lost after broker delivery. Replay/republication
    // must not duplicate already sent events on an existing socket.
    assert!(publish_pending(&chat).await.unwrap());
    pool.execute("UPDATE public.channel_events SET published_at = NULL")
        .await
        .unwrap();
    assert!(publish_pending(&chat).await.unwrap());
    let fourth = persist(
        &pool,
        &channel,
        token,
        Uuid::new_v4(),
        "four, without publisher wakeup",
    )
    .await
    .unwrap();
    // No publish call: periodic authoritative head repair must deliver the final
    // missed message even without any later message to expose the gap.
    assert_eq!(event(&mut socket).await["message"], fourth);
    assert_eq!(event(&mut socket).await["type"], "ready");
    old.begin_shutdown();
    assert_eq!(event(&mut socket).await["type"], "migrating");
    let (mut next, _) = tokio_tungstenite::connect_async(format!(
        "ws://{new_address}/api/chat/events?channelId={channel}&after=3"
    ))
    .await
    .unwrap();
    assert_eq!(event(&mut next).await["seq"], "4");
    assert_eq!(event(&mut next).await["cursor"], "4");
    let fifth = persist(&pool, &channel, token, Uuid::new_v4(), "during overlap")
        .await
        .unwrap();
    publish_pending(&chat).await.unwrap();
    assert_eq!(event(&mut socket).await["message"], fifth);
    assert_eq!(event(&mut next).await["message"], fifth);
    assert!(
        tokio_tungstenite::connect_async(format!(
            "ws://{old_address}/api/chat/events?channelId={channel}&after=5"
        ))
        .await
        .is_err()
    );
    socket.close(None).await.unwrap();
    // Explicitly reject an impossible cursor; never pretend missing history was read.
    let (mut invalid, _) = tokio_tungstenite::connect_async(format!(
        "ws://{new_address}/api/chat/events?channelId={channel}&after=999"
    ))
    .await
    .unwrap();
    assert_eq!(event(&mut invalid).await["type"], "resync_required");
    next.close(None).await.unwrap();
    old_server.abort();
    new_server.abort();

    // Renewing a guest credential must not turn a retry into a second message.
    let other_token = "another-test-guest";
    sqlx::query("INSERT INTO public.chat_sessions (external_id, token_hash, name) VALUES ($1,$2,'Guest Two')")
        .bind(random_id(12)).bind(Sha256::digest(other_token.as_bytes()).as_slice()).execute(&pool).await.unwrap();
    assert_eq!(
        persist(&pool, &channel, other_token, id, "one")
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    // A signed-in chat capability cannot outlive logout of its parent session.
    let user: i64 = sqlx::query_scalar("INSERT INTO public.users (external_id, display_name, username) VALUES ($1,'Account Name','chat_test') RETURNING id")
        .bind(random_id(12)).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.account_sessions (token_hash,user_id,expires_at) VALUES ($1,$2,now()+interval '1 day')")
        .bind(b"parent-session".as_slice()).bind(user).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.chat_sessions (external_id, token_hash, name, user_id, account_session_hash) VALUES ($1,$2,'Not authoritative',$3,$4)")
        .bind(random_id(12)).bind(Sha256::digest(b"account-chat").as_slice()).bind(user).bind(b"parent-session".as_slice()).execute(&pool).await.unwrap();
    let account_message = persist(&pool, &channel, "account-chat", Uuid::new_v4(), "signed in")
        .await
        .unwrap();
    assert_eq!(account_message["author"]["name"], "Account Name");
    assert_eq!(account_message["author"]["isGuest"], false);
    assert_eq!(
        typing_command(&app, &channel, Some("account-chat"), json!({"typing":true})).await,
        StatusCode::NO_CONTENT
    );
    sqlx::query("UPDATE public.account_sessions SET revoked_at = now() WHERE user_id = $1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        persist(
            &pool,
            &channel,
            "account-chat",
            Uuid::new_v4(),
            "after logout"
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        typing_command(&app, &channel, Some("account-chat"), json!({"typing":true})).await,
        StatusCode::UNAUTHORIZED
    );
    // Boundary: 30 new sends/minute per guest; an already committed retry still
    // succeeds at the limit and does not consume another rate-limit slot.
    for index in 5..30 {
        persist(
            &pool,
            &channel,
            token,
            Uuid::new_v4(),
            &format!("rate {index}"),
        )
        .await
        .unwrap();
    }
    assert_eq!(
        persist(&pool, &channel, token, Uuid::new_v4(), "over limit")
            .await
            .unwrap_err()
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        persist(&pool, &channel, token, id, "one").await.unwrap(),
        one
    );
    for index in 0..22 {
        persist(
            &pool,
            &channel,
            other_token,
            Uuid::new_v4(),
            &format!("history {index}"),
        )
        .await
        .unwrap();
    }
    let latest = history_page(&pool, &channel, None).await.unwrap();
    assert_eq!(latest["messages"].as_array().unwrap().len(), 50);
    assert_eq!(latest["messages"][0]["seq"], "4");
    assert_eq!(latest["cursor"], "53");
    assert_eq!(latest["hasMore"], true);
    let older = history_page(&pool, &channel, Some(4)).await.unwrap();
    assert_eq!(older["messages"].as_array().unwrap().len(), 3);
    assert_eq!(older["hasMore"], false);
    // Existing non-demo channels must be denied, not just unknown identifiers.
    let private_space: i64 = sqlx::query_scalar(
        "INSERT INTO public.spaces (external_id,name) VALUES ($1,'Private') RETURNING id",
    )
    .bind(random_id(12))
    .fetch_one(&pool)
    .await
    .unwrap();
    let private_channel = random_id(12);
    sqlx::query("INSERT INTO public.channels (external_id,space_id,name) VALUES ($1,$2,'General')")
        .bind(&private_channel)
        .bind(private_space)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        public_channel(&pool, &private_channel)
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        typing_command(&app, &private_channel, Some(token), json!({"typing":true})).await,
        StatusCode::NOT_FOUND
    );
    sqlx::query("UPDATE public.chat_sessions SET expires_at = now() - interval '1 second' WHERE token_hash = $1")
        .bind(Sha256::digest(token.as_bytes()).as_slice()).execute(&pool).await.unwrap();
    assert_eq!(
        typing_command(&app, &channel, Some(token), json!({"typing":true})).await,
        StatusCode::UNAUTHORIZED
    );
    pool.close().await;
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}
