use super::*;
use crate::{Cloudflare, Config, gateway, spaces::channel_access};
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use std::{future::IntoFuture, sync::Arc, time::Duration};
use tokio::sync::Notify;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use tower::ServiceExt;
use uuid::Uuid;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    account: Option<&str>,
    sender: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(account) = account {
        request = request.header("authorization", format!("Bearer {account}"));
    }
    if let Some(sender) = sender {
        request = request.header("x-caper-chat-token", sender);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn two_person_privacy_durability_read_cursors_and_gateway_replay(pool: PgPool) {
    let mut users = Vec::new();
    for name in ["alice", "bob", "outsider"] {
        let user: i64 = sqlx::query_scalar(
            "INSERT INTO users (external_id,username,display_name) VALUES ($1,$2,$2) RETURNING id",
        )
        .bind(random_id(12))
        .bind(name)
        .fetch_one(&pool)
        .await
        .unwrap();
        let account = Sha256::digest(name.as_bytes()).to_vec();
        sqlx::query("INSERT INTO account_sessions (token_hash,user_id,expires_at) VALUES ($1,$2,now()+interval '1 day')")
            .bind(&account).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO chat_sessions (external_id,token_hash,user_id,account_session_hash,name) VALUES ($1,$2,$3,$4,$5)")
            .bind(random_id(12)).bind(Sha256::digest(format!("chat-{name}").as_bytes()).to_vec()).bind(user).bind(account).bind(name).execute(&pool).await.unwrap();
        users.push(user);
    }
    let chat = chat::Chat {
        pool: pool.clone(),
        broker: redis::Client::open(std::env::var("CHAT_TEST_VALKEY_URL").unwrap()).unwrap(),
        wake: Arc::new(Notify::new()),
        cdn: None,
    };
    let mut config = Config::test(false);
    config.auth_fixture = false;
    let mut state =
        AppState::with_database(config, Arc::new(Cloudflare::new()), Some(pool.clone()));
    state.chat = Some(chat.clone());
    let app = crate::app(state);
    assert_eq!(
        request(&app, "GET", "/api/dms", None, None, Value::Null)
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &app,
            "GET",
            "/api/push/config",
            Some("alice"),
            None,
            Value::Null
        )
        .await,
        (StatusCode::OK, json!({"platforms":[]})),
    );
    for method in ["POST", "DELETE"] {
        assert_eq!(
            request(
                &app,
                method,
                "/api/push/devices",
                Some("alice"),
                None,
                json!({"platform":"fcm","token":"test-device"}),
            )
            .await
            .0,
            StatusCode::NOT_FOUND,
            "device registration is deferred until direct provider integrations exist",
        );
    }
    assert!(create_conversation(&pool, users[0], "alice").await.is_err());
    assert!(
        create_conversation(&pool, users[0], "missing")
            .await
            .is_err()
    );
    let (one, other) = tokio::join!(
        create_conversation(&pool, users[0], " @BOB "),
        create_conversation(&pool, users[1], "alice")
    );
    let id = one.unwrap();
    assert_eq!(id, other.unwrap());
    assert_eq!(
        conversations(&pool, users[0]).await.unwrap()[0]["peer"]["username"],
        "bob"
    );
    assert_eq!(
        conversations(&pool, users[1]).await.unwrap()[0]["peer"]["username"],
        "alice"
    );
    assert!(conversations(&pool, users[2]).await.unwrap().is_empty());
    assert!(
        channel_access(&pool, &id, Some(users[0]))
            .await
            .unwrap()
            .space_id
            .is_none()
    );
    assert_eq!(
        channel_access(&pool, &id, Some(users[2]))
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM channel_joins")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "DM participants do not join space channels",
    );
    for (sender, expected) in [
        ("chat-alice", StatusCode::NO_CONTENT),
        ("chat-bob", StatusCode::NO_CONTENT),
        ("chat-outsider", StatusCode::NOT_FOUND),
    ] {
        assert_eq!(
            request(
                &app,
                "POST",
                &format!("/api/chat/channels/{id}/typing"),
                None,
                Some(sender),
                json!({"typing":true}),
            )
            .await
            .0,
            expected,
        );
    }
    let path = format!("/api/chat/channels/{id}/messages");
    assert_eq!(
        request(&app, "GET", &path, Some("outsider"), None, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "GET",
            &format!("/api/channels/{id}/media/status"),
            Some("alice"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let body = json!({"clientMessageId":Uuid::new_v4(),"text":"Only Alice and Bob 🙂"});
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            None,
            Some("chat-outsider"),
            body.clone()
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (first, retry) = tokio::join!(
        request(&app, "POST", &path, None, Some("chat-alice"), body.clone()),
        request(&app, "POST", &path, None, Some("chat-alice"), body.clone())
    );
    assert_eq!(first.0, StatusCode::OK);
    assert_eq!(first, retry);
    assert_eq!(first.1["seq"], "1");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM push_notifications")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "DM sends must not enqueue deferred push notifications",
    );
    let history = request(&app, "GET", &path, Some("bob"), None, Value::Null)
        .await
        .1;
    assert_eq!(history["channel"]["direct"], true);
    assert_eq!(history["channel"]["name"], "alice");
    assert_eq!(history["space"]["id"], "");
    assert_eq!(
        history["messages"][0]["content"]["text"],
        "Only Alice and Bob 🙂"
    );
    mark_read(&pool, users[1], &id, 999).await.unwrap();
    mark_read(&pool, users[1], &id, 0).await.unwrap();
    assert_eq!(
        conversations(&pool, users[1]).await.unwrap()[0]["readSeq"],
        "1"
    );
    assert!(mark_read(&pool, users[2], &id, 1).await.is_err());
    sqlx::query("ALTER TABLE channel_events ADD CONSTRAINT reject_dm CHECK (seq<2)")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            None,
            Some("chat-bob"),
            json!({"clientMessageId":Uuid::new_v4(),"text":"must roll back"})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        sqlx::query_as::<_, (i64, i64, i64)>(
            "SELECT (SELECT count(*) FROM messages),
                    (SELECT count(*) FROM channel_events),
                    (SELECT count(*) FROM push_notifications)",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        (1, 1, 0),
        "failed outbox inserts still roll back the message without queuing push",
    );
    sqlx::query("ALTER TABLE channel_events DROP CONSTRAINT reject_dm")
        .execute(&pool)
        .await
        .unwrap();

    let gateway = gateway::Gateway::new(chat);
    gateway.start();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server =
        tokio::spawn(axum::serve(listener, gateway::router(gateway.clone())).into_future());
    let socket_request = |account: &str, after: &str| {
        let mut request = format!("ws://{addr}/api/chat/events?channelId={id}&after={after}")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "cookie",
            format!("caper_session={account}").parse().unwrap(),
        );
        request
    };
    assert!(
        connect_async(socket_request("outsider", "0"))
            .await
            .is_err()
    );
    let (mut socket, _) = connect_async(socket_request("bob", "0")).await.unwrap();
    let mut replay = Vec::new();
    while replay.len() < 2 {
        let frame = tokio::time::timeout(Duration::from_secs(6), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match frame {
            Message::Text(text) => replay.push(serde_json::from_str::<Value>(&text).unwrap()),
            Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await.unwrap(),
            _ => {}
        }
    }
    assert_eq!(replay[0]["message"], first.1);
    assert_eq!(replay[1]["type"], "ready");
    sqlx::query("UPDATE account_sessions SET revoked_at=now() WHERE user_id=$1")
        .bind(users[0])
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(
            &app,
            "POST",
            &path,
            None,
            Some("chat-alice"),
            json!({"clientMessageId":Uuid::new_v4(),"text":"logged out"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    socket.close(None).await.unwrap();
    gateway.begin_shutdown();
    server.abort();
}
