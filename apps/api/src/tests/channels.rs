//! Real channel authorization and room storage; SFU is explicitly mocked.
use super::*;
use sqlx::{
    Connection, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::str::FromStr;

async fn voice_activity(state: &AppState, account: Option<&str>) -> (StatusCode, Value) {
    let mut request = Request::builder().uri("/api/spaces/voice-activity");
    if let Some(account) = account {
        request = request.header("cookie", format!("caper_session={account}"));
    }
    let response = app(state.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), BODY_LIMIT).await.unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}

// Exercise the real authenticated HTTP route against both room backends.
// Database/registry fixtures model revocation and in-flight/monitor-only rooms;
// the SFU remains mocked, as in the surrounding authorization tests.
async fn voice_activity_contract(state: &AppState, pool: &sqlx::PgPool, member: i64) {
    let empty = json!({"spaceIds":[]});
    let active = json!({"spaceIds":["SpaceExample"]});
    assert_eq!(
        voice_activity(state, None).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        voice_activity(state, Some("outside-session")).await,
        (StatusCode::OK, empty.clone())
    );
    // Two occupied rooms produce only one space, even before tracks publish.
    assert_eq!(
        voice_activity(state, Some("member-session")).await,
        (StatusCode::OK, active.clone())
    );

    for (hide, restore) in [
        (
            "UPDATE channel_joins SET deleted_at=now() WHERE user_id=$1",
            "UPDATE channel_joins SET deleted_at=NULL WHERE user_id=$1",
        ),
        (
            "UPDATE channels SET private=true WHERE external_id IN ('ChannelAlpha','ChannelBravo') AND $1::bigint IS NOT NULL",
            "UPDATE channels SET private=false WHERE external_id IN ('ChannelAlpha','ChannelBravo') AND $1::bigint IS NOT NULL",
        ),
        (
            "UPDATE channels SET deleted_at=now() WHERE external_id IN ('ChannelAlpha','ChannelBravo') AND $1::bigint IS NOT NULL",
            "UPDATE channels SET deleted_at=NULL WHERE external_id IN ('ChannelAlpha','ChannelBravo') AND $1::bigint IS NOT NULL",
        ),
        (
            "UPDATE space_members SET deleted_at=now() WHERE user_id=$1",
            "UPDATE space_members SET deleted_at=NULL WHERE user_id=$1",
        ),
        (
            "UPDATE spaces SET deleted_at=now() WHERE external_id='SpaceExample' AND $1::bigint IS NOT NULL",
            "UPDATE spaces SET deleted_at=NULL WHERE external_id='SpaceExample' AND $1::bigint IS NOT NULL",
        ),
        (
            "UPDATE spaces SET demo=true,owner_id=NULL WHERE external_id='SpaceExample' AND $1::bigint IS NOT NULL",
            "UPDATE spaces SET demo=false,owner_id=(SELECT id FROM users WHERE username='owner') WHERE external_id='SpaceExample' AND $1::bigint IS NOT NULL",
        ),
    ] {
        sqlx::query(hide).bind(member).execute(pool).await.unwrap();
        assert_eq!(
            voice_activity(state, Some("member-session")).await,
            (StatusCode::OK, empty.clone()),
            "{hide}"
        );
        if hide.contains("private=true") {
            assert_eq!(
                voice_activity(state, Some("owner-session")).await,
                (StatusCode::OK, active.clone())
            );
            sqlx::query(
                "INSERT INTO channel_members(channel_id,user_id) SELECT id,$1 FROM channels WHERE external_id IN ('ChannelAlpha','ChannelBravo')",
            )
            .bind(member)
            .execute(pool)
            .await
            .unwrap();
            assert_eq!(
                voice_activity(state, Some("member-session")).await,
                (StatusCode::OK, active.clone())
            );
            sqlx::query("UPDATE channel_members SET deleted_at=now() WHERE user_id=$1")
                .bind(member)
                .execute(pool)
                .await
                .unwrap();
            assert_eq!(
                voice_activity(state, Some("member-session")).await,
                (StatusCode::OK, empty.clone())
            );
        }
        sqlx::query(restore)
            .bind(member)
            .execute(pool)
            .await
            .unwrap();
    }

    // Room discovery includes reservations and monitors, but neither is voice
    // activity. Preserve the joined participants for the surrounding tests.
    for channel in ["ChannelAlpha", "ChannelBravo"] {
        let mut room = state.clone();
        room.media_channel = Some(channel.into());
        room.update(|r| {
            for p in r.participants.values_mut() {
                p.monitor = Some(Monitor {
                    parent: p.id,
                    role: MonitorRole::Sender,
                });
            }
            r.reservations.insert(
                Uuid::nil(),
                JoinReservation {
                    started: Timestamp::now(),
                    monitor: None,
                    user: None,
                },
            );
            Ok(())
        })
        .await
        .unwrap();
    }
    assert_eq!(
        voice_activity(state, Some("member-session")).await,
        (StatusCode::OK, empty.clone())
    );
    for channel in ["ChannelAlpha", "ChannelBravo"] {
        let mut room = state.clone();
        room.media_channel = Some(channel.into());
        let participants = room
            .update(|r| Ok(std::mem::take(&mut r.participants)))
            .await
            .unwrap();
        assert_eq!(
            voice_activity(state, Some("member-session")).await,
            (StatusCode::OK, empty.clone())
        );
        room.update(|r| {
            r.participants = participants.clone();
            for p in r.participants.values_mut() {
                p.monitor = None;
            }
            r.reservations.clear();
            Ok(())
        })
        .await
        .unwrap();
        // The first restored room is now active; mark it unjoined until both
        // reservation-only checks have completed.
        sqlx::query("UPDATE channel_joins SET deleted_at=now() WHERE channel_id=(SELECT id FROM channels WHERE external_id=$1) AND user_id=$2")
            .bind(channel).bind(member).execute(pool).await.unwrap();
    }
    sqlx::query("UPDATE channel_joins SET deleted_at=NULL WHERE user_id=$1")
        .bind(member)
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        voice_activity(state, Some("member-session")).await,
        (StatusCode::OK, active)
    );
}

async fn request(
    state: &AppState,
    channel: &str,
    operation: &str,
    account: Option<&str>,
    media: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method(if operation == "presence" || operation == "status" {
            "GET"
        } else {
            "POST"
        })
        .uri(format!("/api/channels/{channel}/media/{operation}"))
        .header("content-type", "application/json");
    if let Some(account) = account {
        request = request.header("cookie", format!("caper_session={account}"));
    }
    if let Some(media) = media {
        request = request.header("x-caper-media-token", media);
    }
    let response = app(state.clone())
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), BODY_LIMIT).await.unwrap();
    (
        status,
        if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        },
    )
}

async fn isolation_and_revocation(shared: bool) {
    let url = std::env::var("CHAT_TEST_DATABASE_URL").expect("disposable database required");
    let options = PgConnectOptions::from_str(&url).unwrap();
    assert!(matches!(options.get_host(), "localhost" | "127.0.0.1"));
    let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
    let database = format!("media_channel_test_{}", Uuid::new_v4().simple());
    admin
        .execute(format!("CREATE DATABASE {database}").as_str())
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(6)
        .connect_with(options.database(&database))
        .await
        .unwrap();
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();
    let mut users = vec![];
    for (name, token) in [
        ("owner", "owner-session"),
        ("member", "member-session"),
        ("outside", "outside-session"),
    ] {
        let id: i64 = sqlx::query_scalar("INSERT INTO public.users (external_id,username,display_name) VALUES ($1,$1,$1) RETURNING id")
            .bind(name).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.account_sessions (token_hash,user_id,expires_at) VALUES ($1,$2,now()+interval '1 hour')")
            .bind(Sha256::digest(token.as_bytes()).as_slice()).bind(id).execute(&pool).await.unwrap();
        users.push(id);
    }
    let space: i64 = sqlx::query_scalar("INSERT INTO public.spaces (external_id,name,owner_id) VALUES ('SpaceExample','Example',$1) RETURNING id")
        .bind(users[0]).fetch_one(&pool).await.unwrap();
    for user in &users[..2] {
        sqlx::query("INSERT INTO public.space_members (space_id,user_id) VALUES ($1,$2)")
            .bind(space)
            .bind(user)
            .execute(&pool)
            .await
            .unwrap();
    }
    for (external, name) in [("ChannelAlpha", "alpha"), ("ChannelBravo", "bravo")] {
        sqlx::query("INSERT INTO public.channels (external_id,space_id,name) VALUES ($1,$2,$3)")
            .bind(external)
            .bind(space)
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) SELECT c.id,sm.user_id FROM public.channels c JOIN public.space_members sm ON sm.space_id=c.space_id WHERE c.space_id=$1")
        .bind(space).execute(&pool).await.unwrap();
    // An account with a different rail space must not discover this room.
    sqlx::query("WITH s AS (INSERT INTO spaces(external_id,name,owner_id) VALUES('OutsideSpace','Outside',$1) RETURNING id)
                 INSERT INTO space_members(space_id,user_id) SELECT id,$1 FROM s")
        .bind(users[2]).execute(&pool).await.unwrap();
    let mock = Arc::new(Mock::new());
    let mut state = AppState::with_database(Config::test(true), mock.clone(), Some(pool.clone()));
    state.auth = auth::AuthVerifier::new();
    let key = format!("caper:{{channel-test-{}}}:state", Uuid::new_v4());
    let broker = if shared {
        let url = std::env::var("TEST_VALKEY_URL").expect("disposable broker required");
        state.connect_media_store(&url, &key).await.unwrap();
        Some(url)
    } else {
        None
    };
    voice_start_notifications(&state, &pool).await;
    assert_eq!(
        voice_activity(&state, Some("member-session")).await,
        (StatusCode::OK, json!({"spaceIds":[]}))
    );
    assert_eq!(
        voice_activity(&state, Some("invalid-session")).await.0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &state,
            "ChannelAlpha",
            "join",
            None,
            None,
            json!({"name":"guest"})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &state,
            "ChannelAlpha",
            "presence",
            Some("outside-session"),
            None,
            json!({})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    let clicked = Timestamp::now() - Duration::from_millis(2_345);
    let (status, alpha) = request(
        &state,
        "ChannelAlpha",
        "join",
        Some("owner-session"),
        None,
        json!({"name":"Owner","joinStartedAt":clicked}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{alpha}");
    let (status, bravo) = request(
        &state,
        "ChannelBravo",
        "join",
        Some("member-session"),
        None,
        json!({"name":"Member"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{bravo}");
    let alpha_token = alpha["token"].as_str().unwrap();
    let bravo_token = bravo["token"].as_str().unwrap();
    voice_activity_contract(&state, &pool, users[1]).await;
    let (_, roster) = request(
        &state,
        "ChannelAlpha",
        "presence",
        Some("member-session"),
        None,
        json!({}),
    )
    .await;
    assert_eq!(roster["participants"].as_array().unwrap().len(), 1);
    assert_eq!(roster["participants"][0]["id"], alpha["id"]);
    assert_eq!(
        roster["sessionStartedAt"],
        json!(clicked),
        "another member sees the initiating member's click time"
    );
    let (status, own) = request(
        &state,
        "ChannelAlpha",
        "snapshot",
        Some("owner-session"),
        Some(alpha_token),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(own["sessionStartedAt"], roster["sessionStartedAt"]);
    assert_eq!(
        state.read(|r| Ok(r.participants.len())).await.unwrap(),
        0,
        "demo must stay empty"
    );
    assert_eq!(
        request(
            &state,
            "ChannelBravo",
            "snapshot",
            Some("owner-session"),
            Some(alpha_token),
            json!({})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &state,
            "ChannelAlpha",
            "snapshot",
            Some("member-session"),
            Some(alpha_token),
            json!({})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED,
        "same-channel token bound to account session"
    );
    assert_eq!(
        call(
            app(state.clone()),
            "POST",
            "/api/media/snapshot",
            Some(alpha_token),
            json!({})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let (status, published) = request(
        &state,
        "ChannelAlpha",
        "publish",
        Some("owner-session"),
        Some(alpha_token),
        json!({"kind":"microphone","mid":"0","sessionDescription":{"type":"offer","sdp":"offer"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{published}");
    assert_eq!(
        request(
            &state,
            "ChannelBravo",
            "subscribe",
            Some("member-session"),
            Some(bravo_token),
            json!({"trackId":published["trackId"]})
        )
        .await
        .0,
        StatusCode::NOT_FOUND,
        "cross-channel track must never reach provider"
    );

    // Leaving an otherwise public channel gates the existing voice capability.
    sqlx::query("UPDATE public.channel_joins SET deleted_at=now() WHERE channel_id=(SELECT id FROM public.channels WHERE external_id='ChannelBravo') AND user_id=$1 AND deleted_at IS NULL")
        .bind(users[1]).execute(&pool).await.unwrap();
    assert_eq!(
        request(
            &state,
            "ChannelBravo",
            "snapshot",
            Some("member-session"),
            Some(bravo_token),
            json!({})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) SELECT id,$1 FROM public.channels WHERE external_id='ChannelBravo'")
        .bind(users[1]).execute(&pool).await.unwrap();

    // Privacy changes immediately gate reads and the existing participant token.
    sqlx::query("UPDATE public.channels SET private=true WHERE external_id='ChannelBravo'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(
            &state,
            "ChannelBravo",
            "snapshot",
            Some("member-session"),
            Some(bravo_token),
            json!({})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    // A fresh pod discovers the active room and revokes the old SFU session.
    let mut cleanup = if shared {
        let mut pod = AppState::with_database(Config::test(true), mock.clone(), Some(pool.clone()));
        pod.connect_media_store(broker.as_deref().unwrap(), &key)
            .await
            .unwrap();
        pod
    } else {
        state.clone()
    };
    let rooms = cleanup.media_rooms().await.unwrap();
    assert!(
        rooms
            .iter()
            .any(|r| r.media_channel.as_deref() == Some("ChannelBravo"))
    );
    cleanup.media_channel = Some("ChannelBravo".into());
    cleanup.revoke_media_access().await.unwrap();
    assert_eq!(cleanup.read(|r| Ok(r.participants.len())).await.unwrap(), 0);
    retry_backlog(&cleanup).await;
    assert!(
        !mock.revocations.lock().await.is_empty(),
        "TURN credentials revoked"
    );
    let idle_revision = cleanup.read(|r| Ok(r.revision)).await.unwrap();
    assert!(idle_revision > 0);
    assert!(
        !state
            .media_rooms()
            .await
            .unwrap()
            .iter()
            .any(|r| r.media_channel.as_deref() == Some("ChannelBravo"))
    );
    if let Some(url) = &broker {
        let mut connection = redis::Client::open(url.as_str())
            .unwrap()
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let ttl: i64 = redis::cmd("TTL")
            .arg(format!("{key}:channel:ChannelBravo"))
            .query_async(&mut connection)
            .await
            .unwrap();
        assert_eq!(
            ttl, -1,
            "idle metadata must not expire and reset a spectator's revision"
        );
    }

    // Grant enables a fresh call, then removing space membership closes its SSE.
    sqlx::query("INSERT INTO public.channel_members (channel_id,user_id) SELECT id,$1 FROM public.channels WHERE external_id='ChannelBravo'")
        .bind(users[1]).execute(&pool).await.unwrap();
    let (status, rejoined) = request(
        &state,
        "ChannelBravo",
        "join",
        Some("member-session"),
        None,
        json!({"name":"Member"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{rejoined}");
    assert!(cleanup.read(|r| Ok(r.revision)).await.unwrap() > idle_revision);
    let response = app(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/channels/ChannelBravo/media/presence/events")
                .header("cookie", "caper_session=member-session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut events = response.into_body().into_data_stream();
    assert!(events.next().await.unwrap().is_ok()); // ready
    assert!(events.next().await.unwrap().is_ok()); // initial snapshot
    sqlx::query("UPDATE public.space_members SET deleted_at=now() WHERE space_id=$1 AND user_id=$2 AND deleted_at IS NULL")
        .bind(space)
        .bind(users[1])
        .execute(&pool)
        .await
        .unwrap();
    cleanup.revoke_media_access().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), events.next())
            .await
            .unwrap()
            .is_none()
    );
    retry_backlog(&cleanup).await;

    // Participants who still take part keep their calls.
    cleanup.media_channel = Some("ChannelAlpha".into());
    let present = cleanup.read(|r| Ok(r.participants.len())).await.unwrap();
    assert!(present > 0);
    cleanup.revoke_media_access().await.unwrap();
    assert_eq!(
        cleanup.read(|r| Ok(r.participants.len())).await.unwrap(),
        present
    );

    sqlx::query("UPDATE public.account_sessions SET revoked_at=now() WHERE user_id=$1")
        .bind(users[0])
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        request(
            &state,
            "ChannelAlpha",
            "snapshot",
            Some("owner-session"),
            Some(alpha_token),
            json!({})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    cleanup.media_channel = Some("ChannelAlpha".into());
    cleanup.revoke_media_access().await.unwrap();
    retry_backlog(&cleanup).await;
    assert_eq!(
        mock.closes.lock().await.len(),
        1,
        "published microphone closed after logout"
    );
    sqlx::query("INSERT INTO public.account_sessions (token_hash,user_id,expires_at) VALUES ($1,$2,now()+interval '1 hour')")
        .bind(Sha256::digest(b"fresh-owner-session").as_slice()).bind(users[0]).execute(&pool).await.unwrap();
    for (channel, deletion) in [
        (
            "ChannelAlpha",
            "UPDATE public.channels SET deleted_at=now() WHERE external_id='ChannelAlpha'",
        ),
        (
            "ChannelBravo",
            "UPDATE public.spaces SET deleted_at=now() WHERE external_id='SpaceExample'",
        ),
    ] {
        assert_eq!(
            request(
                &state,
                channel,
                "join",
                Some("fresh-owner-session"),
                None,
                json!({"name":"Owner"})
            )
            .await
            .0,
            StatusCode::OK
        );
        pool.execute(deletion).await.unwrap();
        assert_eq!(
            request(
                &state,
                channel,
                "presence",
                Some("fresh-owner-session"),
                None,
                json!({})
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
        cleanup.media_channel = Some(channel.into());
        cleanup.revoke_media_access().await.unwrap();
        assert_eq!(
            cleanup.read(|r| Ok(r.participants.len())).await.unwrap(),
            0,
            "deletion ends existing voice sessions"
        );
        retry_backlog(&cleanup).await;
    }
    if let Some(url) = broker {
        assert!(
            state
                .store
                .as_ref()
                .unwrap()
                .active_channels()
                .await
                .unwrap()
                .is_empty()
        );
        let mut connection = redis::Client::open(url)
            .unwrap()
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        redis::cmd("DEL")
            .arg(&[
                key.clone(),
                format!("{key}:active"),
                format!("{key}:channel:ChannelAlpha"),
                format!("{key}:channel:ChannelBravo"),
            ])
            .query_async::<()>(&mut connection)
            .await
            .unwrap();
    }
    drop(events);
    pool.close().await;
    // Backend shutdown can lag pool closure. This uniquely named loopback
    // database belongs only to this test, like the chat/space test databases.
    admin
        .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL"]
async fn memory_channels_isolate_rosters_tokens_tracks_and_revoke_access() {
    isolation_and_revocation(false).await;
}

#[tokio::test]
#[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL and TEST_VALKEY_URL"]
async fn shared_channels_isolate_and_recover_cleanup_on_fresh_pod() {
    isolation_and_revocation(true).await;
}

/// Real join/leave routes, real Postgres and both media stores; provider mocked.
async fn voice_start_notifications(state: &AppState, pool: &sqlx::PgPool) {
    async fn count(pool: &sqlx::PgPool) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM notifications WHERE kind='voice.started'")
            .fetch_one(pool)
            .await
            .unwrap()
    }
    async fn enter(state: &AppState, channel: &str, account: &str) -> Value {
        let (status, body) = request(state, channel, "join", Some(account), None, json!({})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }
    async fn exit(state: &AppState, channel: &str, account: &str, joined: &Value) {
        let (status, body) = request(
            state,
            channel,
            "leave",
            Some(account),
            joined["token"].as_str(),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    }
    async fn cool(pool: &sqlx::PgPool) {
        pool.execute(
            "UPDATE voice_notification_state SET notified_at=now()-interval '1801 seconds'",
        )
        .await
        .unwrap();
    }
    // Failed authorization cannot start voice or notify.
    assert_eq!(
        request(
            state,
            "ChannelAlpha",
            "join",
            Some("outside-session"),
            None,
            json!({})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(count(pool).await, 0);
    // Concurrent first joins in distinct rooms generate only ONE event. Neither
    // provisional insertion may suppress both events, even on separate stores.
    let (a, b) = tokio::join!(
        enter(state, "ChannelAlpha", "owner-session"),
        enter(state, "ChannelBravo", "member-session")
    );
    assert_eq!(count(pool).await, 1);
    let (recipient, actor): (i64, i64) =
        sqlx::query_as("SELECT user_id,actor_id FROM notifications WHERE kind='voice.started'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_ne!(recipient, actor);
    let events = crate::push::live::read(pool, recipient, 0, i64::MAX, None)
        .await
        .unwrap();
    let event = events.into_iter().find_map(|(_, event)| event).unwrap();
    assert_eq!(event["kind"], "voice.started");
    assert!(event.get("notificationId").is_some());
    assert!(event.get("messageId").is_none());
    assert_eq!(event["spaceId"], "SpaceExample");
    assert!(
        event["body"]
            .as_str()
            .unwrap()
            .contains("joined voice in #")
    );
    exit(state, "ChannelAlpha", "owner-session", &a).await;
    // Even after the cooldown, another occupied channel suppresses the alert.
    cool(pool).await;
    let a = enter(state, "ChannelAlpha", "owner-session").await;
    assert_eq!(count(pool).await, 1);
    exit(state, "ChannelAlpha", "owner-session", &a).await;
    exit(state, "ChannelBravo", "member-session", &b).await;
    let a = enter(state, "ChannelAlpha", "owner-session").await;
    assert_eq!(count(pool).await, 2);
    exit(state, "ChannelAlpha", "owner-session", &a).await;
    pool.execute("UPDATE voice_notification_state SET notified_at=now()-interval '29 minutes'")
        .await
        .unwrap();
    let b = enter(state, "ChannelBravo", "member-session").await;
    assert_eq!(count(pool).await, 2, "empty-room rejoin after 29 minutes");
    exit(state, "ChannelBravo", "member-session", &b).await;
    // Private activity never exposes a channel to a member without a grant.
    pool.execute("UPDATE channels SET private=true WHERE external_id='ChannelAlpha'")
        .await
        .unwrap();
    cool(pool).await;
    let a = enter(state, "ChannelAlpha", "owner-session").await;
    assert_eq!(count(pool).await, 2);
    exit(state, "ChannelAlpha", "owner-session", &a).await;
    pool.execute("INSERT INTO channel_members(channel_id,user_id) SELECT c.id,u.id FROM channels c CROSS JOIN users u WHERE c.external_id='ChannelAlpha' AND u.username='member'").await.unwrap();
    cool(pool).await;
    let a = enter(state, "ChannelAlpha", "owner-session").await;
    assert_eq!(count(pool).await, 3);
    // Expired leases do not keep a space occupied; the cooldown still applies.
    let mut room = state.clone();
    room.media_channel = Some("ChannelAlpha".into());
    room.update(|r| {
        for p in r.participants.values_mut() {
            p.lease = Timestamp::now() - LEASE - Duration::from_secs(1);
        }
        Ok(())
    })
    .await
    .unwrap();
    cool(pool).await;
    let b = enter(state, "ChannelBravo", "member-session").await;
    assert_eq!(count(pool).await, 4);
    // Restore fixtures for the surrounding authorization/cleanup contract.
    let id = Uuid::parse_str(a["id"].as_str().unwrap()).unwrap();
    remove_participant(&room, id).await;
    exit(state, "ChannelBravo", "member-session", &b).await;
    for channel in ["ChannelAlpha", "ChannelBravo"] {
        room.media_channel = Some(channel.into());
        while room.read(|r| Ok(!r.cleanup.is_empty())).await.unwrap() {
            retry_backlog(&room).await;
        }
    }
    pool.execute("UPDATE channels SET private=false WHERE external_id='ChannelAlpha'; DELETE FROM channel_members").await.unwrap();
}
