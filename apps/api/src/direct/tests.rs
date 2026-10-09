use super::*;
use crate::{Cloudflare, Config, gateway, spaces::channel_access};
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use futures_util::{SinkExt, StreamExt};
use sha2::{Digest, Sha256};
use std::{future::IntoFuture, sync::Arc, time::Duration};
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
    let chat = chat::Chat::new(
        pool.clone(),
        redis::Client::open(std::env::var("CHAT_TEST_VALKEY_URL").unwrap()).unwrap(),
    );
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
    // With notifications off no platform is advertised, so registration is
    // refused; unregistering stays idempotent.
    for (method, expected) in [
        ("POST", StatusCode::BAD_REQUEST),
        ("DELETE", StatusCode::NO_CONTENT),
    ] {
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
            expected,
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
    // Account messages enqueue one notification job each; expansion (see
    // push::tests) decides that notes notify nobody.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM notification_jobs")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1,
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
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM notification_jobs")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2,
        "a retried send enqueues its notification job once",
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
                    (SELECT count(*) FROM notification_jobs)",
        )
        .fetch_one(&pool)
        .await
        .unwrap(),
        (2, 2, 2),
        "only the earlier note and peer message remain after a failed outbox insert, with one notification job each",
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
    state.chat = Some(chat::Chat::new(
        pool.clone(),
        redis::Client::open(std::env::var("CHAT_TEST_VALKEY_URL").unwrap()).unwrap(),
    ));
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

    // Recipients identify senders by @username, so a sender needs a profile.
    let incomplete: i64 =
        sqlx::query_scalar("INSERT INTO users (external_id) VALUES ($1) RETURNING id")
            .bind(random_id(12))
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO account_sessions (token_hash,user_id,expires_at) VALUES ($1,$2,now()+interval '1 day')")
        .bind(Sha256::digest(b"incomplete").to_vec()).bind(incomplete).execute(&pool).await.unwrap();
    assert_eq!(
        open("incomplete", "bob").await,
        (
            StatusCode::FORBIDDEN,
            json!({"error":"complete profile required"})
        )
    );
    // Chat capabilities also need a profile and always carry its name, so
    // nobody can write under a name of their choosing.
    let chat_session = |account: &'static str| {
        let app = app.clone();
        async move {
            request(
                &app,
                "POST",
                "/api/chat/session",
                Some(account),
                None,
                json!({"name":"Caper Support"}),
            )
            .await
        }
    };
    assert_eq!(
        chat_session("incomplete").await,
        (
            StatusCode::FORBIDDEN,
            json!({"error":"complete profile required"})
        )
    );
    sqlx::query("INSERT INTO chat_sessions (external_id,token_hash,user_id,account_session_hash,name) VALUES ($1,$2,$3,$4,'Caper Support')")
        .bind(random_id(12)).bind(Sha256::digest(b"chat-incomplete").to_vec()).bind(incomplete).bind(Sha256::digest(b"incomplete").to_vec()).execute(&pool).await.unwrap();
    // The submitted name is ignored: the author is always the profile name.
    let (code, session) = chat_session("eve").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(session["author"]["name"], "eve");

    // Requests from strangers don't use up the recipient's own DM quota.
    for n in 0..20 {
        let sender: i64 = sqlx::query_scalar(
            "INSERT INTO users (external_id,username,display_name) VALUES ($1,$2,$2) RETURNING id",
        )
        .bind(random_id(12))
        .bind(format!("stranger{n}"))
        .fetch_one(&pool)
        .await
        .unwrap();
        let channel: i64 = sqlx::query_scalar("INSERT INTO channels (external_id,name,private) VALUES ($1,'direct',true) RETURNING id")
            .bind(random_id(12)).fetch_one(&pool).await.unwrap();
        let target = users["target0"];
        sqlx::query("INSERT INTO direct_conversations (channel_id,low_user_id,high_user_id,requested_by,accepted_at) VALUES ($1,$2,$3,$4,NULL)")
            .bind(channel).bind(sender.min(target)).bind(sender.max(target)).bind(sender).execute(&pool).await.unwrap();
    }
    assert_eq!(open("target0", "carol").await.0, StatusCode::OK);

    // People who share a space skip the request.
    let (code, shared) = open("alice", "carol").await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(shared["status"], "accepted");
    assert!(shared["peer"]["avatarId"].is_number());
    let shared = shared["id"].as_str().unwrap().to_owned();
    assert_eq!(
        send("incomplete", shared.clone()).await.0,
        StatusCode::UNAUTHORIZED
    );
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
    let destinations = |name: &'static str| {
        let app = app.clone();
        async move {
            let (_, body) = request(
                &app,
                "GET",
                "/api/chat/forward-destinations",
                Some(name),
                None,
                Value::Null,
            )
            .await;
            body["destinations"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|d| d["id"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        }
    };
    assert!(
        !destinations("bob").await.contains(&request_dm),
        "a declined request isn't offered for forwarding"
    );
    assert!(destinations("alice").await.contains(&request_dm));
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
    let (_, source) = send("alice", request_dm.clone()).await;
    let forward = |from: &'static str| {
        let app = app.clone();
        let conversation = request_dm.clone();
        let source = source["id"].clone();
        async move {
            request(
                &app,
                "POST",
                &format!("/api/chat/channels/{conversation}/forwards"),
                None,
                Some(&format!("chat-{from}")),
                json!({"sourceChannelId":conversation,"sourceMessageId":source,"clientMessageId":Uuid::new_v4()}),
            )
            .await
        }
    };
    assert_eq!(forward("bob").await.0, StatusCode::OK);
    let source_id = source["id"].as_str().unwrap().to_owned();
    // Reactions, pins, edits and typing are DM writes too.
    let write = |from: &'static str, kind: &'static str, active: bool| {
        let app = app.clone();
        let conversation = request_dm.clone();
        let message = source_id.clone();
        async move {
            let base = format!("/api/chat/channels/{conversation}");
            let (method, path, body) = match kind {
                "reaction" => (
                    "PUT",
                    format!("{base}/messages/{message}/reactions"),
                    json!({"emoji":"👍","active":active}),
                ),
                "pin" => (
                    "PUT",
                    format!("{base}/messages/{message}/pin"),
                    json!({ "active": active }),
                ),
                "edit" => (
                    "PUT",
                    format!("{base}/messages/{message}"),
                    json!({"text":format!("edited {active}"),"expectedRevision":1}),
                ),
                _ => (
                    "POST",
                    format!("{base}/typing"),
                    json!({ "typing": active }),
                ),
            };
            request(
                &app,
                method,
                &path,
                None,
                Some(&format!("chat-{from}")),
                body,
            )
            .await
        }
    };
    assert_eq!(write("alice", "reaction", true).await.0, StatusCode::OK);
    assert_eq!(write("bob", "pin", true).await.0, StatusCode::OK);
    assert_eq!(
        write("alice", "typing", true).await.0,
        StatusCode::NO_CONTENT
    );
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
    // Forwards are sends too.
    assert_eq!(forward("alice").await.1["code"], "dm_not_accepted");
    assert_eq!(forward("bob").await.1["code"], "dm_blocked");
    for kind in ["reaction", "pin", "edit", "typing"] {
        for active in [true, false] {
            assert_eq!(
                write("alice", kind, active).await,
                (
                    StatusCode::FORBIDDEN,
                    json!({"error":"this person isn't accepting direct messages","code":"dm_not_accepted"})
                ),
                "{kind} {active}"
            );
            assert_eq!(
                write("bob", kind, active).await.1["code"],
                "dm_blocked",
                "{kind} {active}"
            );
        }
    }
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn blocks_stop_reactions_pins_edits_and_typing_in_dms(pool: PgPool) {
    let mut ids = std::collections::HashMap::new();
    let mut users = Vec::new();
    for name in ["alice", "bob"] {
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
        ids.insert(name, external);
        users.push(user);
    }
    // Sharing a space makes the DM accepted, so only the block matters.
    let space: i64 = sqlx::query_scalar(
        "INSERT INTO spaces (external_id,name,owner_id) VALUES ('blocks','Blocks',$1) RETURNING id",
    )
    .bind(users[0])
    .fetch_one(&pool)
    .await
    .unwrap();
    for user in &users {
        sqlx::query("INSERT INTO space_members (space_id,user_id) VALUES ($1,$2)")
            .bind(space)
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
    }
    let mut config = Config::test(false);
    config.auth_fixture = false;
    let mut state =
        AppState::with_database(config, Arc::new(Cloudflare::new()), Some(pool.clone()));
    state.chat = Some(chat::Chat::new(
        pool.clone(),
        redis::Client::open(std::env::var("CHAT_TEST_VALKEY_URL").unwrap()).unwrap(),
    ));
    let app = crate::app(state);
    let (status, dm) = request(
        &app,
        "POST",
        "/api/dms",
        Some("alice"),
        None,
        json!({"username":"bob"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dm}");
    let dm = dm["id"].as_str().unwrap().to_owned();
    let (status, sent) = request(
        &app,
        "POST",
        &format!("/api/chat/channels/{dm}/messages"),
        None,
        Some("chat-alice"),
        json!({"clientMessageId":Uuid::new_v4(),"text":"hello"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{sent}");
    let message = sent["id"].as_str().unwrap().to_owned();
    let interact = |who: &'static str, action: &'static str| {
        let app = app.clone();
        let (dm, message) = (dm.clone(), message.clone());
        async move {
            let (method, path, body) = match action {
                "react" => (
                    "PUT",
                    format!("/api/chat/channels/{dm}/messages/{message}/reactions"),
                    json!({"emoji":"👍","active":true}),
                ),
                "unreact" => (
                    "PUT",
                    format!("/api/chat/channels/{dm}/messages/{message}/reactions"),
                    json!({"emoji":"👍","active":false}),
                ),
                "pin" => (
                    "PUT",
                    format!("/api/chat/channels/{dm}/messages/{message}/pin"),
                    json!({"active":true}),
                ),
                "edit" => (
                    "PUT",
                    format!("/api/chat/channels/{dm}/messages/{message}"),
                    json!({"text":"edited","expectedRevision":1}),
                ),
                _ => (
                    "POST",
                    format!("/api/chat/channels/{dm}/typing"),
                    json!({"typing":true}),
                ),
            };
            let (status, body) = request(
                &app,
                method,
                &path,
                None,
                Some(&format!("chat-{who}")),
                body,
            )
            .await;
            (status, body["code"].as_str().map(str::to_owned))
        }
    };
    assert_eq!(interact("bob", "react").await.0, StatusCode::OK);
    assert_eq!(interact("bob", "unreact").await.0, StatusCode::OK);

    let block = |method: &'static str| {
        let app = app.clone();
        let alice = ids["alice"].clone();
        async move {
            request(
                &app,
                method,
                &format!("/api/blocks/{alice}"),
                Some("bob"),
                None,
                Value::Null,
            )
            .await
            .0
        }
    };
    assert_eq!(block("PUT").await, StatusCode::NO_CONTENT);
    // Neither side can interact while the block stands, in any form.
    for action in ["react", "pin", "edit", "typing"] {
        assert_eq!(
            interact("alice", action).await,
            (StatusCode::FORBIDDEN, Some("dm_not_accepted".to_owned())),
            "blocked person: {action}"
        );
    }
    for action in ["react", "pin", "typing"] {
        assert_eq!(
            interact("bob", action).await,
            (StatusCode::FORBIDDEN, Some("dm_blocked".to_owned())),
            "blocker: {action}"
        );
    }
    let reactions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM message_reactions WHERE deleted_at IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(reactions, 0);

    assert_eq!(block("DELETE").await, StatusCode::NO_CONTENT);
    for action in ["react", "pin", "edit"] {
        assert_eq!(
            interact("alice", action).await.0,
            StatusCode::OK,
            "after unblocking: {action}"
        );
    }
}
