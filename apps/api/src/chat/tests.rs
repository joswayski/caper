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
fn reactions_accept_one_standard_emoji_and_canonicalize_qualification() {
    assert_eq!(normalized_emoji("🚀").unwrap(), "🚀");
    assert_eq!(normalized_emoji("🐿").unwrap(), "🐿️");
    assert_eq!(normalized_emoji("👩‍💻").unwrap(), "👩‍💻");
    assert_eq!(normalized_emoji("👍🏽").unwrap(), "👍🏽");
    assert_eq!(normalized_emoji("🫨").unwrap(), "🫨");
    for invalid in ["", "hello", "🚀🚀", ":rocket:", "A️", "🫩"] {
        assert!(normalized_emoji(invalid).is_err(), "accepted {invalid:?}");
    }
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

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL"]
async fn threads_isolate_replies_and_broadcast_once_with_transactional_summaries(pool: PgPool) {
    let mut users = Vec::new();
    for name in ["alice", "bob", "preview", "outsider"] {
        let user: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name,avatar_id) VALUES($1,$1,$1,$2) RETURNING id")
            .bind(name).bind(31 + users.len() as i16).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(name.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,$1,$3,$2)")
            .bind(name).bind(hash).bind(user).execute(&pool).await.unwrap();
        users.push(user);
    }
    let space: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES('thread-space','Threads',$1) RETURNING id")
        .bind(users[0]).fetch_one(&pool).await.unwrap();
    for user in &users[..3] {
        sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
            .bind(space)
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
    }
    for channel in ["thread-channel", "other-channel"] {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,$1) RETURNING id",
        )
        .bind(channel)
        .bind(space)
        .fetch_one(&pool)
        .await
        .unwrap();
        for user in &users[..2] {
            sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2)")
                .bind(id)
                .bind(user)
                .execute(&pool)
                .await
                .unwrap();
        }
    }
    let channel = "thread-channel";
    let root = persist(&pool, channel, "alice", Uuid::new_v4(), "parent")
        .await
        .unwrap();
    let root = root["id"].as_str().unwrap();
    let other = persist(&pool, channel, "alice", Uuid::new_v4(), "different parent")
        .await
        .unwrap();
    let other = other["id"].as_str().unwrap();
    let empty = conversation_page(&pool, channel, None, Some(users[2]), Some(root))
        .await
        .unwrap();
    assert_eq!(empty["messages"], json!([]));
    assert_eq!(empty["root"]["id"], root);

    let key = Uuid::new_v4();
    let (one, retry) = tokio::join!(
        persist_message(&pool, channel, "bob", key, "thread only", Some(root), false),
        persist_message(&pool, channel, "bob", key, "thread only", Some(root), false)
    );
    let one = one.unwrap();
    assert_eq!(one, retry.unwrap());
    assert_eq!(one["seq"], "3");
    assert_eq!(one["threadRootId"], root);
    assert_eq!(one["thread"]["replyCount"], 1);
    let two = persist_message(
        &pool,
        channel,
        "alice",
        Uuid::new_v4(),
        "also in channel",
        Some(root),
        true,
    )
    .await
    .unwrap();
    let three = persist_message(
        &pool,
        channel,
        "bob",
        Uuid::new_v4(),
        "another thread",
        Some(other),
        false,
    )
    .await
    .unwrap();
    assert_eq!(three["thread"]["replyCount"], 1);
    let history = history_page(&pool, channel, None, Some(users[2]))
        .await
        .unwrap();
    assert_eq!(
        history["cursor"], "5",
        "thread-only events still advance the shared replay head"
    );
    let ids: Vec<&str> = history["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| message["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [root, other, two["id"].as_str().unwrap()]);
    assert_eq!(history["messages"][0]["thread"]["replyCount"], 2);
    assert_eq!(history["messages"][0]["thread"]["seq"], "4");
    assert_eq!(
        history["messages"][0]["thread"]["participants"],
        json!([
            {"id":"bob","name":"bob","isGuest":false,"avatarId":32},
            {"id":"alice","name":"alice","isGuest":false,"avatarId":31}
        ])
    );
    let thread = conversation_page(&pool, channel, None, Some(users[2]), Some(root))
        .await
        .unwrap();
    assert_eq!(thread["messages"], json!([one, two]));
    assert_eq!(thread["root"], history["messages"][0]);
    assert_eq!(thread["hasMore"], false);
    let before = conversation_page(&pool, channel, Some(4), Some(users[0]), Some(root))
        .await
        .unwrap();
    assert_eq!(before["messages"], json!([one]));
    let reaction = persist_reaction(
        &pool,
        channel,
        one["id"].as_str().unwrap(),
        "alice",
        "👍",
        true,
    )
    .await
    .unwrap();
    let reacted = conversation_page(&pool, channel, None, Some(users[0]), Some(root))
        .await
        .unwrap();
    assert_eq!(reacted["messages"][0]["reactions"], reaction["reactions"]);

    for (target, broadcast, text) in [
        (root, true, "thread only"),
        (other, false, "thread only"),
        (root, false, "changed"),
    ] {
        assert_eq!(
            persist_message(&pool, channel, "bob", key, text, Some(target), broadcast)
                .await
                .unwrap_err()
                .status,
            StatusCode::CONFLICT
        );
    }
    assert_eq!(
        persist(&pool, channel, "bob", key, "thread only")
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    for (channel, target) in [
        (channel, one["id"].as_str().unwrap()),
        (channel, "absent"),
        ("other-channel", root),
    ] {
        assert_eq!(
            persist_message(
                &pool,
                channel,
                "alice",
                Uuid::new_v4(),
                "invalid",
                Some(target),
                false
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            conversation_page(&pool, channel, None, Some(users[0]), Some(target))
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
    }
    for reader in [None, Some(users[3])] {
        assert_eq!(
            conversation_page(&pool, channel, None, reader, Some(root))
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
    }
    for sender in ["preview", "outsider"] {
        assert_eq!(
            persist_message(
                &pool,
                channel,
                sender,
                Uuid::new_v4(),
                "denied",
                Some(root),
                false
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        persist_message(
            &pool,
            channel,
            "alice",
            Uuid::new_v4(),
            "invalid broadcast",
            None,
            true
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::BAD_REQUEST
    );
    // Outbox failure must not increment the parent summary or leave a reply.
    pool.execute("ALTER TABLE public.channel_events ADD CONSTRAINT thread_reject CHECK (seq < 7)")
        .await
        .unwrap();
    assert!(
        persist_message(
            &pool,
            channel,
            "alice",
            Uuid::new_v4(),
            "rollback",
            Some(root),
            false
        )
        .await
        .is_err()
    );
    let after = conversation_page(&pool, channel, None, Some(users[0]), Some(root))
        .await
        .unwrap();
    assert_eq!(after, reacted);
    pool.execute("ALTER TABLE public.channel_events DROP CONSTRAINT thread_reject")
        .await
        .unwrap();
    let events: Vec<Value> =
        sqlx::query_scalar("SELECT payload FROM public.channel_events ORDER BY seq")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(events[2]["message"], one);
    assert_eq!(events[3]["message"], two);
    assert_eq!(events.len(), 6);

    // Cross the 50-reply boundary with alternating authors and an older root.
    // The channel head includes replies, but the channel page must not include them.
    for index in 0..51 {
        let message = persist_message(
            &pool,
            channel,
            if index % 2 == 0 { "alice" } else { "bob" },
            Uuid::new_v4(),
            &format!("paged reply {index}"),
            Some(root),
            false,
        )
        .await
        .unwrap();
        assert_eq!(message["seq"], (7 + index).to_string());
    }
    let latest = conversation_page(&pool, channel, None, Some(users[0]), Some(root))
        .await
        .unwrap();
    assert_eq!(latest["hasMore"], true);
    assert_eq!(latest["cursor"], "57");
    assert_eq!(
        latest["root"]["seq"], "1",
        "summary revisions never revise root identity"
    );
    assert_eq!(latest["root"]["thread"]["replyCount"], 53);
    let replies = latest["messages"].as_array().unwrap();
    assert_eq!(replies.len(), 50);
    assert_eq!(replies[0]["seq"], "8");
    assert_eq!(replies[0]["content"]["text"], "paged reply 1");
    assert_eq!(replies[49]["seq"], "57");
    assert_eq!(replies[49]["content"]["text"], "paged reply 50");
    let older = conversation_page(&pool, channel, Some(8), Some(users[0]), Some(root))
        .await
        .unwrap();
    assert_eq!(older["hasMore"], false);
    assert_eq!(
        older["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|message| message["seq"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["3", "4", "7"]
    );
    let channel_page = history_page(&pool, channel, None, Some(users[0]))
        .await
        .unwrap();
    assert_eq!(channel_page["messages"].as_array().unwrap().len(), 3);
    assert_eq!(channel_page["cursor"], "57");
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL"]
async fn pins_are_shared_idempotent_authorized_and_transactional() {
    let options =
        PgConnectOptions::from_str(&std::env::var("CHAT_TEST_DATABASE_URL").unwrap()).unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("pins_test_{}", Uuid::new_v4().simple());
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
    for token in ["pin-owner", "pin-member"] {
        let id: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name,avatar_id) VALUES($1,$1,$1,255) RETURNING id")
            .bind(token).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(id).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,$1,$3,$2)")
            .bind(token).bind(&hash).bind(id).execute(&pool).await.unwrap();
        actors.push(id);
    }
    let owner = actors[0];
    let member = actors[1];
    let space: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES('pin-space','Pins',$1) RETURNING id")
        .bind(owner).fetch_one(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO public.space_members(space_id,user_id) SELECT $1,id FROM public.users",
    )
    .bind(space)
    .execute(&pool)
    .await
    .unwrap();
    let channel = "pin-channel";
    let channel_id: i64 = sqlx::query_scalar(
        "INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,'pins') RETURNING id",
    )
    .bind(channel)
    .bind(space)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO public.channel_joins(channel_id,user_id) SELECT $1,id FROM public.users",
    )
    .bind(channel_id)
    .execute(&pool)
    .await
    .unwrap();
    let original = persist(
        &pool,
        channel,
        "pin-owner",
        Uuid::new_v4(),
        "An old shared pin",
    )
    .await
    .unwrap();
    let message = original["id"].as_str().unwrap();
    let pin = persist_pin(&pool, channel, message, "pin-member", true)
        .await
        .unwrap();
    assert_eq!(pin["seq"], "2");
    assert_eq!(pin["message"]["seq"], "1");
    assert_eq!(pin["message"]["pinSeq"], "2");
    assert_eq!(pin["message"]["pin"]["author"]["id"], "pin-member");
    assert_eq!(pin["message"]["author"]["avatarId"], 255);
    let (one, two) = tokio::join!(
        persist_pin(&pool, channel, message, "pin-owner", true),
        persist_pin(&pool, channel, message, "pin-member", true)
    );
    assert_eq!(one.unwrap(), pin);
    assert_eq!(two.unwrap(), pin);
    persist_reaction(&pool, channel, message, "pin-owner", "🚀", true)
        .await
        .unwrap();
    let history = history_page(&pool, channel, None, Some(owner))
        .await
        .unwrap();
    assert_eq!(history["cursor"], "3");
    assert_eq!(history["pinnedMessages"][0]["reactionSeq"], "3");
    assert_eq!(history["messages"][0]["pin"], pin["message"]["pin"]);
    let unpin = persist_pin(&pool, channel, message, "pin-owner", false)
        .await
        .unwrap();
    assert_eq!(unpin["seq"], "4");
    assert!(unpin["message"]["pin"].is_null());
    assert_eq!(unpin["message"]["reactionSeq"], "3");
    pool.execute("ALTER TABLE public.channel_events ADD CONSTRAINT reject_pin CHECK (seq < 5)")
        .await
        .unwrap();
    assert!(
        persist_pin(&pool, channel, message, "pin-owner", true)
            .await
            .is_err()
    );
    let rolled_back = history_page(&pool, channel, None, Some(owner))
        .await
        .unwrap();
    assert_eq!(rolled_back["cursor"], "4");
    assert_eq!(rolled_back["pinnedMessages"], json!([]));
    let activity: i64 = sqlx::query_scalar("SELECT count(*) FROM public.message_pin_activity")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(activity, 2);
    pool.execute("ALTER TABLE public.channel_events DROP CONSTRAINT reject_pin")
        .await
        .unwrap();

    let broker_url = std::env::var("CHAT_TEST_VALKEY_URL").unwrap();
    assert!(
        broker_url.starts_with("redis://127.0.0.1:")
            || broker_url.starts_with("redis://localhost:")
    );
    let chat = Chat {
        pool: pool.clone(),
        broker: redis::Client::open(broker_url).unwrap(),
        wake: Arc::new(Notify::new()),
        cdn: None,
    };
    let mut state = AppState::new(
        crate::Config::test(false),
        Arc::new(crate::Cloudflare::new()),
    );
    state.chat = Some(chat.clone());
    let app = crate::app(state);
    for (token, body, expected) in [
        (None, json!({"active":true}), StatusCode::UNAUTHORIZED),
        (
            Some("pin-owner"),
            json!({"active":"true"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            Some("pin-owner"),
            json!({"active":true,"authorId":"spoofed"}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (Some("pin-owner"), json!({"active":false}), StatusCode::OK),
    ] {
        let mut request = axum::http::Request::builder()
            .method("PUT")
            .uri(format!(
                "/api/chat/channels/{channel}/messages/{message}/pin"
            ))
            .header("content-type", "application/json");
        if let Some(token) = token {
            request = request.header("x-caper-chat-token", token);
        }
        assert_eq!(
            app.clone()
                .oneshot(
                    request
                        .body(axum::body::Body::from(body.to_string()))
                        .unwrap()
                )
                .await
                .unwrap()
                .status(),
            expected
        );
    }
    // Two authenticated readers see the exact shared mutation and replay it.
    let gateway = crate::gateway::Gateway::new(chat.clone());
    gateway.start();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server =
        tokio::spawn(axum::serve(listener, crate::gateway::router(gateway.clone())).into_future());
    tokio::time::timeout(Duration::from_secs(5), async {
        while reqwest::get(format!("http://{address}/readyz"))
            .await
            .unwrap()
            .status()
            != StatusCode::NO_CONTENT
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let mut readers = Vec::new();
    for token in ["pin-owner", "pin-member"] {
        let (mut reader, _) = tokio_tungstenite::connect_async(account_socket(
            format!("ws://{address}/api/chat/events?channelId={channel}&after=3"),
            token,
        ))
        .await
        .unwrap();
        let replayed = event(&mut reader).await;
        assert_eq!(replayed["type"], "message.pin");
        assert_eq!(replayed["message"]["author"]["avatarId"], 255);
        assert!(replayed["message"]["pin"].is_null());
        assert_eq!(event(&mut reader).await["cursor"], "4");
        readers.push(reader);
    }
    let shared = persist_pin(&pool, channel, message, "pin-member", true)
        .await
        .unwrap();
    publish_pending(&chat).await.unwrap();
    for reader in &mut readers {
        assert_eq!(event(reader).await, shared);
    }
    persist_pin(&pool, channel, message, "pin-owner", false)
        .await
        .unwrap();
    for mut reader in readers {
        reader.close(None).await.unwrap();
    }
    gateway.begin_shutdown();
    server.abort();

    // Readable previews do not grant mutations; private grants gate reads too.
    sqlx::query(
        "UPDATE public.channel_joins SET deleted_at=now() WHERE channel_id=$1 AND user_id=$2",
    )
    .bind(channel_id)
    .bind(member)
    .execute(&pool)
    .await
    .unwrap();
    assert!(
        history_page(&pool, channel, None, Some(member))
            .await
            .is_ok()
    );
    for active in [true, false] {
        assert_eq!(
            persist_pin(&pool, channel, message, "pin-member", active)
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
    }
    pool.execute("UPDATE public.channels SET private=true")
        .await
        .unwrap();
    sqlx::query(
        "UPDATE public.channel_joins SET deleted_at=NULL WHERE channel_id=$1 AND user_id=$2",
    )
    .bind(channel_id)
    .bind(member)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO public.channel_members(channel_id,user_id,deleted_at) VALUES($1,$2,now())",
    )
    .bind(channel_id)
    .bind(member)
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        history_page(&pool, channel, None, Some(member))
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        persist_pin(&pool, channel, message, "pin-member", true)
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        persist_pin(&pool, channel, message, "wrong-token", true)
            .await
            .unwrap_err()
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        persist_pin(&pool, channel, "missing-message", "pin-owner", true)
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );

    // Disposable SQL fixtures bypass send rate limits, not production behavior.
    sqlx::query("INSERT INTO public.messages(external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload) SELECT 'pinfixture'||lpad(n::text,5,'0'),m.channel_id,m.session_id,md5(n::text)::uuid,m.request_hash,n,m.payload || jsonb_build_object('id','pinfixture'||lpad(n::text,5,'0'),'seq',n::text,'clientMessageId',md5(n::text)::uuid) FROM public.messages m CROSS JOIN generate_series(5,106) n WHERE m.external_id=$1")
        .bind(message).execute(&pool).await.unwrap();
    pool.execute("UPDATE public.channels SET last_seq=106")
        .await
        .unwrap();
    let repin = persist_pin(&pool, channel, message, "pin-owner", true)
        .await
        .unwrap();
    let page = history_page(&pool, channel, None, Some(member)).await;
    assert!(page.is_err());
    let page = history_page(&pool, channel, None, Some(owner))
        .await
        .unwrap();
    assert_eq!(page["messages"].as_array().unwrap().len(), 50);
    assert!(
        page["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["id"] != message)
    );
    assert_eq!(page["pinnedMessages"][0]["id"], message);
    assert_eq!(page["pinnedMessages"][0]["pinSeq"], "107");
    assert_eq!(page["cursor"], "107");
    sqlx::query("UPDATE public.messages SET payload=payload || jsonb_build_object('pin',$1::jsonb,'pinSeq',channel_seq::text) WHERE channel_seq BETWEEN 5 AND 103")
        .bind(&repin["message"]["pin"]).execute(&pool).await.unwrap();
    assert_eq!(
        persist_pin(&pool, channel, "pinfixture00106", "pin-owner", true)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    persist_pin(&pool, channel, message, "pin-owner", false)
        .await
        .unwrap();
    let new_pin = persist_pin(&pool, channel, "pinfixture00106", "pin-owner", true)
        .await
        .unwrap();
    let page = history_page(&pool, channel, None, Some(owner))
        .await
        .unwrap();
    assert_eq!(page["pinnedMessages"].as_array().unwrap().len(), 100);
    assert_eq!(page["pinnedMessages"][0]["id"], "pinfixture00106");

    // No-op retries remain free at the rate boundary; real toggles do not.
    sqlx::query("INSERT INTO public.message_pin_activity(message_id,user_id) SELECT m.id,$1 FROM public.messages m CROSS JOIN generate_series(1,60) WHERE m.external_id=$2")
        .bind(owner).bind(message).execute(&pool).await.unwrap();
    assert_eq!(
        persist_pin(&pool, channel, "pinfixture00106", "pin-owner", true)
            .await
            .unwrap(),
        new_pin
    );
    assert_eq!(
        persist_pin(&pool, channel, "pinfixture00106", "pin-owner", false)
            .await
            .unwrap_err()
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    pool.execute("UPDATE public.message_pin_activity SET created_at=now()-interval '2 minutes'")
        .await
        .unwrap();
    persist_pin(&pool, channel, "pinfixture00106", "pin-owner", false)
        .await
        .unwrap();

    // A revocation that wins the space lock must beat a waiting mutation.
    pool.execute("UPDATE public.channels SET private=false")
        .await
        .unwrap();
    sqlx::query(
        "UPDATE public.channel_joins SET deleted_at=NULL WHERE channel_id=$1 AND user_id=$2",
    )
    .bind(channel_id)
    .bind(member)
    .execute(&pool)
    .await
    .unwrap();
    let mut revocation = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM public.spaces WHERE id=$1 FOR UPDATE")
        .bind(space)
        .execute(&mut *revocation)
        .await
        .unwrap();
    let pending_pool = pool.clone();
    let pending_message = message.to_owned();
    let pending = tokio::spawn(async move {
        persist_pin(&pending_pool, channel, &pending_message, "pin-member", true).await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!pending.is_finished());
    sqlx::query(
        "UPDATE public.space_members SET deleted_at=now() WHERE space_id=$1 AND user_id=$2",
    )
    .bind(space)
    .bind(member)
    .execute(&mut *revocation)
    .await
    .unwrap();
    revocation.commit().await.unwrap();
    assert_eq!(
        pending.await.unwrap().unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    pool.close().await;
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL"]
async fn reactions_are_durable_idempotent_authorized_and_transactional() {
    let options =
        PgConnectOptions::from_str(&std::env::var("CHAT_TEST_DATABASE_URL").unwrap()).unwrap();
    assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("reactions_test_{}", Uuid::new_v4().simple());
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
    // Keep user primary keys distinct from chat-session keys and public IDs.
    pool.execute("ALTER TABLE public.users ALTER COLUMN id RESTART WITH 1000")
        .await
        .unwrap();
    // Keep the original actor IDs while exercising account-only channels.
    for token in ["guest-one", "guest-two"] {
        let user: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,$1,$1) RETURNING id")
            .bind(token).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,$1,$3,$2)")
            .bind(token).bind(&hash).bind(user).execute(&pool).await.unwrap();
    }
    let reader: i64 =
        sqlx::query_scalar("SELECT id FROM public.users WHERE external_id='guest-one'")
            .fetch_one(&pool)
            .await
            .unwrap();
    let shared_space: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES('reaction-space','Reactions',$1) RETURNING id")
        .bind(reader).fetch_one(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO public.space_members(space_id,user_id) SELECT $1,id FROM public.users",
    )
    .bind(shared_space)
    .execute(&pool)
    .await
    .unwrap();
    let channel = "reaction-channel".to_string();
    sqlx::query("INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,'reactions')")
        .bind(&channel)
        .bind(shared_space)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) SELECT c.id,u.id FROM public.channels c CROSS JOIN public.users u WHERE c.external_id=$1")
        .bind(&channel).execute(&pool).await.unwrap();
    let original = persist(
        &pool,
        &channel,
        "guest-one",
        Uuid::new_v4(),
        "reaction target",
    )
    .await
    .unwrap();
    let id = original["id"].as_str().unwrap();

    // A capability for one joined channel does not confer participation in
    // another readable channel. Reads remain available as a preview.
    let preview_channel = "reaction-preview-channel";
    let preview_channel_id: i64 = sqlx::query_scalar(
        "INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,'preview') RETURNING id",
    )
    .bind(preview_channel)
    .bind(shared_space)
    .fetch_one(&pool)
    .await
    .unwrap();
    let guest_two: i64 =
        sqlx::query_scalar("SELECT id FROM public.users WHERE external_id='guest-two'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2)")
        .bind(preview_channel_id)
        .bind(guest_two)
        .execute(&pool)
        .await
        .unwrap();
    let preview_message = persist(
        &pool,
        preview_channel,
        "guest-two",
        Uuid::new_v4(),
        "readable preview",
    )
    .await
    .unwrap();
    let preview_message_id = preview_message["id"].as_str().unwrap();
    assert!(
        history_page(&pool, preview_channel, None, Some(reader))
            .await
            .is_ok()
    );
    for active in [true, false] {
        assert_eq!(
            persist_reaction(
                &pool,
                preview_channel,
                preview_message_id,
                "guest-one",
                "👍",
                active,
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::NOT_FOUND
        );
    }
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2)")
        .bind(preview_channel_id)
        .bind(reader)
        .execute(&pool)
        .await
        .unwrap();
    persist_reaction(
        &pool,
        preview_channel,
        preview_message_id,
        "guest-one",
        "👍",
        true,
    )
    .await
    .unwrap();
    sqlx::query("UPDATE public.channel_joins SET deleted_at=now() WHERE channel_id=$1 AND user_id=$2 AND deleted_at IS NULL")
        .bind(preview_channel_id)
        .bind(reader)
        .execute(&pool)
        .await
        .unwrap();
    for active in [false, true] {
        assert_eq!(
            persist_reaction(
                &pool,
                preview_channel,
                preview_message_id,
                "guest-one",
                "👍",
                active,
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::NOT_FOUND
        );
    }

    // A reaction queued behind leave must use a fresh READ COMMITTED snapshot
    // after the space lock and leave every reaction-related row unchanged.
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2)")
        .bind(preview_channel_id)
        .bind(reader)
        .execute(&pool)
        .await
        .unwrap();
    let before: (Value, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT m.payload,c.last_seq,
                (SELECT count(*) FROM public.message_reactions WHERE message_id=m.id),
                (SELECT count(*) FROM public.message_reaction_activity WHERE message_id=m.id),
                (SELECT count(*) FROM public.channel_events WHERE channel_id=c.id)
         FROM public.messages m JOIN public.channels c ON c.id=m.channel_id
         WHERE m.external_id=$1",
    )
    .bind(preview_message_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let mut leaving = pool.begin().await.unwrap();
    let blocker: i32 =
        sqlx::query_scalar("SELECT pg_backend_pid() FROM public.spaces WHERE id=$1 FOR UPDATE")
            .bind(shared_space)
            .fetch_one(&mut *leaving)
            .await
            .unwrap();
    sqlx::query("UPDATE public.channel_joins SET deleted_at=now() WHERE channel_id=$1 AND user_id=$2 AND deleted_at IS NULL")
        .bind(preview_channel_id)
        .bind(reader)
        .execute(&mut *leaving)
        .await
        .unwrap();
    let reaction_pool = pool.clone();
    let queued_message = preview_message_id.to_owned();
    let queued = tokio::spawn(async move {
        persist_reaction(
            &reaction_pool,
            preview_channel,
            &queued_message,
            "guest-one",
            "👍",
            false,
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
    }).await.expect("reaction should wait behind leave");
    leaving.commit().await.unwrap();
    assert_eq!(
        queued.await.unwrap().unwrap_err().status,
        StatusCode::NOT_FOUND
    );
    let after: (Value, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT m.payload,c.last_seq,
                (SELECT count(*) FROM public.message_reactions WHERE message_id=m.id),
                (SELECT count(*) FROM public.message_reaction_activity WHERE message_id=m.id),
                (SELECT count(*) FROM public.channel_events WHERE channel_id=c.id)
         FROM public.messages m JOIN public.channels c ON c.id=m.channel_id
         WHERE m.external_id=$1",
    )
    .bind(preview_message_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        after, before,
        "denial must not mutate snapshot, sequence, reactions, activity, or outbox"
    );

    assert_eq!(
        persist_reaction(&pool, &channel, id, "wrong", "👍", true)
            .await
            .unwrap_err()
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        persist_reaction(&pool, &channel, "absent", "guest-one", "👍", true)
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        persist_reaction(&pool, &channel, id, "guest-one", "👍", false)
            .await
            .unwrap()["seq"],
        "0"
    );
    let (first, duplicate) = tokio::join!(
        persist_reaction(&pool, &channel, id, "guest-one", "👍", true),
        persist_reaction(&pool, &channel, id, "guest-one", "👍", true),
    );
    let first = first.unwrap();
    assert_eq!(first, duplicate.unwrap());
    assert_eq!(first["seq"], "2");
    assert_eq!(
        first["reactions"],
        json!([{"emoji":"👍","authorIds":["guest-one"]}])
    );
    for table in ["message_reactions", "message_reaction_activity"] {
        let stored_user: i64 = sqlx::query_scalar(&format!("SELECT user_id FROM public.{table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            stored_user, reader,
            "{table} must store the user's primary key"
        );
    }
    for statement in [
        "INSERT INTO public.message_reactions(message_id,emoji,user_id) SELECT id,'🤔',$2 FROM public.messages WHERE external_id=$1",
        "INSERT INTO public.message_reaction_activity(message_id,user_id) SELECT id,$2 FROM public.messages WHERE external_id=$1",
    ] {
        let error = sqlx::query(statement)
            .bind(id)
            .bind(i64::MAX)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(
            error
                .as_database_error()
                .and_then(|error| error.code())
                .as_deref(),
            Some("23503"),
            "both tables must reject a nonexistent user via a foreign key"
        );
    }
    let other = persist_reaction(&pool, &channel, id, "guest-two", "👍", true)
        .await
        .unwrap();
    assert_eq!(other["seq"], "3");
    assert_eq!(
        other["reactions"][0]["authorIds"],
        json!(["guest-one", "guest-two"])
    );
    let removed = persist_reaction(&pool, &channel, id, "guest-one", "👍", false)
        .await
        .unwrap();
    assert_eq!(removed["seq"], "4");
    assert_eq!(removed["reactions"][0]["authorIds"], json!(["guest-two"]));
    assert_eq!(
        removed,
        persist_reaction(&pool, &channel, id, "guest-one", "👍", false)
            .await
            .unwrap()
    );
    // Failure at the outbox must also roll back membership, payload and head.
    pool.execute("ALTER TABLE public.channel_events ADD CONSTRAINT reject_next CHECK(seq < 5)")
        .await
        .unwrap();
    assert!(
        persist_reaction(&pool, &channel, id, "guest-one", "🎉", true)
            .await
            .is_err()
    );
    pool.execute("ALTER TABLE public.channel_events DROP CONSTRAINT reject_next")
        .await
        .unwrap();
    // History must refresh the avatar without dropping the reaction snapshot.
    sqlx::query("UPDATE public.users SET avatar_id=719 WHERE id=$1")
        .bind(reader)
        .execute(&pool)
        .await
        .unwrap();
    let page = history_page(&pool, &channel, None, Some(reader))
        .await
        .unwrap();
    assert_eq!(page["cursor"], "4");
    assert_eq!(page["messages"][0]["seq"], "1");
    assert_eq!(page["messages"][0]["reactionSeq"], "4");
    assert_eq!(page["messages"][0]["reactions"], removed["reactions"]);
    assert_eq!(page["messages"][0]["author"]["avatarId"], 719);
    assert_eq!(page["messages"][0]["author"]["id"], "guest-one");
    let events: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM public.channel_events WHERE channel_id=(SELECT id FROM public.channels WHERE external_id=$1) ORDER BY seq",
    )
    .bind(&channel)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(events[1], first);
    assert_eq!(events[3], removed);
    let next = persist(
        &pool,
        &channel,
        "guest-one",
        Uuid::new_v4(),
        "after reactions",
    )
    .await
    .unwrap();
    assert_eq!(next["seq"], "5");

    // Multiple capabilities for one account are one actor, not two votes.
    let user: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES('account','reactor','Reactor') RETURNING id").fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
        .bind(shared_space)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) SELECT id,$2 FROM public.channels WHERE external_id=$1")
        .bind(&channel).bind(user).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
        .bind(b"parent".as_slice()).bind(user).execute(&pool).await.unwrap();
    for token in ["account-one", "account-two"] {
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,'Reactor',$3,$4)")
            .bind(token).bind(Sha256::digest(token.as_bytes()).as_slice()).bind(user).bind(b"parent".as_slice()).execute(&pool).await.unwrap();
    }
    let account = persist_reaction(&pool, &channel, id, "account-one", "❤️", true)
        .await
        .unwrap();
    assert_eq!(
        account,
        persist_reaction(&pool, &channel, id, "account-two", "❤️", true)
            .await
            .unwrap()
    );
    assert!(
        account["reactions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == &json!({"emoji":"❤️","authorIds":["account"]}))
    );
    let space: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES('private-space','Private',$1) RETURNING id").bind(user).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
        .bind(space)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    let private_id: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name,private) VALUES('private-channel',$1,'private',true) RETURNING id").bind(space).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2)")
        .bind(private_id)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    let private_message = persist(
        &pool,
        "private-channel",
        "account-one",
        Uuid::new_v4(),
        "private",
    )
    .await
    .unwrap();
    let private_message = private_message["id"].as_str().unwrap();
    assert_eq!(
        persist_reaction(
            &pool,
            "private-channel",
            private_message,
            "guest-one",
            "👍",
            true
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        persist_reaction(&pool, &channel, private_message, "account-one", "👍", true)
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    // Turn the actor into a non-owner member to check explicit grants.
    let owner: i64 = sqlx::query_scalar(
        "INSERT INTO public.users(external_id) VALUES('other-owner') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE public.spaces SET owner_id=$1 WHERE id=$2")
        .bind(owner)
        .bind(space)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        persist_reaction(
            &pool,
            "private-channel",
            private_message,
            "account-one",
            "👍",
            true
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
    sqlx::query("INSERT INTO public.channel_members(channel_id,user_id) VALUES($1,$2)")
        .bind(private_id)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        persist_reaction(
            &pool,
            "private-channel",
            private_message,
            "account-one",
            "👍",
            true
        )
        .await
        .is_ok()
    );
    sqlx::query("UPDATE public.space_members SET deleted_at=now() WHERE space_id=$1 AND user_id=$2 AND deleted_at IS NULL")
        .bind(space)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        persist_reaction(
            &pool,
            "private-channel",
            private_message,
            "account-one",
            "👍",
            false
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
    sqlx::query("UPDATE public.account_sessions SET revoked_at=now() WHERE user_id=$1")
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        persist_reaction(&pool, &channel, id, "account-two", "❤️", false)
            .await
            .unwrap_err()
            .status,
        StatusCode::UNAUTHORIZED
    );
    // The 60-mutation budget counts toggles, but not no-op retries.
    for index in 0..58 {
        persist_reaction(&pool, &channel, id, "guest-one", "🔥", index % 2 == 0)
            .await
            .unwrap();
    }
    assert_eq!(
        persist_reaction(&pool, &channel, id, "guest-one", "🔥", true)
            .await
            .unwrap_err()
            .status,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(
        persist_reaction(&pool, &channel, id, "guest-one", "🔥", false)
            .await
            .is_ok()
    );
    // A full set of kinds still allows joining an existing kind. Total
    // contributions have a separate boundary, and removal must work at it.
    let bounded = persist(&pool, &channel, "guest-two", Uuid::new_v4(), "limits")
        .await
        .unwrap();
    let bounded_id = bounded["id"].as_str().unwrap();
    // 50 distinct kinds, U+1F600 (😀) through U+1F631; 👍 is not among them.
    let kinds: Vec<String> = (0x1F600..0x1F632)
        .map(|code| char::from_u32(code).unwrap().to_string())
        .collect();
    sqlx::query("INSERT INTO public.message_reactions(message_id,emoji,user_id) SELECT m.id,emoji,$3 FROM public.messages m CROSS JOIN unnest($2::text[]) AS emoji WHERE m.external_id=$1")
        .bind(bounded_id).bind(kinds.as_slice()).bind(reader).execute(&pool).await.unwrap();
    assert!(
        persist_reaction(&pool, &channel, bounded_id, "guest-two", "😀", true)
            .await
            .is_ok()
    );
    assert_eq!(
        persist_reaction(&pool, &channel, bounded_id, "guest-two", "👍", true)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    sqlx::query("UPDATE public.message_reactions SET deleted_at=now() WHERE message_id=(SELECT id FROM public.messages WHERE external_id=$1) AND deleted_at IS NULL")
        .bind(bounded_id).execute(&pool).await.unwrap();
    sqlx::query("WITH actors AS (INSERT INTO public.users(external_id) SELECT 'fixture-'||n FROM generate_series(1,999) AS n RETURNING id) INSERT INTO public.message_reactions(message_id,emoji,user_id) SELECT m.id,'👍',actors.id FROM public.messages m CROSS JOIN actors WHERE m.external_id=$1")
        .bind(bounded_id).execute(&pool).await.unwrap();
    let full = persist_reaction(&pool, &channel, bounded_id, "guest-two", "👍", true)
        .await
        .unwrap();
    assert_eq!(
        full["reactions"][0]["authorIds"].as_array().unwrap().len(),
        1000
    );
    assert_eq!(
        persist_reaction(&pool, &channel, bounded_id, "guest-two", "🎉", true)
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    assert!(
        persist_reaction(&pool, &channel, bounded_id, "guest-two", "👍", true)
            .await
            .is_ok()
    );
    let removed = persist_reaction(&pool, &channel, bounded_id, "guest-two", "👍", false)
        .await
        .unwrap();
    assert_eq!(
        removed["reactions"][0]["authorIds"]
            .as_array()
            .unwrap()
            .len(),
        999
    );
    pool.close().await;
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL"]
async fn reactor_lists_name_people_in_reaction_order_for_readers_only(pool: PgPool) {
    let mut users = Vec::new();
    for (token, username, name) in [
        ("alice", "alice", "Alice A"),
        ("bob", "bob", "Bob B"),
        ("carol", "carol", "Carol C"),
        ("outsider", "outsider", "Outsider"),
    ] {
        let user: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name,avatar_id) VALUES($1,$2,$3,$4) RETURNING id")
            .bind(token).bind(username).bind(name).bind(100 + users.len() as i16).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(token.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,$3,$4,$2)")
            .bind(token).bind(hash).bind(name).bind(user).execute(&pool).await.unwrap();
        users.push(user);
    }
    let space: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES('reactor-space','Reactors',$1) RETURNING id")
        .bind(users[0]).fetch_one(&pool).await.unwrap();
    for user in &users[..3] {
        sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
            .bind(space)
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
    }
    let channel = "reactor-channel".to_owned();
    let channel_id: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,'general') RETURNING id")
        .bind(&channel).bind(space).fetch_one(&pool).await.unwrap();
    // Carol is a member who has not joined: she may read, and so see who reacted.
    for user in &users[..2] {
        sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2)")
            .bind(channel_id)
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
    }
    let message = persist(&pool, &channel, "alice", Uuid::new_v4(), "react to me")
        .await
        .unwrap();
    let message = message["id"].as_str().unwrap();
    persist_reaction(&pool, &channel, message, "bob", "👍", true)
        .await
        .unwrap();
    persist_reaction(&pool, &channel, message, "alice", "👍", true)
        .await
        .unwrap();
    let snapshot = persist_reaction(&pool, &channel, message, "alice", "🎉", true)
        .await
        .unwrap();

    let list = reactor_list(&pool, &channel, message, Some(users[2]))
        .await
        .unwrap();
    assert_eq!(list["messageId"], message);
    assert_eq!(list["reactionSeq"], snapshot["seq"]);
    let emoji = |value: &Value| -> Vec<Value> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["emoji"].clone())
            .collect()
    };
    assert_eq!(emoji(&list["reactions"]), emoji(&snapshot["reactions"]));
    let alice = json!({"id":"alice","username":"alice","displayName":"Alice A","avatarId":100});
    let bob = json!({"id":"bob","username":"bob","displayName":"Bob B","avatarId":101});
    let authors = |list: &Value, wanted: &str| -> Value {
        list["reactions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["emoji"] == wanted)
            .map(|r| r["authors"].clone())
            .unwrap_or(Value::Null)
    };
    assert_eq!(
        authors(&list, "👍"),
        json!([bob, alice]),
        "first reactor first"
    );
    assert_eq!(authors(&list, "🎉"), json!([alice]));

    // Removed reactions stay stored but are no longer listed.
    persist_reaction(&pool, &channel, message, "bob", "👍", false)
        .await
        .unwrap();
    let list = reactor_list(&pool, &channel, message, Some(users[1]))
        .await
        .unwrap();
    assert_eq!(authors(&list, "👍"), json!([alice]));

    for reader in [Some(users[3]), None] {
        assert_eq!(
            reactor_list(&pool, &channel, message, reader)
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
    }
    let other = "reactor-other".to_owned();
    sqlx::query("INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,'other')")
        .bind(&other)
        .bind(space)
        .execute(&pool)
        .await
        .unwrap();
    for (channel, message) in [(channel.as_str(), "absent"), (other.as_str(), message)] {
        assert_eq!(
            reactor_list(&pool, channel, message, Some(users[0]))
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
    }
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL"]
async fn direct_reactions_require_participants_and_preserve_history_sequences(pool: PgPool) {
    let mut users = Vec::new();
    for name in ["alice", "bob", "outsider"] {
        let user: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,$1,$1) RETURNING id")
            .bind(name).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(name.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,$1,$3,$2)")
            .bind(name).bind(hash).bind(user).execute(&pool).await.unwrap();
        users.push(user);
    }
    let channel = "reaction-dm".to_owned();
    let channel_id: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,name,private) VALUES($1,'direct',true) RETURNING id")
        .bind(&channel).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.direct_conversations(channel_id,low_user_id,high_user_id) VALUES($1,$2,$3)")
        .bind(channel_id).bind(users[0]).bind(users[1]).execute(&pool).await.unwrap();
    let original = persist(
        &pool,
        &channel,
        "alice",
        Uuid::new_v4(),
        "DM reaction target",
    )
    .await
    .unwrap();
    let message = original["id"].as_str().unwrap();
    let added = persist_reaction(&pool, &channel, message, "bob", "🎉", true)
        .await
        .unwrap();
    assert_eq!(added["seq"], "2");
    assert_eq!(
        added["reactions"],
        json!([{"emoji":"🎉","authorIds":["bob"]}])
    );
    assert_eq!(
        persist_reaction(&pool, &channel, message, "bob", "🎉", true)
            .await
            .unwrap(),
        added,
        "no-op returns the same snapshot without another event",
    );
    for active in [true, false] {
        assert_eq!(
            persist_reaction(&pool, &channel, message, "outsider", "🎉", active)
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND,
        );
    }
    let history = history_page(&pool, &channel, None, Some(users[0]))
        .await
        .unwrap();
    assert_eq!(history["channel"]["direct"], true);
    assert_eq!(history["channel"]["name"], "bob");
    assert_eq!(history["cursor"], "2");
    assert_eq!(history["messages"][0]["seq"], "1");
    assert_eq!(history["messages"][0]["reactionSeq"], "2");
    assert_eq!(history["messages"][0]["reactions"], added["reactions"]);
    for reader in [None, Some(users[2])] {
        assert_eq!(
            history_page(&pool, &channel, None, reader)
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
    }
    let removed = persist_reaction(&pool, &channel, message, "bob", "🎉", false)
        .await
        .unwrap();
    assert_eq!(removed["seq"], "3");
    assert_eq!(removed["reactions"], json!([]));
    // The removed reaction stops counting but its record is retained.
    let (head, events, activity, reactions, retained): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT last_seq,
            (SELECT count(*) FROM public.channel_events WHERE channel_id=c.id),
            (SELECT count(*) FROM public.message_reaction_activity),
            (SELECT count(*) FROM public.message_reactions WHERE deleted_at IS NULL),
            (SELECT count(*) FROM public.message_reactions)
         FROM public.channels c WHERE external_id=$1",
    )
    .bind(&channel)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (head, events, activity, reactions, retained),
        (3, 3, 2, 0, 1)
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM public.channel_joins")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "DMs do not require space-channel joins"
    );
    sqlx::query("UPDATE public.users SET deleted_at=now() WHERE id=$1")
        .bind(users[0])
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        persist_reaction(&pool, &channel, message, "bob", "🎉", false)
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND,
        "even no-ops require two active participants"
    );
    assert_eq!(
        history_page(&pool, &channel, None, Some(users[1]))
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
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

fn account_socket(url: String, cookie: &str) -> axum::http::Request<()> {
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("cookie", format!("caper_session={cookie}").parse().unwrap());
    request
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
async fn durable_account_delivery_replay_handoff_and_demo_retirement() {
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
    let user_external = random_id(12);
    let user: i64 = sqlx::query_scalar("INSERT INTO public.users (external_id, display_name, username, avatar_id) VALUES ($1,'Account Name','chat_test',255) RETURNING id")
        .bind(&user_external).fetch_one(&pool).await.unwrap();
    let account_cookie = "durable-account-cookie";
    let account_hash = Sha256::digest(account_cookie.as_bytes()).to_vec();
    sqlx::query("INSERT INTO public.account_sessions (token_hash,user_id,expires_at) VALUES ($1,$2,now()+interval '1 day')")
        .bind(&account_hash).bind(user).execute(&pool).await.unwrap();
    let space_id: i64 = sqlx::query_scalar("INSERT INTO public.spaces (external_id,name,owner_id) VALUES ($1,'Account space',$2) RETURNING id")
        .bind(random_id(12)).bind(user).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.space_members (space_id,user_id) VALUES ($1,$2)")
        .bind(space_id)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    let channel = random_id(12);
    sqlx::query("INSERT INTO public.channels (external_id,space_id,name) VALUES ($1,$2,'general')")
        .bind(&channel)
        .bind(space_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) SELECT id,$2 FROM public.channels WHERE external_id=$1")
        .bind(&channel).bind(user).execute(&pool).await.unwrap();
    assert_eq!(channel.len(), 12);
    let seeded_name: String =
        sqlx::query_scalar("SELECT name FROM public.channels WHERE external_id=$1")
            .bind(&channel)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(seeded_name, "general");
    assert_eq!(
        history_page(&pool, &channel, None, Some(user))
            .await
            .unwrap()["channel"]["name"],
        "general"
    );
    let token = "test-account-capability";
    sqlx::query("INSERT INTO public.chat_sessions (external_id, token_hash, name, user_id, account_session_hash) VALUES ($1,$2,'Account Name',$3,$4)").bind(random_id(12)).bind(Sha256::digest(token.as_bytes()).as_slice()).bind(user).bind(&account_hash).execute(&pool).await.unwrap();
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
    assert_eq!(one["author"]["avatarId"], 255);
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
        history_page(&pool, &channel, None, Some(user))
            .await
            .unwrap()["cursor"],
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
    // Simulate history/outbox written before avatars existed. Reads and replay
    // must use the saved profile without rewriting those persisted snapshots.
    pool.execute("UPDATE public.messages SET payload = payload #- '{author,avatarId}'")
        .await
        .unwrap();
    pool.execute(
        "UPDATE public.channel_events SET payload = payload #- '{message,author,avatarId}'",
    )
    .await
    .unwrap();
    let history = history_page(&pool, &channel, Some(3), Some(user))
        .await
        .unwrap();
    assert_eq!(history["messages"].as_array().unwrap().len(), 2);
    assert_eq!(history["messages"][0]["seq"], "1");
    assert_eq!(history["messages"][1]["seq"], "2");
    assert_eq!(history["messages"][0]["author"]["avatarId"], 255);
    assert_eq!(history["messages"][1]["author"]["avatarId"], 255);

    let broker_url = std::env::var("CHAT_TEST_VALKEY_URL").expect("disposable broker required");
    let broker = redis::Client::open(broker_url).unwrap();
    let chat = Chat {
        pool: pool.clone(),
        broker,
        wake: Arc::new(Notify::new()),
        cdn: None,
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
    // One slow publisher must not block other replicas. Hold the first row as
    // another worker would, then exercise the real publisher on the remaining
    // rows. A blocking FOR UPDATE or global publisher lock fails this check.
    let mut claimed = pool.begin().await.unwrap();
    sqlx::query("SELECT seq FROM public.channel_events WHERE seq=1 FOR UPDATE")
        .fetch_all(&mut *claimed)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(3), publish_pending(&chat))
            .await
            .expect("publisher blocked behind another claim")
            .unwrap()
    );
    let published: Vec<i64> = sqlx::query_scalar(
        "SELECT seq FROM public.channel_events WHERE published_at IS NOT NULL ORDER BY seq",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(published, [2, 3]);
    claimed.rollback().await.unwrap();
    assert!(publish_pending(&chat).await.unwrap());
    assert!(!publish_pending(&chat).await.unwrap());
    pool.execute("UPDATE public.channel_events SET published_at = NULL")
        .await
        .unwrap();
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
    let (mut socket, _) = tokio_tungstenite::connect_async(account_socket(
        format!("ws://{old_address}/api/chat/events?channelId={channel}&after=0"),
        account_cookie,
    ))
    .await
    .unwrap();
    for seq in ["1", "2", "3"] {
        let replayed = event(&mut socket).await;
        assert_eq!(replayed["seq"], seq);
        assert_eq!(replayed["message"]["author"]["avatarId"], 255);
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
    let (mut typing_old, _) = tokio_tungstenite::connect_async(account_socket(
        format!("ws://{old_address}/api/chat/events?channelId={channel}&after=3&typing=true"),
        account_cookie,
    ))
    .await
    .unwrap();
    let (mut typing_new, _) = tokio_tungstenite::connect_async(account_socket(
        format!("ws://{new_address}/api/chat/events?channelId={channel}&after=3&typing=true"),
        account_cookie,
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
        history_page(&pool, &channel, None, Some(user))
            .await
            .unwrap()["cursor"],
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
    let (mut next, _) = tokio_tungstenite::connect_async(account_socket(
        format!("ws://{new_address}/api/chat/events?channelId={channel}&after=3"),
        account_cookie,
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
        tokio_tungstenite::connect_async(account_socket(
            format!("ws://{old_address}/api/chat/events?channelId={channel}&after=5"),
            account_cookie,
        ))
        .await
        .is_err()
    );
    socket.close(None).await.unwrap();
    // Explicitly reject an impossible cursor; never pretend missing history was read.
    let (mut invalid, _) = tokio_tungstenite::connect_async(account_socket(
        format!("ws://{new_address}/api/chat/events?channelId={channel}&after=999"),
        account_cookie,
    ))
    .await
    .unwrap();
    assert_eq!(event(&mut invalid).await["type"], "resync_required");
    next.close(None).await.unwrap();

    let demo_channel: String = sqlx::query_scalar("SELECT c.external_id FROM public.channels c JOIN public.spaces s ON s.id=c.space_id WHERE s.demo")
        .fetch_one(&pool).await.unwrap();
    let demo_space: i64 =
        sqlx::query_scalar("SELECT space_id FROM public.channels WHERE external_id=$1")
            .bind(&demo_channel)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
        .bind(demo_space)
        .bind(user)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        tokio_tungstenite::connect_async(format!(
            "ws://{new_address}/api/chat/events?channelId={demo_channel}&after=0"
        ))
        .await
        .is_err()
    );
    assert!(
        tokio_tungstenite::connect_async(account_socket(
            format!("ws://{new_address}/api/chat/events?channelId={demo_channel}&after=0"),
            account_cookie,
        ))
        .await
        .is_err()
    );
    old_server.abort();
    new_server.abort();

    // Renewing an account chat credential must not turn a retry into a second message.
    let other_token = "another-account-chat-token";
    sqlx::query("INSERT INTO public.chat_sessions (external_id, token_hash, name, user_id, account_session_hash) VALUES ($1,$2,'Account Name',$3,$4)")
        .bind(random_id(12)).bind(Sha256::digest(other_token.as_bytes()).as_slice()).bind(user).bind(&account_hash).execute(&pool).await.unwrap();
    assert_eq!(
        persist(&pool, &channel, other_token, id, "one")
            .await
            .unwrap_err()
            .status,
        StatusCode::CONFLICT
    );
    // A signed-in chat capability cannot outlive logout of its parent session.
    let logout_user: i64 = sqlx::query_scalar("INSERT INTO public.users (external_id, display_name, username) VALUES ($1,'Logout Account','chat_logout') RETURNING id")
        .bind(random_id(12)).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.account_sessions (token_hash,user_id,expires_at) VALUES ($1,$2,now()+interval '1 day')")
        .bind(b"parent-session".as_slice()).bind(logout_user).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.space_members (space_id,user_id) VALUES ($1,$2)")
        .bind(space_id)
        .bind(logout_user)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) SELECT id,$2 FROM public.channels WHERE external_id=$1")
        .bind(&channel)
        .bind(logout_user)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO public.chat_sessions (external_id, token_hash, name, user_id, account_session_hash) VALUES ($1,$2,'Not authoritative',$3,$4)")
        .bind(random_id(12)).bind(Sha256::digest(b"account-chat").as_slice()).bind(logout_user).bind(b"parent-session".as_slice()).execute(&pool).await.unwrap();
    let account_message = persist(&pool, &channel, "account-chat", Uuid::new_v4(), "signed in")
        .await
        .unwrap();
    assert_eq!(account_message["author"]["name"], "Logout Account");
    assert_eq!(account_message["author"]["isGuest"], false);
    assert_eq!(
        typing_command(&app, &channel, Some("account-chat"), json!({"typing":true})).await,
        StatusCode::NO_CONTENT
    );
    sqlx::query("UPDATE public.account_sessions SET revoked_at = now() WHERE user_id = $1")
        .bind(logout_user)
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
    // Boundary: 30 new sends/minute per chat session; an already committed retry still
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
    let latest = history_page(&pool, &channel, None, Some(user))
        .await
        .unwrap();
    assert_eq!(latest["messages"].as_array().unwrap().len(), 50);
    assert_eq!(latest["messages"][0]["seq"], "4");
    assert_eq!(latest["cursor"], "53");
    assert_eq!(latest["hasMore"], true);
    let older = history_page(&pool, &channel, Some(4), Some(user))
        .await
        .unwrap();
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
    // Existing deployments retain their original row/ID and demo data, but no
    // principal can regain access to it through a membership grant.
    sqlx::query("UPDATE public.channels SET name='General' WHERE external_id=$1")
        .bind(&channel)
        .execute(&pool)
        .await
        .unwrap();
    seed(&pool).await.unwrap();
    let demo_channels: i64 = sqlx::query_scalar("SELECT count(*) FROM public.channels c JOIN public.spaces s ON s.id=c.space_id WHERE s.demo")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(demo_channels, 1);
    for principal in [None, Some(user)] {
        assert_eq!(
            history_page(&pool, &demo_channel, None, principal)
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
    }
    let guest_token = "retired-demo-guest";
    sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name) VALUES($1,$2,'Retired guest')")
        .bind(random_id(12)).bind(Sha256::digest(guest_token.as_bytes()).as_slice()).execute(&pool).await.unwrap();
    for denied_token in [guest_token, token] {
        assert_eq!(
            persist(&pool, &demo_channel, denied_token, Uuid::new_v4(), "denied")
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            typing_command(
                &app,
                &demo_channel,
                Some(denied_token),
                json!({"typing":true})
            )
            .await,
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(
        typing_command(&app, &demo_channel, None, json!({"typing":true})).await,
        StatusCode::UNAUTHORIZED
    );
    let guest_session = axum::http::Request::builder()
        .method("POST")
        .uri("/api/chat/session")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(r#"{"name":"Guest"}"#))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(guest_session).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        history_page(&pool, &channel, None, Some(user))
            .await
            .unwrap()["channel"]["name"],
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
        sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2)")
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
    assert!(
        history_page(&pool, &public, None, Some(member))
            .await
            .is_ok(),
        "public preview can read without joining"
    );
    assert_eq!(
        persist(
            &pool,
            &public,
            chat_token,
            Uuid::new_v4(),
            "preview cannot send"
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) SELECT id,$2 FROM public.channels WHERE external_id=$1")
        .bind(&public).bind(member).execute(&pool).await.unwrap();
    assert_eq!(
        persist(
            &pool,
            &public,
            chat_token,
            Uuid::new_v4(),
            "explicitly joined"
        )
        .await
        .unwrap()["seq"],
        "1"
    );
    sqlx::query("UPDATE public.channel_joins SET deleted_at=now() WHERE channel_id=(SELECT id FROM public.channels WHERE external_id=$1) AND user_id=$2 AND deleted_at IS NULL")
        .bind(&public).bind(member).execute(&pool).await.unwrap();
    assert!(
        history_page(&pool, &public, None, Some(member))
            .await
            .is_ok(),
        "leaving public retains preview access"
    );
    assert_eq!(
        persist(
            &pool,
            &public,
            chat_token,
            Uuid::new_v4(),
            "left cannot send"
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
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
    sqlx::query("UPDATE public.channel_members SET deleted_at=now() WHERE channel_id=$1 AND user_id=$2 AND deleted_at IS NULL")
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
        cdn: None,
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
    sqlx::query("UPDATE public.channel_members SET deleted_at=now() WHERE channel_id=$1 AND user_id=$2 AND deleted_at IS NULL")
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
    assert_eq!(event(&mut public_socket).await["type"], "message.created");
    assert_eq!(event(&mut public_socket).await["cursor"], "1");
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

#[test]
fn served_messages_use_current_author_profile_when_known() {
    let message = json!({"author":{"id":"a","name":"Old","isGuest":false}});
    let renamed = enrich_author(message.clone(), Some(7), Some("New"));
    assert_eq!(renamed["author"]["name"], "New");
    assert_eq!(renamed["author"]["avatarId"], 7);
    let kept = enrich_author(message, None, None);
    assert_eq!(kept["author"]["name"], "Old");
    assert_eq!(kept["author"]["avatarId"], Value::Null);
    let event = json!({"type":"message.created","message":{"author":{"name":"Old"}}});
    assert_eq!(
        enrich_author(event, None, Some("New"))["message"]["author"]["name"],
        "New"
    );
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL"]
async fn history_shows_current_display_names_but_stores_the_original(pool: PgPool) {
    let mut users = Vec::new();
    for name in ["alice", "bob"] {
        let user: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1||'-id',$1,$1) RETURNING id")
            .bind(name).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(name.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,$1,$3,$2)")
            .bind(name).bind(hash).bind(user).execute(&pool).await.unwrap();
        users.push(user);
    }
    let [alice, bob] = users[..] else {
        unreachable!()
    };
    let space: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES('names-space','Names',$1) RETURNING id")
        .bind(alice).fetch_one(&pool).await.unwrap();
    let channel: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name) VALUES('names-channel',$1,'names') RETURNING id")
        .bind(space).fetch_one(&pool).await.unwrap();
    for user in [alice, bob] {
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
    let retry = Uuid::new_v4();
    let sent = persist(&pool, "names-channel", "alice", retry, "hello")
        .await
        .unwrap();
    assert_eq!(sent["author"]["name"], "alice");
    sqlx::query("UPDATE public.users SET display_name='Alice Renamed' WHERE id=$1")
        .bind(alice)
        .execute(&pool)
        .await
        .unwrap();
    let history = history_page(&pool, "names-channel", None, Some(bob))
        .await
        .unwrap();
    assert_eq!(history["messages"][0]["author"]["name"], "Alice Renamed");
    // An idempotent retry answers with the same current identity.
    let retried = persist(&pool, "names-channel", "alice", retry, "hello")
        .await
        .unwrap();
    assert_eq!(retried["author"]["name"], "Alice Renamed");
    // The stored message keeps what was shown at send time.
    let stored: Value =
        sqlx::query_scalar("SELECT payload FROM public.messages WHERE channel_id=$1")
            .bind(channel)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored["author"]["name"], "alice");
    // A deleted account falls back to the stored name.
    sqlx::query("UPDATE public.users SET deleted_at=now() WHERE id=$1")
        .bind(alice)
        .execute(&pool)
        .await
        .unwrap();
    let history = history_page(&pool, "names-channel", None, Some(bob))
        .await
        .unwrap();
    assert_eq!(history["messages"][0]["author"]["name"], "alice");
}
