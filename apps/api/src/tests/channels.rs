//! Real channel authorization and room storage; SFU is explicitly mocked.
use super::*;
use sqlx::{
    Connection, Executor,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::str::FromStr;

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
    let mock = Arc::new(Mock::new());
    let mut state = AppState::with_database(Config::test(true), mock.clone(), Some(pool.clone()));
    let key = format!("caper:{{channel-test-{}}}:state", Uuid::new_v4());
    let broker = if shared {
        let url = std::env::var("TEST_VALKEY_URL").expect("disposable broker required");
        state.connect_media_store(&url, &key).await.unwrap();
        Some(url)
    } else {
        None
    };
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
    let (status, alpha) = request(
        &state,
        "ChannelAlpha",
        "join",
        Some("owner-session"),
        None,
        json!({"name":"Owner"}),
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
    sqlx::query("DELETE FROM public.space_members WHERE space_id=$1 AND user_id=$2")
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
    admin
        .execute(format!("DROP DATABASE {database}").as_str())
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
