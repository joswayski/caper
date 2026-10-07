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
    let (notes, reopened) = tokio::join!(
        create_conversation(&pool, users[0], "alice"),
        create_conversation(&pool, users[0], " @ALICE ")
    );
    let notes = notes.unwrap();
    assert_eq!(
        notes,
        reopened.unwrap(),
        "concurrent self creation is canonical"
    );
    let listed = conversations(&pool, users[0]).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["peer"]["username"], "alice");
    let notes_path = format!("/api/chat/channels/{notes}/messages");
    let note = request(
        &app,
        "POST",
        &notes_path,
        None,
        Some("chat-alice"),
        json!({"clientMessageId":Uuid::new_v4(),"text":"A private reminder to myself"}),
    )
    .await;
    assert_eq!(note.0, StatusCode::OK);
    let history = request(&app, "GET", &notes_path, Some("alice"), None, Value::Null).await;
    assert_eq!(history.0, StatusCode::OK);
    assert_eq!(history.1["channel"]["direct"], true);
    assert_eq!(history.1["channel"]["name"], "alice");
    assert_eq!(
        history.1["messages"][0]["content"]["text"],
        "A private reminder to myself"
    );
    for other in ["bob", "outsider"] {
        assert_eq!(
            request(&app, "GET", &notes_path, Some(other), None, Value::Null)
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            request(
                &app,
                "POST",
                &notes_path,
                None,
                Some(&format!("chat-{other}")),
                json!({"clientMessageId":Uuid::new_v4(),"text":"not my notes"})
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM push_notifications")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "notes must not send notifications to their author"
    );
    mark_read(&pool, users[0], &notes, 99).await.unwrap();
    mark_read(&pool, users[0], &notes, 0).await.unwrap();
    assert_eq!(
        conversations(&pool, users[0]).await.unwrap()[0]["readSeq"],
        "1"
    );
    assert!(mark_read(&pool, users[1], &notes, 1).await.is_err());
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
        (2, 2, 0),
        "only the earlier note and peer message remain after a failed outbox insert; no push is queued",
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
    let notes_request = |account: &str| {
        let mut request = format!("ws://{addr}/api/chat/events?channelId={notes}&after=0")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "cookie",
            format!("caper_session={account}").parse().unwrap(),
        );
        request
    };
    assert!(connect_async(notes_request("bob")).await.is_err());
    let (mut notes_socket, _) = connect_async(notes_request("alice")).await.unwrap();
    let mut notes_replay = Vec::new();
    while notes_replay.len() < 2 {
        match tokio::time::timeout(Duration::from_secs(6), notes_socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Text(text) => notes_replay.push(serde_json::from_str::<Value>(&text).unwrap()),
            Message::Ping(bytes) => notes_socket.send(Message::Pong(bytes)).await.unwrap(),
            _ => {}
        }
    }
    assert_eq!(notes_replay[0]["message"], note.1);
    assert_eq!(notes_replay[1]["type"], "ready");
    notes_socket.close(None).await.unwrap();
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn message_requests_blocks_and_privacy(pool: PgPool) {
    let mut users = std::collections::HashMap::new();
    let mut ids = std::collections::HashMap::new();
    let mut names: Vec<String> = ["alice", "bob", "carol", "dave", "eve", "frank"]
        .map(String::from)
        .to_vec();
    names.extend((0..13).map(|n| format!("target{n}")));
    for name in &names {
        let external = random_id(12);
        let user: i64 = sqlx::query_scalar(
            "INSERT INTO users (external_id,username,display_name) VALUES ($1,$2,$2) RETURNING id",
        )
        .bind(&external)
        .bind(name)
        .fetch_one(&pool)
        .await
        .unwrap();
        let account = Sha256::digest(name.as_bytes()).to_vec();
        sqlx::query("INSERT INTO account_sessions (token_hash,user_id,expires_at) VALUES ($1,$2,now()+interval '1 day')")
            .bind(&account).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO chat_sessions (external_id,token_hash,user_id,account_session_hash,name) VALUES ($1,$2,$3,$4,$5)")
            .bind(random_id(12)).bind(Sha256::digest(format!("chat-{name}").as_bytes()).to_vec()).bind(user).bind(account).bind(name).execute(&pool).await.unwrap();
        users.insert(name.clone(), user);
        ids.insert(name.clone(), external);
    }
    let space: i64 = sqlx::query_scalar(
        "INSERT INTO spaces (external_id,name,owner_id) VALUES ('shared','Shared',$1) RETURNING id",
    )
    .bind(users["alice"])
    .fetch_one(&pool)
    .await
    .unwrap();
    for name in ["alice", "carol"] {
        sqlx::query("INSERT INTO space_members (space_id,user_id) VALUES ($1,$2)")
            .bind(space)
            .bind(users[name])
            .execute(&pool)
            .await
            .unwrap();
    }
    let mut config = Config::test(false);
    config.auth_fixture = false;
    let mut state =
        AppState::with_database(config, Arc::new(Cloudflare::new()), Some(pool.clone()));
    state.chat = Some(chat::Chat {
        pool: pool.clone(),
        broker: redis::Client::open(std::env::var("CHAT_TEST_VALKEY_URL").unwrap()).unwrap(),
        wake: Arc::new(Notify::new()),
    });
    let app = crate::app(state);
    let open = |from: &'static str, to: &'static str| {
        let app = app.clone();
        async move {
            request(
                &app,
                "POST",
                "/api/dms",
                Some(from),
                None,
                json!({ "username": to }),
            )
            .await
        }
    };
    let send = |from: &'static str, conversation: String| {
        let app = app.clone();
        async move {
            request(
                &app,
                "POST",
                &format!("/api/chat/channels/{conversation}/messages"),
                None,
                Some(&format!("chat-{from}")),
                json!({"clientMessageId":Uuid::new_v4(),"text":format!("hi from {from}")}),
            )
            .await
        }
    };
    let status = |name: &'static str, conversation: String| {
        let pool = pool.clone();
        let user = users[name];
        async move {
            conversations(&pool, user)
                .await
                .unwrap()
                .into_iter()
                .find(|c| c["id"] == conversation.as_str())
                .map(|c| c["status"].as_str().unwrap().to_owned())
        }
    };

    // Privacy defaults to requests from anyone and only takes the three values.
    assert_eq!(
        request(
            &app,
            "GET",
            "/api/account/privacy",
            Some("bob"),
            None,
            Value::Null
        )
        .await,
        (StatusCode::OK, json!({"directMessages":"anyone"}))
    );
    for body in [
        json!({"directMessages":"friends"}),
        json!({"directMessages":"anyone","extra":1}),
    ] {
        assert!(
            request(&app, "PUT", "/api/account/privacy", Some("bob"), None, body)
                .await
                .0
                .is_client_error()
        );
    }

    // People who share a space skip the request.
    let (code, shared) = open("alice", "carol").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(shared["status"], "accepted");
    assert!(shared["peer"]["avatarId"].is_number());
    let shared = shared["id"].as_str().unwrap().to_owned();
    assert_eq!(
        status("carol", shared.clone()).await.as_deref(),
        Some("accepted")
    );

    // A stranger's DM is a request until the recipient answers it. The sender
    // can keep writing; a decline hides it from the recipient only.
    let (code, request_dm) = open("alice", "bob").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(request_dm["status"], "outgoing");
    let request_dm = request_dm["id"].as_str().unwrap().to_owned();
    assert_eq!(
        status("bob", request_dm.clone()).await.as_deref(),
        Some("incoming")
    );
    assert_eq!(send("alice", request_dm.clone()).await.0, StatusCode::OK);
    assert_eq!(
        status("bob", request_dm.clone()).await.as_deref(),
        Some("incoming")
    );
    let decline_path = format!("/api/dms/{request_dm}/decline");
    assert_eq!(
        request(
            &app,
            "POST",
            &decline_path,
            Some("alice"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "only the recipient answers a request"
    );
    for _ in 0..2 {
        assert_eq!(
            request(&app, "POST", &decline_path, Some("bob"), None, Value::Null)
                .await
                .0,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(status("bob", request_dm.clone()).await, None);
    assert_eq!(
        status("alice", request_dm.clone()).await.as_deref(),
        Some("outgoing")
    );
    assert_eq!(send("alice", request_dm.clone()).await.0, StatusCode::OK);
    assert_eq!(
        status("bob", request_dm.clone()).await,
        None,
        "new messages don't resurface it"
    );
    // Choosing to message the sender accepts it.
    let (code, reopened) = open("bob", "alice").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(
        (reopened["id"].as_str(), reopened["status"].as_str()),
        (Some(request_dm.as_str()), Some("accepted"))
    );
    assert_eq!(
        status("alice", request_dm.clone()).await.as_deref(),
        Some("accepted")
    );
    assert_eq!(
        request(&app, "POST", &decline_path, Some("bob"), None, Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND,
        "an accepted conversation can't be declined"
    );

    // Accept explicitly, or by replying.
    let (_, accepted) = open("frank", "target0").await;
    let accepted = accepted["id"].as_str().unwrap().to_owned();
    let (code, body) = request(
        &app,
        "POST",
        &format!("/api/dms/{accepted}/accept"),
        Some("target0"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        (code, body["status"].as_str()),
        (StatusCode::OK, Some("accepted"))
    );
    assert_eq!(
        status("frank", accepted.clone()).await.as_deref(),
        Some("accepted")
    );
    assert_eq!(
        request(
            &app,
            "POST",
            &format!("/api/dms/{accepted}/accept"),
            Some("dave"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (_, replied) = open("frank", "target1").await;
    let replied = replied["id"].as_str().unwrap().to_owned();
    assert_eq!(send("target1", replied.clone()).await.0, StatusCode::OK);
    assert_eq!(
        status("frank", replied.clone()).await.as_deref(),
        Some("accepted")
    );
    // Ten new requests an hour; accepted ones don't count.
    for n in 2..12 {
        let target: &'static str = Box::leak(format!("target{n}").into_boxed_str());
        assert_eq!(open("frank", target).await.0, StatusCode::OK, "{target}");
    }
    let (code, body) = open("frank", "target12").await;
    assert_eq!(code, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error"], "too many message requests; try again later");

    // Privacy: spaces only, then nobody new. Existing conversations stay.
    assert_eq!(
        request(
            &app,
            "PUT",
            "/api/account/privacy",
            Some("dave"),
            None,
            json!({"directMessages":"spaces"})
        )
        .await,
        (StatusCode::OK, json!({"directMessages":"spaces"}))
    );
    let (code, body) = open("alice", "dave").await;
    assert_eq!(
        (code, body["code"].as_str()),
        (StatusCode::FORBIDDEN, Some("dm_not_accepted"))
    );
    sqlx::query("INSERT INTO space_members (space_id,user_id) VALUES ($1,$2)")
        .bind(space)
        .bind(users["dave"])
        .execute(&pool)
        .await
        .unwrap();
    let (code, body) = open("alice", "dave").await;
    assert_eq!(
        (code, body["status"].as_str()),
        (StatusCode::OK, Some("accepted"))
    );
    let alice_dave = body["id"].as_str().unwrap().to_owned();
    request(
        &app,
        "PUT",
        "/api/account/privacy",
        Some("dave"),
        None,
        json!({"directMessages":"nobody"}),
    )
    .await;
    assert_eq!(open("carol", "dave").await.1["code"], "dm_not_accepted");
    assert_eq!(open("alice", "dave").await.1["id"], alice_dave.as_str());
    assert_eq!(send("alice", alice_dave).await.0, StatusCode::OK);

    // Blocks are idempotent, listed, stop both sides sending and survive reopening.
    let alice = ids["alice"].clone();
    for _ in 0..2 {
        assert_eq!(
            request(
                &app,
                "PUT",
                &format!("/api/blocks/{alice}"),
                Some("bob"),
                None,
                Value::Null
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM user_blocks")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let (_, listed) = request(&app, "GET", "/api/blocks", Some("bob"), None, Value::Null).await;
    assert_eq!(listed["blocks"][0]["id"], alice.as_str());
    assert_eq!(listed["blocks"][0]["username"], "alice");
    let bob_view = conversations(&pool, users["bob"]).await.unwrap();
    assert_eq!(
        bob_view
            .iter()
            .find(|c| c["id"] == request_dm.as_str())
            .unwrap()["blocked"],
        true
    );
    assert_eq!(
        conversations(&pool, users["alice"])
            .await
            .unwrap()
            .iter()
            .find(|c| c["id"] == request_dm.as_str())
            .unwrap()["blocked"],
        false,
        "the blocked person is not told"
    );
    assert_eq!(
        send("alice", request_dm.clone()).await.1["code"],
        "dm_not_accepted"
    );
    assert_eq!(
        send("bob", request_dm.clone()).await.1["code"],
        "dm_blocked"
    );
    assert_eq!(open("alice", "bob").await.1["id"], request_dm.as_str());
    assert_eq!(
        request(
            &app,
            "PUT",
            &format!("/api/blocks/{}", ids["bob"]),
            Some("bob"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &app,
            "PUT",
            "/api/blocks/missing",
            Some("bob"),
            None,
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    for _ in 0..2 {
        assert_eq!(
            request(
                &app,
                "DELETE",
                &format!("/api/blocks/{alice}"),
                Some("bob"),
                None,
                Value::Null
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        request(&app, "GET", "/api/blocks", Some("bob"), None, Value::Null)
            .await
            .1,
        json!({"blocks":[]})
    );
    assert_eq!(send("alice", request_dm.clone()).await.0, StatusCode::OK);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM user_blocks WHERE deleted_at IS NOT NULL"
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        1,
        "unblocking keeps the record"
    );

    // Blocking a requester declines the request; a block stops new requests both ways.
    let (_, from_carol) = open("carol", "bob").await;
    let from_carol = from_carol["id"].as_str().unwrap().to_owned();
    assert_eq!(
        status("bob", from_carol.clone()).await.as_deref(),
        Some("incoming")
    );
    request(
        &app,
        "PUT",
        &format!("/api/blocks/{}", ids["carol"]),
        Some("bob"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status("bob", from_carol.clone()).await, None);
    assert_eq!(
        status("carol", from_carol.clone()).await.as_deref(),
        Some("outgoing")
    );
    request(
        &app,
        "DELETE",
        &format!("/api/blocks/{}", ids["carol"]),
        Some("bob"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(
        status("bob", from_carol).await,
        None,
        "unblocking doesn't restore the request"
    );
    request(
        &app,
        "PUT",
        &format!("/api/blocks/{}", ids["eve"]),
        Some("dave"),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(open("eve", "dave").await.1["code"], "dm_not_accepted");
    request(
        &app,
        "PUT",
        "/api/account/privacy",
        Some("dave"),
        None,
        json!({"directMessages":"anyone"}),
    )
    .await;
    assert_eq!(open("eve", "dave").await.1["code"], "dm_not_accepted");
    assert_eq!(open("dave", "eve").await.1["code"], "dm_blocked");
}
