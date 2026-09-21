//! Real Valkey/Redis transactions and Pub/Sub, with a mocked Cloudflare provider.
//! Run explicitly against a disposable server; never against production.
use super::*;

async fn shared() -> (AppState, AppState, Arc<Mock>, String, String) {
    let url = std::env::var("TEST_VALKEY_URL").expect("set TEST_VALKEY_URL to disposable Valkey");
    let key = format!("caper:{{test-{}}}:v1:state", Uuid::new_v4());
    let provider = Arc::new(Mock::new());
    let mut a = AppState::new(Config::test(true), provider.clone());
    let mut b = AppState::new(Config::test(true), provider.clone());
    a.connect_media_store(&url, &key).await.unwrap();
    b.connect_media_store(&url, &key).await.unwrap();
    (a, b, provider, url, key)
}

async fn request(s: &AppState, route: &str, token: &str, body: Value) -> Value {
    let (status, body) = call(
        app(s.clone()),
        "POST",
        &format!("/api/media/{route}"),
        Some(token),
        body,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{route}: {body}");
    body
}

async fn open(s: &AppState, token: &str) -> axum::body::BodyDataStream {
    let response = app(s.clone())
        .oneshot(
            Request::builder()
                .uri("/api/media/events?snapshots=1")
                .header("x-caper-media-token", token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.into_body().into_data_stream()
}

async fn event(stream: &mut axum::body::BodyDataStream, expected: &str) -> Value {
    let bytes = tokio::time::timeout(Duration::from_secs(2), stream.next())
        .await
        .expect("push must arrive without waiting for a heartbeat")
        .unwrap()
        .unwrap();
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(text.starts_with(&format!("event: {expected}\n")), "{text}");
    serde_json::from_str(
        text.lines()
            .find_map(|line| line.strip_prefix("data: "))
            .unwrap(),
    )
    .unwrap()
}

async fn delete(url: &str, key: &str) {
    let mut connection = redis::Client::open(url)
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    redis::cmd("DEL")
        .arg(key)
        .query_async::<()>(&mut connection)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires disposable TEST_VALKEY_URL"]
async fn retiring_streams_keep_pushing_after_replacement_registration() {
    fn http_events(response: reqwest::Response) -> axum::body::BodyDataStream {
        // HTTP chunks need not align with SSE frames.
        let frames = stream::unfold(
            (response, Vec::<u8>::new()),
            |(mut response, mut buffer)| async move {
                loop {
                    if let Some(end) = buffer.windows(2).position(|pair| pair == b"\n\n") {
                        let frame =
                            axum::body::Bytes::from(buffer.drain(..end + 2).collect::<Vec<_>>());
                        return Some((
                            Ok::<_, std::convert::Infallible>(frame),
                            (response, buffer),
                        ));
                    }
                    buffer.extend(response.chunk().await.unwrap()?);
                }
            },
        );
        Body::from_stream(frames).into_data_stream()
    }
    async fn serve(
        s: AppState,
    ) -> (
        String,
        tokio::sync::oneshot::Sender<()>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (stop, stopped) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app(s.clone()))
                .with_graceful_shutdown(async move {
                    stopped.await.unwrap();
                    s.begin_shutdown();
                })
                .await
                .unwrap();
        });
        (url, stop, server)
    }
    let (a, b, provider, url, key) = shared().await;
    let person = joined(&a, "phone").await;
    let token = person["token"].as_str().unwrap();
    let (old_url, stop_old, old_server) = serve(a.clone()).await;
    let (new_url, stop_new, new_server) = serve(b.clone()).await;
    let http = reqwest::Client::new();
    let response = http
        .get(format!("{old_url}/api/media/events?snapshots=1&handoff=1"))
        .header("x-caper-media-token", token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let mut old = http_events(response);
    let response = http
        .get(format!("{old_url}/api/media/presence/events?handoff=1"))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let mut spectator = http_events(response);
    for stream in [&mut old, &mut spectator] {
        event(stream, "ready").await;
        event(stream, "snapshot").await;
    }
    stop_old.send(()).unwrap();
    event(&mut old, "migrating").await;
    event(&mut spectator, "migrating").await;
    // Registration replaces the shared connection ID before a new snapshot is
    // delivered. Deliberately leave its body unread while old streams carry updates.
    let response = http
        .get(format!("{new_url}/api/media/events?snapshots=1&handoff=1"))
        .header("x-caper-media-token", token)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let mut replacement = http_events(response);
    for (muted, deafened) in [(true, false), (true, true), (false, false), (false, true)] {
        assert_eq!(
            call(
                app(b.clone()),
                "POST",
                "/api/media/state",
                Some(token),
                json!({"muted":muted,"deafened":deafened})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        for stream in [&mut old, &mut spectator] {
            let snapshot = event(stream, "snapshot").await;
            assert_eq!(snapshot["participants"][0]["muted"], muted);
            assert_eq!(snapshot["participants"][0]["deafened"], deafened);
        }
    }
    event(&mut replacement, "ready").await;
    let snapshot = event(&mut replacement, "snapshot").await;
    // Read all real HTTP frames queued while the replacement was unread.
    let mut snapshot = snapshot;
    while snapshot["participants"][0]["muted"] != false
        || snapshot["participants"][0]["deafened"] != true
    {
        snapshot = event(&mut replacement, "snapshot").await;
    }
    assert_eq!(snapshot["participants"][0]["muted"], false);
    assert_eq!(snapshot["participants"][0]["deafened"], true);
    assert!(provider.closes.lock().await.is_empty());
    assert_eq!(
        call(
            app(b.clone()),
            "POST",
            "/api/media/leave",
            Some(token),
            json!({})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    // Overlap never bypasses capability revocation.
    assert!(
        tokio::time::timeout(Duration::from_secs(2), old.next())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        event(&mut spectator, "snapshot").await["participants"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    drop(old);
    drop(spectator);
    drop(replacement);
    stop_new.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), old_server)
        .await
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), new_server)
        .await
        .unwrap()
        .unwrap();
    delete(&url, &key).await;
}

#[tokio::test]
#[ignore = "requires disposable TEST_VALKEY_URL"]
async fn stale_state_writes_cannot_win_across_pods() {
    let (a, b, _, url, key) = shared().await;
    reliability::exercise_state_ordering(&a, &b).await;
    a.begin_shutdown();
    b.begin_shutdown();
    delete(&url, &key).await;
}

#[tokio::test]
#[ignore = "requires disposable TEST_VALKEY_URL"]
async fn spectators_receive_cross_pod_leave_before_provider_cleanup() {
    let (a, b, _, url, key) = shared().await;
    exercise_public_presence(&a, &b).await;
    b.begin_shutdown();
    delete(&url, &key).await;
}

#[tokio::test]
#[ignore = "requires disposable TEST_VALKEY_URL"]
async fn renewal_replays_across_api_instances() {
    let (a, b, mock, url, key) = shared().await;
    renewal::exercise_rotation(&a, &b, &mock).await;
    a.begin_shutdown();
    b.begin_shutdown();
    delete(&url, &key).await;
}

#[tokio::test]
#[ignore = "requires disposable TEST_VALKEY_URL"]
async fn long_calls_keep_their_session_across_pods() {
    let (a, b, _, url, key) = shared().await;
    reliability::exercise_long_call(&a, &b).await;
    a.begin_shutdown();
    b.begin_shutdown();
    delete(&url, &key).await;
}

#[tokio::test]
#[ignore = "requires disposable TEST_VALKEY_URL"]
async fn cross_pod_media_events_shutdown_and_replacement() {
    let (a, b, provider, url, key) = shared().await;
    let alice = joined(&a, "Alice").await;
    let alice_token = alice["token"].as_str().unwrap();
    let bob = joined(&b, "Bob").await;
    let bob_token = bob["token"].as_str().unwrap();
    assert!(
        a.registry.lock().await.participants.is_empty(),
        "no local authoritative copy"
    );
    assert!(b.registry.lock().await.participants.is_empty());
    let mut events = open(&b, bob_token).await;
    event(&mut events, "ready").await;
    assert_eq!(
        event(&mut events, "snapshot").await["participants"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let published = request(&b, "publish", alice_token,
        json!({"kind":"microphone","mid":"alice-mic","sessionDescription":{"type":"offer","sdp":"v=0"}})).await;
    let pushed = event(&mut events, "snapshot").await;
    assert_eq!(
        pushed["participants"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "Alice")
            .unwrap()["tracks"][0]["id"],
        published["trackId"]
    );
    request(
        &a,
        "subscribe",
        bob_token,
        json!({"trackId":published["trackId"]}),
    )
    .await;
    request(
        &b,
        "negotiate",
        bob_token,
        json!({"sessionDescription":{"type":"answer","sdp":"v=0"}}),
    )
    .await;
    let raw = b
        .read(|r| Ok(serde_json::to_string(r).unwrap()))
        .await
        .unwrap();
    assert!(
        !raw.contains(alice_token) && !raw.contains(bob_token),
        "store only token hashes"
    );
    assert!(!raw.contains("v=0"), "never store SDP");
    for muted in [true, false, true, false] {
        assert_eq!(
            call(
                app(a.clone()),
                "POST",
                "/api/media/state",
                Some(alice_token),
                json!({"muted":muted,"deafened":false})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        let snapshot = event(&mut events, "snapshot").await;
        assert_eq!(
            snapshot["participants"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == alice["id"])
                .unwrap()["muted"],
            muted
        );
    }
    let expected_mapping = b
        .read(|r| {
            Ok(r.participants
                .values()
                .map(|p| (p.id, p.session.clone()))
                .collect::<HashMap<_, _>>())
        })
        .await
        .unwrap();
    shutdown_cleanup(&a).await;
    shutdown_cleanup(&b).await;
    event(&mut events, "draining").await;
    assert!(events.next().await.is_none());
    assert!(provider.closes.lock().await.is_empty());
    assert!(provider.revocations.lock().await.is_empty());
    drop(events);
    drop(a);
    drop(b);

    let mut replacement = AppState::new(Config::test(true), provider.clone());
    replacement.connect_media_store(&url, &key).await.unwrap();
    assert_eq!(
        replacement
            .read(|r| Ok(r
                .participants
                .values()
                .map(|p| (p.id, p.session.clone()))
                .collect::<HashMap<_, _>>()))
            .await
            .unwrap(),
        expected_mapping
    );
    request(&replacement, "snapshot", alice_token, json!({})).await;
    let mut old = open(&replacement, bob_token).await;
    event(&mut old, "ready").await;
    event(&mut old, "snapshot").await;
    let mut other = AppState::new(Config::test(true), provider.clone());
    other.connect_media_store(&url, &key).await.unwrap();
    let mut resumed = open(&other, bob_token).await;
    event(&mut resumed, "ready").await;
    event(&mut resumed, "snapshot").await;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), old.next())
            .await
            .unwrap()
            .is_none(),
        "replacement revokes the old pod's stream"
    );
    for (index, muted) in [true, false, true, false].into_iter().enumerate() {
        let writer = if index % 2 == 0 { &replacement } else { &other };
        assert_eq!(
            call(
                app(writer.clone()),
                "POST",
                "/api/media/state",
                Some(alice_token),
                json!({"muted":muted,"deafened":false})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        let snapshot = event(&mut resumed, "snapshot").await;
        assert_eq!(
            snapshot["participants"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["id"] == alice["id"])
                .unwrap()["muted"],
            muted
        );
    }
    assert!(provider.closes.lock().await.is_empty());
    assert!(provider.revocations.lock().await.is_empty());
    assert_eq!(
        call(
            app(other.clone()),
            "POST",
            "/api/media/leave",
            Some(alice_token),
            json!({})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        event(&mut resumed, "snapshot").await["participants"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        !other.read(|r| Ok(r.cleanup.is_empty())).await.unwrap(),
        "leave persisted cleanup atomically"
    );
    tokio::join!(retry_backlog(&replacement), retry_backlog(&other));
    let closes = provider.closes.lock().await.clone();
    assert_eq!(
        closes.iter().filter(|(_, mid)| mid == "alice-mic").count(),
        1
    );
    assert_eq!(
        closes.iter().filter(|(_, mid)| mid == "remote-mid").count(),
        1
    );
    assert_eq!(provider.revocations.lock().await.len(), 1);
    drop(old);
    drop(resumed);
    drop(replacement);
    drop(other);
    delete(&url, &key).await;
}

#[tokio::test]
#[ignore = "requires disposable TEST_VALKEY_URL"]
async fn shared_capacity_operations_and_abandoned_work() {
    let (a, b, provider, url, key) = shared().await;
    let results = futures_util::future::join_all((0..18).map(|n| {
        let s = if n % 2 == 0 { a.clone() } else { b.clone() };
        async move {
            call(
                app(s),
                "POST",
                "/api/media/join",
                None,
                json!({"name":format!("p{n}")}),
            )
            .await
        }
    }))
    .await;
    assert_eq!(
        results
            .iter()
            .filter(|(status, _)| *status == StatusCode::OK)
            .count(),
        MAX_PARTICIPANTS,
        "join statuses/errors: {:?}",
        results
            .iter()
            .map(|(status, body)| (*status, body.get("error")))
            .collect::<Vec<_>>()
    );
    assert!(
        results
            .iter()
            .all(|(status, _)| matches!(*status, StatusCode::OK | StatusCode::CONFLICT)),
        "join statuses/errors: {:?}",
        results
            .iter()
            .map(|(status, body)| (*status, body.get("error")))
            .collect::<Vec<_>>()
    );
    let joined = &results
        .iter()
        .find(|(status, _)| *status == StatusCode::OK)
        .unwrap()
        .1;
    let token = joined["token"].as_str().unwrap();
    let id = joined["id"].as_str().unwrap().parse::<Uuid>().unwrap();
    let acquire = |r: &mut Registry| begin_operation(r.participants.get_mut(&id).unwrap());
    let (one, two) = tokio::join!(a.update(acquire), b.update(acquire));
    assert_ne!(
        one.is_ok(),
        two.is_ok(),
        "exactly one cross-pod operation may own the peer"
    );
    b.update(|r| {
        r.participants.get_mut(&id).unwrap().operation_started =
            Some(Timestamp::now() - Duration::from_secs(31));
        r.reservations.insert(
            Uuid::nil(),
            JoinReservation {
                started: Timestamp::now() - Duration::from_secs(31),
                monitor: None,
            },
        );
        Ok(())
    })
    .await
    .unwrap();
    expire_sessions(&a).await.unwrap();
    b.read(|r| {
        assert!(!r.participants.contains_key(&id));
        assert!(r.reservations.is_empty());
        assert!(
            r.cleanup
                .iter()
                .any(|job| matches!(job.action, CleanupAction::Discover { .. }))
        );
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(
        call(
            app(b.clone()),
            "POST",
            "/api/media/snapshot",
            Some(token),
            json!({})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    // A dead worker must not take its job with it. An unexpired claim is not stolen.
    a.update(|r| {
        for job in &mut r.cleanup {
            job.claim = Some(Uuid::nil());
            job.not_before = Timestamp::now() + Duration::from_secs(30);
        }
        Ok(())
    })
    .await
    .unwrap();
    retry_backlog(&b).await;
    assert!(provider.revocations.lock().await.is_empty());
    a.update(|r| {
        for job in &mut r.cleanup {
            job.not_before = Timestamp::now() - Duration::from_secs(1);
        }
        Ok(())
    })
    .await
    .unwrap();
    drop(a);
    retry_backlog(&b).await;
    assert_eq!(provider.revocations.lock().await.len(), 1);
    assert!(b.read(|r| Ok(r.cleanup.is_empty())).await.unwrap());
    drop(b);
    delete(&url, &key).await;
}

#[tokio::test]
#[ignore = "requires disposable TEST_VALKEY_URL"]
async fn transaction_conflicts_retry_until_deadline_without_replaying_mutations() {
    let (a, b, _, url, key) = shared().await;
    a.update(|r| {
        r.joins.push_back(Timestamp::now());
        Ok(())
    })
    .await
    .unwrap();
    let competing = std::sync::Mutex::new(
        redis::Client::open(url.as_str())
            .unwrap()
            .get_connection()
            .unwrap(),
    );
    // This test-only competing client changes the TTL after WATCH/read and before
    // EXEC. No production callback performs I/O. Each expiry change invalidates
    // WATCH without changing the stored registry's fields.
    let conflict = || {
        let changed: i64 = redis::cmd("PEXPIRE")
            .arg(&key)
            .arg(60_000)
            .query(&mut *competing.lock().unwrap())
            .unwrap();
        assert_eq!(changed, 1);
    };
    let attempts = std::cell::Cell::new(0);
    a.update(|r| {
        attempts.set(attempts.get() + 1);
        if attempts.get() <= 20 {
            conflict();
        }
        r.joins.push_back(Timestamp::now());
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(attempts.get(), 21);
    assert_eq!(b.read(|r| Ok(r.joins.len())).await.unwrap(), 2);

    let error = tokio::time::timeout(
        Duration::from_secs(5),
        a.update(|r| {
            conflict();
            r.joins.clear();
            Ok(())
        }),
    )
    .await
    .expect("persistent contention must still respect the store I/O deadline")
    .unwrap_err();
    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(b.read(|r| Ok(r.joins.len())).await.unwrap(), 2);
    delete(&url, &key).await;
}

#[tokio::test]
#[ignore = "requires disposable TEST_VALKEY_URL with ACL administration"]
async fn store_outage_fails_closed_then_resumes_current_state() {
    let (a, b, provider, url, key) = shared().await;
    let alice = joined(&a, "Alice").await;
    let token = alice["token"].as_str().unwrap();
    let user = format!("test-{}", Uuid::new_v4());
    let mut admin = redis::Client::open(url.as_str())
        .unwrap()
        .get_multiplexed_async_connection()
        .await
        .unwrap();
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg(&user)
        .arg("on")
        .arg(">test-only-password")
        .arg(format!("~{key}"))
        .arg("&*")
        .arg("+@all")
        .query_async::<()>(&mut admin)
        .await
        .unwrap();
    let mut restricted_url = reqwest::Url::parse(&url).unwrap();
    restricted_url.set_username(&user).unwrap();
    restricted_url
        .set_password(Some("test-only-password"))
        .unwrap();
    let mut isolated = AppState::new(Config::test(true), provider.clone());
    isolated
        .connect_media_store(restricted_url.as_str(), &key)
        .await
        .unwrap();
    let mut events = open(&isolated, token).await;
    event(&mut events, "ready").await;
    event(&mut events, "snapshot").await;
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg(&user)
        .arg("off")
        .arg("-@all")
        .query_async::<()>(&mut admin)
        .await
        .unwrap();
    redis::cmd("CLIENT")
        .arg("KILL")
        .arg("USER")
        .arg(&user)
        .query_async::<usize>(&mut admin)
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), events.next())
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ready(State(isolated.clone())).await.unwrap_err().status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        call(
            app(isolated.clone()),
            "POST",
            "/api/media/join",
            None,
            json!({"name":"must not provision"})
        )
        .await
        .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        provider.next.load(Ordering::SeqCst),
        2,
        "outage must not create a local-only call"
    );
    for muted in [true, false, true] {
        assert_eq!(
            call(
                app(b.clone()),
                "POST",
                "/api/media/state",
                Some(token),
                json!({"muted":muted,"deafened":false})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
    }
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg(&user)
        .arg("on")
        .arg("+@all")
        .query_async::<()>(&mut admin)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if ready(State(isolated.clone())).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap();
    let mut resumed = open(&isolated, token).await;
    event(&mut resumed, "ready").await;
    let latest = event(&mut resumed, "snapshot").await;
    assert_eq!(latest["participants"][0]["muted"], true);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), resumed.next())
            .await
            .is_err(),
        "do not replay transitions missed while disconnected"
    );
    drop(events);
    drop(resumed);
    drop(isolated);
    drop(a);
    drop(b);
    redis::cmd("ACL")
        .arg("DELUSER")
        .arg(&user)
        .query_async::<()>(&mut admin)
        .await
        .unwrap();
    delete(&url, &key).await;
}
