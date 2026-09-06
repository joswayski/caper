use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tower::ServiceExt;

struct Mock {
    next: AtomicUsize,
    provisioning: AtomicUsize,
    closes: Mutex<Vec<(String, String)>>,
    revocations: Mutex<Vec<String>>,
    remote_offer: AtomicBool,
    block_provision: AtomicBool,
}
impl Mock {
    fn new() -> Self {
        Self {
            next: AtomicUsize::new(1),
            provisioning: AtomicUsize::new(0),
            closes: Mutex::new(vec![]),
            revocations: Mutex::new(vec![]),
            remote_offer: AtomicBool::new(true),
            block_provision: AtomicBool::new(false),
        }
    }
}
#[async_trait]
impl Provider for Mock {
    async fn create_session(&self, _: &Config) -> Result<String, ProviderError> {
        self.provisioning.fetch_or(1, Ordering::SeqCst);
        while self.provisioning.load(Ordering::SeqCst) & 2 == 0 {
            tokio::task::yield_now().await;
        }
        while self.block_provision.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        Ok(format!("s{}", self.next.fetch_add(1, Ordering::SeqCst)))
    }
    async fn turn(&self, _: &Config) -> Result<Vec<IceServer>, ProviderError> {
        self.provisioning.fetch_or(2, Ordering::SeqCst);
        while self.provisioning.load(Ordering::SeqCst) & 1 == 0 {
            tokio::task::yield_now().await;
        }
        while self.block_provision.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
        Ok(vec![IceServer {
            urls: json!(["stun:test"]),
            username: Some("temporary-user".into()),
            credential: None,
        }])
    }
    async fn revoke_turn(&self, _: &Config, username: &str) -> Result<(), ProviderError> {
        self.revocations.lock().await.push(username.into());
        Ok(())
    }
    async fn session_tracks(&self, _: &Config, _: &str) -> Result<Value, ProviderError> {
        Ok(json!({"tracks":[]}))
    }
    async fn tracks_new(&self, _: &Config, _: &str, body: Value) -> Result<Value, ProviderError> {
        if body["tracks"][0]["location"] == "remote" {
            if self.remote_offer.load(Ordering::SeqCst) {
                Ok(
                    json!({"requiresImmediateRenegotiation":true,"tracks":[{"mid":"remote-mid"}],"sessionDescription":{"type":"offer","sdp":"offer"}}),
                )
            } else {
                Ok(json!({"requiresImmediateRenegotiation":false,"tracks":[{"mid":"remote-mid"}]}))
            }
        } else {
            Ok(
                json!({"tracks":[{"mid":body["tracks"][0]["mid"]}],"sessionDescription":{"type":"answer","sdp":"answer"}}),
            )
        }
    }
    async fn negotiate(&self, _: &Config, _: &str, _: Value) -> Result<Value, ProviderError> {
        Ok(json!({}))
    }
    async fn close(&self, _: &Config, s: &str, m: &str) -> Result<Value, ProviderError> {
        self.closes.lock().await.push((s.into(), m.into()));
        Ok(json!({"tracks":[{"mid":m}]}))
    }
}
fn state() -> (AppState, Arc<Mock>) {
    let m = Arc::new(Mock::new());
    (AppState::new(Config::test(true), m.clone()), m)
}
async fn call(
    app: Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let response = app
        .oneshot(b.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), BODY_LIMIT).await.unwrap();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, value)
}
async fn joined(s: &AppState, name: &str) -> Value {
    tokio::time::timeout(
        Duration::from_secs(1),
        call(
            app(s.clone()),
            "POST",
            "/api/media/join",
            None,
            json!({"name":name}),
        ),
    )
    .await
    .expect("session and TURN provisioning should run concurrently")
    .1
}
async fn monitor_joined(s: &AppState, token: &str, role: &str) -> (StatusCode, Value) {
    call(
        app(s.clone()),
        "POST",
        "/api/media/join",
        Some(token),
        json!({"name":"Microphone test","monitor":role}),
    )
    .await
}

async fn event_response(s: &AppState, token: Option<&str>, cross_site: bool) -> Response {
    let mut request = Request::builder().method("GET").uri("/api/media/events");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    if cross_site {
        request = request.header("sec-fetch-site", "cross-site");
    }
    app(s.clone())
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn next_event(stream: &mut axum::body::BodyDataStream) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .expect("event should arrive")
        .transpose()
        .unwrap()
        .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn events_enforce_access_and_emit_ready_immediately() {
    let (s, _) = state();
    assert_eq!(
        event_response(&s, None, false).await.status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        event_response(&s, Some("bad"), false).await.status(),
        StatusCode::UNAUTHORIZED
    );
    let participant = joined(&s, "public").await;
    let token = participant["token"].as_str().unwrap();
    assert_eq!(
        event_response(&s, Some(token), true).await.status(),
        StatusCode::FORBIDDEN
    );

    let monitor = monitor_joined(&s, token, "sender").await.1;
    assert_eq!(
        event_response(&s, Some(monitor["token"].as_str().unwrap()), false)
            .await
            .status(),
        StatusCode::FORBIDDEN
    );
    let response = event_response(&s, Some(token), false).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-accel-buffering"], "no");
    let mut stream = response.into_body().into_data_stream();
    assert_eq!(
        next_event(&mut stream).await.as_deref(),
        Some("event: ready\ndata: {}\n\n")
    );

    let (mut disabled, _) = state();
    disabled.config.enabled = false;
    assert_eq!(
        event_response(&disabled, Some(token), false).await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn public_events_are_coalesced_and_connections_are_replaced_or_revoked() {
    let (s, _) = state();
    let participant = joined(&s, "public").await;
    let token = participant["token"].as_str().unwrap();
    let mut first = event_response(&s, Some(token), false)
        .await
        .into_body()
        .into_data_stream();
    assert!(next_event(&mut first).await.unwrap().contains("ready"));

    call(
        app(s.clone()),
        "POST",
        "/api/media/state",
        Some(token),
        json!({"muted":true,"deafened":false}),
    )
    .await;
    call(
        app(s.clone()),
        "POST",
        "/api/media/state",
        Some(token),
        json!({"muted":true,"deafened":true}),
    )
    .await;
    assert_eq!(
        next_event(&mut first).await.as_deref(),
        Some("event: changed\ndata: {}\n\n")
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(50), first.next())
            .await
            .is_err()
    );

    let mut replacement = event_response(&s, Some(token), false)
        .await
        .into_body()
        .into_data_stream();
    assert!(
        next_event(&mut replacement)
            .await
            .unwrap()
            .contains("ready")
    );
    assert!(next_event(&mut first).await.is_none());
    remove_participant(&s, participant["id"].as_str().unwrap().parse().unwrap()).await;
    assert!(next_event(&mut replacement).await.is_none());
}

#[tokio::test]
async fn public_join_publish_close_and_leave_emit_changes() {
    let (s, _) = state();
    let observer = joined(&s, "observer").await;
    let observer_token = observer["token"].as_str().unwrap();
    let mut stream = event_response(&s, Some(observer_token), false)
        .await
        .into_body()
        .into_data_stream();
    assert!(next_event(&mut stream).await.unwrap().contains("ready"));

    let subject = joined(&s, "subject").await;
    let subject_token = subject["token"].as_str().unwrap();
    assert!(next_event(&mut stream).await.unwrap().contains("changed"));
    call(
        app(s.clone()),
        "POST",
        "/api/media/publish",
        Some(subject_token),
        json!({"kind":"microphone","mid":"mic","sessionDescription":{"type":"offer","sdp":"v=0"}}),
    )
    .await;
    assert!(next_event(&mut stream).await.unwrap().contains("changed"));
    call(
        app(s.clone()),
        "POST",
        "/api/media/close",
        Some(subject_token),
        json!({"mid":"mic"}),
    )
    .await;
    assert!(next_event(&mut stream).await.unwrap().contains("changed"));
    call(
        app(s.clone()),
        "POST",
        "/api/media/leave",
        Some(subject_token),
        json!({}),
    )
    .await;
    assert!(next_event(&mut stream).await.unwrap().contains("changed"));
}

#[tokio::test]
async fn private_monitor_mutations_do_not_emit_public_events() {
    let (s, _) = state();
    let parent = joined(&s, "parent").await;
    let token = parent["token"].as_str().unwrap();
    let mut stream = event_response(&s, Some(token), false)
        .await
        .into_body()
        .into_data_stream();
    assert!(next_event(&mut stream).await.unwrap().contains("ready"));
    let monitor = monitor_joined(&s, token, "sender").await.1;
    call(
        app(s.clone()),
        "POST",
        "/api/media/state",
        Some(monitor["token"].as_str().unwrap()),
        json!({"muted":true,"deafened":true}),
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), stream.next())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn disabled_and_invalid_bodies() {
    let (mut s, _) = state();
    s.config.enabled = false;
    assert_eq!(
        call(app(s), "POST", "/api/media/join", None, json!({"name":"a"}))
            .await
            .0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let (s, _) = state();
    assert_eq!(
        call(app(s), "POST", "/api/media/join", None, json!({"name":""}))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );
}

#[tokio::test]
async fn private_monitor_authorization_visibility_and_media_permissions() {
    let (s, _) = state();
    let parent = joined(&s, "parent").await;
    let other = joined(&s, "other").await;
    let parent_token = parent["token"].as_str().unwrap();
    let other_token = other["token"].as_str().unwrap();

    assert_eq!(
        monitor_joined(&s, "invalid", "sender").await.0,
        StatusCode::UNAUTHORIZED
    );
    let (status, sender) = monitor_joined(&s, parent_token, "sender").await;
    assert_eq!(status, StatusCode::OK);
    let sender_token = sender["token"].as_str().unwrap();
    assert_eq!(
        monitor_joined(&s, parent_token, "sender").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        monitor_joined(&s, sender_token, "receiver").await.0,
        StatusCode::FORBIDDEN
    );
    let receiver = monitor_joined(&s, parent_token, "receiver").await.1;
    let receiver_token = receiver["token"].as_str().unwrap();
    let other_receiver = monitor_joined(&s, other_token, "receiver").await.1;
    let other_sender = monitor_joined(&s, other_token, "sender").await.1;

    let normal_snapshot = call(
        app(s.clone()),
        "POST",
        "/api/media/snapshot",
        Some(parent_token),
        json!({}),
    )
    .await
    .1;
    assert_eq!(normal_snapshot["participants"].as_array().unwrap().len(), 2);
    let private_snapshot = call(
        app(s.clone()),
        "POST",
        "/api/media/snapshot",
        Some(sender_token),
        json!({}),
    )
    .await
    .1;
    assert!(
        private_snapshot["participants"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/publish",
            Some(receiver_token),
            json!({"kind":"microphone","mid":"r","sessionDescription":{"type":"offer","sdp":"v=0"}}),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let (status, published) = call(
        app(s.clone()),
        "POST",
        "/api/media/publish",
        Some(sender_token),
        json!({"kind":"microphone","mid":"s","sessionDescription":{"type":"offer","sdp":"v=0"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let track_id = published["trackId"].clone();
    assert!(track_id.is_string());
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/subscribe",
            Some(parent_token),
            json!({"trackId":track_id})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/subscribe",
            Some(other_sender["token"].as_str().unwrap()),
            json!({"trackId":track_id})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/subscribe",
            Some(other_receiver["token"].as_str().unwrap()),
            json!({"trackId":track_id})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/subscribe",
            Some(receiver_token),
            json!({"trackId":track_id})
        )
        .await
        .0,
        StatusCode::OK
    );
    let public_track = call(
        app(s.clone()),
        "POST",
        "/api/media/publish",
        Some(parent_token),
        json!({"kind":"microphone","mid":"p","sessionDescription":{"type":"offer","sdp":"v=0"}}),
    )
    .await
    .1["trackId"]
        .clone();
    assert_eq!(
        call(
            app(s),
            "POST",
            "/api/media/subscribe",
            Some(receiver_token),
            json!({"trackId":public_track})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn parent_leave_rejects_in_flight_monitor_and_cascades_children() {
    let (s, mock) = state();
    let parent = joined(&s, "parent").await;
    let parent_token = parent["token"].as_str().unwrap().to_owned();
    let child = monitor_joined(&s, &parent_token, "sender").await.1;
    let child_token = child["token"].as_str().unwrap().to_owned();

    mock.block_provision.store(true, Ordering::SeqCst);
    let join_state = s.clone();
    let join_parent_token = parent_token.clone();
    let joining =
        tokio::spawn(
            async move { monitor_joined(&join_state, &join_parent_token, "receiver").await },
        );
    while s.registry.lock().await.joining == 0 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        monitor_joined(&s, &parent_token, "receiver").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/leave",
            Some(&parent_token),
            json!({})
        )
        .await
        .0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/snapshot",
            Some(&child_token),
            json!({})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    mock.block_provision.store(false, Ordering::SeqCst);
    assert_eq!(joining.await.unwrap().0, StatusCode::UNAUTHORIZED);
    assert!(
        s.registry
            .lock()
            .await
            .participants
            .values()
            .all(|p| p.monitor.is_none())
    );
    assert_eq!(mock.revocations.lock().await.len(), 3);
}

#[tokio::test]
async fn parent_expiry_cascades_monitor_cleanup() {
    let (s, mock) = state();
    let parent = joined(&s, "parent").await;
    let parent_token = parent["token"].as_str().unwrap();
    let sender = monitor_joined(&s, parent_token, "sender").await.1;
    let sender_token = sender["token"].as_str().unwrap();
    call(
        app(s.clone()),
        "POST",
        "/api/media/publish",
        Some(sender_token),
        json!({"kind":"microphone","mid":"s","sessionDescription":{"type":"offer","sdp":"v=0"}}),
    )
    .await;
    {
        let parent_id = parent["id"].as_str().unwrap().parse::<Uuid>().unwrap();
        s.registry
            .lock()
            .await
            .participants
            .get_mut(&parent_id)
            .unwrap()
            .lease = Instant::now() - LEASE;
    }
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/snapshot",
            Some(sender_token),
            json!({})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    spawn_cleanup(s.clone());
    tokio::time::timeout(Duration::from_secs(1), async {
        while !s.registry.lock().await.participants.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(mock.closes.lock().await.iter().any(|(_, mid)| mid == "s"));
    assert_eq!(mock.revocations.lock().await.len(), 2);
}
#[tokio::test]
async fn capacity_is_enforced() {
    let (s, _) = state();
    for n in 0..MAX_PARTICIPANTS {
        assert_eq!(joined(&s, &format!("p{n}")).await["name"], Value::Null);
    }
    assert_eq!(
        call(
            app(s),
            "POST",
            "/api/media/join",
            None,
            json!({"name":"extra"})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}
#[tokio::test]
async fn ownership_subscription_and_negotiation() {
    let (s, m) = state();
    let a = joined(&s, "a").await;
    let b = joined(&s, "b").await;
    let ta = a["token"].as_str().unwrap();
    let tb = b["token"].as_str().unwrap();
    let (_, snap) = call(
        app(s.clone()),
        "POST",
        "/api/media/snapshot",
        Some(ta),
        json!({}),
    )
    .await;
    assert_eq!(snap["participants"].as_array().unwrap().len(), 2);
    let (_, other_snapshot) = call(
        app(s.clone()),
        "POST",
        "/api/media/snapshot",
        Some(tb),
        json!({}),
    )
    .await;
    assert_eq!(
        snap, other_snapshot,
        "both visitors see the same names and IDs"
    );
    let (_, published) = call(
        app(s.clone()),
        "POST",
        "/api/media/publish",
        Some(ta),
        json!({"kind":"microphone","mid":"0","sessionDescription":{"type":"offer","sdp":"v=0"}}),
    )
    .await;
    assert_eq!(published["sessionDescription"]["type"], "answer");
    let (_, snap) = call(
        app(s.clone()),
        "POST",
        "/api/media/snapshot",
        Some(tb),
        json!({}),
    )
    .await;
    let track = &snap["participants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "a")
        .unwrap()["tracks"][0]["id"];
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/subscribe",
            Some(ta),
            json!({"trackId":track})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/subscribe",
            Some(tb),
            json!({"trackId":track})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/subscribe",
            Some(tb),
            json!({"trackId":track})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/negotiate",
            Some(tb),
            json!({"sessionDescription":{"type":"answer","sdp":"v=0"}})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/close",
            Some(tb),
            json!({"mid":"0"})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        call(
            app(s),
            "POST",
            "/api/media/close",
            Some(ta),
            json!({"mid":"0"})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert!(
        m.closes
            .lock()
            .await
            .iter()
            .any(|(_, mid)| mid == "remote-mid")
    );
}
#[tokio::test]
async fn expiry_removes_and_cleans() {
    let (s, m) = state();
    let a = joined(&s, "a").await;
    let token = a["token"].as_str().unwrap();
    call(
        app(s.clone()),
        "POST",
        "/api/media/publish",
        Some(token),
        json!({"kind":"microphone","mid":"1","sessionDescription":{"type":"offer","sdp":"v=0"}}),
    )
    .await;
    {
        let mut r = s.registry.lock().await;
        r.participants.values_mut().next().unwrap().lease = Instant::now() - LEASE;
    }
    let id = {
        s.registry
            .lock()
            .await
            .participants
            .keys()
            .next()
            .copied()
            .unwrap()
    };
    remove_participant(&s, id).await;
    assert!(s.registry.lock().await.participants.is_empty());
    assert!(m.closes.lock().await.iter().any(|(_, mid)| mid == "1"));
    assert_eq!(*m.revocations.lock().await, vec!["temporary-user"]);
}

#[test]
fn provider_errors_and_unsupported_turn_are_rejected() {
    assert!(validate_provider_envelope(&json!({"errorCode":"bad"})).is_err());
    assert!(validate_provider_envelope(&json!({"tracks":[{"errorCode":123}]})).is_err());
    let supported = json!([
        "stun:stun.cloudflare.com:3478",
        "turn:turn.cloudflare.com:3478?transport=udp",
        "turn:turn.cloudflare.com:3478?transport=tcp",
        "turn:turn.cloudflare.com:80?transport=tcp",
        "turns:turn.cloudflare.com:5349?transport=tcp",
        "turns:turn.cloudflare.com:443?transport=tcp"
    ]);
    let mut urls = supported.clone();
    for url in [
        "stun:stun.cloudflare.com:53",
        "stuns:relay.example:53",
        "turn:turn.cloudflare.com:53?transport=udp",
        "turn:relay.example:53?transport=tcp",
        "turns:relay.example:53?transport=tcp",
    ] {
        urls.as_array_mut().unwrap().push(json!(url));
        let mut single = json!(url);
        filter_unsupported_ice_urls(&mut single);
        assert_eq!(single, json!([]));
    }
    filter_unsupported_ice_urls(&mut urls);
    assert_eq!(urls, supported);
    let mut single = json!("stun:stun.cloudflare.com:3478");
    filter_unsupported_ice_urls(&mut single);
    assert_eq!(single, json!("stun:stun.cloudflare.com:3478"));
    assert_eq!(TURN_TTL, MAX_CALL_DURATION.as_secs());
    assert_eq!(MAX_TRACKS, 1);
    assert_eq!(MAX_SUBSCRIPTIONS, 11);
}

#[tokio::test]
async fn expired_tokens_nonmicrophone_kinds_and_extra_publications_are_rejected() {
    let (s, _) = state();
    let joined = joined(&s, "a").await;
    let token = joined["token"].as_str().unwrap();
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/publish",
            Some(token),
            json!({"kind":"camera","mid":"1","sessionDescription":{"type":"offer","sdp":"v=0"}})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/publish",
            Some(token),
            json!({"kind":"microphone","mid":"1","sessionDescription":{"type":"offer","sdp":"v=0"}})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/publish",
            Some(token),
            json!({"kind":"microphone","mid":"2","sessionDescription":{"type":"offer","sdp":"v=0"}})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    s.registry
        .lock()
        .await
        .participants
        .values_mut()
        .next()
        .unwrap()
        .lease = Instant::now() - LEASE;
    assert_eq!(
        call(
            app(s),
            "POST",
            "/api/media/snapshot",
            Some(token),
            json!({})
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn subscription_without_offer_does_not_require_negotiation_and_cannot_duplicate() {
    let (s, mock) = state();
    mock.remote_offer.store(false, Ordering::SeqCst);
    let a = joined(&s, "a").await;
    let b = joined(&s, "b").await;
    let ta = a["token"].as_str().unwrap();
    let tb = b["token"].as_str().unwrap();
    call(
        app(s.clone()),
        "POST",
        "/api/media/publish",
        Some(ta),
        json!({"kind":"microphone","mid":"0","sessionDescription":{"type":"offer","sdp":"v=0"}}),
    )
    .await;
    let snapshot = call(
        app(s.clone()),
        "POST",
        "/api/media/snapshot",
        Some(tb),
        json!({}),
    )
    .await
    .1;
    let track = snapshot["participants"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "a")
        .unwrap()["tracks"][0]["id"]
        .clone();
    assert_eq!(
        call(
            app(s.clone()),
            "POST",
            "/api/media/subscribe",
            Some(tb),
            json!({"trackId":track})
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
            Some(tb),
            json!({"sessionDescription":{"type":"answer","sdp":"v=0"}})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        call(
            app(s),
            "POST",
            "/api/media/subscribe",
            Some(tb),
            json!({"trackId":track})
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn cloudflare_session_creation_sends_no_body() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = Config::test(true);
    config.provider_base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().route(
        "/apps/app/sessions/new",
        post(|body: axum::body::Bytes| async move {
            assert!(body.is_empty(), "Cloudflare rejects an empty JSON object");
            (
                StatusCode::CREATED,
                Json(json!({"sessionId":"test-session"})),
            )
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let result = Cloudflare::new().create_session(&config).await;
    server.abort();
    assert_eq!(result.unwrap(), "test-session");
}

#[tokio::test]
async fn cloudflare_turn_filters_blocked_stun_without_losing_relay_credentials() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut config = Config::test(true);
    config.provider_base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().route(
        "/turn/keys/turn/credentials/generate-ice-servers",
        post(|| async {
            (StatusCode::CREATED, Json(json!({"iceServers":[
                {"urls":["stun:stun.cloudflare.com:3478","stun:stun.cloudflare.com:53"]},
                {"urls":"stun:stun.cloudflare.com:53"},
                {"urls":["turn:turn.cloudflare.com:53?transport=udp","turns:turn.cloudflare.com:443?transport=tcp"],"username":"temporary-user","credential":"temporary-credential"}
            ]})))
        }),
    );
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let result = Cloudflare::new().turn(&config).await;
    server.abort();
    let servers = result.unwrap();
    assert_eq!(servers.len(), 2);
    assert_eq!(servers[0].urls, json!(["stun:stun.cloudflare.com:3478"]));
    assert_eq!(
        servers[1].urls,
        json!(["turns:turn.cloudflare.com:443?transport=tcp"])
    );
    assert_eq!(servers[1].username.as_deref(), Some("temporary-user"));
    assert_eq!(
        servers[1].credential.as_deref(),
        Some("temporary-credential")
    );
}

#[test]
fn database_url_requires_postgres_and_tls() {
    let options = crate::db::connect_options(
        "postgres://caper:secret@example.horizon.psdb.cloud:5432/caper?sslmode=disable",
    )
    .unwrap();
    assert!(matches!(
        options.get_ssl_mode(),
        sqlx::postgres::PgSslMode::VerifyFull
    ));
    assert_eq!(
        crate::db::connect_options("mysql://caper:secret@db/caper").unwrap_err(),
        "DATABASE_URL must be a PostgreSQL URL"
    );
}
