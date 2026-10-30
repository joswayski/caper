use super::joining::{
    accept_channel_invitation, decline_channel_invitation, join_channel, leave_channel,
};
use super::*;
use crate::{Cloudflare, Config, accounts::User};
use sqlx::{
    Connection, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::{str::FromStr, sync::Arc};
use uuid::Uuid;

fn principal(id: i64, name: &str) -> Principal {
    Principal {
        user: User {
            id,
            external_id: format!("{name:0<12}"),
            avatar_id: 0,
            avatar_asset: None,
            email: None,
            username: Some(name.into()),
            display_name: Some(name.into()),
        },
        token_hash: vec![],
    }
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL"]
async fn joining_consent_and_migration_preserve_access_without_silent_joins() {
    let options =
        PgConnectOptions::from_str(&std::env::var("CHAT_TEST_DATABASE_URL").unwrap()).unwrap();
    assert!(matches!(options.get_host(), "localhost" | "127.0.0.1"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("channel_join_test_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE DATABASE {database}").as_str())
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options.database(&database))
        .await
        .unwrap();
    // Seed the pre-join schema, then exercise the actual rollout backfill.
    for migration in sqlx::migrate!("./migrations")
        .iter()
        .filter(|migration| migration.version < 202610010001)
    {
        pool.execute(migration.sql.as_ref()).await.unwrap();
    }
    let mut people = vec![];
    for name in ["owner", "member", "other", "newcomer"] {
        let id: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,$2,$2) RETURNING id")
            .bind(format!("{name:0<12}")).bind(name).fetch_one(&pool).await.unwrap();
        people.push(principal(id, name));
    }
    let [owner, member, other, newcomer] = people.as_slice() else {
        unreachable!()
    };
    let space: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES('space1234567','Studio',$1) RETURNING id")
        .bind(owner.user.id).fetch_one(&pool).await.unwrap();
    for person in [owner, member, other] {
        sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
            .bind(space)
            .bind(person.user.id)
            .execute(&pool)
            .await
            .unwrap();
    }
    let general: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name) VALUES('first1234567',$1,'general') RETURNING id")
        .bind(space).fetch_one(&pool).await.unwrap();
    let private: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name,private) VALUES('other1234567',$1,'planning',true) RETURNING id")
        .bind(space).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO public.channel_members(channel_id,user_id) VALUES($1,$2)")
        .bind(private)
        .bind(member.user.id)
        .execute(&pool)
        .await
        .unwrap();
    pool.execute(include_str!(
        "../../migrations/202610010001_channel_joining.sql"
    ))
    .await
    .unwrap();
    let joined: Vec<(i64, i64)> = sqlx::query_as(
        "SELECT channel_id,user_id FROM public.channel_joins ORDER BY channel_id,user_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        joined,
        vec![
            (general, owner.user.id),
            (general, member.user.id),
            (general, other.user.id),
            (private, owner.user.id),
            (private, member.user.id)
        ]
    );
    // Exercise participation with the complete current schema after checking
    // the joining migration's backfill in isolation.
    for migration in sqlx::migrate!("./migrations")
        .iter()
        .filter(|migration| migration.version > 202610010001)
    {
        pool.execute(migration.sql.as_ref()).await.unwrap();
    }
    let state = AppState::with_database(
        Config::test(false),
        Arc::new(Cloudflare::new()),
        Some(pool.clone()),
    );
    let target = ("space1234567".to_owned(), "first1234567".to_owned());
    leave_channel(
        State(state.clone()),
        Extension(member.clone()),
        Path(target.clone()),
    )
    .await
    .unwrap();
    assert!(
        channel_access(&pool, "first1234567", Some(member.user.id))
            .await
            .is_ok()
    );
    assert_eq!(
        channel_participation(&pool, "first1234567", Some(member.user.id))
            .await
            .unwrap_err()
            .status,
        StatusCode::NOT_FOUND
    );
    let detail = get_space(
        State(state.clone()),
        Extension(member.clone()),
        Path("space1234567".into()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(detail["channels"][0]["joined"], false);
    // A private channel's readers (owner and grants) lead its member list;
    // public channels have no entry.
    assert_eq!(
        detail["channelMembers"],
        json!({"other1234567": [owner.user.external_id, member.user.external_id]})
    );
    let (left, right) = tokio::join!(
        join_channel(
            State(state.clone()),
            Extension(member.clone()),
            Path(target.clone())
        ),
        join_channel(
            State(state.clone()),
            Extension(member.clone()),
            Path(target.clone())
        )
    );
    assert!(left.unwrap().0.joined);
    assert!(right.unwrap().0.joined);
    assert!(
        channel_participation(&pool, "first1234567", Some(member.user.id))
            .await
            .is_ok()
    );

    // A private invitation shares metadata only, never a grant or participation.
    let private_target = ("space1234567".into(), "other1234567".into());
    sqlx::query("UPDATE public.users SET avatar_id=37 WHERE id=$1")
        .bind(other.user.id)
        .execute(&pool)
        .await
        .unwrap();
    let (_, Json(invited)) = add_channel_member(
        State(state.clone()),
        Extension(owner.clone()),
        Path(private_target.clone()),
        Json(MemberInput {
            username: "other".into(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(invited.avatar_id, 37);
    assert_eq!(invited.username, "other");
    assert_eq!(invited.display_name, "other");
    let Json(managed) = list_channel_members(
        State(state.clone()),
        Extension(owner.clone()),
        Path(private_target.clone()),
    )
    .await
    .unwrap();
    assert_eq!(managed["invitations"][0]["avatarId"], 37);
    assert_eq!(managed["invitations"][0]["username"], "other");
    assert_eq!(
        add_channel_member(
            State(state.clone()),
            Extension(owner.clone()),
            Path(private_target.clone()),
            Json(MemberInput {
                username: "other".into()
            })
        )
        .await
        .err()
        .unwrap()
        .status,
        StatusCode::CONFLICT
    );
    let pending = get_space(
        State(state.clone()),
        Extension(other.clone()),
        Path("space1234567".into()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(pending["channels"].as_array().unwrap().len(), 1);
    // Invited but not granted: who is in the private channel stays hidden.
    assert_eq!(pending["channelMembers"], json!({}));
    assert_eq!(
        pending["channelInvitations"][0]["channel"]["name"],
        "planning"
    );
    assert_eq!(
        pending["channelInvitations"][0]["inviter"]["username"],
        "owner"
    );
    assert!(pending["channelInvitations"][0].get("messages").is_none());
    assert!(
        channel_access(&pool, "other1234567", Some(other.user.id))
            .await
            .is_err()
    );
    assert!(
        join_channel(
            State(state.clone()),
            Extension(other.clone()),
            Path(private_target.clone())
        )
        .await
        .is_err()
    );
    // Even the leave endpoint must not disclose a hidden private channel or
    // cancel an invitation as a side effect of an unauthorized request.
    assert_eq!(
        leave_channel(
            State(state.clone()),
            Extension(other.clone()),
            Path(private_target.clone())
        )
        .await
        .unwrap_err()
        .status,
        StatusCode::NOT_FOUND
    );
    assert!(
        accept_channel_invitation(
            State(state.clone()),
            Extension(newcomer.clone()),
            Path(private_target.clone())
        )
        .await
        .is_err()
    );
    let accepted = accept_channel_invitation(
        State(state.clone()),
        Extension(other.clone()),
        Path(private_target.clone()),
    )
    .await
    .unwrap()
    .0;
    assert!(accepted.joined);
    assert!(
        channel_participation(&pool, "other1234567", Some(other.user.id))
            .await
            .is_ok()
    );
    assert!(
        accept_channel_invitation(
            State(state.clone()),
            Extension(other.clone()),
            Path(private_target.clone())
        )
        .await
        .is_err()
    );
    leave_channel(
        State(state.clone()),
        Extension(other.clone()),
        Path(private_target.clone()),
    )
    .await
    .unwrap();
    assert!(
        channel_access(&pool, "other1234567", Some(other.user.id))
            .await
            .is_err()
    );
    assert_eq!(
        add_channel_member(
            State(state.clone()),
            Extension(owner.clone()),
            Path(private_target.clone()),
            Json(MemberInput {
                username: "other".into()
            })
        )
        .await
        .err()
        .unwrap()
        .status,
        StatusCode::CONFLICT
    );

    // Decline, expiration and cancellation cannot be accepted from a stale UI.
    leave_channel(
        State(state.clone()),
        Extension(member.clone()),
        Path(private_target.clone()),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE public.channel_invitations SET updated_at=now()-interval '25 hours' WHERE channel_id=$1").bind(private).execute(&pool).await.unwrap();
    let _ = add_channel_member(
        State(state.clone()),
        Extension(owner.clone()),
        Path(private_target.clone()),
        Json(MemberInput {
            username: "member".into(),
        }),
    )
    .await
    .unwrap();
    decline_channel_invitation(
        State(state.clone()),
        Extension(member.clone()),
        Path(private_target.clone()),
    )
    .await
    .unwrap();
    assert!(
        accept_channel_invitation(
            State(state.clone()),
            Extension(member.clone()),
            Path(private_target.clone())
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE public.channel_invitations SET status='pending',updated_at=now()-interval '8 days' WHERE channel_id=$1 AND user_id=$2").bind(private).bind(member.user.id).execute(&pool).await.unwrap();
    assert!(
        accept_channel_invitation(
            State(state.clone()),
            Extension(member.clone()),
            Path(private_target.clone())
        )
        .await
        .is_err()
    );
    let _ = add_channel_member(
        State(state.clone()),
        Extension(owner.clone()),
        Path(private_target.clone()),
        Json(MemberInput {
            username: "member".into(),
        }),
    )
    .await
    .unwrap();
    remove_channel_member(
        State(state.clone()),
        Extension(owner.clone()),
        Path((
            "space1234567".into(),
            "other1234567".into(),
            member.user.external_id.clone(),
        )),
    )
    .await
    .unwrap();
    assert!(
        accept_channel_invitation(
            State(state.clone()),
            Extension(member.clone()),
            Path(private_target.clone())
        )
        .await
        .is_err()
    );

    // Space consent opts into one public starter, not all existing channels.
    sqlx::query(
        "INSERT INTO public.channels(external_id,space_id,name) VALUES('third1234567',$1,'design')",
    )
    .bind(space)
    .execute(&pool)
    .await
    .unwrap();
    let _ = add_space_member(
        State(state.clone()),
        Extension(owner.clone()),
        Path("space1234567".into()),
        Json(MemberInput {
            username: "newcomer".into(),
        }),
    )
    .await
    .unwrap();
    let _ = accept_invitation(
        State(state.clone()),
        Extension(newcomer.clone()),
        Path("space1234567".into()),
    )
    .await
    .unwrap();
    let detail = get_space(
        State(state.clone()),
        Extension(newcomer.clone()),
        Path("space1234567".into()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(detail["channels"].as_array().unwrap().len(), 2);
    assert_eq!(detail["channels"][0]["joined"], true);
    assert_eq!(detail["channels"][1]["joined"], false);
    remove_space_member(
        State(state),
        Extension(owner.clone()),
        Path(("space1234567".into(), newcomer.user.external_id.clone())),
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM public.channel_joins WHERE user_id=$1 AND deleted_at IS NULL"
        )
        .bind(newcomer.user.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    pool.close().await;
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}

#[sqlx::test(migrations = "./migrations")]
#[ignore = "requires disposable loopback DATABASE_URL"]
async fn unread_navigation_tracks_only_visible_messages_and_monotonic_channel_reads(pool: PgPool) {
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;
    let mut users = vec![];
    for name in ["reader", "author", "blocked"] {
        let id = sqlx::query_scalar(
            "INSERT INTO users(external_id,username,display_name) VALUES($1,$2,$2) RETURNING id",
        )
        .bind(format!("{name:0<12}"))
        .bind(name)
        .fetch_one(&pool)
        .await
        .unwrap();
        users.push(principal(id, name));
    }
    let reader = &users[0];
    let space: i64 = sqlx::query_scalar("INSERT INTO spaces(external_id,name,owner_id) VALUES('space1234567','Test unread',$1) RETURNING id")
        .bind(users[1].user.id).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO space_members(space_id,user_id) VALUES($1,$2)")
        .bind(space)
        .bind(reader.user.id)
        .execute(&pool)
        .await
        .unwrap();
    for (id, private, joined) in [
        ("first1234567", false, true),
        ("other1234567", false, true),
        ("quiet1234567", false, false),
        ("privt1234567", true, true),
    ] {
        let channel: i64 = sqlx::query_scalar("INSERT INTO channels(external_id,space_id,name,private,last_seq) VALUES($1,$2,$1,$3,10) RETURNING id")
            .bind(id).bind(space).bind(private).fetch_one(&pool).await.unwrap();
        if joined {
            sqlx::query("INSERT INTO channel_joins(channel_id,user_id) VALUES($1,$2)")
                .bind(channel)
                .bind(reader.user.id)
                .execute(&pool)
                .await
                .unwrap();
        }
    }
    sqlx::query("INSERT INTO user_blocks(blocker_id,blocked_id) VALUES($1,$2)")
        .bind(reader.user.id)
        .bind(users[2].user.id)
        .execute(&pool)
        .await
        .unwrap();
    for (channel, seq, author, mentions) in [
        ("first1234567", 1, 1, json!([])),
        ("first1234567", 3, 0, json!([])),
        ("other1234567", 2, 1, json!([])),
        ("other1234567", 8, 2, json!([])),
        ("quiet1234567", 4, 1, json!([])),
        (
            "quiet1234567",
            7,
            1,
            json!([{"type":"user","id":reader.user.external_id}]),
        ),
        (
            "privt1234567",
            9,
            1,
            json!([{"type":"user","id":reader.user.external_id}]),
        ),
    ] {
        let session: i64 = sqlx::query_scalar("INSERT INTO chat_sessions(external_id,token_hash,user_id,name) VALUES($1,$2,$3,'Test') RETURNING id")
            .bind(random_id(12)).bind(random_id(16).as_bytes()).bind(users[author].user.id).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO messages(external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload) SELECT $1,id,$2,$3,''::bytea,$4,$5 FROM channels WHERE external_id=$6")
            .bind(random_id(12)).bind(session).bind(Uuid::new_v4()).bind(seq).bind(json!({"content":{"mentions":mentions}})).bind(channel).execute(&pool).await.unwrap();
    }
    // Non-broadcast replies need a personal mention. Broadcast replies are
    // channel messages; copied mention metadata on forwards never notifies.
    // A joined-channel mention must count once, not once in each unread set.
    sqlx::query("UPDATE messages SET payload=$1 WHERE channel_id=(SELECT id FROM channels WHERE external_id='other1234567') AND channel_seq=2")
        .bind(json!({"content":{"mentions":[{"type":"user","id":reader.user.external_id}]}}))
        .execute(&pool).await.unwrap();
    for (seq, broadcast) in [(4, false), (5, false), (6, true)] {
        sqlx::query("INSERT INTO messages(external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload,thread_root_id,broadcast) SELECT $1,m.channel_id,m.session_id,$2,''::bytea,$3,m.payload,m.id,$4 FROM messages m JOIN channels c ON c.id=m.channel_id WHERE c.external_id='other1234567' AND m.channel_seq=2")
            .bind(random_id(12)).bind(Uuid::new_v4()).bind(seq).bind(broadcast).execute(&pool).await.unwrap();
    }
    pool.execute("UPDATE messages SET payload='{}'::jsonb WHERE channel_id=(SELECT id FROM channels WHERE external_id='other1234567') AND channel_seq=4").await.unwrap();
    pool.execute("UPDATE messages SET thread_root_id=(SELECT m.id FROM messages m JOIN channels c ON c.id=m.channel_id WHERE c.external_id='quiet1234567' AND m.channel_seq=4) WHERE channel_seq=7").await.unwrap();
    sqlx::query("INSERT INTO messages(external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload,forward_source_id) SELECT $1,m.channel_id,m.session_id,$2,''::bytea,9,m.payload,m.id FROM messages m JOIN channels c ON c.id=m.channel_id WHERE c.external_id='quiet1234567' AND m.channel_seq=7")
        .bind(random_id(12)).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO messages(external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload,deleted_at) SELECT $1,m.channel_id,m.session_id,$2,''::bytea,10,m.payload,now() FROM messages m JOIN channels c ON c.id=m.channel_id WHERE c.external_id='quiet1234567' AND m.channel_seq=7")
        .bind(random_id(12)).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    let state = AppState::with_database(
        Config::test(false),
        Arc::new(Cloudflare::new()),
        Some(pool.clone()),
    );
    let app = routes().layer(Extension(reader.clone())).with_state(state);
    async fn call(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
    }
    let snapshot = |channels: Value, count: i64| json!({"spaces":[{"id":"space1234567","channels":channels}],"feedbackWaiting":0,"badgeCount":count});
    assert_eq!(
        call(&app, "GET", "/api/spaces/unread", Value::Null).await,
        (
            StatusCode::OK,
            snapshot(
                json!([
                    {"id":"first1234567","lastSeq":"1","readSeq":"0","unreadCount":1},{"id":"other1234567","lastSeq":"6","readSeq":"0","unreadCount":3},{"id":"quiet1234567","lastSeq":"7","readSeq":"0","unreadCount":1}
                ]),
                5
            )
        )
    );
    let read = "/api/spaces/space1234567/channels/first1234567/read";
    for seq in ["1", "0"] {
        assert_eq!(
            call(&app, "POST", read, json!({"seq":seq})).await.0,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        call(&app, "GET", "/api/spaces/unread", Value::Null).await.1,
        snapshot(
            json!([
                {"id":"other1234567","lastSeq":"6","readSeq":"0","unreadCount":3},{"id":"quiet1234567","lastSeq":"7","readSeq":"0","unreadCount":1}
            ]),
            4
        ),
        "reading one channel must preserve the space's other unread channels"
    );
    let other_read = "/api/spaces/space1234567/channels/other1234567/read";
    for seq in ["2", "1"] {
        assert_eq!(
            call(&app, "POST", other_read, json!({"seq":seq})).await.0,
            StatusCode::NO_CONTENT
        );
    }
    assert_eq!(
        call(&app, "GET", "/api/spaces/unread", Value::Null).await.1,
        snapshot(
            json!([
                {"id":"other1234567","lastSeq":"6","readSeq":"2","unreadCount":2},{"id":"quiet1234567","lastSeq":"7","readSeq":"0","unreadCount":1}
            ]),
            3
        ),
        "returning clients receive the saved read boundary, not the newest message"
    );
    for (path, status) in [
        (
            "/api/spaces/space1234567/channels/privt1234567/read",
            StatusCode::NOT_FOUND,
        ),
        (
            "/api/spaces/wrong1234567/channels/other1234567/read",
            StatusCode::NOT_FOUND,
        ),
    ] {
        assert_eq!(call(&app, "POST", path, json!({"seq":"9"})).await.0, status);
    }
    assert_eq!(
        call(&app, "POST", read, json!({"seq":"-1"})).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&app, "POST", read, json!({"seq":"999"})).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT r.seq FROM channel_reads r JOIN channels c ON c.id=r.channel_id WHERE c.external_id='first1234567'")
            .fetch_one(&pool)
            .await
            .unwrap(),
        10,
        "future reads are clamped"
    );
    pool.execute("UPDATE space_members SET deleted_at=now()")
        .await
        .unwrap();
    assert_eq!(
        call(&app, "GET", "/api/spaces/unread", Value::Null).await.1,
        json!({"spaces":[],"feedbackWaiting":0,"badgeCount":0})
    );
    assert_eq!(
        call(&app, "POST", read, json!({"seq":"10"})).await.0,
        StatusCode::NOT_FOUND
    );
}
