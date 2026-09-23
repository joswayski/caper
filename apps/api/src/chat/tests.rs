use super::*;
use futures_util::{SinkExt, StreamExt};
use sqlx::{
    Connection, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{future::IntoFuture, str::FromStr};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
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

async fn wait_closed(socket: &mut Socket) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match socket.next().await {
                None | Some(Err(_)) => return,
                Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => return,
                Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(bytes))) => {
                    socket
                        .send(tokio_tungstenite::tungstenite::Message::Pong(bytes))
                        .await
                        .unwrap();
                }
                Some(Ok(frame)) => panic!("unexpected frame before close: {frame:?}"),
            }
        }
    })
    .await
    .expect("socket was not revoked");
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
    let seeded_name: String =
        sqlx::query_scalar("SELECT name FROM public.channels WHERE external_id=$1")
            .bind(&channel)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(seeded_name, "general");
    assert_eq!(
        history_page(&pool, &channel, None, None).await.unwrap()["channel"]["name"],
        "general"
    );
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
        history_page(&pool, &channel, None, None).await.unwrap()["cursor"],
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
    let history = history_page(&pool, &channel, Some(3), None).await.unwrap();
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
    for typing in [true, false] {
        assert_eq!(
            typing_command(&app, &channel, Some(token), json!({"typing":typing})).await,
            StatusCode::NO_CONTENT
        );
        let update = event(&mut typing_old).await;
        assert_eq!(event(&mut typing_new).await, update);
        assert_eq!(update["typing"], typing);
    }
    assert_eq!(
        typing_command(&app, &channel, Some(token), json!({"typing":true})).await,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(
        history_page(&pool, &channel, None, None).await.unwrap()["cursor"],
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
    let latest = history_page(&pool, &channel, None, None).await.unwrap();
    assert_eq!(latest["messages"].as_array().unwrap().len(), 50);
    assert_eq!(latest["messages"][0]["seq"], "4");
    assert_eq!(latest["cursor"], "53");
    assert_eq!(latest["hasMore"], true);
    let older = history_page(&pool, &channel, Some(4), None).await.unwrap();
    assert_eq!(older["messages"].as_array().unwrap().len(), 3);
    assert_eq!(older["hasMore"], false);
    // Existing non-demo channels must be denied, not just unknown identifiers.
    let owner: i64 = sqlx::query_scalar("INSERT INTO public.users (external_id,username,display_name) VALUES($1,'private_owner','Private Owner') RETURNING id")
    .bind(random_id(12)).fetch_one(&pool).await.unwrap();
    let private_space: i64 = sqlx::query_scalar(
        "INSERT INTO public.spaces (external_id,name,owner_id) VALUES ($1,'Private',$2) RETURNING id",
    )
    .bind(random_id(12)).bind(owner)
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
        channel_access(&pool, &private_channel, None)
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        typing_command(&app, &private_channel, Some(token), json!({"typing":true})).await,
        StatusCode::NOT_FOUND
    );
    // Existing deployments retain their original row/ID; mixed-version readers
    // can still use it while the new API exposes the canonical lowercase name.
    sqlx::query("UPDATE public.channels SET name='General' WHERE external_id=$1")
        .bind(&channel)
        .execute(&pool)
        .await
        .unwrap();
    seed(&pool).await.unwrap();
    let demo_channels: i64 = sqlx::query_scalar("SELECT count(*) FROM public.channels c JOIN public.spaces s ON s.id=c.space_id WHERE s.demo")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(demo_channels, 1);
    assert_eq!(
        history_page(&pool, &channel, None, None).await.unwrap()["channel"]["name"],
        "general"
    );
    assert_eq!(
        persist(&pool, &channel, token, id, "one").await.unwrap(),
        one
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

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn account_channels_isolate_sequences_and_gateway_revokes_live_access() {
    let url = std::env::var("CHAT_TEST_DATABASE_URL").expect("disposable test database required");
    let options = PgConnectOptions::from_str(&url).unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("chat_auth_test_{}", Uuid::new_v4().simple());
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

    let owner: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,'owner','Owner') RETURNING id")
        .bind(random_id(12)).fetch_one(&pool).await.unwrap();
    let member_external = random_id(12);
    let member: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,'member','Member') RETURNING id")
        .bind(&member_external).fetch_one(&pool).await.unwrap();
    let outsider: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,'outsider','Outsider') RETURNING id")
        .bind(random_id(12)).fetch_one(&pool).await.unwrap();
    let space = random_id(12);
    let space_id: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,'Account space',$2) RETURNING id")
        .bind(&space).bind(owner).fetch_one(&pool).await.unwrap();
    for user in [owner, member, outsider] {
        sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
            .bind(space_id)
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
    }
    let first = random_id(12);
    let second = random_id(12);
    let public = random_id(12);
    let first_id: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name,private) VALUES($1,$2,'first',true) RETURNING id")
        .bind(&first).bind(space_id).fetch_one(&pool).await.unwrap();
    let second_id: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name,private) VALUES($1,$2,'second',true) RETURNING id")
        .bind(&second).bind(space_id).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,'public')")
        .bind(&public)
        .bind(space_id)
        .execute(&pool)
        .await
        .unwrap();
    for channel in [first_id, second_id] {
        sqlx::query("INSERT INTO public.channel_members(channel_id,user_id) VALUES($1,$2)")
            .bind(channel)
            .bind(member)
            .execute(&pool)
            .await
            .unwrap();
    }

    let cookie = "member-account-cookie";
    let account_hash = Sha256::digest(cookie.as_bytes()).to_vec();
    sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
        .bind(&account_hash).bind(member).execute(&pool).await.unwrap();
    let chat_token = "member-chat-token";
    sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,user_id,name,account_session_hash) VALUES($1,$2,$3,'Member',$4)")
        .bind(random_id(12)).bind(Sha256::digest(chat_token.as_bytes()).as_slice()).bind(member).bind(&account_hash).execute(&pool).await.unwrap();
    let outsider_cookie = "outsider-account-cookie";
    let outsider_hash = Sha256::digest(outsider_cookie.as_bytes()).to_vec();
    sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
        .bind(&outsider_hash).bind(outsider).execute(&pool).await.unwrap();
    let outsider_chat = "outsider-chat-token";
    sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,user_id,name,account_session_hash) VALUES($1,$2,$3,'Outsider',$4)")
        .bind(random_id(12)).bind(Sha256::digest(outsider_chat.as_bytes()).as_slice()).bind(outsider).bind(&outsider_hash).execute(&pool).await.unwrap();
    let guest = "non-demo-guest";
    sqlx::query(
        "INSERT INTO public.chat_sessions(external_id,token_hash,name) VALUES($1,$2,'Guest')",
    )
    .bind(random_id(12))
    .bind(Sha256::digest(guest.as_bytes()).as_slice())
    .execute(&pool)
    .await
    .unwrap();

    let first_message = persist(&pool, &first, chat_token, Uuid::new_v4(), "first channel")
        .await
        .unwrap();
    let second_message = persist(&pool, &second, chat_token, Uuid::new_v4(), "second channel")
        .await
        .unwrap();
    assert_eq!(first_message["seq"], "1");
    assert_eq!(second_message["seq"], "1");
    assert_eq!(
        persist(&pool, &first, outsider_chat, Uuid::new_v4(), "denied")
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        persist(&pool, &public, guest, Uuid::new_v4(), "denied")
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let history = history_page(&pool, &first, None, Some(member))
        .await
        .unwrap();
    assert_eq!(history["space"], json!({"id":space,"name":"Account space"}));
    assert_eq!(history["channel"], json!({"id":first,"name":"first"}));
    assert_eq!(
        history_page(&pool, &first, None, None)
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );

    // A send waiting behind grant removal must authorize against a fresh
    // snapshot after it acquires the space lock, not the pre-wait snapshot.
    let mut removal = pool.begin().await.unwrap();
    let blocker: i32 =
        sqlx::query_scalar("SELECT pg_backend_pid() FROM public.spaces WHERE id=$1 FOR UPDATE")
            .bind(space_id)
            .fetch_one(&mut *removal)
            .await
            .unwrap();
    sqlx::query("DELETE FROM public.channel_members WHERE channel_id=$1 AND user_id=$2")
        .bind(first_id)
        .bind(member)
        .execute(&mut *removal)
        .await
        .unwrap();
    let sending_pool = pool.clone();
    let sending_channel = first.clone();
    let sending = tokio::spawn(async move {
        persist(
            &sending_pool,
            &sending_channel,
            chat_token,
            Uuid::new_v4(),
            "racing removal",
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)))")
                .bind(blocker).fetch_one(&pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.expect("send should wait behind membership mutation");
    removal.commit().await.unwrap();
    assert_eq!(
        sending.await.unwrap().unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    sqlx::query("INSERT INTO public.channel_members(channel_id,user_id) VALUES($1,$2)")
        .bind(first_id)
        .bind(member)
        .execute(&pool)
        .await
        .unwrap();

    let broker = redis::Client::open(std::env::var("CHAT_TEST_VALKEY_URL").unwrap()).unwrap();
    let chat = Chat {
        pool: pool.clone(),
        broker,
        wake: Arc::new(Notify::new()),
    };
    let gateway = crate::gateway::Gateway::new(chat);
    gateway.start();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(axum::serve(listener, crate::gateway::router(gateway)).into_future());
    let mut request = format!("ws://{address}/api/chat/events?channelId={first}&after=0")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("cookie", format!("caper_session={cookie}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    assert_eq!(event(&mut socket).await["message"], first_message);
    assert_eq!(
        event(&mut socket).await,
        json!({"type":"ready","cursor":"1"})
    );

    // Removing the private grant closes an already-established subscription on
    // the next periodic authorization tick, without requiring another event.
    sqlx::query("DELETE FROM public.channel_members WHERE channel_id=$1 AND user_id=$2")
        .bind(first_id)
        .bind(member)
        .execute(&pool)
        .await
        .unwrap();
    wait_closed(&mut socket).await;
    let mut denied = format!("ws://{address}/api/chat/events?channelId={first}&after=1")
        .into_client_request()
        .unwrap();
    denied
        .headers_mut()
        .insert("cookie", format!("caper_session={cookie}").parse().unwrap());
    assert!(tokio_tungstenite::connect_async(denied).await.is_err());

    // A privacy conversion similarly invalidates a member without a grant.
    let mut public_request = format!("ws://{address}/api/chat/events?channelId={public}&after=0")
        .into_client_request()
        .unwrap();
    public_request.headers_mut().insert(
        "cookie",
        format!("caper_session={outsider_cookie}").parse().unwrap(),
    );
    let (mut public_socket, _) = tokio_tungstenite::connect_async(public_request)
        .await
        .unwrap();
    assert_eq!(event(&mut public_socket).await["cursor"], "0");
    sqlx::query("UPDATE public.channels SET private=true WHERE external_id=$1")
        .bind(&public)
        .execute(&pool)
        .await
        .unwrap();
    wait_closed(&mut public_socket).await;

    server.abort();
    pool.close().await;
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}
