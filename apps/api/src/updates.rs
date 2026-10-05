//! Public desktop update metadata. Cache the exact manifest bytes and detached
//! signature in one response; clients still verify with their compiled-in key.
use crate::{ApiError, AppState};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const MANIFEST_URL: &str =
    "https://github.com/joswayski/caper/releases/download/native-latest/latest.json";
const CACHE_TTL: Duration = Duration::from_secs(60);
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_SIGNATURE_BYTES: usize = 1024;

pub(super) struct Updates {
    client: reqwest::Client,
    upstream: String,
    cache: Mutex<Cache>,
}

struct Cache {
    value: Option<Envelope>,
    fetched: Instant,
    refresh_after: Instant,
}

#[derive(Clone, Serialize)]
struct Envelope {
    /// Base64 avoids changing the exact bytes covered by the signature.
    manifest: String,
    signature: String,
}

impl Updates {
    pub(super) fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .https_only(true)
                .user_agent("caper-api-updates")
                .timeout(Duration::from_secs(5))
                .build()
                .expect("valid updater HTTP client"),
            upstream: MANIFEST_URL.into(),
            cache: Mutex::new(Cache {
                value: None,
                fetched: Instant::now(),
                refresh_after: Instant::now(),
            }),
        }
    }

    async fn response(&self) -> Result<Response, ApiError> {
        // Hold the lock across the bounded refresh: concurrent misses share one
        // GitHub fetch per process, including during a release or upstream outage.
        let mut cache = self.cache.lock().await;
        if Instant::now() >= cache.refresh_after {
            cache.value = self.fetch().await.ok();
            cache.fetched = Instant::now();
            // Back off failed refreshes too. Do not serve indefinitely stale
            // releases: a 502 lets the desktop try GitHub directly instead.
            cache.refresh_after = cache.fetched + CACHE_TTL;
        }
        let value = cache.value.as_ref().ok_or_else(unavailable)?;
        let mut response = Json(value.clone()).into_response();
        response.headers_mut().insert(
            "cache-control",
            HeaderValue::from_static("public, max-age=60"),
        );
        response.headers_mut().insert(
            "age",
            HeaderValue::from_str(&cache.fetched.elapsed().as_secs().to_string()).unwrap(),
        );
        Ok(response)
    }

    async fn fetch(&self) -> Result<Envelope, ApiError> {
        let manifest = self
            .fetch_limited(&self.upstream, MAX_MANIFEST_BYTES)
            .await?;
        let signature = self
            .fetch_limited(&format!("{}.sig", self.upstream), MAX_SIGNATURE_BYTES)
            .await?;
        let parsed: Value = serde_json::from_slice(&manifest).map_err(|_| unavailable())?;
        let signature = String::from_utf8(signature).map_err(|_| unavailable())?;
        if parsed["schema"] != 1
            || parsed["build"].as_u64().is_none_or(|build| build == 0)
            || parsed["version"]
                .as_str()
                .is_none_or(|version| version.trim().is_empty())
            || !parsed["platforms"].is_object()
            || !STANDARD
                .decode(signature.trim())
                .is_ok_and(|bytes| bytes.len() == 64)
        {
            return Err(unavailable());
        }
        // Structural checks only. Authenticity and release-race mismatches are
        // checked by the updater, never by trusting this site or its cache.
        Ok(Envelope {
            manifest: STANDARD.encode(manifest),
            signature,
        })
    }

    async fn fetch_limited(&self, url: &str, limit: usize) -> Result<Vec<u8>, ApiError> {
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| unavailable())?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
            if bytes.len() + chunk.len() > limit {
                return Err(unavailable());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

fn unavailable() -> ApiError {
    ApiError::new(StatusCode::BAD_GATEWAY, "update metadata unavailable")
}

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/updates/native", get(metadata))
}

async fn metadata(State(state): State<AppState>) -> Result<Response, ApiError> {
    state.updates.response().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use std::sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    };
    use tower::ServiceExt;

    struct Upstream {
        build: AtomicU64,
        calls: AtomicUsize,
        signature_status: AtomicUsize,
        manifest_override: Mutex<Option<String>>,
        signature_override: Mutex<Option<String>>,
    }

    fn manifest(build: u64) -> String {
        format!(
            "{{\n  \"schema\":1,\"build\":{build},\"version\":\"0.1.{build}\",\"platforms\":{{}}\n}}\n"
        )
    }

    async fn fixture() -> (Arc<Updates>, Arc<Upstream>) {
        let upstream = Arc::new(Upstream {
            build: AtomicU64::new(7),
            calls: AtomicUsize::new(0),
            signature_status: AtomicUsize::new(200),
            manifest_override: Mutex::new(None),
            signature_override: Mutex::new(None),
        });
        let router = Router::new()
            .route(
                "/latest.json",
                get(|State(state): State<Arc<Upstream>>| async move {
                    state.calls.fetch_add(1, Ordering::SeqCst);
                    state
                        .manifest_override
                        .lock()
                        .await
                        .clone()
                        .unwrap_or_else(|| manifest(state.build.load(Ordering::SeqCst)))
                }),
            )
            .route(
                "/latest.json.sig",
                get(|State(state): State<Arc<Upstream>>| async move {
                    state.calls.fetch_add(1, Ordering::SeqCst);
                    (
                        StatusCode::from_u16(state.signature_status.load(Ordering::SeqCst) as u16)
                            .unwrap(),
                        state
                            .signature_override
                            .lock()
                            .await
                            .clone()
                            .unwrap_or_else(|| {
                                STANDARD.encode([state.build.load(Ordering::SeqCst) as u8; 64])
                            }),
                    )
                }),
            )
            .with_state(upstream.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let mut updates = Updates::new();
        // Insecure loopback is only injected by these tests, not by a request or env.
        updates.client = reqwest::Client::new();
        updates.upstream = format!("http://{address}/latest.json");
        (Arc::new(updates), upstream)
    }

    async fn body(response: Response) -> Value {
        serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn cache_coalesces_concurrent_misses_and_refreshes_the_whole_pair() {
        let (updates, upstream) = fixture().await;
        let (first, second) = tokio::join!(updates.response(), updates.response());
        let first = first.unwrap();
        assert_eq!(first.headers()["cache-control"], "public, max-age=60");
        assert_eq!(first.headers()["age"], "0");
        let first = body(first).await;
        assert_eq!(first, body(second.unwrap()).await);
        assert_eq!(
            STANDARD
                .decode(first["manifest"].as_str().unwrap())
                .unwrap(),
            manifest(7).as_bytes()
        );
        assert_eq!(first["signature"], STANDARD.encode([7; 64]));
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 2);

        upstream.build.store(8, Ordering::SeqCst);
        updates.cache.lock().await.fetched -= Duration::from_secs(9);
        let hit = updates.response().await.unwrap();
        assert_eq!(hit.headers()["age"], "9");
        assert_eq!(first, body(hit).await);
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 2);
        {
            let mut cache = updates.cache.lock().await;
            cache.fetched -= Duration::from_secs(60);
            cache.refresh_after = Instant::now();
        }
        let refreshed = body(updates.response().await.unwrap()).await;
        assert_eq!(
            STANDARD
                .decode(refreshed["manifest"].as_str().unwrap())
                .unwrap(),
            manifest(8).as_bytes()
        );
        assert_eq!(refreshed["signature"], STANDARD.encode([8; 64]));
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 4);
    }

    #[tokio::test]
    async fn failed_signature_fetch_never_serves_a_partial_or_stale_pair_and_backs_off() {
        let (updates, upstream) = fixture().await;
        updates.response().await.unwrap();
        upstream.build.store(8, Ordering::SeqCst);
        upstream.signature_status.store(503, Ordering::SeqCst);
        updates.cache.lock().await.refresh_after = Instant::now();
        assert_eq!(
            updates.response().await.unwrap_err().status,
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(
            updates.response().await.unwrap_err().status,
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 4);
        assert!(updates.cache.lock().await.value.is_none());
        upstream.signature_status.store(200, Ordering::SeqCst);
        updates.cache.lock().await.refresh_after = Instant::now();
        assert_eq!(updates.response().await.unwrap().status(), StatusCode::OK);
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 6);
    }

    #[tokio::test]
    async fn production_route_is_public_cacheable_and_not_an_arbitrary_proxy() {
        let (updates, upstream) = fixture().await;
        let mut state = AppState::new(
            crate::Config::from_env(&crate::RuntimeEnvironment::default()).unwrap(),
            Arc::new(crate::Cloudflare::new()),
        );
        state.updates = updates.clone();
        let router = crate::app(state);
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/updates/native?url=http://example.invalid/secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "application/json");
        assert_eq!(response.headers()["cache-control"], "public, max-age=60");
        assert_eq!(upstream.calls.load(Ordering::SeqCst), 2);
        upstream.signature_status.store(502, Ordering::SeqCst);
        updates.cache.lock().await.refresh_after = Instant::now();
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/updates/native")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(response.headers()["cache-control"], "no-store");
        let private = router
            .oneshot(
                Request::builder()
                    .uri("/api/account/me")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(private.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(private.headers()["cache-control"], "no-store");
    }

    #[tokio::test]
    async fn enforces_upstream_metadata_size_boundaries_and_structure() {
        let (updates, upstream) = fixture().await;
        let mut at_limit = manifest(7);
        at_limit.push_str(&" ".repeat(MAX_MANIFEST_BYTES - at_limit.len()));
        *upstream.manifest_override.lock().await = Some(at_limit);
        let mut signature = STANDARD.encode([7; 64]);
        signature.push_str(&" ".repeat(MAX_SIGNATURE_BYTES - signature.len()));
        *upstream.signature_override.lock().await = Some(signature);
        let value = body(updates.response().await.unwrap()).await;
        assert_eq!(
            STANDARD
                .decode(value["manifest"].as_str().unwrap())
                .unwrap()
                .len(),
            65_536
        );
        assert_eq!(value["signature"].as_str().unwrap().len(), 1024);
        for invalid in [
            "not JSON".to_string(),
            "{}".to_string(),
            " ".repeat(MAX_MANIFEST_BYTES + 1),
        ] {
            *upstream.manifest_override.lock().await = Some(invalid);
            updates.cache.lock().await.refresh_after = Instant::now();
            assert_eq!(
                updates.response().await.unwrap_err().status,
                StatusCode::BAD_GATEWAY
            );
            assert!(updates.cache.lock().await.value.is_none());
        }
        *upstream.manifest_override.lock().await = None;
        for invalid in [
            "invalid base64".to_string(),
            STANDARD.encode([7; 63]),
            "a".repeat(MAX_SIGNATURE_BYTES + 1),
        ] {
            *upstream.signature_override.lock().await = Some(invalid);
            updates.cache.lock().await.refresh_after = Instant::now();
            assert_eq!(
                updates.response().await.unwrap_err().status,
                StatusCode::BAD_GATEWAY
            );
            assert!(updates.cache.lock().await.value.is_none());
        }
    }
}
