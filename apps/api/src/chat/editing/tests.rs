use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use sqlx::{
    Connection, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::str::FromStr;
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
    let chat = Chat {
        pool: pool.clone(),
        broker: redis::Client::open(broker_url).unwrap(),
        wake: Arc::new(Notify::new()),
    };
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
        .execute(format!("DROP DATABASE {database}").as_str())
        .await
        .unwrap();
}
