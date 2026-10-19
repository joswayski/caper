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
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::OnceCell;

const BASE_URL: &str = "https://github.com/joswayski/caper/releases/download";
const MAX_BYTES: usize = 64 * 1024;
const TTL: Duration = Duration::from_secs(60);
/// Each map's bound. Feeds that loaded and lookups that have not are counted
/// apart, so unknown revisions can never displace or block a working one.
const CAPACITY: usize = 32;

pub(super) struct WhatsNew {
    client: reqwest::Client,
    upstream: String,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    /// Revisions whose feed loaded, refreshed in place once stale.
    found: BTreeMap<String, Arc<Slot>>,
    /// Lookups in flight or failed. Anyone can name an unknown revision, so
    /// when full a new revision replaces the oldest settled failure rather
    /// than wait it out; only lookups still in flight hold their places, which
    /// bounds concurrent GitHub traffic.
    pending: BTreeMap<String, Arc<Slot>>,
}

/// One fetch per revision at a time: concurrent requests share its result.
struct Slot(OnceCell<Cached>, Instant);

impl Slot {
    fn new() -> Arc<Self> {
        Arc::new(Self(OnceCell::new(), Instant::now()))
    }

    /// Fetches time out within seconds, so a slot still unfilled after `TTL`
    /// was abandoned by every request waiting on it and must free its place.
    fn stale(&self) -> bool {
        self.0
            .get()
            .map_or(self.1, |cached| cached.fetched)
            .elapsed()
            >= TTL
    }

    fn found(&self) -> bool {
        self.0.get().is_some_and(|cached| cached.value.is_some())
    }

    fn age(&self) -> Duration {
        self.0
            .get()
            .map_or(Duration::ZERO, |cached| cached.fetched.elapsed())
    }
}

struct Cached {
    value: Option<Feed>,
    fetched: Instant,
}

impl Cache {
    /// The slot serving `key`, or `None` while every unknown lookup's place is
    /// held by one still in flight.
    fn slot(&mut self, key: &str) -> Option<Arc<Slot>> {
        self.pending.retain(|_, slot| !slot.stale());
        // A failed refresh drops a revision back to the bounded unknown path.
        self.found.retain(|_, slot| !slot.stale() || slot.found());
        if let Some(slot) = self.found.get_mut(key) {
            if slot.stale() {
                *slot = Slot::new();
            }
            return Some(slot.clone());
        }
        if let Some(slot) = self.pending.get(key) {
            return Some(slot.clone());
        }
        if self.pending.len() >= CAPACITY {
            let oldest = self
                .pending
                .iter()
                .filter(|(_, slot)| slot.0.initialized())
                .max_by_key(|(_, slot)| slot.age())
                .map(|(key, _)| key.clone())?;
            self.pending.remove(&oldest);
        }
        let slot = Slot::new();
        self.pending.insert(key.to_owned(), slot.clone());
        Some(slot)
    }

    /// Moves a lookup that just loaded into `found`, evicting the stalest.
    fn promote(&mut self, key: &str, slot: &Arc<Slot>) {
        if !self
            .pending
            .get(key)
            .is_some_and(|pending| Arc::ptr_eq(pending, slot))
        {
            return;
        }
        self.pending.remove(key);
        if self.found.len() >= CAPACITY
            && let Some(oldest) = self
                .found
                .iter()
                .max_by_key(|(_, slot)| slot.age())
                .map(|(key, _)| key.clone())
        {
            self.found.remove(&oldest);
        }
        self.found.insert(key.to_owned(), slot.clone());
    }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<String>,
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
                || release
                    .version
                    .as_ref()
                    .is_some_and(|version| !text_valid(version, 64))
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
            cache: Mutex::default(),
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
        // Bounded refreshes coalesce per revision without holding the map
        // across GitHub; missing assets back off while asynchronous generation
        // finishes. Failures never substitute a different revision.
        let slot = self
            .cache
            .lock()
            .expect("highlights cache lock")
            .slot(&key)
            .ok_or_else(unavailable)?;
        let entry = slot
            .0
            .get_or_init(|| async {
                Cached {
                    value: self.fetch(&request).await.ok(),
                    fetched: Instant::now(),
                }
            })
            .await;
        let feed = entry.value.as_ref().ok_or_else(unavailable)?;
        self.cache
            .lock()
            .expect("highlights cache lock")
            .promote(&key, &slot);
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
    use tokio::sync::{Mutex, Semaphore};
    use tower::ServiceExt;

    fn query(revision: &str) -> HttpRequest<Body> {
        HttpRequest::builder()
            .uri(format!("/api/whats-new?platform=web&revision={revision}"))
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn public_route_is_revision_bound_cacheable_and_fails_closed() {
        let revision = "a".repeat(40);
        let value = Arc::new(Mutex::new(
            json!({"schema":1,"platform":"web","revision":revision,"sequence":2,
            "releases":[{"revision":"b".repeat(40),"sequence":1,"publishedAt":"2026-10-10T12:00:00Z","highlights":[]},
                {"revision":revision,"sequence":2,"version":"0.1.112","publishedAt":"2026-10-11T12:00:00Z","highlights":[{"title":"Edit messages","body":"View earlier versions."}]}]}),
        ));
        let calls = Arc::new(AtomicUsize::new(0));
        // Holds every lookup of the all-`f` revision until permits are added.
        let gate = Arc::new(Semaphore::new(0));
        let upstream = Router::new().route(
            "/whats-new-web/{revision}",
            get({
                let value = value.clone();
                let calls = calls.clone();
                let gate = gate.clone();
                move |axum::extract::Path(file): axum::extract::Path<String>| {
                    let value = value.clone();
                    let calls = calls.clone();
                    let gate = gate.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        if file.starts_with('f') {
                            gate.acquire().await.unwrap().forget();
                        }
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
            assert_eq!(body["releases"][1]["version"], "0.1.112");
            assert!(body["releases"][0].get("version").is_none());
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
            *cache.cache.lock().unwrap() = Cache::default();
            let response = router.clone().oneshot(request()).await.unwrap();
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
            assert_eq!(response.headers()["cache-control"], "no-store");
            *value.lock().await = original;
        }
        *cache.cache.lock().unwrap() = Cache::default();
        // A slow lookup holds neither the map nor other revisions.
        let before = calls.load(Ordering::SeqCst);
        let slow = tokio::spawn(router.clone().oneshot(query(&"f".repeat(40))));
        while calls.load(Ordering::SeqCst) == before {
            tokio::task::yield_now().await;
        }
        let response =
            tokio::time::timeout(Duration::from_secs(5), router.clone().oneshot(request()))
                .await
                .expect("not blocked behind a slow lookup")
                .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        gate.add_permits(1);
        assert_eq!(
            slow.await.unwrap().unwrap().status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(calls.load(Ordering::SeqCst) - before, 2);

        // Unknown revisions fill only their own bound. Once it is full of
        // failures, a new revision replaces the oldest one, so made-up
        // revisions can't keep a new build's notes from loading; a working
        // revision is still served and refreshed.
        let before = calls.load(Ordering::SeqCst);
        for number in 0..40 {
            let response = router
                .clone()
                .oneshot(query(&format!("{number:040x}")))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        }
        assert_eq!(calls.load(Ordering::SeqCst) - before, 40);
        assert_eq!(cache.cache.lock().unwrap().pending.len(), CAPACITY);
        // A failure still inside its backoff is answered without a lookup.
        let response = router
            .clone()
            .oneshot(query(&format!("{:040x}", 39)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert_eq!(calls.load(Ordering::SeqCst) - before, 40);
        assert_eq!(
            router.clone().oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(calls.load(Ordering::SeqCst) - before, 40);
        {
            let mut map = cache.cache.lock().unwrap();
            let slot = map.found.get_mut(&format!("web:{revision}")).unwrap();
            let value = slot.0.get().unwrap().value.clone();
            let old = Instant::now() - TTL;
            *slot = Arc::new(Slot(
                OnceCell::new_with(Some(Cached {
                    value,
                    fetched: old,
                })),
                old,
            ));
            // Lookups abandoned by every waiter free their places too.
            for slot in map.pending.values_mut() {
                *slot = Arc::new(Slot(OnceCell::new(), old));
            }
        }
        assert_eq!(
            router.clone().oneshot(request()).await.unwrap().status(),
            StatusCode::OK
        );
        assert_eq!(calls.load(Ordering::SeqCst) - before, 41);
        assert_eq!(
            router
                .clone()
                .oneshot(query(&"9".repeat(40)))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(calls.load(Ordering::SeqCst) - before, 42);

        // Only lookups still in flight keep a new revision waiting.
        *cache.cache.lock().unwrap() = Cache::default();
        let before = calls.load(Ordering::SeqCst);
        let held: Vec<_> = (0..CAPACITY)
            .map(|number| tokio::spawn(router.clone().oneshot(query(&format!("f{number:039x}")))))
            .collect();
        while calls.load(Ordering::SeqCst) - before < CAPACITY {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            router
                .clone()
                .oneshot(query(&"8".repeat(40)))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(calls.load(Ordering::SeqCst) - before, CAPACITY);
        gate.add_permits(CAPACITY);
        for lookup in held {
            assert_eq!(
                lookup.await.unwrap().unwrap().status(),
                StatusCode::BAD_GATEWAY
            );
        }
        assert_eq!(
            router
                .oneshot(query(&"8".repeat(40)))
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_GATEWAY
        );
        assert_eq!(calls.load(Ordering::SeqCst) - before, CAPACITY + 1);
    }
}
