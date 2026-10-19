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
                .body(if matches!(method, "GET" | "DELETE") {
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
async fn deletion_keeps_the_row_scrubs_content_and_only_lets_the_author_delete() {
    let options =
        PgConnectOptions::from_str(&std::env::var("CHAT_TEST_DATABASE_URL").unwrap()).unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("deletion_test_{}", Uuid::new_v4().simple());
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
    for token in ["delete-author", "delete-peer", "delete-outsider"] {
        let id: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name,avatar_id) VALUES($1,$1,$1,255) RETURNING id")
            .bind(token).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(id).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,$1,$3,$2)")
            .bind(token).bind(&hash).bind(id).execute(&pool).await.unwrap();
    }
    let space: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) SELECT 'delete-space','Deletes',id FROM public.users WHERE external_id='delete-peer' RETURNING id")
        .fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.space_members(space_id,user_id) SELECT $1,id FROM public.users WHERE external_id!='delete-outsider'")
        .bind(space).execute(&pool).await.unwrap();
    let channel: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name) VALUES('delete-channel',$1,'deletes') RETURNING id")
        .bind(space).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) SELECT $1,id FROM public.users WHERE external_id!='delete-outsider'")
        .bind(channel).execute(&pool).await.unwrap();
    let broker_url = std::env::var("CHAT_TEST_VALKEY_URL").unwrap();
    let chat = Chat::new(pool.clone(), redis::Client::open(broker_url).unwrap());
    let mut state = AppState::new(
        crate::Config::test(false),
        Arc::new(crate::Cloudflare::new()),
    );
    state.chat = Some(chat.clone());
    let app = crate::app(state);
    let base = "/api/chat/channels/delete-channel/messages";
    let (status, root) = command(
        &app,
        "POST",
        base,
        "delete-author",
        json!({"clientMessageId":Uuid::new_v4(),"text":"secret plans @delete-peer"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let root_id = root["id"].as_str().unwrap().to_owned();
    let path = format!("{base}/{root_id}");
    let (status, reply) = command(
        &app,
        "POST",
        base,
        "delete-peer",
        json!({"clientMessageId":Uuid::new_v4(),"text":"noted","replyToId":root_id}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(reply["replyTo"]["text"], "secret plans @delete-peer");
    persist_pin(&pool, "delete-channel", &root_id, "delete-peer", true)
        .await
        .unwrap();
    let (status, forward) = command(&app, "POST", "/api/chat/channels/delete-channel/forwards", "delete-peer", json!({"sourceChannelId":"delete-channel","sourceMessageId":root_id,"clientMessageId":Uuid::new_v4()})).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        forward["forward"]["message"]["content"]["text"],
        "secret plans @delete-peer"
    );

    // Only the author deletes; others can't, outsiders can't even see it.
    assert_eq!(
        command(&app, "DELETE", &path, "delete-peer", Value::Null)
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        command(&app, "DELETE", &path, "delete-outsider", Value::Null)
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let (status, deleted) = command(&app, "DELETE", &path, "delete-author", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        deleted["content"],
        json!({"version":1,"type":"text","text":""})
    );
    assert_eq!(deleted["revision"], 2);
    assert_eq!(deleted["seq"], root["seq"]);
    assert!(deleted["deletedAt"].is_string());
    assert!(deleted["pin"].is_null());
    assert_eq!(deleted["editSeq"], "5");
    assert_eq!(deleted["pinSeq"], "6");
    // A retry returns the same tombstone without new events.
    let (status, again) = command(&app, "DELETE", &path, "delete-author", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again["deletedAt"], deleted["deletedAt"]);
    let last: i64 = sqlx::query_scalar("SELECT last_seq FROM public.channels WHERE id=$1")
        .bind(channel)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(last, 6);

    // The row and its content are retained; nothing readers get carries it.
    let (deleted_at, kept): (Option<chrono::DateTime<Utc>>, Value) = sqlx::query_as("SELECT m.deleted_at,v.content FROM public.messages m JOIN public.message_versions v ON v.message_id=m.id AND v.revision=1 WHERE m.external_id=$1")
        .bind(&root_id).fetch_one(&pool).await.unwrap();
    assert!(deleted_at.is_some());
    assert_eq!(kept["text"], "secret plans @delete-peer");
    let events: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM public.channel_events WHERE channel_id=$1 ORDER BY seq",
    )
    .bind(channel)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        events
            .iter()
            .all(|event| !event.to_string().contains("secret plans"))
    );
    let types: Vec<&str> = events
        .iter()
        .filter_map(|event| event["type"].as_str())
        .collect();
    assert_eq!(
        types,
        [
            "message.created",
            "message.created",
            "message.pin",
            "message.created",
            "message.edited",
            "message.pin"
        ]
    );
    let (status, history) = command(&app, "GET", base, "delete-peer", Value::Null).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!history.to_string().contains("secret plans"));
    let quoted = history["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["id"] == reply["id"])
        .unwrap();
    assert_eq!(quoted["replyTo"]["deleted"], true);
    assert_eq!(quoted["replyTo"]["text"], "");
    let forwarded = history["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|message| message["id"] == forward["id"])
        .unwrap();
    assert!(forwarded["forward"]["message"].is_null());

    // A deleted message can't be edited, reacted to, pinned, quoted or have
    // its earlier versions read.
    assert_eq!(
        command(
            &app,
            "PUT",
            &path,
            "delete-author",
            json!({"text":"back","expectedRevision":2})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert!(
        persist_reaction(&pool, "delete-channel", &root_id, "delete-peer", "🚀", true)
            .await
            .is_err()
    );
    assert!(
        persist_pin(&pool, "delete-channel", &root_id, "delete-peer", true)
            .await
            .is_err()
    );
    assert_eq!(
        command(
            &app,
            "POST",
            base,
            "delete-peer",
            json!({"clientMessageId":Uuid::new_v4(),"text":"hm","replyToId":root_id})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        command(
            &app,
            "GET",
            &format!("{path}/versions"),
            "delete-author",
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    drop(pool);
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}
