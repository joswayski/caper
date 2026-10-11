//! Public, revision-bound release highlights. Never serves the rolling latest
//! feed: old clients must not announce features from a newer app.
use crate::{ApiError, AppState};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const BASE_URL: &str = "https://github.com/joswayski/caper/releases/download";
const MAX_BYTES: usize = 64 * 1024;
const TTL: Duration = Duration::from_secs(60);

pub(super) struct WhatsNew {
    client: reqwest::Client,
    upstream: String,
    cache: Mutex<BTreeMap<String, Cached>>,
}

struct Cached {
    value: Option<Feed>,
    fetched: Instant,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    platform: String,
    revision: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct Feed {
    schema: u8,
    platform: String,
    revision: String,
    sequence: u64,
    releases: Vec<Release>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Release {
    sequence: u64,
    revision: String,
    #[serde(rename = "publishedAt")]
    published_at: String,
    highlights: Vec<Highlight>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Highlight {
    title: String,
    body: String,
}

fn revision_valid(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn text_valid(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty()
        && value.chars().count() <= maximum
        && !value
            .chars()
            .any(|character| character.is_control() || matches!(character, '<' | '>'))
}

impl Feed {
    fn valid(&self, request: &Request) -> bool {
        if self.schema != 1
            || self.platform != request.platform
            || self.revision != request.revision
            || self.sequence == 0
            || self.sequence > 9_007_199_254_740_991
            || self.releases.is_empty()
            || self.releases.len() > 30
        {
            return false;
        }
        let mut previous = 0;
        for release in &self.releases {
            if release.sequence <= previous
                || release.sequence > self.sequence
                || !revision_valid(&release.revision)
                || chrono::DateTime::parse_from_rfc3339(&release.published_at).is_err()
                || release.highlights.len() > 5
                || release.highlights.iter().any(|highlight| {
                    !text_valid(&highlight.title, 100) || !text_valid(&highlight.body, 500)
                })
            {
                return false;
            }
            previous = release.sequence;
        }
        self.releases
            .last()
            .is_some_and(|last| last.sequence == self.sequence && last.revision == self.revision)
    }
}

impl WhatsNew {
    pub(super) fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .https_only(true)
                .timeout(Duration::from_secs(5))
                .user_agent("caper-whats-new")
                .build()
                .expect("valid highlights HTTP client"),
            upstream: BASE_URL.into(),
            cache: Mutex::new(BTreeMap::new()),
        }
    }

    async fn response(&self, request: Request) -> Result<Response, ApiError> {
        if !matches!(
            request.platform.as_str(),
            "web" | "android" | "ios" | "macos" | "windows" | "linux"
        ) || !revision_valid(&request.revision)
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid release query",
            ));
        }
        let key = format!("{}:{}", request.platform, request.revision);
        // Bounded refreshes coalesce; missing assets back off while asynchronous
        // generation finishes. Failures never substitute a different revision.
        let mut cache = self.cache.lock().await;
        cache.retain(|_, entry| entry.fetched.elapsed() < TTL);
        if !cache.contains_key(&key) {
            // A stream of arbitrary SHAs must not evict the failure backoff and
            // turn this public endpoint into unbounded GitHub traffic.
            if cache.len() >= 32 {
                return Err(unavailable());
            }
            let value = self.fetch(&request).await.ok();
            cache.insert(
                key.clone(),
                Cached {
                    value,
                    fetched: Instant::now(),
                },
            );
        }
        let entry = &cache[&key];
        let feed = entry.value.as_ref().ok_or_else(unavailable)?;
        let mut response = Json(feed).into_response();
        response.headers_mut().insert(
            "cache-control",
            HeaderValue::from_static("public, max-age=60"),
        );
        response.headers_mut().insert(
            "age",
            HeaderValue::from_str(&entry.fetched.elapsed().as_secs().to_string()).unwrap(),
        );
        Ok(response)
    }

    async fn fetch(&self, request: &Request) -> Result<Feed, ApiError> {
        let url = format!(
            "{}/whats-new-{}/{}.json",
            self.upstream, request.platform, request.revision
        );
        let mut response = self
            .client
            .get(url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| unavailable())?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| unavailable())? {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err(unavailable());
            }
            bytes.extend_from_slice(&chunk);
        }
        let feed: Feed = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        if !feed.valid(request) {
            return Err(unavailable());
        }
        Ok(feed)
    }
}

fn unavailable() -> ApiError {
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        "release highlights are not available yet",
    )
}

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/api/whats-new", get(metadata))
}

async fn metadata(
    State(state): State<AppState>,
    Query(request): Query<Request>,
) -> Result<Response, ApiError> {
    state.whats_new.response(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request as HttpRequest,
    };
    use serde_json::json;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn public_route_is_revision_bound_cacheable_and_fails_closed() {
        let revision = "a".repeat(40);
        let value = Arc::new(Mutex::new(
            json!({"schema":1,"platform":"web","revision":revision,"sequence":2,
            "releases":[{"revision":"b".repeat(40),"sequence":1,"publishedAt":"2026-10-10T12:00:00Z","highlights":[]},
                {"revision":revision,"sequence":2,"publishedAt":"2026-10-11T12:00:00Z","highlights":[{"title":"Edit messages","body":"View earlier versions."}]}]}),
        ));
        let calls = Arc::new(AtomicUsize::new(0));
        let upstream = Router::new().route(
            "/whats-new-web/{revision}",
            get({
                let value = value.clone();
                let calls = calls.clone();
                move || {
                    let value = value.clone();
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Json(value.lock().await.clone())
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, upstream).await.unwrap();
        });
        let mut state = AppState::new(
            crate::Config::from_env(&crate::RuntimeEnvironment::default()).unwrap(),
            Arc::new(crate::Cloudflare::new()),
        );
        let mut highlights = WhatsNew::new();
        highlights.client = reqwest::Client::new();
        highlights.upstream = format!("http://{address}");
        state.whats_new = Arc::new(highlights);
        let cache = state.whats_new.clone();
        let router = crate::app(state);
        let request = || {
            HttpRequest::builder()
                .uri(format!("/api/whats-new?platform=web&revision={revision}"))
                .body(Body::empty())
                .unwrap()
        };
        let (first, second) = tokio::join!(
            router.clone().oneshot(request()),
            router.clone().oneshot(request())
        );
        for response in [first.unwrap(), second.unwrap()] {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()["cache-control"], "public, max-age=60");
            let body: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), MAX_BYTES).await.unwrap())
                    .unwrap();
            assert_eq!(body["revision"], revision);
            assert_eq!(
                body["releases"][1]["highlights"][0]["title"],
                "Edit messages"
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        for query in [
            "platform=web&revision=../latest",
            "platform=unknown&revision=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "platform=web&revision=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa&url=https://example.com",
        ] {
            let response = router
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .uri(format!("/api/whats-new?{query}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert!(response.status().is_client_error());
        }
        for invalid in [
            json!({"platform":"android"}),
            json!({"revision":"c".repeat(40)}),
            json!({"sequence":9}),
            json!({"releases":[]}),
        ] {
            let original = value.lock().await.clone();
            for (key, field) in invalid.as_object().unwrap() {
                value.lock().await[key] = field.clone();
            }
            cache.cache.lock().await.clear();
            let response = router.clone().oneshot(request()).await.unwrap();
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
            assert_eq!(response.headers()["cache-control"], "no-store");
            *value.lock().await = original;
        }
        cache.cache.lock().await.clear();
        let before = calls.load(Ordering::SeqCst);
        assert_eq!(
            router.clone().oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
        for number in 0..40 {
            let response = router
                .clone()
                .oneshot(
                    HttpRequest::builder()
                        .uri(format!(
                            "/api/whats-new?platform=web&revision={number:040x}"
                        ))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        }
        assert_eq!(calls.load(Ordering::SeqCst) - before, 32);
        assert_eq!(
            router.oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
    }
}
