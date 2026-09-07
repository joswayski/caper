use super::*;

async fn upstream(router: Router) -> (Config, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = Config::test(true);
    config.provider_base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (config, server)
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
    s.registry.lock().await.cleanup[0].not_before = Instant::now();
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
    assert_eq!(r.joining, 0);
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
        .header(
            "authorization",
            format!("Bearer {}", b["token"].as_str().unwrap()),
        )
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
        assert!(r.cleanup[0].not_before > Instant::now());
        r.cleanup[0].not_before = Instant::now();
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
        .lease = Instant::now() - LEASE;
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
