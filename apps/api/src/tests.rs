use super::*;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tower::ServiceExt;

struct Mock {
    next: AtomicUsize,
    closes: Mutex<Vec<(String, String)>>,
    revocations: Mutex<Vec<String>>,
    remote_offer: AtomicBool,
}
impl Mock {
    fn new() -> Self {
        Self {
            next: AtomicUsize::new(1),
            closes: Mutex::new(vec![]),
            revocations: Mutex::new(vec![]),
            remote_offer: AtomicBool::new(true),
        }
    }
}
#[async_trait]
impl Provider for Mock {
    async fn create_session(&self, _: &Config) -> Result<String, ProviderError> {
        Ok(format!("s{}", self.next.fetch_add(1, Ordering::SeqCst)))
    }
    async fn turn(&self, _: &Config) -> Result<Vec<IceServer>, ProviderError> {
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
    call(
        app(s.clone()),
        "POST",
        "/api/media/join",
        None,
        json!({"name":name}),
    )
    .await
    .1
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
    let mut urls = json!([
        "turn:relay.example:53?transport=tcp",
        "turn:relay.example:3478",
        "stun:relay.example:53"
    ]);
    filter_unsupported_turn_urls(&mut urls);
    assert_eq!(
        urls,
        json!(["turn:relay.example:3478", "stun:relay.example:53"])
    );
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
