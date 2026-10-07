use super::*;
use crate::{Cloudflare, Config};
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use futures_util::StreamExt;
use tower::ServiceExt;

async fn request(
    app: &Router,
    method: &str,
    path: &str,
    user: &str,
    body: Value,
) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {user}"))
                .header("x-caper-chat-token", user)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
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

#[test]
fn shared_reply_never_grants_parent_or_sibling_access() {
    let original = json!({"id":"reply","clientMessageId":"retry","threadRootId":"private-root","broadcast":true,"thread":{"replyCount":20},"pin":{"author":"private"},"pinSeq":"8","forward":{"message":{"text":"private"}},"forwardSeq":"9","content":{"text":"shared"},"revision":4,"reactions":[{"emoji":"👀","authorIds":["alice"]}]});
    let shared = shared_message(original);
    assert_eq!(shared["content"]["text"], "shared");
    assert_eq!(shared["revision"], 4);
    assert_eq!(shared["reactions"][0]["emoji"], "👀");
    assert!(shared.get("threadRootId").is_none());
    assert!(shared.get("thread").is_none());
    assert!(shared.get("pin").is_none());
    assert!(shared.get("forward").is_none());
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and Valkey on 6388 (or FORWARD_TEST_VALKEY_URL)"]
async fn live_forwards_are_destination_authorized_flattened_and_replayable(pool: PgPool) {
    assert!(matches!(
        reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap())
            .unwrap()
            .host_str(),
        Some("127.0.0.1" | "localhost")
    ));
    let mut users = Vec::new();
    for name in ["alice", "bob", "carol", "dave"] {
        let user: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name,avatar_id) VALUES($1,$1,$1,31) RETURNING id")
            .bind(name).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(name.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,user_id,account_session_hash,name) VALUES($1,$2,$3,$2,$1)")
            .bind(name).bind(hash).bind(user).execute(&pool).await.unwrap();
        users.push(user);
    }
    let mut channels = Vec::new();
    for (name, members) in [
        ("source", vec![users[0], users[1]]),
        ("destination", vec![users[0], users[2]]),
    ] {
        let space: i64 = sqlx::query_scalar(
            "INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,$1,$2) RETURNING id",
        )
        .bind(name)
        .bind(users[0])
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO public.space_members(space_id,user_id) SELECT $1,unnest($2::bigint[])",
        )
        .bind(space)
        .bind(&members)
        .execute(&pool)
        .await
        .unwrap();
        let id: i64=sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name,private) VALUES($1,$2,$1,true) RETURNING id")
            .bind(name).bind(space).fetch_one(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO public.channel_members(channel_id,user_id) SELECT $1,unnest($2::bigint[])",
        )
        .bind(id)
        .bind(&members)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO public.channel_joins(channel_id,user_id) SELECT $1,unnest($2::bigint[])",
        )
        .bind(id)
        .bind(members)
        .execute(&pool)
        .await
        .unwrap();
        channels.push(id);
    }
    let original = persist(
        &pool,
        "source",
        "alice",
        Uuid::new_v4(),
        "The live original",
    )
    .await
    .unwrap();
    let unrelated = persist(
        &pool,
        "source",
        "bob",
        Uuid::new_v4(),
        "Unrelated private history",
    )
    .await
    .unwrap();
    let first_reply = persist_message(
        &pool,
        "source",
        "bob",
        Uuid::new_v4(),
        "First reply",
        original["id"].as_str(),
        false,
    )
    .await
    .unwrap();
    let mut config = Config::test(false);
    config.auth_fixture = false;
    let mut state =
        AppState::with_database(config, Arc::new(Cloudflare::new()), Some(pool.clone()));
    let broker_url = std::env::var("FORWARD_TEST_VALKEY_URL")
        .unwrap_or_else(|_| "redis://127.0.0.1:6388".into());
    assert!(matches!(
        reqwest::Url::parse(&broker_url).unwrap().host_str(),
        Some("127.0.0.1" | "localhost")
    ));
    state.chat = Some(Chat {
        pool: pool.clone(),
        broker: redis::Client::open(broker_url).unwrap(),
        wake: Arc::new(Notify::new()),
    });
    let chat = state.chat.clone().unwrap();
    let app = crate::app(state);
    let key = Uuid::new_v4();
    let input = json!({"sourceChannelId":"source","sourceMessageId":original["id"],"clientMessageId":key,"text":"Watch this conversation"});
    let (status, forward) = request(
        &app,
        "POST",
        "/api/chat/channels/destination/forwards",
        "alice",
        input.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{forward}");
    assert_eq!(
        forward["forward"]["message"]["content"]["text"],
        "The live original"
    );
    assert_eq!(forward["forward"]["message"]["thread"]["replyCount"], 1);
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/chat/channels/destination/forwards",
            "alice",
            input.clone()
        )
        .await
        .1["id"],
        forward["id"]
    );
    let mut changed = input.clone();
    changed["text"] = json!("different");
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/chat/channels/destination/forwards",
            "alice",
            changed
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    let path = format!(
        "/api/chat/channels/destination/forwards/{}/thread",
        forward["id"].as_str().unwrap()
    );
    assert_eq!(
        request(
            &app,
            "PUT",
            &format!(
                "/api/chat/channels/destination/messages/{}",
                forward["id"].as_str().unwrap()
            ),
            "alice",
            json!({"text":"Cannot edit even my own wrapper note", "expectedRevision":1})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let (status, view) = request(&app, "GET", &path, "carol", Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["messages"].as_array().unwrap().len(), 1);
    assert_eq!(view["messages"][0]["content"]["text"], "First reply");
    assert!(!view.to_string().contains("Unrelated private history"));
    assert_eq!(
        request(
            &app,
            "GET",
            "/api/chat/channels/source/messages",
            "carol",
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(&app, "GET", &path, "dave", Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    let guessed = format!(
        "/api/chat/channels/destination/forwards/{}/thread",
        unrelated["id"].as_str().unwrap()
    );
    assert_eq!(
        request(&app, "GET", &guessed, "carol", Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        request(
            &app,
            "POST",
            "/api/chat/channels/destination/forwards",
            "carol",
            input.clone()
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "knowing a source ID is not permission to forward it"
    );
    let (status, dm) = request(
        &app,
        "POST",
        "/api/dms",
        "carol",
        json!({"username":"dave"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{dm}");
    let dm_id = dm["id"].as_str().unwrap();
    let (status,again)=request(&app,"POST",&format!("/api/chat/channels/{dm_id}/forwards"),"carol",json!({"sourceChannelId":"destination","sourceMessageId":forward["id"],"clientMessageId":Uuid::new_v4()})).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["forward"]["message"]["id"], original["id"]);
    assert!(again["forward"]["message"].get("forward").is_none());
    let dm_path = format!(
        "/api/chat/channels/{dm_id}/forwards/{}/thread",
        again["id"].as_str().unwrap()
    );
    assert_eq!(
        request(&app, "GET", &dm_path, "dave", Value::Null).await.0,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &app,
            "PUT",
            &format!(
                "/api/chat/channels/source/messages/{}/reactions",
                original["id"].as_str().unwrap()
            ),
            "dave",
            json!({"emoji":"👀","active":true})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let reaction = persist_reaction(
        &pool,
        "source",
        original["id"].as_str().unwrap(),
        "bob",
        "👀",
        true,
    )
    .await
    .unwrap();
    let reaction_seq = reaction["seq"].as_str().unwrap().parse().unwrap();
    let event = reaction;
    let mut tx = pool.begin().await.unwrap();
    project_events(&mut tx, &[(channels[0], reaction_seq, event.clone(), None)])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let history = history_page(&pool, "destination", None, Some(users[2]))
        .await
        .unwrap();
    assert_eq!(
        history["messages"][0]["forward"]["message"]["reactions"][0]["emoji"],
        "👀"
    );
    assert_eq!(
        history["messages"][0]["seq"], "1",
        "source updates do not move the destination message"
    );
    assert_eq!(
        history["messages"][0]["content"]["text"],
        "Watch this conversation"
    );
    let update: Value = sqlx::query_scalar(
        "SELECT payload FROM public.channel_events WHERE channel_id=$1 ORDER BY seq DESC LIMIT 1",
    )
    .bind(channels[1])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(update["type"], "message.forward");
    assert_eq!(update["seq"], "2");
    assert_eq!(update["message"]["forwardSeq"], "2");
    assert_eq!(
        update["message"]["forward"]["message"]["id"],
        original["id"]
    );
    // Replayed outbox work emits no duplicate destination revision.
    let mut tx = pool.begin().await.unwrap();
    project_events(&mut tx, &[(channels[0], reaction_seq, event, None)])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        history_page(&pool, "destination", None, Some(users[2]))
            .await
            .unwrap()["cursor"],
        "2"
    );
    // The real author-edit handler persists history and projects its outbox.
    let edit_path = format!(
        "/api/chat/channels/source/messages/{}",
        original["id"].as_str().unwrap()
    );
    let (status, edited) = request(
        &app,
        "PUT",
        &edit_path,
        "alice",
        json!({"text":"Edited live original", "expectedRevision":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["revision"], 2);
    assert_eq!(edited["seq"], original["seq"]);
    assert_eq!(edited["thread"], forward["forward"]["message"]["thread"]);
    assert_eq!(
        request(
            &app,
            "GET",
            &format!("{edit_path}/versions"),
            "carol",
            Value::Null
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "sharing current content does not grant source history"
    );
    let versions = request(
        &app,
        "GET",
        &format!("{edit_path}/versions"),
        "alice",
        Value::Null,
    )
    .await
    .1;
    assert_eq!(
        versions["versions"][0]["content"]["text"],
        "Edited live original"
    );
    assert_eq!(
        versions["versions"][1]["content"]["text"],
        "The live original"
    );
    while publish_pending(&chat).await.unwrap() {}
    assert_eq!(
        history_page(&pool, "destination", None, Some(users[2]))
            .await
            .unwrap()["messages"][0]["forward"]["message"]["content"]["text"],
        "Edited live original"
    );
    // Existing and future reply pagination is selected by root, not channel.
    for index in 0i64..53 {
        persist_message(
            &pool,
            "source",
            if index % 2 == 0 { "alice" } else { "bob" },
            Uuid::new_v4(),
            &format!("reply {index}"),
            original["id"].as_str(),
            false,
        )
        .await
        .unwrap();
    }
    let page = request(&app, "GET", &path, "carol", Value::Null).await.1;
    assert_eq!(page["messages"].as_array().unwrap().len(), 50);
    assert_eq!(page["messages"][0]["content"]["text"], "reply 3");
    assert_eq!(page["messages"][49]["content"]["text"], "reply 52");
    assert_eq!(page["hasMore"], true);
    let earlier = request(
        &app,
        "GET",
        &format!(
            "{path}?before={}",
            page["messages"][0]["seq"].as_str().unwrap()
        ),
        "carol",
        Value::Null,
    )
    .await
    .1;
    assert_eq!(earlier["messages"].as_array().unwrap().len(), 4);
    assert_eq!(earlier["messages"][0]["content"]["text"], "First reply");
    assert_eq!(earlier["hasMore"], false);
    let root_summary = page["root"]["thread"].clone();
    let (status, edited_reply) = request(
        &app,
        "PUT",
        &format!(
            "/api/chat/channels/source/messages/{}",
            first_reply["id"].as_str().unwrap()
        ),
        "bob",
        json!({"text":"Edited first reply", "expectedRevision":1}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited_reply}");
    assert_eq!(edited_reply["seq"], first_reply["seq"]);
    let earlier = request(
        &app,
        "GET",
        &format!(
            "{path}?before={}",
            page["messages"][0]["seq"].as_str().unwrap()
        ),
        "carol",
        Value::Null,
    )
    .await
    .1;
    assert_eq!(
        earlier["messages"][0]["content"]["text"],
        "Edited first reply"
    );
    assert_eq!(
        earlier["root"]["thread"], root_summary,
        "content edits do not alter reply count/revision"
    );
    // Exercise the real publisher and Valkey fanout, not just the projection.
    while publish_pending(&chat).await.unwrap() {}
    let mut subscriber = chat.broker.get_async_pubsub().await.unwrap();
    subscriber
        .subscribe(format!("{TOPIC}:destination"))
        .await
        .unwrap();
    persist_reaction(
        &pool,
        "source",
        original["id"].as_str().unwrap(),
        "bob",
        "🎉",
        true,
    )
    .await
    .unwrap();
    while publish_pending(&chat).await.unwrap() {}
    let mut stream = subscriber.on_message();
    let published = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .unwrap()
        .unwrap();
    let projected: Value =
        serde_json::from_str(&published.get_payload::<String>().unwrap()).unwrap();
    assert_eq!(projected["type"], "message.forward");
    assert_eq!(projected["channelId"], "destination");
    assert_eq!(
        projected["message"]["forward"]["message"]["content"]["text"],
        "Edited live original"
    );
    assert_eq!(
        projected["message"]["forward"]["message"]["thread"]["replyCount"],
        54
    );
    assert!(
        projected["message"]["forward"]["message"]["reactions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reaction| reaction["emoji"] == "🎉")
    );
    assert_eq!(projected["message"]["seq"], "1");
    assert!(
        !publish_pending(&chat).await.unwrap(),
        "drained replay creates no duplicate events"
    );
    let destination_reply = persist_message(
        &pool,
        "destination",
        "carol",
        Uuid::new_v4(),
        "Destination-only reply",
        forward["id"].as_str(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(destination_reply["threadRootId"], forward["id"]);
    let local_path = format!(
        "/api/chat/channels/destination/messages/{}/thread",
        forward["id"].as_str().unwrap()
    );
    let local_thread = request(&app, "GET", &local_path, "carol", Value::Null)
        .await
        .1;
    assert_eq!(local_thread["messages"].as_array().unwrap().len(), 1);
    assert_eq!(
        local_thread["messages"][0]["content"]["text"],
        "Destination-only reply"
    );
    assert_eq!(local_thread["root"]["thread"]["replyCount"], 1);
    assert_eq!(
        request(&app, "GET", &path, "carol", Value::Null).await.1["root"]["thread"]["replyCount"],
        54
    );
    // Removing destination membership revokes scoped reads, not just browsing.
    sqlx::query("UPDATE public.space_members SET deleted_at=now() WHERE user_id=$1")
        .bind(users[2])
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(&app, "GET", &path, "carol", Value::Null).await.0,
        StatusCode::NOT_FOUND
    );
    // The independent DM forward remains usable, even after the intermediate
    // conversation becomes inaccessible to the person who re-forwarded it.
    assert_eq!(
        request(&app, "GET", &dm_path, "dave", Value::Null).await.0,
        StatusCode::OK
    );
}
