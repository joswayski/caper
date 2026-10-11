use super::*;
use crate::{Cloudflare, Config};
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use tower::ServiceExt;

/// Account and chat tokens are both the username, as in the forwarding tests.
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

async fn send(app: &Router, user: &str, channel: &str, text: &str, root: Option<&str>) -> Value {
    let mut body = json!({"clientMessageId": Uuid::new_v4(), "text": text});
    if let Some(root) = root {
        body["threadRootId"] = json!(root);
    }
    let (status, message) = request(
        app,
        "POST",
        &format!("/api/chat/channels/{channel}/messages"),
        user,
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{message}");
    message
}

async fn history(app: &Router, user: &str, channel: &str) -> (StatusCode, Value) {
    request(
        app,
        "GET",
        &format!("/api/chat/channels/{channel}/messages"),
        user,
        Value::Null,
    )
    .await
}

async fn directs(app: &Router, user: &str) -> Vec<Value> {
    let (status, body) = request(app, "GET", "/api/dms", user, Value::Null).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["conversations"].as_array().unwrap().clone()
}

/// `(outcome, forwarded original's text, note)` of a message's report.
async fn report(pool: &PgPool, message: &Value) -> (String, Option<String>, Option<String>) {
    sqlx::query_as(
        "SELECT r.outcome,f.payload->'forward'->'message'->'content'->>'text',f.payload->'content'->>'text'
         FROM public.feedback_reports r JOIN public.messages m ON m.id=r.message_id
         LEFT JOIN public.messages f ON f.id=r.forward_id WHERE m.external_id=$1",
    )
    .bind(message["id"].as_str().unwrap())
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn forward_all(chat: &Chat) {
    while forward_pending(chat).await.unwrap() {}
}

#[test]
fn outcomes_match_the_recorded_names() {
    assert_eq!(Outcome::Forwarded(1).name(), "forwarded");
    assert_eq!(Outcome::Duplicate.name(), "duplicate");
    assert_eq!(Outcome::Unavailable.name(), "unavailable");
    assert_eq!(Outcome::Limited.name(), "limited");
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn feedback_reaches_the_team_with_its_thread_or_context_and_they_can_reply(pool: PgPool) {
    assert!(matches!(
        reqwest::Url::parse(&std::env::var("DATABASE_URL").unwrap())
            .unwrap()
            .host_str(),
        Some("127.0.0.1" | "localhost")
    ));
    let (team_id, team_username, team_name, team_avatar): (String, String, String, i16) =
        sqlx::query_as("SELECT external_id,username,display_name,avatar_id FROM public.users WHERE team_account")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (team_username.as_str(), team_name.as_str(), team_avatar),
        ("caper", "Caper", 1)
    );
    assert_eq!(team_id.len(), 12);
    let mut users = std::collections::HashMap::new();
    for name in ["alice", "bob", "jose", "outsider"] {
        let user: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name,avatar_id) VALUES($1,$1,$1,31) RETURNING id")
            .bind(name).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(name.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,user_id,account_session_hash,name) VALUES($1,$2,$3,$2,$1)")
            .bind(name).bind(hash).bind(user).execute(&pool).await.unwrap();
        users.insert(name, user);
    }
    sqlx::query("INSERT INTO public.feedback_team(user_id) VALUES($1)")
        .bind(users["jose"])
        .execute(&pool)
        .await
        .unwrap();
    let space: i64 = sqlx::query_scalar(
        "INSERT INTO public.spaces(external_id,name,owner_id) VALUES('studio','Studio',$1) RETURNING id",
    )
    .bind(users["alice"])
    .fetch_one(&pool)
    .await
    .unwrap();
    let general: i64 = sqlx::query_scalar(
        "INSERT INTO public.channels(external_id,space_id,name,private) VALUES('general',$1,'general',true) RETURNING id",
    )
    .bind(space)
    .fetch_one(&pool)
    .await
    .unwrap();
    for name in ["alice", "bob"] {
        for table in ["space_members", "channel_members", "channel_joins"] {
            let column = if table == "space_members" {
                "space_id"
            } else {
                "channel_id"
            };
            sqlx::query(&format!(
                "INSERT INTO public.{table}({column},user_id) VALUES($1,$2)"
            ))
            .bind(if table == "space_members" {
                space
            } else {
                general
            })
            .bind(users[name])
            .execute(&pool)
            .await
            .unwrap();
        }
    }
    let mut config = Config::test(false);
    config.auth_fixture = false;
    let mut state =
        AppState::with_database(config, Arc::new(Cloudflare::new()), Some(pool.clone()));
    let broker = redis::Client::open(std::env::var("CHAT_TEST_VALKEY_URL").unwrap()).unwrap();
    state.chat = Some(Chat::new(pool.clone(), broker));
    let chat = state.chat.clone().unwrap();
    let app = crate::app(state);

    // Any alias is a mention of the team account, highlighted as typed.
    let first = send(
        &app,
        "alice",
        "general",
        "@Feedback uploads fail on iOS",
        None,
    )
    .await;
    assert_eq!(
        first["content"]["mentions"],
        json!([{"type":"user","id":team_id,"username":"feedback"}])
    );
    assert!(
        directs(&app, "alice").await.is_empty(),
        "reports wait for the publisher"
    );
    forward_all(&chat).await;
    assert_eq!(
        report(&pool, &first).await,
        (
            "forwarded".into(),
            Some("@Feedback uploads fail on iOS".into()),
            Some(String::new())
        )
    );

    // The person sees one conversation with the team; the team sees the person.
    let mine = directs(&app, "alice").await;
    assert_eq!(mine.len(), 1);
    assert_eq!(mine[0]["feedback"], true);
    assert_eq!(mine[0]["blocked"], false);
    assert_eq!(mine[0]["status"], "accepted");
    assert_eq!(mine[0]["peer"]["id"], team_id.as_str());
    assert_eq!(mine[0]["peer"]["displayName"], "Caper");
    let conversation = mine[0]["id"].as_str().unwrap().to_owned();
    // The forward counts as sent by the reporter; Caper's reply is unread.
    assert_eq!(mine[0]["readSeq"], "1");
    assert_eq!(mine[0]["lastSeq"], "2");
    let theirs = directs(&app, "jose").await;
    assert_eq!(theirs.len(), 1);
    assert_eq!(theirs[0]["id"], conversation.as_str());
    assert_eq!(theirs[0]["feedback"], true);
    assert_eq!(theirs[0]["peer"]["username"], "alice");
    assert_eq!(theirs[0]["readSeq"], "0");
    for outsider in ["bob", "outsider"] {
        assert!(directs(&app, outsider).await.is_empty());
        assert_eq!(
            history(&app, outsider, &conversation).await.0,
            StatusCode::NOT_FOUND
        );
    }
    let (status, page) = history(&app, "jose", &conversation).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert_eq!(page["channel"]["name"], "alice");
    assert_eq!(page["channel"]["direct"], true);
    let forward = &page["messages"][0];
    assert_eq!(forward["author"]["name"], "alice");
    assert_eq!(
        forward["forward"]["message"]["content"]["text"],
        "@Feedback uploads fail on iOS"
    );
    assert_eq!(
        forward["author"].get("admin"),
        None,
        "only the team is badged"
    );
    let acknowledgement = &page["messages"][1];
    assert_eq!(acknowledgement["content"]["text"], ACKNOWLEDGEMENT);
    assert_eq!(acknowledgement["author"]["id"], team_id.as_str());
    assert_eq!(acknowledgement["author"]["name"], "Caper");
    assert_eq!(acknowledgement["author"]["admin"], true);
    assert_eq!(acknowledgement["author"]["bot"], true, "badged ADMIN BOT");
    let (_, own) = history(&app, "alice", &conversation).await;
    assert_eq!(own["channel"]["name"], "Caper");

    // The team member replies as themselves and can open the live original.
    let reply = send(&app, "jose", &conversation, "Thanks, looking now", None).await;
    assert_eq!(reply["author"]["name"], "jose");
    let (_, own) = history(&app, "alice", &conversation).await;
    let latest = own["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(latest["content"]["text"], "Thanks, looking now");
    assert_eq!(latest["author"]["name"], "jose");
    assert_eq!(latest["author"]["admin"], true, "team replies are badged");
    assert_eq!(latest["author"].get("bot"), None, "people are never bots");
    // Team members are badged everywhere they write; others never are.
    assert_eq!(first["author"].get("admin"), None);
    let (status, original) = request(
        &app,
        "GET",
        &format!(
            "/api/chat/channels/{conversation}/forwards/{}/thread",
            forward["id"].as_str().unwrap()
        ),
        "jose",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{original}");
    assert_eq!(
        original["root"]["content"]["text"],
        "@Feedback uploads fail on iOS"
    );
    assert_eq!(
        original["source"],
        Value::Null,
        "no access to the source channel"
    );
    let (status, read) = request(
        &app,
        "POST",
        &format!("/api/dms/{conversation}/read"),
        "jose",
        json!({"seq": reply["seq"]}),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{read}");

    // A bare `@admins` points at the message before it, from someone else.
    send(&app, "bob", "general", "the composer freezes", None).await;
    let bare = send(&app, "alice", "general", "@admins ^", None).await;
    forward_all(&chat).await;
    assert_eq!(
        report(&pool, &bare).await,
        (
            "forwarded".into(),
            Some("the composer freezes".into()),
            Some(String::new())
        )
    );

    // A reply reports its whole thread, with the reply's words as the note.
    let root = send(&app, "bob", "general", "Crash on launch", None).await;
    let root_id = root["id"].as_str().unwrap();
    send(
        &app,
        "bob",
        "general",
        "stack trace attached",
        Some(root_id),
    )
    .await;
    let reported = send(&app, "alice", "general", "@mods see this", Some(root_id)).await;
    forward_all(&chat).await;
    assert_eq!(
        report(&pool, &reported).await,
        (
            "forwarded".into(),
            Some("Crash on launch".into()),
            Some("@mods see this".into())
        )
    );
    let (_, page) = history(&app, "jose", &conversation).await;
    let thread = page["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(thread["forward"]["message"]["thread"]["replyCount"], 2);
    let (_, replies) = request(
        &app,
        "GET",
        &format!(
            "/api/chat/channels/{conversation}/forwards/{}/thread",
            thread["id"].as_str().unwrap()
        ),
        "jose",
        Value::Null,
    )
    .await;
    assert_eq!(replies["messages"].as_array().unwrap().len(), 2);
    // Repeating it right away adds nothing; the forward is already live.
    let again = send(&app, "alice", "general", "@feedback still", Some(root_id)).await;
    forward_all(&chat).await;
    assert_eq!(report(&pool, &again).await.0, "duplicate");

    // In a DM, and opened by name: each person has exactly one conversation.
    send(&app, "bob", "general", "hi", None).await;
    let (status, dm) = request(&app, "POST", "/api/dms", "bob", json!({"username":"alice"})).await;
    assert_eq!(status, StatusCode::OK, "{dm}");
    let dm = dm["id"].as_str().unwrap().to_owned();
    let private = send(&app, "bob", &dm, "@caper login loops", None).await;
    forward_all(&chat).await;
    assert_eq!(report(&pool, &private).await.0, "forwarded");
    let bobs = directs(&app, "bob").await;
    let bobs: Vec<&Value> = bobs.iter().filter(|c| c["feedback"] == true).collect();
    assert_eq!(bobs.len(), 1);
    assert_eq!(directs(&app, "jose").await.len(), 2);
    assert_eq!(
        directs(&app, "alice")
            .await
            .iter()
            .filter(|c| c["feedback"] == true)
            .count(),
        1,
        "alice never sees bob's feedback conversation"
    );
    for alias in ["feedback", "@Caper", "admins"] {
        let (status, opened) = request(
            &app,
            "POST",
            "/api/dms",
            "alice",
            json!({"username": alias}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{opened}");
        assert_eq!(opened["id"], conversation.as_str());
        assert_eq!(opened["feedback"], true);
    }

    // Inside the conversation `@feedback` is just a mention.
    send(
        &app,
        "alice",
        &conversation,
        "@feedback one more thing",
        None,
    )
    .await;
    send(&app, "jose", &conversation, "@feedback noted", None).await;
    let inside: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.feedback_reports r JOIN public.messages m ON m.id=r.message_id JOIN public.channels c ON c.id=m.channel_id WHERE c.external_id=$1",
    )
    .bind(&conversation)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(inside, 0);

    // Adding `@feedback` in an edit reports the message once.
    let typo = send(&app, "bob", "general", "search is slow", None).await;
    for text in ["search is slow @feedback", "search is slow @feedback!"] {
        let revision = if text.ends_with('!') { 2 } else { 1 };
        let (status, edited) = request(
            &app,
            "PUT",
            &format!(
                "/api/chat/channels/general/messages/{}",
                typo["id"].as_str().unwrap()
            ),
            "bob",
            json!({"text": text, "expectedRevision": revision}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{edited}");
    }
    forward_all(&chat).await;
    assert_eq!(report(&pool, &typo).await.0, "forwarded");

    // The team account can't be blocked, and blocks never close the conversation.
    let (status, error) = request(
        &app,
        "PUT",
        &format!("/api/blocks/{team_id}"),
        "alice",
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error["code"], "team_account");
    sqlx::query("INSERT INTO public.user_blocks(blocker_id,blocked_id) VALUES($1,$2)")
        .bind(users["alice"])
        .bind(users["jose"])
        .execute(&pool)
        .await
        .unwrap();
    send(&app, "jose", &conversation, "Fixed in the next build", None).await;
    send(&app, "alice", &conversation, "Thank you!", None).await;

    // Caper acknowledged each person once within the hour, however many reports.
    let acknowledgements: Vec<(String, String)> = sqlx::query_as(
        "SELECT r.username,m.payload->'content'->>'text' FROM public.messages m
         JOIN public.chat_sessions cs ON cs.id=m.session_id JOIN public.users u ON u.id=cs.user_id
         JOIN public.direct_conversations d ON d.channel_id=m.channel_id JOIN public.users r ON r.id=d.feedback_user_id
         WHERE u.team_account ORDER BY r.username",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        acknowledgements,
        [
            ("alice".into(), ACKNOWLEDGEMENT.into()),
            ("bob".into(), ACKNOWLEDGEMENT.into())
        ]
    );

    // Leaving the team removes access to everyone else's conversations.
    sqlx::query("UPDATE public.feedback_team SET deleted_at=now() WHERE user_id=$1")
        .bind(users["jose"])
        .execute(&pool)
        .await
        .unwrap();
    assert!(directs(&app, "jose").await.is_empty());
    assert_eq!(
        history(&app, "jose", &conversation).await.0,
        StatusCode::NOT_FOUND
    );
    let (status, _) = request(
        &app,
        "POST",
        &format!("/api/chat/channels/{conversation}/messages"),
        "jose",
        json!({"clientMessageId": Uuid::new_v4(), "text": "still here?"}),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL and CHAT_TEST_VALKEY_URL"]
async fn reports_are_limited_and_never_share_what_the_reporter_lost(pool: PgPool) {
    let mut users = Vec::new();
    for name in ["alice", "bob"] {
        let user: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,$1,$1) RETURNING id")
            .bind(name).fetch_one(&pool).await.unwrap();
        let hash = Sha256::digest(name.as_bytes()).to_vec();
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
            .bind(&hash).bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.chat_sessions(external_id,token_hash,user_id,account_session_hash,name) VALUES($1,$2,$3,$2,$1)")
            .bind(name).bind(hash).bind(user).execute(&pool).await.unwrap();
        users.push(user);
    }
    let space: i64 = sqlx::query_scalar(
        "INSERT INTO public.spaces(external_id,name,owner_id) VALUES('studio','Studio',$1) RETURNING id",
    )
    .bind(users[1])
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO public.space_members(space_id,user_id) SELECT $1,unnest($2::bigint[])",
    )
    .bind(space)
    .bind(&users)
    .execute(&pool)
    .await
    .unwrap();
    let channel: i64 = sqlx::query_scalar(
        "INSERT INTO public.channels(external_id,space_id,name) VALUES('general',$1,'general') RETURNING id",
    )
    .bind(space)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO public.channel_joins(channel_id,user_id) SELECT $1,unnest($2::bigint[])",
    )
    .bind(channel)
    .bind(&users)
    .execute(&pool)
    .await
    .unwrap();
    let broker = redis::Client::open(std::env::var("CHAT_TEST_VALKEY_URL").unwrap()).unwrap();
    let chat = Chat::new(pool.clone(), broker);

    // Removed from the space before the publisher ran: nothing is shared.
    let lost = persist(
        &pool,
        "general",
        "alice",
        Uuid::new_v4(),
        "@feedback secret plans",
    )
    .await
    .unwrap();
    sqlx::query("UPDATE public.space_members SET deleted_at=now() WHERE user_id=$1")
        .bind(users[0])
        .execute(&pool)
        .await
        .unwrap();
    forward_all(&chat).await;
    assert_eq!(
        report(&pool, &lost).await,
        ("unavailable".into(), None, None)
    );
    let conversations: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.direct_conversations WHERE feedback_user_id=$1",
    )
    .bind(users[0])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(conversations, 0);

    // One account can't flood the team: past the hourly limit reports are kept only.
    let mut last = Value::Null;
    for index in 0..=HOURLY_FORWARDS {
        last = persist(
            &pool,
            "general",
            "bob",
            Uuid::new_v4(),
            &format!("@feedback issue {index}"),
        )
        .await
        .unwrap();
        // Spread over channel history so no two in a row are within the rate window.
        sqlx::query("UPDATE public.messages SET created_at=now()-interval '2 minutes'")
            .execute(&pool)
            .await
            .unwrap();
        forward_all(&chat).await;
    }
    assert_eq!(report(&pool, &last).await.0, "limited");
    let forwarded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.feedback_reports WHERE reporter_id=$1 AND outcome='forwarded'",
    )
    .bind(users[1])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(forwarded, HOURLY_FORWARDS);
}
