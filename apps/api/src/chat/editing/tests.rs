use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use futures_util::StreamExt;
use sqlx::{
    Connection, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{future::IntoFuture, str::FromStr};
use tower::ServiceExt;

async fn command(
    app: &Router,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("x-caper-chat-token", token)
                .header("authorization", format!("Bearer {token}"))
                .body(if method == "GET" {
                    Body::empty()
                } else {
                    Body::from(body.to_string())
                })
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn edits_retain_versions_authorize_accounts_and_commit_with_thread_projections() {
    let options =
        PgConnectOptions::from_str(&std::env::var("CHAT_TEST_DATABASE_URL").unwrap()).unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("edits_test_{}", Uuid::new_v4().simple());
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
    let mut actors = Vec::new();
    for token in ["edit-author", "edit-peer", "edit-outsider"] {
        let id: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name,avatar_id) VALUES($1,$1,$1,255) RETURNING id")
            .bind(token).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(id).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,$1,$3,$2)")
            .bind(token).bind(&hash).bind(id).execute(&pool).await.unwrap();
        actors.push(id);
    }
    let space: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES('edit-space','Edits',$1) RETURNING id")
        .bind(actors[0]).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.space_members(space_id,user_id) SELECT $1,id FROM public.users WHERE external_id!='edit-outsider'")
        .bind(space).execute(&pool).await.unwrap();
    let channel: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name,private) VALUES('edit-channel',$1,'edits',true) RETURNING id")
        .bind(space).fetch_one(&pool).await.unwrap();
    for table in ["channel_members", "channel_joins"] {
        sqlx::query(&format!("INSERT INTO public.{table}(channel_id,user_id) SELECT $1,id FROM public.users WHERE external_id!='edit-outsider'"))
            .bind(channel).execute(&pool).await.unwrap();
    }
    let broker_url = std::env::var("CHAT_TEST_VALKEY_URL").unwrap();
    assert!(
        broker_url.starts_with("redis://127.0.0.1:")
            || broker_url.starts_with("redis://localhost:")
    );
    let chat = Chat::new(pool.clone(), redis::Client::open(broker_url).unwrap());
    let mut state = AppState::new(
        crate::Config::test(false),
        Arc::new(crate::Cloudflare::new()),
    );
    state.chat = Some(chat.clone());
    let app = crate::app(state);
    let root_path = "/api/chat/channels/edit-channel/messages";
    let send_key = Uuid::new_v4();
    let (status, root) = command(
        &app,
        "POST",
        root_path,
        "edit-author",
        json!({"clientMessageId":send_key,"text":"Meet Friday 🙂"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let root_id = root["id"].as_str().unwrap();
    let path = format!("{root_path}/{root_id}");
    let original_versions = versions_page(&pool, "edit-channel", root_id, None, Some(actors[1]))
        .await
        .unwrap();
    assert_eq!(original_versions["versions"][0]["revision"], 1);
    assert_eq!(original_versions["versions"][0]["content"], root["content"]);
    assert!(
        versions_page(&pool, "edit-channel", root_id, None, Some(actors[2]))
            .await
            .is_err()
    );
    assert!(
        versions_page(&pool, "edit-channel", root_id, None, None)
            .await
            .is_err()
    );
    for (token, body, expected) in [
        (
            "edit-peer",
            json!({"text":"forged", "expectedRevision":1}),
            StatusCode::FORBIDDEN,
        ),
        (
            "edit-outsider",
            json!({"text":"forged", "expectedRevision":1}),
            StatusCode::NOT_FOUND,
        ),
        (
            "edit-author",
            json!({"text":"forged", "expectedRevision":1,"authorId":"edit-peer"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "edit-author",
            json!({"text":"", "expectedRevision":1}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "edit-author",
            json!({"text":"a\u{0}b", "expectedRevision":1}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "edit-author",
            json!({"text":"valid", "expectedRevision":0}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        assert_eq!(command(&app, "PUT", &path, token, body).await.0, expected);
    }
    let noop = persist_edit(
        &pool,
        "edit-channel",
        root_id,
        "edit-author",
        "Meet Friday 🙂",
        1,
    )
    .await
    .unwrap();
    assert_eq!(noop, root);
    let (status, reply) = command(&app, "POST", root_path, "edit-author", json!({"clientMessageId":Uuid::new_v4(),"text":"At 9?","threadRootId":root_id,"broadcast":true})).await;
    assert_eq!(status, StatusCode::OK);
    let pinned = persist_pin(&pool, "edit-channel", root_id, "edit-peer", true)
        .await
        .unwrap();
    let reaction = persist_reaction(&pool, "edit-channel", root_id, "edit-peer", "🚀", true)
        .await
        .unwrap();
    let edited = persist_edit(
        &pool,
        "edit-channel",
        root_id,
        "edit-author",
        "Meet Saturday 🚀",
        1,
    )
    .await
    .unwrap();
    assert_eq!(edited["revision"], 2);
    assert_eq!(edited["editSeq"], "5");
    assert_eq!(edited["seq"], root["seq"]);
    assert_eq!(edited["createdAt"], root["createdAt"]);
    assert_eq!(edited["clientMessageId"], root["clientMessageId"]);
    assert_eq!(edited["pin"], pinned["message"]["pin"]);
    assert_eq!(edited["reactionSeq"], reaction["seq"]);
    assert_eq!(edited["thread"]["replyCount"], 1);
    assert_eq!(edited["thread"]["seq"], "2");
    let retry = persist_edit(
        &pool,
        "edit-channel",
        root_id,
        "edit-author",
        "Meet Saturday 🚀",
        1,
    )
    .await
    .unwrap();
    assert_eq!(retry, edited);
    assert_eq!(
        persist_edit(
            &pool,
            "edit-channel",
            root_id,
            "edit-author",
            "Stale draft",
            1
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::CONFLICT
    );
    // The original POST retry must still acknowledge the same edited message,
    // not restore original content or append another durable message.
    let (status, retry_send) = command(
        &app,
        "POST",
        root_path,
        "edit-author",
        json!({"clientMessageId":send_key,"text":"Meet Friday 🙂"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(retry_send, edited);
    let reply_id = reply["id"].as_str().unwrap();
    let edited_reply = persist_edit(&pool, "edit-channel", reply_id, "edit-author", "At 19!", 1)
        .await
        .unwrap();
    assert_eq!(edited_reply["threadRootId"], root_id);
    assert_eq!(edited_reply["broadcast"], true);
    assert_eq!(edited_reply["seq"], "2");
    let (_, history) = command(&app, "GET", root_path, "edit-peer", Value::Null).await;
    let (_, thread) = command(
        &app,
        "GET",
        &format!("{path}/thread"),
        "edit-peer",
        Value::Null,
    )
    .await;
    assert_eq!(history["cursor"], "6");
    assert_eq!(history["messages"][1]["content"]["text"], "At 19!");
    assert_eq!(thread["messages"][0]["content"]["text"], "At 19!");
    assert_eq!(thread["root"]["content"]["text"], "Meet Saturday 🚀");
    let versions = versions_page(&pool, "edit-channel", root_id, None, Some(actors[1]))
        .await
        .unwrap();
    assert_eq!(versions["versions"].as_array().unwrap().len(), 2);
    assert_eq!(versions["versions"][0]["revision"], 2);
    assert_eq!(versions["versions"][1]["content"], root["content"]);
    assert_eq!(
        versions_page(&pool, "edit-channel", root_id, Some(2), Some(actors[1]))
            .await
            .unwrap()["versions"][0]["revision"],
        1
    );
    assert!(
        versions_page(&pool, "edit-channel", root_id, Some(1), Some(actors[1]))
            .await
            .unwrap()["versions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Competing tabs cannot both save an old revision; retrying a committed edit
    // is a no-op, whereas a different draft gets a visible conflict.
    let (a, b) = tokio::join!(
        persist_edit(&pool, "edit-channel", root_id, "edit-author", "Tab A", 2),
        persist_edit(&pool, "edit-channel", root_id, "edit-author", "Tab B", 2)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(
        a.err().or_else(|| b.err()).unwrap().status,
        StatusCode::CONFLICT
    );
    pool.execute("ALTER TABLE public.channel_events ADD CONSTRAINT reject_edit CHECK (seq < 8)")
        .await
        .unwrap();
    assert!(
        persist_edit(
            &pool,
            "edit-channel",
            root_id,
            "edit-author",
            "Must roll back",
            3
        )
        .await
        .is_err()
    );
    let retained = versions_page(&pool, "edit-channel", root_id, None, Some(actors[0]))
        .await
        .unwrap();
    assert_eq!(retained["versions"][0]["revision"], 3);
    assert_eq!(retained["versions"].as_array().unwrap().len(), 3);
    let head: i64 = sqlx::query_scalar("SELECT last_seq FROM public.channels WHERE id=$1")
        .bind(channel)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(head, 7);
    pool.execute("ALTER TABLE public.channel_events DROP CONSTRAINT reject_edit")
        .await
        .unwrap();
    // Account identity, not the expired/rotated capability which sent it, owns editing.
    let hash = Sha256::digest(b"edit-new-session").to_vec();
    sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) SELECT 'edit-new-session',$1,name,user_id,account_session_hash FROM public.chat_sessions WHERE external_id='edit-author'")
        .bind(hash).execute(&pool).await.unwrap();
    assert!(
        persist_edit(
            &pool,
            "edit-channel",
            root_id,
            "edit-new-session",
            "From new session",
            3
        )
        .await
        .is_ok()
    );
    let (status, peers_message) = command(
        &app,
        "POST",
        root_path,
        "edit-peer",
        json!({"clientMessageId":Uuid::new_v4(),"text":"The member owns this text"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        persist_edit(
            &pool,
            "edit-channel",
            peers_message["id"].as_str().unwrap(),
            "edit-author",
            "Owner override",
            1
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::FORBIDDEN,
        "space ownership does not grant editing another account's text"
    );
    // Preview access remains read-only, and revoked grants hide retained history.
    sqlx::query(
        "UPDATE public.channel_joins SET deleted_at=now() WHERE channel_id=$1 AND user_id=$2",
    )
    .bind(channel)
    .bind(actors[0])
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        versions_page(&pool, "edit-channel", root_id, None, Some(actors[0]))
            .await
            .is_ok()
    );
    assert_eq!(
        persist_edit(
            &pool,
            "edit-channel",
            root_id,
            "edit-author",
            "Not joined",
            4
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
    sqlx::query(
        "UPDATE public.channel_members SET deleted_at=now() WHERE channel_id=$1 AND user_id=$2",
    )
    .bind(channel)
    .bind(actors[1])
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        versions_page(&pool, "edit-channel", root_id, None, Some(actors[1]))
            .await
            .is_err()
    );
    let events: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM public.channel_events WHERE channel_id=$1 ORDER BY seq",
    )
    .bind(channel)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        events[0]["message"]["content"], root["content"],
        "original replay event is immutable"
    );
    assert_eq!(events[4]["type"], "message.edited");
    assert_eq!(events[4]["message"]["seq"], "1");
    assert_eq!(events[4]["message"]["editSeq"], events[4]["seq"]);
    // Outbox publisher understands edit snapshots without a second notification/message.
    assert!(publish_pending(&chat).await.unwrap());
    let unpublished: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.channel_events WHERE published_at IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(unpublished, 0);
    // DMs use the same edit contract without giving a space owner/outsider access.
    let dm: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,name,private) VALUES('edit-direct','direct',true) RETURNING id")
        .fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.direct_conversations(channel_id,low_user_id,high_user_id) VALUES($1,$2,$3)")
        .bind(dm).bind(actors[0].min(actors[1])).bind(actors[0].max(actors[1])).execute(&pool).await.unwrap();
    let (_, direct) = command(
        &app,
        "POST",
        "/api/chat/channels/edit-direct/messages",
        "edit-author",
        json!({"clientMessageId":Uuid::new_v4(),"text":"Original DM"}),
    )
    .await;
    let direct_id = direct["id"].as_str().unwrap();
    let direct_edit = persist_edit(
        &pool,
        "edit-direct",
        direct_id,
        "edit-author",
        "Version 2",
        1,
    )
    .await
    .unwrap();
    assert_eq!(direct_edit["revision"], 2);
    assert!(
        versions_page(&pool, "edit-direct", direct_id, None, Some(actors[1]))
            .await
            .is_ok()
    );
    assert!(
        versions_page(&pool, "edit-direct", direct_id, None, Some(actors[2]))
            .await
            .is_err()
    );
    assert_eq!(
        persist_edit(&pool, "edit-direct", direct_id, "edit-peer", "Not mine", 2)
            .await
            .unwrap_err()
            .status,
        StatusCode::FORBIDDEN
    );
    for revision in 2..31 {
        persist_edit(
            &pool,
            "edit-direct",
            direct_id,
            "edit-author",
            &format!("Version {}", revision + 1),
            revision,
        )
        .await
        .unwrap();
    }
    assert_eq!(
        persist_edit(
            &pool,
            "edit-direct",
            direct_id,
            "edit-author",
            "Version 32",
            31
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(
        persist_edit(
            &pool,
            "edit-direct",
            direct_id,
            "edit-author",
            "Version 31",
            30
        )
        .await
        .is_ok(),
        "retry consumes no edit budget"
    );
    // Explicit fixture clock shift, not pruning: retained versions survive rate expiry.
    sqlx::query("UPDATE public.message_versions SET created_at=now()-interval '2 minutes' WHERE message_id=(SELECT id FROM public.messages WHERE external_id=$1)")
        .bind(direct_id).execute(&pool).await.unwrap();
    for revision in 31..53 {
        persist_edit(
            &pool,
            "edit-direct",
            direct_id,
            "edit-author",
            &format!("Version {}", revision + 1),
            revision,
        )
        .await
        .unwrap();
    }
    let page = versions_page(&pool, "edit-direct", direct_id, None, Some(actors[1]))
        .await
        .unwrap();
    assert_eq!(page["versions"].as_array().unwrap().len(), 50);
    assert_eq!(page["versions"][0]["revision"], 53);
    assert_eq!(page["versions"][49]["revision"], 4);
    assert_eq!(page["hasMore"], true);
    let older = versions_page(&pool, "edit-direct", direct_id, Some(4), Some(actors[1]))
        .await
        .unwrap();
    assert_eq!(
        older["versions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["revision"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
    assert_eq!(older["versions"][2]["content"], direct["content"]);
    assert_eq!(older["hasMore"], false);
    let notes: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,name,private) VALUES('edit-notes','direct',true) RETURNING id")
        .fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.direct_conversations(channel_id,low_user_id,high_user_id) VALUES($1,$2,$2)")
        .bind(notes).bind(actors[0]).execute(&pool).await.unwrap();
    let notes_path = "/api/chat/channels/edit-notes/messages";
    let (_, note) = command(
        &app,
        "POST",
        notes_path,
        "edit-author",
        json!({"clientMessageId":Uuid::new_v4(),"text":"Note root"}),
    )
    .await;
    let (_, hidden) = command(&app, "POST", notes_path, "edit-author", json!({"clientMessageId":Uuid::new_v4(),"text":"Thread-only draft","threadRootId":note["id"]})).await;
    let hidden_id = hidden["id"].as_str().unwrap();
    persist_edit(
        &pool,
        "edit-notes",
        hidden_id,
        "edit-author",
        "Thread-only corrected",
        1,
    )
    .await
    .unwrap();
    let (_, notes_history) = command(&app, "GET", notes_path, "edit-author", Value::Null).await;
    assert_eq!(notes_history["cursor"], "3");
    assert_eq!(
        notes_history["messages"].as_array().unwrap().len(),
        1,
        "editing must not broadcast a thread-only reply"
    );
    let (_, notes_thread) = command(
        &app,
        "GET",
        &format!("{notes_path}/{}/thread", note["id"].as_str().unwrap()),
        "edit-author",
        Value::Null,
    )
    .await;
    assert_eq!(
        notes_thread["messages"][0]["content"]["text"],
        "Thread-only corrected"
    );
    assert!(
        versions_page(&pool, "edit-notes", hidden_id, None, Some(actors[1]))
            .await
            .is_err()
    );
    pool.close().await;
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL"]
async fn edits_re_resolve_mentions(pool: PgPool) {
    let mut users = Vec::new();
    for name in ["writer", "maya"] {
        let user: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1||'-id',$1,$1) RETURNING id")
            .bind(name).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(name.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,$1,$3,$2)")
            .bind(name).bind(hash).bind(user).execute(&pool).await.unwrap();
        users.push(user);
    }
    let space: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES('mention-edit-space','Edits',$1) RETURNING id")
        .bind(users[0]).fetch_one(&pool).await.unwrap();
    let channel: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name) VALUES('mention-edit',$1,'edits') RETURNING id")
        .bind(space).fetch_one(&pool).await.unwrap();
    for user in &users {
        sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
            .bind(space)
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2)")
            .bind(channel)
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
    }
    let sent = persist(&pool, "mention-edit", "writer", Uuid::new_v4(), "hello")
        .await
        .unwrap();
    let id = sent["id"].as_str().unwrap();
    assert!(sent["content"].get("mentions").is_none());
    let added = persist_edit(&pool, "mention-edit", id, "writer", "hello @maya @here", 1)
        .await
        .unwrap();
    assert_eq!(
        added["content"]["mentions"],
        json!([{"type":"user","id":"maya-id","username":"maya"},{"type":"here"}])
    );
    // A lost response retried with the same text is accepted, not a conflict.
    let retried = persist_edit(&pool, "mention-edit", id, "writer", "hello @maya @here", 1)
        .await
        .unwrap();
    assert_eq!(retried["content"], added["content"]);
    let event: Value = sqlx::query_scalar("SELECT payload FROM public.channel_events WHERE channel_id=$1 AND payload->>'type'='message.edited'")
        .bind(channel).fetch_one(&pool).await.unwrap();
    assert_eq!(event["message"]["content"], added["content"]);
    let removed = persist_edit(&pool, "mention-edit", id, "writer", "hello", 2)
        .await
        .unwrap();
    assert!(removed["content"].get("mentions").is_none());
}

/// A disposable database, app and CDN signer for editing sent files.
struct Files {
    admin: sqlx::PgConnection,
    database: String,
    pool: PgPool,
    app: Router,
    chat: Chat,
    assets: crate::assets::Assets,
    signer: Arc<CdnSigner>,
    author: i64,
    peer: i64,
    space: i64,
    channel: i64,
    elsewhere: i64,
}

const FILES_PATH: &str = "/api/chat/channels/files-channel/messages";

async fn files_fixture() -> Files {
    let options =
        PgConnectOptions::from_str(&std::env::var("CHAT_TEST_DATABASE_URL").unwrap()).unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("file_edits_test_{}", Uuid::new_v4().simple());
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
    let mut actors = Vec::new();
    for token in ["files-author", "files-peer"] {
        let id: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name,avatar_id) VALUES($1,$1,$1,255) RETURNING id")
            .bind(token).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(id).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,$1,$3,$2)")
            .bind(token).bind(&hash).bind(id).execute(&pool).await.unwrap();
        actors.push(id);
    }
    let space: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES('files-space','Files',$1) RETURNING id")
        .bind(actors[0]).fetch_one(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO public.space_members(space_id,user_id) SELECT $1,id FROM public.users",
    )
    .bind(space)
    .execute(&pool)
    .await
    .unwrap();
    let mut channels = Vec::new();
    for name in ["files-channel", "files-elsewhere"] {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,$1) RETURNING id",
        )
        .bind(name)
        .bind(space)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO public.channel_joins(channel_id,user_id) SELECT $1,id FROM public.users",
        )
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
        channels.push(id);
    }
    let broker_url = std::env::var("CHAT_TEST_VALKEY_URL").unwrap();
    assert!(
        broker_url.starts_with("redis://127.0.0.1:")
            || broker_url.starts_with("redis://localhost:")
    );
    let mut chat = Chat::new(pool.clone(), redis::Client::open(broker_url).unwrap());
    let signer = Arc::new(CdnSigner::new("https://cdn.test".into(), vec![7; 32]));
    chat.cdn = Some(signer.clone());
    // Objects are never touched: nothing these tests store is purged.
    let assets = crate::assets::Assets::new(
        pool.clone(),
        crate::assets::R2::new(
            "http://127.0.0.1:1".into(),
            "test".into(),
            "key".into(),
            "secret".into(),
        ),
        1 << 30,
    );
    let mut state = AppState::with_database(
        crate::Config::test(false),
        Arc::new(crate::Cloudflare::new()),
        Some(pool.clone()),
    );
    state.auth = crate::auth::AuthVerifier::new();
    state.chat = Some(chat.clone());
    state.assets = Some(assets.clone());
    Files {
        admin,
        database,
        app: crate::app(state),
        pool,
        chat,
        assets,
        signer,
        author: actors[0],
        peer: actors[1],
        space,
        channel: channels[0],
        elsewhere: channels[1],
    }
}

impl Files {
    /// A completed upload, as `/complete` leaves it; photos and videos have previews.
    async fn upload(&self, owner: i64, channel: i64, id: &str, content_type: &str) {
        sqlx::query("INSERT INTO public.assets(external_id,owner_id,purpose,channel_id,kind,content_type,filename,byte_size,width,height,preview_content_type,preview_byte_size,uploaded_at)
                     VALUES($1,$2,'attachment',$3,$4,$5,$1||'.bin',1000,40,30,CASE WHEN $4 IN ('image','video') THEN 'image/webp' END,CASE WHEN $4 IN ('image','video') THEN 100 END,now())")
            .bind(id).bind(owner).bind(channel).bind(crate::assets::kind(content_type)).bind(content_type)
            .execute(&self.pool).await.unwrap();
    }

    async fn send(&self, files: &[&str], spoilers: &[&str]) -> Value {
        let (status, sent) = command(
            &self.app,
            "POST",
            FILES_PATH,
            "files-author",
            json!({"clientMessageId":Uuid::new_v4(),"text":"Trip","attachmentIds":files,"spoilerAttachmentIds":spoilers}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{sent}");
        sent
    }

    async fn edit(
        &self,
        message: &str,
        token: &str,
        revision: i32,
        text: &str,
        files: Value,
    ) -> (StatusCode, Value) {
        command(
            &self.app,
            "PUT",
            &format!("{FILES_PATH}/{message}"),
            token,
            json!({"text":text,"expectedRevision":revision,"attachments":files}),
        )
        .await
    }

    /// `(linked, position, retired, first-sent source, edit)` of an asset row.
    async fn row(&self, id: &str) -> (bool, Option<i16>, bool, Option<String>, Option<Value>) {
        sqlx::query_as("SELECT a.message_id IS NOT NULL,a.position,a.retired_at IS NOT NULL,s.external_id,a.edit FROM public.assets a LEFT JOIN public.assets s ON s.id=a.edit_source_id WHERE a.external_id=$1")
            .bind(id).fetch_one(&self.pool).await.unwrap()
    }

    async fn count(&self, sql: &str) -> i64 {
        sqlx::query_scalar(sql).fetch_one(&self.pool).await.unwrap()
    }

    async fn close(mut self) {
        self.pool.close().await;
        self.admin
            .execute(format!("DROP DATABASE {} WITH (FORCE)", self.database).as_str())
            .await
            .unwrap();
    }
}

fn ids(message: &Value) -> Vec<&str> {
    message["content"]["attachments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["id"].as_str().unwrap())
        .collect()
}

fn turned() -> Value {
    json!({"rotation":90,"crop":{"x":0.1,"y":0,"width":0.8,"height":1},"marks":[{"kind":"text","color":"#FFFFFF","size":0.05,"x":0.5,"y":0.5,"text":"Hi"}]})
}

fn drawn() -> Value {
    json!({"rotation":0,"crop":{"x":0,"y":0,"width":1,"height":1},"marks":[{"kind":"stroke","color":"#e5484d","width":0.01,"points":[0.2,0.2,0.8,0.8]}]})
}

fn edit_of(value: &Value) -> attachments::EditDescription {
    serde_json::from_value(value.clone()).unwrap()
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn sent_files_are_replaced_restored_and_retried_like_text_edits() {
    let f = files_fixture().await;
    for (id, content_type) in [
        ("photo-b", "image/avif"),
        ("notes-d", "text/plain"),
        ("clip-v", "video/mp4"),
        ("photo-n1", "image/avif"),
        ("photo-n2", "image/webp"),
        ("clip-x", "video/mp4"),
        ("clip-nv", "video/mp4"),
        ("photo-old", "image/avif"),
    ] {
        f.upload(f.author, f.channel, id, content_type).await;
    }
    f.upload(f.peer, f.channel, "photo-peer", "image/avif")
        .await;
    f.upload(f.author, f.elsewhere, "photo-far", "image/avif")
        .await;
    sqlx::query("UPDATE public.assets SET uploaded_at=now()-interval '25 hours' WHERE external_id='photo-old'")
        .execute(&f.pool).await.unwrap();
    let sent = f
        .send(&["photo-b", "notes-d", "clip-v"], &["notes-d"])
        .await;
    let message = sent["id"].as_str().unwrap();
    let kept = |first: Value| json!([first, {"id":"notes-d","spoiler":true}, {"id":"clip-v"}]);

    // Each refusal leaves the message, its versions and every upload as they were.
    for (token, files, status, code) in [
        (
            "files-author",
            json!([{"id":"photo-b","remove":true},{"id":"notes-d","spoiler":true},{"id":"clip-v"}]),
            StatusCode::UNPROCESSABLE_ENTITY,
            None,
        ),
        (
            "files-author",
            json!([{"id":"photo-b"},{"id":"notes-d","spoiler":true}]),
            StatusCode::BAD_REQUEST,
            None,
        ),
        (
            "files-author",
            json!([{"id":"notes-d","spoiler":true},{"id":"photo-b"},{"id":"clip-v"}]),
            StatusCode::BAD_REQUEST,
            None,
        ),
        (
            "files-author",
            json!([{"id":"photo-b","replaces":"photo-b"},{"id":"notes-d","spoiler":true},{"id":"clip-v"}]),
            StatusCode::BAD_REQUEST,
            None,
        ),
        (
            "files-author",
            json!([{"id":"photo-b","edit":turned()},{"id":"notes-d","spoiler":true},{"id":"clip-v"}]),
            StatusCode::BAD_REQUEST,
            None,
        ),
        (
            "files-author",
            json!([{"id":"photo-b"},{"id":"photo-n1","replaces":"notes-d","edit":turned()},{"id":"clip-v"}]),
            StatusCode::BAD_REQUEST,
            None,
        ),
        (
            "files-author",
            kept(json!({"id":"photo-n1","replaces":"photo-b"})),
            StatusCode::BAD_REQUEST,
            None,
        ),
        (
            "files-author",
            kept(
                json!({"id":"photo-n1","replaces":"photo-b","edit":{"rotation":45,"crop":{"x":0,"y":0,"width":1,"height":1},"marks":[]}}),
            ),
            StatusCode::BAD_REQUEST,
            None,
        ),
        (
            "files-author",
            kept(
                json!({"id":"photo-n1","replaces":"photo-b","edit":{"rotation":0,"crop":{"x":0,"y":0,"width":1,"height":1},"marks":vec![json!({"kind":"stroke","color":"#000000","width":0.01,"points":[0.5,0.5]}); 501]}}),
            ),
            StatusCode::BAD_REQUEST,
            Some("edit_too_large"),
        ),
        (
            "files-author",
            kept(json!({"id":"clip-x","replaces":"photo-b","edit":turned()})),
            StatusCode::BAD_REQUEST,
            None,
        ),
        (
            "files-author",
            kept(json!({"id":"photo-peer","replaces":"photo-b","edit":turned()})),
            StatusCode::CONFLICT,
            Some("attachment_unavailable"),
        ),
        (
            "files-author",
            kept(json!({"id":"photo-far","replaces":"photo-b","edit":turned()})),
            StatusCode::CONFLICT,
            Some("attachment_unavailable"),
        ),
        (
            "files-author",
            kept(json!({"id":"photo-old","replaces":"photo-b","edit":turned()})),
            StatusCode::CONFLICT,
            Some("attachment_unavailable"),
        ),
        (
            "files-peer",
            kept(json!({"id":"photo-peer","replaces":"photo-b","edit":turned()})),
            StatusCode::FORBIDDEN,
            None,
        ),
    ] {
        let (got, body) = f.edit(message, token, 1, "Trip", files.clone()).await;
        assert_eq!(got, status, "{files} → {body}");
        assert_eq!(body["code"].as_str(), code, "{files} → {body}");
    }
    assert_eq!(
        f.count("SELECT count(*) FROM public.message_versions")
            .await,
        0
    );
    assert_eq!(
        f.count("SELECT count(*) FROM public.assets WHERE message_id IS NOT NULL")
            .await,
        3,
        "a refused edit links no upload, including the wrong kind of file"
    );

    // Replace the photo with an edited render and spoiler it in the same revision.
    let replace =
        kept(json!({"id":"photo-n1","replaces":"photo-b","edit":turned(),"spoiler":true}));
    let (status, edited) = f
        .edit(message, "files-author", 1, "Trip", replace.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["revision"], 2);
    assert_eq!(edited["attachmentsSeq"], edited["editSeq"]);
    assert_eq!(ids(&edited), ["photo-n1", "notes-d", "clip-v"]);
    let files = &edited["content"]["attachments"];
    assert_eq!(files[0]["spoiler"], true);
    assert_eq!(files[0]["kind"], "image");
    assert_eq!(files[0]["name"], "photo-n1.bin");
    assert!(
        files[0]["url"]
            .as_str()
            .unwrap()
            .contains("/assets/photo-n1?")
    );
    assert_eq!(files[1]["spoiler"], true);
    assert!(files[2].get("spoiler").is_none());
    let (linked, position, retired, source, _) = f.row("photo-b").await;
    assert!(
        linked && retired,
        "the first-sent file stays stored and linked"
    );
    assert_eq!(position, Some(0));
    assert!(source.is_none());
    let (linked, position, retired, source, edit) = f.row("photo-n1").await;
    assert!(linked && !retired);
    assert_eq!(position, Some(0));
    assert_eq!(source.as_deref(), Some("photo-b"));
    assert_eq!(edit_of(&edit.unwrap()), edit_of(&turned()));
    // The edit description and the replaced file stay out of what others read.
    let event: Value =
        sqlx::query_scalar("SELECT payload FROM public.channel_events ORDER BY seq DESC LIMIT 1")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(event["type"], "message.edited");
    assert_eq!(ids(&event["message"]), ["photo-n1", "notes-d", "clip-v"]);
    let stored: Value =
        sqlx::query_scalar("SELECT content FROM public.message_versions WHERE revision=2")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    for shared in [event.to_string(), stored.to_string()] {
        assert!(
            !shared.contains("rotation") && !shared.contains("photo-b"),
            "{shared}"
        );
        assert!(!shared.contains("\"url\""), "{shared}");
    }
    assert_eq!(
        f.count("SELECT count(*) FROM public.notification_jobs")
            .await,
        1,
        "edits never notify"
    );

    // A lost response retried with the same body returns the stored result,
    // although its upload is now linked; a different list is a conflict.
    let head = f
        .count("SELECT last_seq FROM public.channels WHERE external_id='files-channel'")
        .await;
    let (status, retry) = f
        .edit(message, "files-author", 1, "Trip", replace.clone())
        .await;
    assert_eq!(status, StatusCode::OK, "{retry}");
    assert_eq!(
        (retry["revision"].clone(), ids(&retry)),
        (json!(2), ids(&edited))
    );
    assert_eq!(
        f.count("SELECT count(*) FROM public.message_versions")
            .await,
        2
    );
    assert_eq!(
        f.count("SELECT last_seq FROM public.channels WHERE external_id='files-channel'")
            .await,
        head
    );
    let (status, _) = f
        .edit(
            message,
            "files-author",
            1,
            "Trip",
            kept(json!({"id":"photo-n1","replaces":"photo-b","edit":turned()})),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "the spoiler differs");
    // The same list at the current revision changes nothing.
    let current =
        json!([{"id":"photo-n1","spoiler":true},{"id":"notes-d","spoiler":true},{"id":"clip-v"}]);
    let (status, same) = f.edit(message, "files-author", 2, "Trip", current).await;
    assert_eq!(
        (status, same["revision"].clone()),
        (StatusCode::OK, json!(2))
    );

    // A spoiler toggle alone is an edit, with a version and the edited marker.
    let (status, toggled) = f
        .edit(message, "files-author", 2, "Trip", json!([{"id":"photo-n1"},{"id":"notes-d","spoiler":true},{"id":"clip-v","spoiler":true}]))
        .await;
    assert_eq!(status, StatusCode::OK, "{toggled}");
    assert_eq!(toggled["revision"], 3);
    assert_ne!(toggled["editedAt"], edited["editedAt"]);
    assert_eq!(toggled["attachmentsSeq"], toggled["editSeq"]);
    assert!(
        toggled["content"]["attachments"][0]
            .get("spoiler")
            .is_none()
    );
    assert_eq!(toggled["content"]["attachments"][2]["spoiler"], true);
    assert_eq!(toggled["content"]["text"], "Trip");

    // Re-editing starts from the first-sent file, not the earlier render.
    let (status, chained) = f
        .edit(message, "files-author", 3, "Trip!", json!([{"id":"photo-n2","replaces":"photo-n1","edit":drawn()},{"id":"notes-d","spoiler":true},{"id":"clip-v","spoiler":true}]))
        .await;
    assert_eq!(status, StatusCode::OK, "{chained}");
    assert_eq!(chained["revision"], 4);
    assert_eq!(chained["content"]["text"], "Trip!");
    assert!(f.row("photo-n1").await.2);
    let (_, position, retired, source, edit) = f.row("photo-n2").await;
    assert_eq!(
        (position, retired, source.as_deref()),
        (Some(0), false, Some("photo-b"))
    );
    assert_eq!(edit_of(&edit.unwrap()), edit_of(&drawn()));

    // Only the slot's first-sent file can come back, and without an edit; an
    // earlier render is no fresh upload.
    for (files, status) in [
        (
            json!([{"id":"photo-b","replaces":"photo-n2","edit":drawn()},{"id":"notes-d","spoiler":true},{"id":"clip-v","spoiler":true}]),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!([{"id":"photo-n1","replaces":"photo-n2","edit":drawn()},{"id":"notes-d","spoiler":true},{"id":"clip-v","spoiler":true}]),
            StatusCode::CONFLICT,
        ),
    ] {
        assert_eq!(
            f.edit(message, "files-author", 4, "Trip!", files).await.0,
            status
        );
    }
    let (status, restored) = f
        .edit(message, "files-author", 4, "Trip!", json!([{"id":"photo-b","replaces":"photo-n2"},{"id":"notes-d","spoiler":true},{"id":"clip-v","spoiler":true}]))
        .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(ids(&restored), ["photo-b", "notes-d", "clip-v"]);
    let mut first = sent["content"]["attachments"][0].clone();
    for urls in ["url", "previewUrl"] {
        first.as_object_mut().unwrap().remove(urls);
    }
    let mut back = restored["content"]["attachments"][0].clone();
    for urls in ["url", "previewUrl"] {
        assert!(back.as_object_mut().unwrap().remove(urls).is_some());
    }
    assert_eq!(back, first, "the first-sent file returns as it was sent");
    let (_, position, retired, _, _) = f.row("photo-b").await;
    assert_eq!((position, retired), (Some(0), false));
    assert!(f.row("photo-n2").await.2);
    // Videos are replaced the same way; other files keep only their spoiler.
    let (status, video) = f
        .edit(message, "files-author", 5, "Trip!", json!([{"id":"photo-b"},{"id":"notes-d"},{"id":"clip-nv","replaces":"clip-v","edit":turned()}]))
        .await;
    assert_eq!(status, StatusCode::OK, "{video}");
    assert_eq!(ids(&video), ["photo-b", "notes-d", "clip-nv"]);
    assert_eq!(f.row("clip-nv").await.3.as_deref(), Some("clip-v"));
    // Text may be emptied while files remain, and a text-only edit keeps them.
    let (status, empty) = f
        .edit(
            message,
            "files-author",
            6,
            "",
            json!([{"id":"photo-b"},{"id":"notes-d"},{"id":"clip-nv"}]),
        )
        .await;
    assert_eq!(
        (status, empty["revision"].clone()),
        (StatusCode::OK, json!(7))
    );
    let text_only = persist_edit(&f.pool, "files-channel", message, "files-author", "Back", 7)
        .await
        .unwrap();
    assert_eq!(ids(&text_only), ["photo-b", "notes-d", "clip-nv"]);
    assert_eq!(text_only["attachmentsSeq"], video["attachmentsSeq"]);
    assert_eq!(
        f.count("SELECT count(*) FROM public.assets WHERE retired_at IS NOT NULL")
            .await,
        3,
        "photo-n1, photo-n2 and clip-v stay stored"
    );
    f.close().await;
}

fn placeholder(id: &str, kind: &str, content_type: &str) -> Value {
    json!({"id":id,"kind":kind,"contentType":content_type,"name":"Replaced file","size":0,"unavailable":true})
}

/// Every event a chat subscription replays for `token`, up to `ready`.
async fn replayed(address: std::net::SocketAddr, channel: &str, token: &str) -> Vec<Value> {
    let mut socket = crate::gateway::testing::chat_socket(address, token, channel, "0")
        .await
        .unwrap();
    let mut frames = Vec::new();
    loop {
        let frame = crate::gateway::testing::chat_event(&mut socket, Duration::from_secs(6)).await;
        assert!(frame["type"] != "error", "replay refused: {frame}");
        if frame["type"] == "ready" {
            return frames;
        }
        frames.push(frame);
    }
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn forwarded_files_refresh_only_while_the_shared_excerpt_is_accessible() {
    let f = files_fixture().await;
    for id in ["shared-old", "shared-current", "shared-reply", "unshared"] {
        f.upload(f.author, f.channel, id, "image/avif").await;
    }
    let sent = f.send(&["shared-old"], &[]).await;
    f.send(&["unshared"], &[]).await;
    let (status, reply) = command(
        &f.app, "POST", FILES_PATH, "files-author",
        json!({"clientMessageId":Uuid::new_v4(),"text":"Shared reply","threadRootId":sent["id"],"attachmentIds":["shared-reply"]}),
    ).await;
    assert_eq!(status, StatusCode::OK, "{reply}");
    let (status, forward) = command(
        &f.app, "POST", "/api/chat/channels/files-elsewhere/forwards", "files-author",
        json!({"sourceChannelId":"files-channel","sourceMessageId":sent["id"],"clientMessageId":Uuid::new_v4()}),
    ).await;
    assert_eq!(status, StatusCode::OK, "{forward}");
    let (status, edited) = f
        .edit(
            sent["id"].as_str().unwrap(),
            "files-author",
            1,
            "Current excerpt",
            json!([{"id":"shared-current","replaces":"shared-old","edit":turned()}]),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    // The peer can read the destination, but neither the original channel nor
    // an unrelated attachment in it. The owner retains original access.
    sqlx::query("UPDATE public.channels SET private=true WHERE id=$1")
        .bind(f.channel)
        .execute(&f.pool)
        .await
        .unwrap();
    let thread_path = format!(
        "/api/chat/channels/files-elsewhere/forwards/{}/thread",
        forward["id"].as_str().unwrap()
    );
    let (status, shared) = command(&f.app, "GET", &thread_path, "files-peer", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{shared}");
    assert!(shared["source"].is_null());
    assert!(shared["root"]["content"]["attachments"][0]["url"].is_string());
    assert!(shared["messages"][0]["content"]["attachments"][0]["url"].is_string());
    let input = json!({"ids":["shared-old","shared-current","shared-reply","unshared"]});
    let (status, refreshed) = command(
        &f.app,
        "POST",
        "/api/assets/urls",
        "files-peer",
        input.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{refreshed}");
    assert_eq!(
        refreshed["urls"].as_object().unwrap().len(),
        2,
        "{refreshed}"
    );
    for id in ["shared-current", "shared-reply"] {
        assert!(refreshed["urls"][id]["url"].is_string(), "{refreshed}");
        assert!(
            refreshed["urls"][id]["previewUrl"].is_string(),
            "{refreshed}"
        );
    }
    // Losing destination access revokes the alternative grant.
    sqlx::query("UPDATE public.channels SET private=true WHERE id=$1")
        .bind(f.elsewhere)
        .execute(&f.pool)
        .await
        .unwrap();
    let (_, denied) = command(
        &f.app,
        "POST",
        "/api/assets/urls",
        "files-peer",
        input.clone(),
    )
    .await;
    assert_eq!(denied["urls"], json!({}));
    sqlx::query("UPDATE public.channels SET private=false WHERE id=$1")
        .bind(f.elsewhere)
        .execute(&f.pool)
        .await
        .unwrap();
    // A deleted source channel makes its shared content unavailable too.
    sqlx::query("UPDATE public.channels SET deleted_at=now() WHERE id=$1")
        .bind(f.channel)
        .execute(&f.pool)
        .await
        .unwrap();
    let (_, denied) = command(
        &f.app,
        "POST",
        "/api/assets/urls",
        "files-peer",
        input.clone(),
    )
    .await;
    assert_eq!(denied["urls"], json!({}));
    sqlx::query("UPDATE public.channels SET deleted_at=NULL WHERE id=$1")
        .bind(f.channel)
        .execute(&f.pool)
        .await
        .unwrap();
    // A removed original also revokes its still-live replies' files.
    sqlx::query("UPDATE public.messages SET deleted_at=now() WHERE external_id=$1")
        .bind(sent["id"].as_str())
        .execute(&f.pool)
        .await
        .unwrap();
    let (_, denied) = command(
        &f.app,
        "POST",
        "/api/assets/urls",
        "files-peer",
        input.clone(),
    )
    .await;
    assert_eq!(denied["urls"], json!({}));
    sqlx::query("UPDATE public.messages SET deleted_at=NULL WHERE external_id=$1")
        .bind(sent["id"].as_str())
        .execute(&f.pool)
        .await
        .unwrap();
    // Removing the forward also revokes the alternative URL grant.
    let (status, deleted) = command(
        &f.app,
        "DELETE",
        &format!(
            "/api/chat/channels/files-elsewhere/messages/{}",
            forward["id"].as_str().unwrap()
        ),
        "files-author",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{deleted}");
    assert!(deleted["forward"]["message"].is_null(), "{deleted}");
    let (_, denied) = command(
        &f.app,
        "POST",
        "/api/assets/urls",
        "files-peer",
        input.clone(),
    )
    .await;
    assert_eq!(denied["urls"], json!({}));
    // Source access and the author's retained versions are unaffected.
    let (_, own) = command(&f.app, "POST", "/api/assets/urls", "files-author", input).await;
    assert_eq!(own["urls"].as_object().unwrap().len(), 4, "{own}");
    f.close().await;
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn replaced_files_stay_the_authors_alone_everywhere_they_were_shown() {
    let f = files_fixture().await;
    for (id, content_type) in [
        ("photo-b", "image/avif"),
        ("clip-v", "video/mp4"),
        ("photo-n1", "image/avif"),
    ] {
        f.upload(f.author, f.channel, id, content_type).await;
    }
    let sent = f.send(&["photo-b", "clip-v"], &[]).await;
    let message = sent["id"].as_str().unwrap();
    let message_id: i64 = sqlx::query_scalar("SELECT id FROM public.messages WHERE external_id=$1")
        .bind(message)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    // Queued before the edit: it shows what the message shows when it is read.
    let alert: i64 = sqlx::query_scalar("INSERT INTO public.notifications(user_id,kind,message_id,channel_id,space_id,actor_id) VALUES($1,'channel.message',$2,$3,$4,$5) RETURNING id")
        .bind(f.peer).bind(message_id).bind(f.channel).bind(f.space).bind(f.author).fetch_one(&f.pool).await.unwrap();
    let (status, forward) = command(
        &f.app,
        "POST",
        "/api/chat/channels/files-elsewhere/forwards",
        "files-peer",
        json!({"sourceChannelId":"files-channel","sourceMessageId":message,"clientMessageId":Uuid::new_v4()}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{forward}");
    let forward_id: i64 = sqlx::query_scalar("SELECT id FROM public.messages WHERE external_id=$1")
        .bind(forward["id"].as_str())
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let forward_alert: i64 = sqlx::query_scalar("INSERT INTO public.notifications(user_id,kind,message_id,channel_id,space_id,actor_id) VALUES($1,'channel.message',$2,$3,$4,$5) RETURNING id")
        .bind(f.author).bind(forward_id).bind(f.elsewhere).bind(f.space).bind(f.peer).fetch_one(&f.pool).await.unwrap();
    let (status, edited) = f
        .edit(
            message,
            "files-author",
            1,
            "Trip",
            json!([{"id":"photo-n1","replaces":"photo-b","edit":turned()},{"id":"clip-v"}]),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{edited}");

    // URL refresh: the author can still open the first-sent file; others can't.
    for (token, expected) in [
        ("files-author", vec!["photo-b", "photo-n1"]),
        ("files-peer", vec!["photo-n1"]),
    ] {
        let (status, refreshed) = command(
            &f.app,
            "POST",
            "/api/assets/urls",
            token,
            json!({"ids":["photo-b","photo-n1"]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{refreshed}");
        let mut got: Vec<&str> = refreshed["urls"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        got.sort_unstable();
        assert_eq!(got, expected, "{token}");
    }

    // History: others see that files changed and a placeholder, never a URL.
    let versions = format!("{FILES_PATH}/{message}/versions");
    let (status, theirs) = command(&f.app, "GET", &versions, "files-peer", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{theirs}");
    assert_eq!(theirs["versions"][0]["revision"], 2);
    assert_eq!(theirs["versions"][0]["filesChanged"], true);
    assert!(theirs["versions"][1].get("filesChanged").is_none());
    assert_eq!(ids(&theirs["versions"][0]), ["photo-n1", "clip-v"]);
    assert_eq!(
        theirs["versions"][1]["content"]["attachments"][0],
        placeholder("photo-b", "image", "image/avif")
    );
    assert_eq!(
        theirs["versions"][1]["content"]["attachments"][1]["name"],
        "clip-v.bin"
    );
    assert!(!theirs.to_string().contains("\"url\""), "{theirs}");
    // The author sees every version's files, signed.
    let (status, own) = command(&f.app, "GET", &versions, "files-author", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{own}");
    assert_eq!(own["versions"][0]["filesChanged"], true);
    let first = &own["versions"][1]["content"]["attachments"][0];
    assert_eq!(first["name"], "photo-b.bin");
    assert!(first["url"].as_str().unwrap().contains("/assets/photo-b?"));
    assert!(
        first["previewUrl"]
            .as_str()
            .unwrap()
            .contains("/preview/photo-b?")
    );
    assert!(own["versions"][0]["content"]["attachments"][0]["url"].is_string());

    // Re-editing starts from the first-sent file and the stored edit.
    let sources = format!("{FILES_PATH}/{message}/attachment-sources");
    let (status, editor) = command(&f.app, "GET", &sources, "files-author", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{editor}");
    assert_eq!(editor["messageId"], message);
    assert_eq!(editor["revision"], 2);
    let entries = editor["sources"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["attachmentId"], "photo-n1");
    assert_eq!(entries[0]["source"]["id"], "photo-b");
    assert_eq!(entries[0]["source"]["size"], 1000);
    assert!(
        entries[0]["source"]["url"]
            .as_str()
            .unwrap()
            .contains("/assets/photo-b?")
    );
    assert_eq!(edit_of(&entries[0]["edit"]), edit_of(&turned()));
    assert_eq!(entries[1]["attachmentId"], "clip-v");
    assert_eq!(entries[1]["source"]["id"], "clip-v");
    assert!(entries[1]["source"]["previewUrl"].is_string());
    assert!(entries[1]["edit"].is_null());
    let (status, _) = command(&f.app, "GET", &sources, "files-peer", Value::Null).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let forward_sources = format!(
        "/api/chat/channels/files-elsewhere/messages/{}/attachment-sources",
        forward["id"].as_str().unwrap()
    );
    let (status, _) = command(&f.app, "GET", &forward_sources, "files-peer", Value::Null).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "forwards are not editable");

    // Notifications show the current file, including a forward whose stored
    // snapshot predates the edit until the publisher projects it.
    let alerts = crate::push::live::read(&f.pool, f.peer, 0, alert, Some(&f.signer))
        .await
        .unwrap();
    let shown = alerts[0].1.as_ref().unwrap();
    assert!(
        shown["attachmentUrl"]
            .as_str()
            .unwrap()
            .contains("/assets/photo-n1?"),
        "{shown}"
    );
    let alerts = crate::push::live::read(&f.pool, f.author, alert, forward_alert, Some(&f.signer))
        .await
        .unwrap();
    let shown = alerts[0].1.as_ref().unwrap().to_string();
    assert!(
        !shown.contains("photo-b") && shown.contains("/preview/clip-v?"),
        "{shown}"
    );

    // Replaced files are kept: never purged, still counted, never cancelled.
    sqlx::query("UPDATE public.assets SET created_at=now()-interval '3 days',uploaded_at=now()-interval '3 days'")
        .execute(&f.pool).await.unwrap();
    crate::assets::purge(&f.assets).await.unwrap();
    let (deleted, purged): (bool, bool) = sqlx::query_as("SELECT deleted_at IS NOT NULL,purged_at IS NOT NULL FROM public.assets WHERE external_id='photo-b'")
        .fetch_one(&f.pool).await.unwrap();
    assert!(!deleted && !purged);
    let (status, usage) = command(
        &f.app,
        "GET",
        "/api/assets/usage",
        "files-author",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{usage}");
    assert_eq!(usage["used"], 3 * 1100, "the replaced photo still counts");
    for id in ["photo-b", "photo-n1"] {
        let (status, body) = command(
            &f.app,
            "POST",
            &format!("/api/assets/{id}/cancel"),
            "files-author",
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{id}: {body}");
    }

    // Replays hand nobody the replaced file: the original event shows a
    // placeholder in its slot and the edit event the current files.
    let gateway = crate::gateway::Gateway::new(f.chat.clone());
    gateway.start();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(axum::serve(listener, crate::gateway::router(gateway)).into_future());
    for token in ["files-peer", "files-author"] {
        let frames = replayed(address, "files-channel", token).await;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["type"], "message.created");
        let files = &frames[0]["message"]["content"]["attachments"];
        assert_eq!(files[0], placeholder("photo-b", "image", "image/avif"));
        assert!(
            files[1]["url"]
                .as_str()
                .unwrap()
                .contains("/assets/clip-v?")
        );
        assert_eq!(frames[1]["type"], "message.edited");
        assert!(
            frames[1]["message"]["content"]["attachments"][0]["url"]
                .as_str()
                .unwrap()
                .contains("/assets/photo-n1?")
        );
        assert!(!Value::Array(frames).to_string().contains("/assets/photo-b"));
    }
    let frames = replayed(address, "files-elsewhere", "files-peer").await;
    assert_eq!(
        frames[0]["message"]["forward"]["message"]["content"]["attachments"][0],
        placeholder("photo-b", "image", "image/avif"),
        "a forward's snapshot is concealed too"
    );

    // The publisher conceals the same way, then projects the edit into the forward.
    let mut feed = redis::Client::open(std::env::var("CHAT_TEST_VALKEY_URL").unwrap())
        .unwrap()
        .get_async_pubsub()
        .await
        .unwrap();
    for channel in ["files-channel", "files-elsewhere"] {
        feed.subscribe(format!("{TOPIC}:{channel}")).await.unwrap();
    }
    let mut published = feed.on_message();
    while publish_pending(&f.chat).await.unwrap() {}
    let mut events = Vec::new();
    while events.len() < 4 {
        let next = tokio::time::timeout(Duration::from_secs(5), published.next())
            .await
            .expect("published event")
            .unwrap();
        events.push(serde_json::from_str::<Value>(&next.get_payload::<String>().unwrap()).unwrap());
    }
    events.sort_by_key(|event| {
        (
            event["channelId"].as_str().unwrap().to_owned(),
            event["seq"].as_str().unwrap().parse::<i64>().unwrap(),
        )
    });
    assert_eq!(
        events
            .iter()
            .map(|e| (
                e["channelId"].as_str().unwrap(),
                e["type"].as_str().unwrap()
            ))
            .collect::<Vec<_>>(),
        [
            ("files-channel", "message.created"),
            ("files-channel", "message.edited"),
            ("files-elsewhere", "message.created"),
            ("files-elsewhere", "message.forward"),
        ]
    );
    assert_eq!(
        events[0]["message"]["content"]["attachments"][0],
        placeholder("photo-b", "image", "image/avif")
    );
    assert_eq!(
        events[2]["message"]["forward"]["message"]["content"]["attachments"][0],
        placeholder("photo-b", "image", "image/avif")
    );
    assert_eq!(
        ids(&events[3]["message"]["forward"]["message"]),
        ["photo-n1", "clip-v"],
        "the projected forward shows the current files"
    );
    let (_, history) = command(
        &f.app,
        "GET",
        "/api/chat/channels/files-elsewhere/messages",
        "files-peer",
        Value::Null,
    )
    .await;
    assert_eq!(
        ids(&history["messages"][0]["forward"]["message"]),
        ["photo-n1", "clip-v"]
    );
    let (_, history) = command(&f.app, "GET", FILES_PATH, "files-peer", Value::Null).await;
    assert_eq!(ids(&history["messages"][0]), ["photo-n1", "clip-v"]);
    assert!(!history.to_string().contains("photo-b"));
    server.abort();
    f.close().await;
}
