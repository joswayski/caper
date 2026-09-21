use super::*;

#[tokio::test(start_paused = true)]
async fn handoff_is_opt_in_and_bounded_even_without_replacement() {
    for handoff in [false, true] {
        let (s, _) = state();
        let mut stream = event_stream(s.clone(), None, true, handoff)
            .await
            .unwrap()
            .into_body()
            .into_data_stream();
        next_event(&mut stream).await;
        presence_snapshot_event(&mut stream).await;
        s.begin_shutdown();
        let frame = next_event(&mut stream).await.unwrap();
        assert!(frame.starts_with(if handoff {
            "event: migrating"
        } else {
            "event: draining"
        }));
        if handoff {
            assert!(
                tokio::time::timeout(Duration::from_secs(9), stream.next())
                    .await
                    .is_err()
            );
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        assert!(stream.next().await.is_none());
    }
}

#[tokio::test(start_paused = true)]
async fn unrelated_notifications_cannot_starve_sse_heartbeats() {
    let (s, _) = state();
    let mut stream = presence_response(&s).await.into_body().into_data_stream();
    next_event(&mut stream).await;
    presence_snapshot_event(&mut stream).await;
    let waiting = tokio::spawn(async move { stream.next().await.unwrap().unwrap() });
    tokio::task::yield_now().await;
    for _ in 0..3 {
        tokio::time::advance(Duration::from_secs(4)).await;
        s.events.send_replace(()); // connection changes, not a new roster revision
        tokio::task::yield_now().await;
    }
    assert!(
        waiting.is_finished(),
        "heartbeat must arrive despite repeated non-roster notifications"
    );
    let frame = String::from_utf8(waiting.await.unwrap().to_vec()).unwrap();
    assert!(frame.starts_with("event: heartbeat"), "{frame}");
}

#[tokio::test(start_paused = true)]
async fn heartbeat_delivers_a_new_revision_before_a_delayed_notification() {
    let (s, _) = state();
    let person = joined(&s, "phone").await;
    let id = person["id"].as_str().unwrap().parse().unwrap();
    let mut stream = presence_response(&s).await.into_body().into_data_stream();
    next_event(&mut stream).await;
    presence_snapshot_event(&mut stream).await;
    {
        // Simulate a committed shared-store update whose Pub/Sub wake is delayed.
        let mut r = s.registry.lock().await;
        r.participants.get_mut(&id).unwrap().muted = true;
        r.revision += 1;
    }
    let frame = tokio::time::timeout(Duration::from_secs(11), stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let frame = String::from_utf8(frame.to_vec()).unwrap();
    assert!(
        frame.starts_with("event: snapshot"),
        "new revision must be delivered, not silently acknowledged: {frame}"
    );
    let snapshot: Value = serde_json::from_str(
        frame
            .lines()
            .find_map(|l| l.strip_prefix("data: "))
            .unwrap(),
    )
    .unwrap();
    assert_eq!(snapshot["participants"][0]["muted"], true);
}

pub(super) async fn exercise_state_ordering(writer: &AppState, reader: &AppState) {
    let person = joined(writer, "phone").await;
    let token = person["token"].as_str();
    for (sequence, muted, deafened) in [(2, false, true), (1, true, false), (2, true, false)] {
        assert_eq!(
            call(
                app(writer.clone()),
                "POST",
                "/api/media/state",
                token,
                json!({"sequence":sequence,"muted":muted,"deafened":deafened})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        let (status, snapshot) = call(
            app(reader.clone()),
            "POST",
            "/api/media/snapshot",
            token,
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            snapshot["participants"][0]["muted"], false,
            "late/duplicate writes must not replace newer intent"
        );
        assert_eq!(snapshot["participants"][0]["deafened"], true);
    }
    assert_eq!(
        call(
            app(reader.clone()),
            "POST",
            "/api/media/state",
            token,
            json!({"sequence":3,"muted":true,"deafened":false})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    let (_, snapshot) = call(
        app(writer.clone()),
        "POST",
        "/api/media/snapshot",
        token,
        json!({}),
    )
    .await;
    assert_eq!(snapshot["participants"][0]["muted"], true);
    assert_eq!(snapshot["participants"][0]["deafened"], false);
}

#[tokio::test]
async fn delayed_state_write_cannot_overwrite_newer_intent() {
    let (s, _) = state();
    exercise_state_ordering(&s, &s).await;
}

#[tokio::test]
async fn active_calls_have_no_absolute_age_limit() {
    let (s, _) = state();
    exercise_long_call(&s, &s).await;
}

// Also run against two independent API instances sharing disposable Valkey.
pub(super) async fn exercise_long_call(writer: &AppState, reader: &AppState) {
    let parent = joined(writer, "long-running caller").await;
    let token = parent["token"].as_str().unwrap();
    let id = Uuid::parse_str(parent["id"].as_str().unwrap()).unwrap();
    let publication = call(
        app(writer.clone()),
        "POST",
        "/api/media/publish",
        Some(token),
        json!({"kind":"microphone","mid":"0","sessionDescription":{"type":"offer","sdp":"v=0"}}),
    )
    .await;
    assert_eq!(publication.0, StatusCode::OK);
    let session = writer
        .read(|r| Ok(r.participants[&id].session.clone()))
        .await
        .unwrap();
    writer
        .update(|r| {
            r.participants.get_mut(&id).unwrap().joined =
                Timestamp::now() - Duration::from_secs(49 * 60 * 60);
            Ok(())
        })
        .await
        .unwrap();
    let mut monitors = vec![];
    for role in ["sender", "receiver"] {
        let (status, monitor) = call(
            app(reader.clone()),
            "POST",
            "/api/media/join",
            Some(token),
            json!({"name":"private monitor", "monitor":role}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "an old active parent can start a monitor"
        );
        monitors.push(monitor);
    }
    // Exercise both sides of the old one-hour boundary and beyond TURN's new
    // lifetime. This proves API state policy, not live TURN/media longevity.
    for age in [3599, 3600, 3601, 49 * 60 * 60] {
        writer
            .update(|r| {
                for p in r.participants.values_mut() {
                    p.joined = Timestamp::now() - Duration::from_secs(age);
                    p.lease = Timestamp::now();
                }
                Ok(())
            })
            .await
            .unwrap();
        expire_sessions(reader).await.unwrap();
        let (status, snapshot) = call(
            app(reader.clone()),
            "POST",
            "/api/media/snapshot",
            Some(token),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(snapshot["participants"].as_array().unwrap().len(), 1);
        assert_eq!(snapshot["participants"][0]["id"], parent["id"]);
        assert_eq!(
            snapshot["participants"][0]["tracks"][0]["id"],
            publication.1["trackId"]
        );
        for monitor in &monitors {
            let (status, snapshot) = call(
                app(reader.clone()),
                "POST",
                "/api/media/snapshot",
                monitor["token"].as_str(),
                json!({}),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(snapshot["participants"], json!([]));
        }
        assert_eq!(
            call(
                app(writer.clone()),
                "POST",
                "/api/media/state",
                Some(token),
                json!({"muted":age % 2 == 0,"deafened":false}),
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        reader
            .read(|r| {
                assert_eq!(r.participants.len(), 3);
                assert_eq!(r.participants[&id].session, session);
                assert_eq!(r.participants[&id].muted, age % 2 == 0);
                assert!(
                    r.cleanup.is_empty(),
                    "no age-based track close or credential revocation"
                );
                Ok(())
            })
            .await
            .unwrap();
    }
    writer
        .update(|r| {
            r.participants.get_mut(&id).unwrap().lease = Timestamp::now() - LEASE;
            Ok(())
        })
        .await
        .unwrap();
    for participant in std::iter::once(&parent).chain(&monitors) {
        assert_eq!(
            call(
                app(reader.clone()),
                "POST",
                "/api/media/snapshot",
                participant["token"].as_str(),
                json!({}),
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED,
            "lease expiry still revokes parent and monitors"
        );
    }
    expire_sessions(reader).await.unwrap();
    writer
        .read(|r| {
            assert!(r.participants.is_empty());
            assert!(r.tokens.is_empty());
            assert_eq!(
                r.cleanup
                    .iter()
                    .filter(|job| matches!(job.action, CleanupAction::Revoke { .. }))
                    .count(),
                3
            );
            assert!(r.cleanup.iter().any(|job| job.action
                == CleanupAction::Close {
                    session: session.clone(),
                    mid: "0".into()
                }));
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn drain_rejections_are_marked_retryable_before_any_media_mutation() {
    let (s, provider) = state();
    let a = joined(&s, "listener").await;
    let before = serde_json::to_value(&*s.registry.lock().await).unwrap();
    s.begin_shutdown();
    for (operation, body) in [
        ("join", json!({"name":"new"})),
        (
            "publish",
            json!({"kind":"microphone","mid":"0","sessionDescription":{"type":"offer","sdp":"v=0"}}),
        ),
        ("subscribe", json!({"trackId":Uuid::new_v4()})),
        (
            "negotiate",
            json!({"sessionDescription":{"type":"answer","sdp":"v=0"}}),
        ),
        ("close", json!({"mid":"0"})),
        ("state", json!({"muted":true,"deafened":false})),
        ("snapshot", json!({})),
        ("leave", json!({})),
    ] {
        let (status, body) = call(
            app(s.clone()),
            "POST",
            &format!("/api/media/{operation}"),
            a["token"].as_str(),
            body,
        )
        .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{operation}");
        assert_eq!(body["code"], "api_draining", "{operation}");
    }
    assert_eq!(
        serde_json::to_value(&*s.registry.lock().await).unwrap(),
        before
    );
    assert_eq!(
        provider.next.load(Ordering::SeqCst),
        2,
        "no additional provider sessions"
    );
    assert!(provider.closes.lock().await.is_empty());
}

#[tokio::test]
async fn source_departure_during_subscription_preserves_listener_and_finishes_sdp() {
    for has_offer in [true, false] {
        let (s, provider) = state();
        provider.remote_offer.store(has_offer, Ordering::SeqCst);
        let speaker = joined(&s, "speaker").await;
        let listener = joined(&s, "listener").await;
        let listener_id = Uuid::parse_str(listener["id"].as_str().unwrap()).unwrap();
        let mut source_track = Value::Null;
        for (person, mid) in [(&speaker, "speaker-mic"), (&listener, "listener-mic")] {
            let published = call(app(s.clone()), "POST", "/api/media/publish", person["token"].as_str(),
                json!({"kind":"microphone","mid":mid,"sessionDescription":{"type":"offer","sdp":"v=0"}})).await;
            assert_eq!(published.0, StatusCode::OK);
            if mid == "speaker-mic" {
                source_track = published.1["trackId"].clone();
            }
        }
        let session = s.registry.lock().await.participants[&listener_id]
            .session
            .clone();
        provider.block_subscription.store(true, Ordering::SeqCst);
        let ((status, response), ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(
                call(
                    app(s.clone()),
                    "POST",
                    "/api/media/subscribe",
                    listener["token"].as_str(),
                    json!({"trackId":source_track})
                ),
                async {
                    provider.subscription_started.notified().await;
                    assert_eq!(
                        call(
                            app(s.clone()),
                            "POST",
                            "/api/media/leave",
                            speaker["token"].as_str(),
                            json!({})
                        )
                        .await
                        .0,
                        StatusCode::NO_CONTENT
                    );
                    provider.subscription_resume.notify_one();
                }
            )
        })
        .await
        .expect("subscription/departure interleaving must complete");
        assert_eq!(
            status,
            StatusCode::OK,
            "a departed source must not invalidate the listener"
        );
        assert_eq!(response["requiresImmediateRenegotiation"], has_offer);
        if has_offer {
            assert_eq!(
                call(
                    app(s.clone()),
                    "POST",
                    "/api/media/negotiate",
                    listener["token"].as_str(),
                    json!({"sessionDescription":{"type":"answer","sdp":"v=0"}})
                )
                .await
                .0,
                StatusCode::OK
            );
        }
        // Fresh snapshots omit the departed speaker; the browser closes only its
        // now-unwanted subscription after completing the offer/answer exchange.
        let (status, snapshot) = call(
            app(s.clone()),
            "POST",
            "/api/media/snapshot",
            listener["token"].as_str(),
            json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(snapshot["participants"].as_array().unwrap().len(), 1);
        assert_eq!(
            call(
                app(s.clone()),
                "POST",
                "/api/media/close",
                listener["token"].as_str(),
                json!({"mid":"remote-mid"})
            )
            .await
            .0,
            StatusCode::OK
        );
        retry_backlog(&s).await;
        let r = s.registry.lock().await;
        let remaining = &r.participants[&listener_id];
        assert_eq!(remaining.session, session);
        assert!(remaining.tracks.contains_key("listener-mic"));
        assert!(remaining.subscriptions.is_empty());
        assert!(!remaining.pending_offer && !remaining.operation);
        assert!(
            !provider
                .closes
                .lock()
                .await
                .contains(&(session, "listener-mic".into()))
        );
    }
}

#[tokio::test]
async fn already_departed_track_is_distinguished_from_an_invalid_listener_session() {
    let (s, _) = state();
    let listener = joined(&s, "listener").await;
    let body = json!({"trackId":Uuid::new_v4()});
    let missing = call(
        app(s.clone()),
        "POST",
        "/api/media/subscribe",
        listener["token"].as_str(),
        body.clone(),
    )
    .await;
    assert_eq!(missing.0, StatusCode::NOT_FOUND);
    assert_eq!(missing.1["code"], "track_gone");
    let invalid = call(
        app(s.clone()),
        "POST",
        "/api/media/subscribe",
        Some("invalid"),
        body,
    )
    .await;
    assert_eq!(invalid.0, StatusCode::UNAUTHORIZED);
    assert!(invalid.1.get("code").is_none());
    assert_eq!(
        call(
            app(s),
            "POST",
            "/api/media/snapshot",
            listener["token"].as_str(),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
}

async fn upstream(router: Router) -> (Config, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = Config::test(true);
    config.provider_base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (config, server)
}

#[tokio::test]
async fn provider_rejected_departed_track_preserves_listener_only_without_sdp_mutation() {
    let missing = json!({"requiresImmediateRenegotiation":false,"tracks":[{"mid":"","errorCode":"not_found_track_error"}]});
    let mut offered = missing.clone();
    offered["sessionDescription"] = json!({"type":"offer","sdp":"v=0"});
    let mut pending = missing.clone();
    pending["requiresImmediateRenegotiation"] = json!(true);
    let mut allocated = missing.clone();
    allocated["tracks"][0]["mid"] = json!("1");
    let mut session_error = missing.clone();
    session_error["errorCode"] = json!("session_error");
    for (http_status, response_body, safe) in [
        (StatusCode::OK, missing.clone(), true),
        (StatusCode::OK, offered, false),
        (StatusCode::OK, pending, false),
        (StatusCode::OK, allocated, false),
        (StatusCode::OK, session_error, false),
        (StatusCode::BAD_GATEWAY, missing, false),
    ] {
        let (s, _) = state();
        let speaker = joined(&s, "phone").await;
        let listener = joined(&s, "laptop").await;
        let token = listener["token"].as_str().unwrap();
        let id: Uuid = listener["id"].as_str().unwrap().parse().unwrap();
        let source_id: Uuid = speaker["id"].as_str().unwrap().parse().unwrap();
        let mut track = Value::Null;
        for (person, mid) in [(&speaker, "phone-mic"), (&listener, "laptop-mic")] {
            let published = call(app(s.clone()), "POST", "/api/media/publish", person["token"].as_str(),
                json!({"kind":"microphone","mid":mid,"sessionDescription":{"type":"offer","sdp":"v=0"}})).await;
            assert_eq!(published.0, StatusCode::OK);
            if mid == "phone-mic" {
                track = published.1["trackId"].clone();
            }
        }
        let session = s.registry.lock().await.participants[&id].session.clone();
        let (config, server) = upstream(Router::new().fallback({
            let s = s.clone();
            move || {
                let s = s.clone();
                let body = response_body.clone();
                async move {
                    // The source leaves after Caper validated the pull but before
                    // the real provider adapter receives its HTTP response.
                    remove_participant(&s, source_id).await;
                    (http_status, Json(body))
                }
            }
        }))
        .await;
        let mut api = s.clone();
        api.config = config;
        api.provider = Arc::new(Cloudflare::new());
        let (status, response) = call(
            app(api),
            "POST",
            "/api/media/subscribe",
            Some(token),
            json!({"trackId":track}),
        )
        .await;
        server.abort();
        if safe {
            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(response["code"], "track_gone");
            let r = s.registry.lock().await;
            let p = &r.participants[&id];
            assert_eq!(p.session, session);
            assert!(p.tracks.contains_key("laptop-mic"));
            assert!(!p.operation && !p.pending_offer);
            assert!(p.subscriptions.is_empty());
            assert!(!r.cleanup.iter().any(|job| matches!(&job.action, CleanupAction::Close { session: target, .. } | CleanupAction::Discover { session: target } if target == &session)));
        } else {
            assert_eq!(status, StatusCode::BAD_GATEWAY);
            assert!(
                !s.registry.lock().await.participants.contains_key(&id),
                "ambiguous SDP still requires recovery"
            );
        }
    }
}

#[tokio::test]
async fn provider_failures_preserve_status_ray_code_and_never_replay_mutations() {
    let calls = Arc::new(AtomicUsize::new(0));
    let router = Router::new().fallback({
        let calls = calls.clone();
        move || {
            calls.fetch_add(1, Ordering::SeqCst);
            async {
                (StatusCode::BAD_GATEWAY, [("cf-ray", "abc123-IAD")], Json(json!({
                    "errorCode":"upstream_timeout", "errorDescription":"SECRET SDP", "credential":"SECRET"
                })))
            }
        }
    });
    let (config, server) = upstream(router).await;
    let provider = Cloudflare::new();
    let results = [
        provider.create_session(&config).await.map(|_| ()),
        provider.turn(&config).await.map(|_| ()),
        provider
            .tracks_new(
                &config,
                "private-session",
                json!({"tracks":[{"location":"local"}]}),
            )
            .await
            .map(|_| ()),
        provider
            .tracks_new(
                &config,
                "private-session",
                json!({"tracks":[{"location":"remote"}]}),
            )
            .await
            .map(|_| ()),
        provider
            .negotiate(&config, "private-session", json!({}))
            .await
            .map(|_| ()),
        provider
            .close(&config, "private-session", "0")
            .await
            .map(|_| ()),
        provider.revoke_turn(&config, "private-username").await,
    ];
    for (result, operation) in results.into_iter().zip([
        "create_session",
        "turn_issue",
        "publish",
        "subscribe",
        "negotiate",
        "close",
        "turn_revoke",
    ]) {
        let error = result.unwrap_err();
        assert!(error.transient());
        let ProviderError::Request(ref f) = error else {
            panic!("missing diagnostics")
        };
        assert_eq!(f.operation, operation);
        assert_eq!(f.status, Some(502));
        assert_eq!(f.kind, "http");
        assert_eq!(f.ray.as_deref(), Some("abc123-IAD"));
        assert_eq!(f.code.as_deref(), Some("upstream_timeout"));
        assert!(!format!("{error:?}").contains("SECRET"));
        assert!(!format!("{error:?}").contains("private-"));
        let id = f.id.to_string();
        let response = ApiError::from(error).into_response();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(response.headers()["x-caper-error-id"], id);
        let body = to_bytes(response.into_body(), BODY_LIMIT).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"error":"media provider unavailable"})
        );
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        7,
        "no operation retried inline"
    );
    server.abort();
}

#[tokio::test]
async fn http_success_with_provider_error_or_malformed_body_is_not_transient() {
    for (body, kind, code) in [
        (
            "{\"tracks\":[{\"errorCode\":\"track_error\"}]}",
            "provider_error",
            Some("track_error"),
        ),
        ("<html>SECRET</html>", "invalid_json", None),
        ("{}", "invalid_response", None),
    ] {
        let (config, server) = upstream(Router::new().fallback(move || async move { body })).await;
        let error = Cloudflare::new()
            .session_tracks(&config, "s")
            .await
            .unwrap_err();
        assert!(!error.transient());
        let ProviderError::Request(f) = error else {
            panic!("missing diagnostics")
        };
        assert_eq!(f.status, Some(200));
        assert_eq!(f.kind, kind);
        assert_eq!(f.code.as_deref(), code);
        server.abort();
    }
    assert_eq!(diagnostic_token("unexpected\nSECRET"), None);
    assert_eq!(diagnostic_token(&"a".repeat(97)), None);
}

#[tokio::test]
async fn timeout_is_distinct_from_upstream_http_failure() {
    let (config, server) =
        upstream(Router::new().fallback(|| async { std::future::pending::<StatusCode>().await }))
            .await;
    let provider = Cloudflare {
        client: reqwest::Client::builder()
            .timeout(Duration::from_millis(40))
            .build()
            .unwrap(),
    };
    let error = provider.create_session(&config).await.unwrap_err();
    assert!(error.transient());
    let ProviderError::Request(f) = error else {
        panic!("missing diagnostics")
    };
    assert_eq!(f.kind, "timeout");
    assert_eq!(f.status, None);
    assert!(f.elapsed_ms.unwrap() >= 30);
    server.abort();
}

#[tokio::test]
async fn body_timeout_preserves_the_status_and_ray_received_before_it_stalled() {
    let (config, server) = upstream(Router::new().fallback(|| async {
        let chunks =
            stream::once(async { Ok::<_, std::io::Error>(axum::body::Bytes::from_static(b"{")) })
                .chain(stream::pending());
        (
            StatusCode::BAD_GATEWAY,
            [("cf-ray", "abc-IAD")],
            Body::from_stream(chunks),
        )
    }))
    .await;
    let provider = Cloudflare {
        client: reqwest::Client::builder()
            .timeout(Duration::from_millis(40))
            .build()
            .unwrap(),
    };
    let error = provider.create_session(&config).await.unwrap_err();
    let ProviderError::Request(f) = error else {
        panic!("missing diagnostics")
    };
    assert_eq!(f.kind, "timeout");
    assert_eq!(f.status, Some(502));
    assert_eq!(f.ray.as_deref(), Some("abc-IAD"));
    let error = tokio::time::timeout(
        Duration::from_secs(1),
        Cloudflare::new().create_session(&config),
    )
    .await
    .unwrap()
    .unwrap_err();
    let ProviderError::Request(f) = error else {
        panic!("missing diagnostics")
    };
    assert_eq!(
        f.kind, "http_body_timeout",
        "known HTTP failure only gives diagnostic reads 250 ms"
    );
    assert_eq!(f.status, Some(502));
    assert_eq!(f.ray.as_deref(), Some("abc-IAD"));
    server.abort();
}

#[tokio::test]
async fn revocation_accepts_empty_success_but_rejects_error_envelopes() {
    let (config, server) =
        upstream(Router::new().fallback(|| async { StatusCode::NO_CONTENT })).await;
    assert!(Cloudflare::new().revoke_turn(&config, "u").await.is_ok());
    server.abort();
    let (config, server) = upstream(Router::new().fallback(|| async {
        Json(json!({"success":false,"errors":[{"code":1000,"message":"PRIVATE"}]}))
    }))
    .await;
    let error = Cloudflare::new()
        .revoke_turn(&config, "u")
        .await
        .unwrap_err();
    assert!(!error.transient());
    let ProviderError::Request(f) = error else {
        panic!("missing diagnostics")
    };
    assert_eq!(f.kind, "provider_error");
    assert_eq!(f.code.as_deref(), Some("1000"));
    server.abort();
}

#[tokio::test]
async fn transient_cleanup_recovers_without_recreating_media() {
    let faults = Arc::new(Faults::new());
    let s = AppState::new(Config::test(true), faults.clone());
    faults.cleanup_failure.store(2, Ordering::SeqCst);
    enqueue_action(
        &s,
        CleanupAction::Discover {
            session: "s".into(),
        },
    )
    .await;
    retry_backlog(&s).await;
    assert_eq!(s.registry.lock().await.cleanup[0].attempts, 1);
    s.registry.lock().await.cleanup[0].not_before = Timestamp::now();
    faults.cleanup_failure.store(0, Ordering::SeqCst);
    retry_backlog(&s).await;
    retry_backlog(&s).await;
    assert_eq!(faults.discovers.load(Ordering::SeqCst), 2);
    assert_eq!(
        faults.mock.next.load(Ordering::SeqCst),
        1,
        "no new session provisioned"
    );
    assert_eq!(
        faults.mock.closes.lock().await.as_slice(),
        &[("s".into(), "orphan".into())]
    );
    assert!(s.registry.lock().await.cleanup.is_empty());
}

#[tokio::test(start_paused = true)]
async fn cleanup_worker_wakes_without_waiting_for_reconciliation() {
    let faults = Arc::new(Faults::new());
    let s = AppState::new(Config::test(true), faults.clone());
    spawn_cleanup(s.clone());
    tokio::task::yield_now().await;

    enqueue_cleanup(&s, "session".into(), "mid".into()).await;

    tokio::time::timeout(Duration::from_millis(100), async {
        while faults.cleanup_calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("new cleanup work should wake the worker immediately");
    assert!(s.registry.lock().await.cleanup.is_empty());
    s.begin_shutdown();
}

#[tokio::test(start_paused = true)]
async fn cleanup_worker_wait_tracks_retry_deadline_before_reconciliation() {
    let faults = Arc::new(Faults::new());
    faults.cleanup_failure.store(2, Ordering::SeqCst);
    let s = AppState::new(Config::test(true), faults);
    enqueue_cleanup(&s, "session".into(), "mid".into()).await;

    retry_backlog(&s).await;
    let deadline = s.registry.lock().await.cleanup[0].not_before;
    let wait = retry_backlog(&s).await;

    assert_eq!(wait, deadline.duration_since(Timestamp::now()));
    assert!(wait < CLEANUP_RECONCILE_INTERVAL);
}

struct Faults {
    mock: Mock,
    fail: AtomicBool,
    fail_create: AtomicBool,
    cleanup_failure: AtomicUsize,
    cleanup_calls: AtomicUsize,
    discovers: AtomicUsize,
    failure_id: Uuid,
}
impl Faults {
    fn new() -> Self {
        Self {
            mock: Mock::new(),
            fail: AtomicBool::new(false),
            fail_create: AtomicBool::new(false),
            cleanup_failure: AtomicUsize::new(0),
            cleanup_calls: AtomicUsize::new(0),
            discovers: AtomicUsize::new(0),
            failure_id: Uuid::new_v4(),
        }
    }
    fn error(&self) -> ProviderError {
        ProviderError::Request(Box::new(ProviderFailure {
            id: self.failure_id,
            operation: "subscribe",
            kind: "http",
            status: Some(503),
            ray: Some("ray-IAD".into()),
            code: None,
            elapsed_ms: Some(18),
        }))
    }
    async fn cleanup(&self) -> Result<(), ProviderError> {
        self.cleanup_calls.fetch_add(1, Ordering::SeqCst);
        match self.cleanup_failure.load(Ordering::SeqCst) {
            1 => std::future::pending().await,
            2 => Err(ProviderError::Unavailable),
            3 => Err(ProviderError::Rejected),
            _ => Ok(()),
        }
    }
}
#[async_trait]
impl Provider for Faults {
    async fn create_session(&self, c: &Config) -> Result<String, ProviderError> {
        let session = self.mock.create_session(c).await?;
        if self.fail_create.load(Ordering::SeqCst) {
            Err(self.error())
        } else {
            Ok(session)
        }
    }
    async fn turn(&self, c: &Config) -> Result<Vec<IceServer>, ProviderError> {
        self.mock.turn(c).await
    }
    async fn revoke_turn(&self, c: &Config, username: &str) -> Result<(), ProviderError> {
        self.cleanup().await?;
        self.mock.revoke_turn(c, username).await
    }
    async fn session_tracks(&self, _: &Config, _: &str) -> Result<Value, ProviderError> {
        self.discovers.fetch_add(1, Ordering::SeqCst);
        self.cleanup().await?;
        Ok(json!({"tracks":[{"mid":"orphan"}]}))
    }
    async fn tracks_new(&self, c: &Config, s: &str, body: Value) -> Result<Value, ProviderError> {
        if self.fail.load(Ordering::SeqCst) {
            Err(self.error())
        } else {
            self.mock.tracks_new(c, s, body).await
        }
    }
    async fn negotiate(&self, c: &Config, s: &str, body: Value) -> Result<Value, ProviderError> {
        if self.fail.load(Ordering::SeqCst) {
            Err(self.error())
        } else {
            self.mock.negotiate(c, s, body).await
        }
    }
    async fn close(&self, c: &Config, s: &str, mid: &str) -> Result<Value, ProviderError> {
        self.cleanup().await?;
        self.mock.close(c, s, mid).await
    }
}

#[tokio::test]
async fn failed_session_creation_releases_reservation_without_waiting_for_turn_cleanup() {
    let faults = Arc::new(Faults::new());
    faults.fail_create.store(true, Ordering::SeqCst);
    faults.cleanup_failure.store(1, Ordering::SeqCst);
    let s = AppState::new(Config::test(true), faults.clone());
    let response = tokio::time::timeout(
        Duration::from_millis(100),
        call(
            app(s.clone()),
            "POST",
            "/api/media/join",
            None,
            json!({"name":"a"}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(response.0, StatusCode::BAD_GATEWAY);
    let r = s.registry.lock().await;
    assert!(r.reservations.is_empty());
    assert!(r.participants.is_empty());
    assert_eq!(r.cleanup.len(), 1);
    assert_eq!(r.cleanup[0].action.operation(), "turn_revoke");
    drop(r);
    assert_eq!(faults.cleanup_calls.load(Ordering::SeqCst), 0);
    faults.cleanup_failure.store(0, Ordering::SeqCst);
    retry_backlog(&s).await;
    assert_eq!(faults.mock.revocations.lock().await.len(), 1);
}

#[tokio::test]
async fn subscription_error_returns_original_failure_before_slow_discovery_and_revocation() {
    let faults = Arc::new(Faults::new());
    let s = AppState::new(Config::test(true), faults.clone());
    let a = joined(&s, "a").await;
    let b = joined(&s, "b").await;
    let published = call(
        app(s.clone()),
        "POST",
        "/api/media/publish",
        a["token"].as_str(),
        json!({"kind":"microphone","mid":"0","sessionDescription":{"type":"offer","sdp":"v=0"}}),
    )
    .await;
    assert_eq!(published.0, StatusCode::OK);
    let track = s
        .registry
        .lock()
        .await
        .participants
        .values()
        .flat_map(|p| p.tracks.values())
        .next()
        .unwrap()
        .id;
    faults.fail.store(true, Ordering::SeqCst);
    faults.cleanup_failure.store(1, Ordering::SeqCst);
    let request = Request::builder()
        .method("POST")
        .uri("/api/media/subscribe")
        .header("content-type", "application/json")
        .header("x-caper-media-token", b["token"].as_str().unwrap())
        .body(Body::from(json!({"trackId":track}).to_string()))
        .unwrap();
    let response =
        tokio::time::timeout(Duration::from_millis(100), app(s.clone()).oneshot(request))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(
        response.headers()["x-caper-error-id"],
        faults.failure_id.to_string()
    );
    assert_eq!(faults.cleanup_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/snapshot",
            b["token"].as_str(),
            json!({})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        s.registry.lock().await.participants.len(),
        1,
        "publisher remains connected"
    );
    faults.cleanup_failure.store(0, Ordering::SeqCst);
    retry_backlog(&s).await;
    retry_backlog(&s).await;
    assert_eq!(faults.discovers.load(Ordering::SeqCst), 1);
    assert!(
        faults
            .mock
            .closes
            .lock()
            .await
            .iter()
            .any(|(_, mid)| mid == "orphan")
    );
    assert!(s.registry.lock().await.cleanup.is_empty());
}

#[tokio::test]
async fn cleanup_retries_only_transient_errors_with_delay_and_attempt_cap() {
    let faults = Arc::new(Faults::new());
    let s = AppState::new(Config::test(true), faults.clone());
    faults.cleanup_failure.store(2, Ordering::SeqCst);
    enqueue_cleanup(&s, "s".into(), "0".into()).await;
    retry_backlog(&s).await;
    retry_backlog(&s).await;
    assert_eq!(
        faults.cleanup_calls.load(Ordering::SeqCst),
        1,
        "backoff must be observed"
    );
    for attempt in 1..5 {
        let mut r = s.registry.lock().await;
        assert_eq!(r.cleanup.len(), 1);
        assert_eq!(r.cleanup[0].attempts, attempt);
        assert!(r.cleanup[0].not_before > Timestamp::now());
        r.cleanup[0].not_before = Timestamp::now();
        drop(r);
        retry_backlog(&s).await;
    }
    assert!(s.registry.lock().await.cleanup.is_empty());
    assert_eq!(faults.cleanup_calls.load(Ordering::SeqCst), 5);
    faults.cleanup_failure.store(3, Ordering::SeqCst);
    enqueue_cleanup(&s, "s".into(), "0".into()).await;
    retry_backlog(&s).await;
    assert!(
        s.registry.lock().await.cleanup.is_empty(),
        "permanent rejection is not retried"
    );
}

#[tokio::test]
async fn cleanup_queue_is_bounded_deduplicated_and_does_not_block_leave_or_expiry() {
    let faults = Arc::new(Faults::new());
    let s = AppState::new(Config::test(true), faults.clone());
    let a = joined(&s, "a").await;
    for n in 0..MAX_CLEANUP_BACKLOG + 10 {
        enqueue_cleanup(&s, format!("s{n}"), "0".into()).await;
    }
    enqueue_cleanup(&s, "s0".into(), "0".into()).await;
    assert_eq!(s.registry.lock().await.cleanup.len(), MAX_CLEANUP_BACKLOG);
    faults.cleanup_failure.store(1, Ordering::SeqCst);
    let worker = tokio::spawn({
        let s = s.clone();
        async move { retry_backlog(&s).await }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while faults.cleanup_calls.load(Ordering::SeqCst) < 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(faults.cleanup_calls.load(Ordering::SeqCst), 4);
    assert_eq!(
        tokio::time::timeout(
            Duration::from_millis(100),
            call(
                app(s.clone()),
                "POST",
                "/api/media/leave",
                a["token"].as_str(),
                json!({})
            )
        )
        .await
        .unwrap()
        .0,
        StatusCode::NO_CONTENT
    );
    let _b = joined(&s, "b").await;
    s.registry
        .lock()
        .await
        .participants
        .values_mut()
        .next()
        .unwrap()
        .lease = Timestamp::now() - LEASE;
    spawn_cleanup(s.clone());
    tokio::time::timeout(Duration::from_secs(1), async {
        while !s.registry.lock().await.participants.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        faults.cleanup_calls.load(Ordering::SeqCst),
        4,
        "expiry works even with all worker slots stalled"
    );
    s.begin_shutdown();
    worker.abort();
}

#[tokio::test]
async fn shutdown_drains_multiple_batches_and_discovered_tracks() {
    let faults = Arc::new(Faults::new());
    let s = AppState::new(Config::test(true), faults.clone());
    for n in 0..9 {
        enqueue_cleanup(&s, format!("s{n}"), "0".into()).await;
    }
    enqueue_action(
        &s,
        CleanupAction::Discover {
            session: "orphan-session".into(),
        },
    )
    .await;
    s.begin_shutdown();
    shutdown_cleanup(&s).await;
    assert_eq!(faults.mock.closes.lock().await.len(), 10);
    assert!(s.registry.lock().await.cleanup.is_empty());
}

#[tokio::test]
async fn remote_close_failure_never_invalidates_the_listener() {
    let faults = Arc::new(Faults::new());
    let s = AppState::new(Config::test(true), faults.clone());
    let speaker = joined(&s, "phone").await;
    let listener = joined(&s, "laptop").await;
    let token = listener["token"].as_str().unwrap();
    let id: Uuid = listener["id"].as_str().unwrap().parse().unwrap();
    let mut source = Value::Null;
    for (person, mid) in [(&speaker, "phone-mic"), (&listener, "laptop-mic")] {
        let (status, published) = call(app(s.clone()), "POST", "/api/media/publish", person["token"].as_str(),
            json!({"kind":"microphone","mid":mid,"sessionDescription":{"type":"offer","sdp":"v=0"}})).await;
        assert_eq!(status, StatusCode::OK);
        if mid == "phone-mic" {
            source = published["trackId"].clone();
        }
    }
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/subscribe",
            Some(token),
            json!({"trackId":source})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/negotiate",
            Some(token),
            json!({"sessionDescription":{"type":"answer","sdp":"v=0"}})
        )
        .await
        .0,
        StatusCode::OK
    );
    let session = s.registry.lock().await.participants[&id].session.clone();
    faults.cleanup_failure.store(2, Ordering::SeqCst);
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/close",
            Some(token),
            json!({"mid":"remote-mid"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        faults.cleanup_calls.load(Ordering::SeqCst),
        0,
        "close commits locally without waiting for Cloudflare"
    );
    retry_backlog(&s).await;
    assert_eq!(faults.cleanup_calls.load(Ordering::SeqCst), 1);
    let (status, snapshot) = call(
        app(s.clone()),
        "POST",
        "/api/media/snapshot",
        Some(token),
        json!({}),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "failed remote cleanup must not revoke the listener"
    );
    assert_eq!(snapshot["participants"].as_array().unwrap().len(), 2);
    {
        let mut r = s.registry.lock().await;
        let p = &r.participants[&id];
        assert_eq!(p.session, session);
        assert!(p.tracks.contains_key("laptop-mic"));
        assert!(p.subscriptions.is_empty());
        assert!(!p.operation && !p.pending_offer);
        r.cleanup[0].not_before = Timestamp::now();
    }
    faults.cleanup_failure.store(0, Ordering::SeqCst);
    retry_backlog(&s).await;
    assert_eq!(
        *faults.mock.closes.lock().await,
        vec![(session, "remote-mid".into())]
    );
    assert!(s.registry.lock().await.cleanup.is_empty());
}

#[tokio::test]
async fn first_roster_contains_join_mute_and_deafen_intent() {
    let (s, _) = state();
    let mut stream = presence_response(&s).await.into_body().into_data_stream();
    next_event(&mut stream).await;
    presence_snapshot_event(&mut stream).await;
    for (muted, deafened) in [(true, false), (false, true), (true, true), (false, false)] {
        let (status, joined) = call(
            app(s.clone()),
            "POST",
            "/api/media/join",
            None,
            json!({"name":"phone","muted":muted,"deafened":deafened}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let first = presence_snapshot_event(&mut stream).await;
        assert_eq!(first["participants"][0]["muted"], muted);
        assert_eq!(first["participants"][0]["deafened"], deafened);
        assert_eq!(
            call(
                app(s.clone()),
                "POST",
                "/api/media/leave",
                joined["token"].as_str(),
                json!({})
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            presence_snapshot_event(&mut stream).await["participants"],
            json!([])
        );
    }
}
