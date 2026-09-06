use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures_util::stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::sync::{Mutex, watch};
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};
use uuid::Uuid;

const LEASE: Duration = Duration::from_secs(45);
// Also revoke on leave. Expiry bounds exposure if revocation or process recovery fails.
const MAX_CALL_DURATION: Duration = Duration::from_secs(60 * 60);
const TURN_TTL: u64 = MAX_CALL_DURATION.as_secs();
const MAX_PARTICIPANTS: usize = 12;
const MAX_TRACKS: usize = 1;
const MAX_SUBSCRIPTIONS: usize = MAX_PARTICIPANTS - 1;
const JOIN_LIMIT_PER_MINUTE: usize = 30;
const OP_LIMIT_PER_MINUTE: usize = 120;
const MAX_CLEANUP_BACKLOG: usize = 512;
const BODY_LIMIT: usize = 256 * 1024;

mod db;

pub use db::connect_database;

#[derive(Clone)]
pub struct Config {
    pub enabled: bool,
    pub bind: SocketAddr,
    app_id: Option<String>,
    app_secret: Option<String>,
    turn_key_id: Option<String>,
    turn_token: Option<String>,
    provider_base: String,
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let enabled = std::env::var("MEDIA_ENABLED").is_ok_and(|v| v == "true" || v == "1");
        let get = |key| std::env::var(key).ok().filter(|v| !v.trim().is_empty());
        let config = Self {
            enabled,
            bind: std::env::var("MEDIA_BIND")
                .unwrap_or_else(|_| "0.0.0.0:3001".into())
                .parse()
                .map_err(|_| "MEDIA_BIND must be a socket address")?,
            app_id: get("CF_SFU_APP_ID"),
            app_secret: get("CF_SFU_APP_SECRET"),
            turn_key_id: get("CF_TURN_KEY_ID"),
            turn_token: get("CF_TURN_API_TOKEN"),
            provider_base: "https://rtc.live.cloudflare.com/v1".into(),
        };
        if enabled
            && [
                config.app_id.as_ref(),
                config.app_secret.as_ref(),
                config.turn_key_id.as_ref(),
                config.turn_token.as_ref(),
            ]
            .contains(&None)
        {
            return Err(
                "all CF_SFU_* and CF_TURN_* variables are required when media is enabled".into(),
            );
        }
        Ok(config)
    }

    #[cfg(test)]
    fn test(enabled: bool) -> Self {
        Self {
            enabled,
            bind: "127.0.0.1:0".parse().unwrap(),
            app_id: Some("app".into()),
            app_secret: Some("secret".into()),
            turn_key_id: Some("turn".into()),
            turn_token: Some("token".into()),
            provider_base: "mock".into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider unavailable")]
    Unavailable,
    #[error("provider rejected request")]
    Rejected,
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn create_session(&self, config: &Config) -> Result<String, ProviderError>;
    async fn turn(&self, config: &Config) -> Result<Vec<IceServer>, ProviderError>;
    async fn revoke_turn(&self, config: &Config, username: &str) -> Result<(), ProviderError>;
    async fn session_tracks(&self, config: &Config, session: &str) -> Result<Value, ProviderError>;
    async fn tracks_new(
        &self,
        config: &Config,
        session: &str,
        body: Value,
    ) -> Result<Value, ProviderError>;
    async fn negotiate(
        &self,
        config: &Config,
        session: &str,
        body: Value,
    ) -> Result<Value, ProviderError>;
    async fn close(
        &self,
        config: &Config,
        session: &str,
        mid: &str,
    ) -> Result<Value, ProviderError>;
}

pub struct Cloudflare {
    client: reqwest::Client,
}
impl Cloudflare {
    #[must_use]
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("HTTP client"),
        }
    }
}
impl Default for Cloudflare {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Provider for Cloudflare {
    async fn create_session(&self, c: &Config) -> Result<String, ProviderError> {
        let value = self
            .request(
                reqwest::Method::POST,
                c,
                &format!("apps/{}/sessions/new", required(&c.app_id)),
                Value::Null,
            )
            .await?;
        value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(ProviderError::Rejected)
    }
    async fn turn(&self, c: &Config) -> Result<Vec<IceServer>, ProviderError> {
        let response = self
            .client
            .post(format!(
                "{}/turn/keys/{}/credentials/generate-ice-servers",
                c.provider_base,
                required(&c.turn_key_id)
            ))
            .bearer_auth(required(&c.turn_token))
            .json(&json!({"ttl": TURN_TTL}))
            .send()
            .await
            .map_err(|_| ProviderError::Unavailable)?;
        if response.status() != StatusCode::CREATED {
            return Err(if response.status().is_server_error() {
                ProviderError::Unavailable
            } else {
                ProviderError::Rejected
            });
        }
        #[derive(Deserialize)]
        struct Turn {
            #[serde(rename = "iceServers")]
            ice_servers: Vec<IceServer>,
        }
        let mut servers = response
            .json::<Turn>()
            .await
            .map(|v| v.ice_servers)
            .map_err(|_| ProviderError::Rejected)?;
        for server in &mut servers {
            filter_unsupported_ice_urls(&mut server.urls);
        }
        servers.retain(|server| match &server.urls {
            Value::String(url) => !url.is_empty(),
            Value::Array(urls) => !urls.is_empty(),
            _ => false,
        });
        Ok(servers)
    }
    async fn revoke_turn(&self, c: &Config, username: &str) -> Result<(), ProviderError> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/turn/keys/{}/credentials/",
            c.provider_base,
            required(&c.turn_key_id)
        ))
        .map_err(|_| ProviderError::Rejected)?;
        url.path_segments_mut()
            .map_err(|_| ProviderError::Rejected)?
            .pop_if_empty()
            .push(username)
            .push("revoke");
        let response = self
            .client
            .post(url)
            .bearer_auth(required(&c.turn_token))
            .send()
            .await
            .map_err(|_| ProviderError::Unavailable)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(ProviderError::Rejected)
        }
    }
    async fn session_tracks(&self, c: &Config, session: &str) -> Result<Value, ProviderError> {
        self.request(
            reqwest::Method::GET,
            c,
            &format!("apps/{}/sessions/{session}", required(&c.app_id)),
            json!({}),
        )
        .await
    }
    async fn tracks_new(&self, c: &Config, s: &str, body: Value) -> Result<Value, ProviderError> {
        self.request(
            reqwest::Method::POST,
            c,
            &format!("apps/{}/sessions/{s}/tracks/new", required(&c.app_id)),
            body,
        )
        .await
    }
    async fn negotiate(&self, c: &Config, s: &str, body: Value) -> Result<Value, ProviderError> {
        self.request(
            reqwest::Method::PUT,
            c,
            &format!("apps/{}/sessions/{s}/renegotiate", required(&c.app_id)),
            body,
        )
        .await
    }
    async fn close(&self, c: &Config, s: &str, mid: &str) -> Result<Value, ProviderError> {
        self.request(
            reqwest::Method::PUT,
            c,
            &format!("apps/{}/sessions/{s}/tracks/close", required(&c.app_id)),
            json!({"tracks":[{"mid":mid}],"force":true}),
        )
        .await
    }
}
impl Cloudflare {
    async fn request(
        &self,
        method: reqwest::Method,
        c: &Config,
        path: &str,
        body: Value,
    ) -> Result<Value, ProviderError> {
        let request = self
            .client
            .request(method.clone(), format!("{}/{path}", c.provider_base))
            .bearer_auth(required(&c.app_secret));
        // Session creation has no request body. Sending {} selects the legacy
        // SDP-in-body API path, which rejects it without sessionDescription.
        let request = if method == reqwest::Method::GET || body.is_null() {
            request
        } else {
            request.json(&body)
        };
        let response = request
            .send()
            .await
            .map_err(|_| ProviderError::Unavailable)?;
        if !response.status().is_success() {
            tracing::warn!(status=%response.status(), "Cloudflare request failed");
            return Err(if response.status().is_server_error() {
                ProviderError::Unavailable
            } else {
                ProviderError::Rejected
            });
        }
        let value: Value = response.json().await.map_err(|_| ProviderError::Rejected)?;
        validate_provider_envelope(&value)?;
        Ok(value)
    }
}
fn validate_provider_envelope(value: &Value) -> Result<(), ProviderError> {
    if !value.is_object()
        || value.get("errorCode").is_some_and(|v| !v.is_null())
        || value
            .get("tracks")
            .and_then(Value::as_array)
            .is_some_and(|tracks| {
                tracks
                    .iter()
                    .any(|t| t.get("errorCode").is_some_and(|v| !v.is_null()))
            })
    {
        return Err(ProviderError::Rejected);
    }
    Ok(())
}
fn filter_unsupported_ice_urls(urls: &mut Value) {
    // Browser-blocked alternate ports can hold up non-trickle ICE gathering for
    // five seconds even when the primary STUN/TURN routes are already usable.
    let supported = |url: &str| {
        !url.split('?')
            .next()
            .is_some_and(|authority| authority.ends_with(":53"))
    };
    match urls {
        Value::String(url) if !supported(url) => *urls = Value::Array(vec![]),
        Value::Array(values) => values.retain(|v| v.as_str().is_some_and(supported)),
        _ => {}
    }
}
fn required(value: &Option<String>) -> &str {
    value.as_deref().expect("validated enabled configuration")
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IceServer {
    pub urls: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

#[derive(Clone)]
pub struct AppState {
    config: Config,
    provider: Arc<dyn Provider>,
    registry: Arc<Mutex<Registry>>,
    database: Option<PgPool>,
    events: watch::Sender<()>,
}
impl AppState {
    pub fn new(config: Config, provider: Arc<dyn Provider>) -> Self {
        Self::with_database(config, provider, None)
    }

    pub fn with_database(
        config: Config,
        provider: Arc<dyn Provider>,
        database: Option<PgPool>,
    ) -> Self {
        let (events, _) = watch::channel(());
        Self {
            config,
            provider,
            registry: Arc::new(Mutex::new(Registry::default())),
            database,
            events,
        }
    }

    #[must_use]
    pub fn database(&self) -> Option<&PgPool> {
        self.database.as_ref()
    }
}

#[derive(Default)]
struct Registry {
    participants: HashMap<Uuid, Participant>,
    tokens: HashMap<String, Uuid>,
    joins: VecDeque<Instant>,
    cleanup: VecDeque<CleanupJob>,
    joining: usize,
    monitor_reservations: HashSet<(Uuid, MonitorRole)>,
}
struct Participant {
    id: Uuid,
    token: String,
    name: String,
    session: String,
    turn_usernames: Vec<String>,
    muted: bool,
    deafened: bool,
    lease: Instant,
    joined: Instant,
    tracks: HashMap<String, Track>,
    subscriptions: HashMap<String, Uuid>,
    pending_offer: bool,
    operation: bool,
    operations: VecDeque<Instant>,
    monitor: Option<Monitor>,
    events: Option<watch::Sender<()>>,
}
#[derive(Clone, Copy)]
struct Monitor {
    parent: Uuid,
    role: MonitorRole,
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
enum MonitorRole {
    Sender,
    Receiver,
}
#[derive(Clone)]
struct CleanupJob {
    session: String,
    mid: String,
    attempts: u8,
}
struct Track {
    id: Uuid,
    kind: Kind,
    provider_name: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum Kind {
    Microphone,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: &'static str,
}
impl ApiError {
    fn new(status: StatusCode, message: &'static str) -> Self {
        Self { status, message }
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({"error":self.message}))).into_response()
    }
}
impl From<ProviderError> for ApiError {
    fn from(_: ProviderError) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, "media provider unavailable")
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { StatusCode::NO_CONTENT }))
        .route("/api/media/status", get(status))
        .route("/api/media/join", post(join))
        .route("/api/media/snapshot", post(snapshot))
        .route("/api/media/events", get(events))
        .route("/api/media/publish", post(publish))
        .route("/api/media/subscribe", post(subscribe))
        .route("/api/media/negotiate", post(negotiate))
        .route("/api/media/close", post(close))
        .route("/api/media/state", post(update_state))
        .route("/api/media/leave", post(leave))
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                let mut response = if request
                    .headers()
                    .get("sec-fetch-site")
                    .is_some_and(|value| value == "cross-site")
                {
                    StatusCode::FORBIDDEN.into_response()
                } else {
                    next.run(request).await
                };
                response.headers_mut().insert(
                    "cache-control",
                    axum::http::HeaderValue::from_static("no-store"),
                );
                response
            },
        ))
        .with_state(state)
}
async fn status(State(s): State<AppState>) -> Json<Value> {
    Json(json!({"enabled":s.config.enabled}))
}
fn ensure_enabled(s: &AppState) -> Result<(), ApiError> {
    s.config
        .enabled
        .then_some(())
        .ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "media disabled"))
}
fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"))
}
fn authenticate(r: &Registry, token: &str) -> Result<Uuid, ApiError> {
    let id = r
        .tokens
        .iter()
        .find(|(candidate, _)| candidate.as_bytes().ct_eq(token.as_bytes()).into())
        .map(|(_, id)| *id)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    let p = r
        .participants
        .get(&id)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    if p.lease.elapsed() >= LEASE || p.joined.elapsed() >= MAX_CALL_DURATION {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "session expired"));
    }
    if let Some(monitor) = p.monitor {
        let parent = r
            .participants
            .get(&monitor.parent)
            .filter(|parent| parent.monitor.is_none())
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "parent session ended"))?;
        if parent.lease.elapsed() >= LEASE || parent.joined.elapsed() >= MAX_CALL_DURATION {
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "parent session ended",
            ));
        }
    }
    Ok(id)
}

struct EventStreamState {
    state: AppState,
    token: String,
    updates: watch::Receiver<()>,
    cancellation: watch::Receiver<()>,
    first: bool,
}

async fn events(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    ensure_enabled(&s)?;
    let token = bearer(&headers)?.to_owned();
    let (updates, cancellation) = {
        // Authentication, subscription and connection replacement are atomic with roster
        // mutations, preventing a change between authentication and registration being lost.
        let mut r = s.registry.lock().await;
        let id = authenticate(&r, &token)?;
        let participant = r.participants.get_mut(&id).unwrap();
        if participant.monitor.is_some() {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "monitor sessions cannot receive public events",
            ));
        }
        let (tx, rx) = watch::channel(());
        participant.events = Some(tx);
        (s.events.subscribe(), rx)
    };
    let stream = stream::unfold(
        EventStreamState {
            state: s,
            token,
            updates,
            cancellation,
            first: true,
        },
        |mut stream| async move {
            let event = if stream.first {
                stream.first = false;
                "ready"
            } else {
                tokio::select! {
                    changed = stream.updates.changed() => {
                        if changed.is_err() { return None; }
                        "changed"
                    }
                    cancelled = stream.cancellation.changed() => {
                        let _ = cancelled;
                        return None;
                    }
                    () = tokio::time::sleep(Duration::from_secs(10)) => "heartbeat",
                }
            };
            // Heartbeats do not renew the lease. They do bound expiry/revocation detection
            // even when the cleanup sweep is not running.
            let valid = {
                let r = stream.state.registry.lock().await;
                authenticate(&r, &stream.token).is_ok() && stream.cancellation.has_changed().is_ok()
            };
            valid.then(|| {
                (
                    Ok::<_, std::convert::Infallible>(Event::default().event(event).data("{}")),
                    stream,
                )
            })
        },
    );
    let mut response = Sse::new(stream).into_response();
    response.headers_mut().insert(
        "x-accel-buffering",
        axum::http::HeaderValue::from_static("no"),
    );
    Ok(response)
}

fn begin_operation(p: &mut Participant) -> Result<(), ApiError> {
    let now = Instant::now();
    while p
        .operations
        .front()
        .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
    {
        p.operations.pop_front();
    }
    if p.operation {
        return Err(ApiError::new(StatusCode::CONFLICT, "operation pending"));
    }
    if p.operations.len() >= OP_LIMIT_PER_MINUTE {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded",
        ));
    }
    p.operations.push_back(now);
    p.operation = true;
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Join {
    name: String,
    monitor: Option<MonitorRole>,
}
async fn join(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Join>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    let name = input.name.trim();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(char::is_control) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid name"));
    }
    let monitor = {
        let mut r = s.registry.lock().await;
        let now = Instant::now();
        while r
            .joins
            .front()
            .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
        {
            r.joins.pop_front();
        }
        if r.joins.len() >= JOIN_LIMIT_PER_MINUTE {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate limit exceeded",
            ));
        }
        if r.participants.len() + r.joining >= MAX_PARTICIPANTS {
            return Err(ApiError::new(StatusCode::CONFLICT, "lobby full"));
        }
        let monitor = if let Some(role) = input.monitor {
            let parent = authenticate(&r, bearer(&headers)?)?;
            if r.participants.get(&parent).unwrap().monitor.is_some() {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "monitor sessions cannot create monitors",
                ));
            }
            if r.monitor_reservations.contains(&(parent, role))
                || r.participants.values().any(|p| {
                    p.monitor
                        .is_some_and(|m| m.parent == parent && m.role == role)
                })
            {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "monitor role already active",
                ));
            }
            r.monitor_reservations.insert((parent, role));
            Some(Monitor { parent, role })
        } else {
            None
        };
        r.joins.push_back(now);
        r.joining += 1;
        monitor
    };
    // These independent provider requests run together. Session creation is intentionally
    // never retried: an ambiguous create could orphan a session.
    let (session, ice) = tokio::join!(
        s.provider.create_session(&s.config),
        s.provider.turn(&s.config)
    );
    let session = match session {
        Ok(session) => session,
        Err(error) => {
            release_join_reservation(&s, monitor).await;
            if let Ok(servers) = ice {
                for username in servers
                    .iter()
                    .filter_map(|server| server.username.as_deref())
                {
                    if s.provider.revoke_turn(&s.config, username).await.is_err() {
                        tracing::warn!("TURN revocation failed after session creation failure");
                    }
                }
            }
            return Err(error.into());
        }
    };
    let ice = match ice {
        Ok(ice) => ice,
        Err(error) => {
            release_join_reservation(&s, monitor).await;
            return Err(error.into());
        }
    };
    let id = Uuid::new_v4();
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let p = Participant {
        id,
        token: token.clone(),
        name: name.into(),
        session,
        turn_usernames: ice
            .iter()
            .filter_map(|server| server.username.clone())
            .collect(),
        muted: false,
        deafened: false,
        lease: Instant::now(),
        joined: Instant::now(),
        tracks: HashMap::new(),
        subscriptions: HashMap::new(),
        pending_offer: false,
        operation: false,
        operations: VecDeque::new(),
        monitor,
        events: None,
    };
    let mut r = s.registry.lock().await;
    r.joining -= 1;
    if let Some(monitor) = monitor {
        r.monitor_reservations
            .remove(&(monitor.parent, monitor.role));
        let parent_valid = r.participants.get(&monitor.parent).is_some_and(|parent| {
            parent.monitor.is_none()
                && parent.lease.elapsed() < LEASE
                && parent.joined.elapsed() < MAX_CALL_DURATION
        });
        if !parent_valid {
            drop(r);
            revoke_participant_turn(&s, &p).await;
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "parent session ended",
            ));
        }
    }
    r.tokens.insert(token.clone(), id);
    r.participants.insert(id, p);
    if monitor.is_none() {
        s.events.send_replace(());
    }
    Ok(Json(json!({"token":token,"id":id,"iceServers":ice})))
}

async fn release_join_reservation(s: &AppState, monitor: Option<Monitor>) {
    let mut r = s.registry.lock().await;
    r.joining -= 1;
    if let Some(monitor) = monitor {
        r.monitor_reservations
            .remove(&(monitor.parent, monitor.role));
    }
}

#[derive(Serialize)]
struct View<'a> {
    id: Uuid,
    name: &'a str,
    muted: bool,
    deafened: bool,
    tracks: Vec<TrackView>,
}
#[derive(Serialize)]
struct TrackView {
    id: Uuid,
    kind: Kind,
}
async fn snapshot(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(_): Json<Empty>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    let token = bearer(&headers)?;
    let mut r = s.registry.lock().await;
    let id = authenticate(&r, token)?;
    r.participants.get_mut(&id).unwrap().lease = Instant::now();
    if r.participants.get(&id).unwrap().monitor.is_some() {
        return Ok(Json(json!({"participants":[]})));
    }
    let participants: Vec<_> = r
        .participants
        .values()
        .filter(|p| p.monitor.is_none())
        .map(|p| View {
            id: p.id,
            name: &p.name,
            muted: p.muted,
            deafened: p.deafened,
            tracks: p
                .tracks
                .values()
                .map(|t| TrackView {
                    id: t.id,
                    kind: t.kind,
                })
                .collect(),
        })
        .collect();
    Ok(Json(json!({"participants":participants})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Sdp {
    #[serde(rename = "type")]
    ty: SdpType,
    sdp: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum SdpType {
    Offer,
    Answer,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Publish {
    kind: String,
    mid: String,
    session_description: Sdp,
}
fn valid_text(v: &str, max: usize) -> bool {
    !v.is_empty() && v.len() <= max && !v.chars().any(|c| c == '\0')
}
async fn publish(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<Publish>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    if i.kind != "microphone"
        || !matches!(i.session_description.ty, SdpType::Offer)
        || !valid_text(&i.mid, 64)
        || !valid_text(&i.session_description.sdp, 200_000)
    {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid request"));
    }
    let token = bearer(&headers)?;
    let (id, session) = {
        let mut r = s.registry.lock().await;
        let id = authenticate(&r, token)?;
        let p = r.participants.get_mut(&id).unwrap();
        begin_operation(p)?;
        if p.monitor.is_some_and(|m| m.role != MonitorRole::Sender) {
            p.operation = false;
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "monitor receiver cannot publish",
            ));
        }
        if p.pending_offer {
            p.operation = false;
            return Err(ApiError::new(StatusCode::CONFLICT, "negotiation pending"));
        }
        if p.tracks.len() >= MAX_TRACKS || p.tracks.contains_key(&i.mid) {
            p.operation = false;
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "track limit or duplicate track",
            ));
        }
        (id, p.session.clone())
    };
    let track_id = Uuid::new_v4();
    let provider_name = format!("caper-{track_id}");
    let body = json!({"sessionDescription":{"type":"offer","sdp":i.session_description.sdp},"tracks":[{"location":"local","mid":i.mid,"trackName":provider_name,"kind":"audio"}]});
    let result = s.provider.tracks_new(&s.config, &session, body).await;
    let valid = result.as_ref().is_ok_and(|v| {
        validate_provider_envelope(v).is_ok()
            && v.get("tracks").and_then(Value::as_array).is_some_and(|a| {
                a.iter()
                    .any(|t| t.get("mid").and_then(Value::as_str) == Some(i.mid.as_str()))
            })
            && v.pointer("/sessionDescription/type")
                .and_then(Value::as_str)
                == Some("answer")
            && v.pointer("/sessionDescription/sdp")
                .and_then(Value::as_str)
                .is_some()
    });
    if !valid {
        enqueue_cleanup(&s, session.clone(), i.mid.clone()).await;
        remove_participant(&s, id).await;
        return Err(ApiError::from(
            result.err().unwrap_or(ProviderError::Rejected),
        ));
    }
    let result = result.unwrap();
    let mut r = s.registry.lock().await;
    let Some(p) = r.participants.get_mut(&id) else {
        enqueue_cleanup_locked(&mut r, session, i.mid);
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "session expired"));
    };
    p.operation = false;
    p.tracks.insert(
        i.mid,
        Track {
            id: track_id,
            kind: Kind::Microphone,
            provider_name,
        },
    );
    if p.monitor.is_none() {
        s.events.send_replace(());
    }
    let mut result = result;
    if let Some(object) = result.as_object_mut() {
        object.insert("trackId".into(), json!(track_id));
    }
    Ok(Json(result))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Subscribe {
    track_id: Uuid,
}
async fn subscribe(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<Subscribe>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    let token = bearer(&headers)?;
    let (me, session, source) = {
        let mut r = s.registry.lock().await;
        let me = authenticate(&r, token)?;
        let source = r
            .participants
            .values()
            .find_map(|p| {
                p.tracks
                    .values()
                    .find(|t| t.id == i.track_id)
                    .map(|t| (p.id, p.session.clone(), t.provider_name.clone(), t.kind))
            })
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "track not found"))?;
        if source.0 == me {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "cannot subscribe to own track",
            ));
        }
        let subscriber_monitor = r.participants.get(&me).unwrap().monitor;
        let source_monitor = r.participants.get(&source.0).unwrap().monitor;
        let authorized = match (subscriber_monitor, source_monitor) {
            (None, None) => true,
            (Some(subscriber), Some(owner)) => {
                subscriber.role == MonitorRole::Receiver
                    && owner.role == MonitorRole::Sender
                    && subscriber.parent == owner.parent
            }
            _ => false,
        };
        if !authorized {
            return Err(ApiError::new(StatusCode::FORBIDDEN, "track is private"));
        }
        let p = r.participants.get_mut(&me).unwrap();
        begin_operation(p)?;
        if p.pending_offer {
            p.operation = false;
            return Err(ApiError::new(StatusCode::CONFLICT, "negotiation pending"));
        }
        if p.subscriptions.len() >= MAX_SUBSCRIPTIONS
            || p.subscriptions.values().any(|id| *id == i.track_id)
        {
            p.operation = false;
            return Err(ApiError::new(StatusCode::CONFLICT, "subscription limit"));
        }
        (me, p.session.clone(), source)
    };
    let result = s
        .provider
        .tracks_new(
            &s.config,
            &session,
            json!({"tracks":[{"location":"remote","sessionId":source.1,"trackName":source.2}]}),
        )
        .await;
    let valid_result = result
        .as_ref()
        .ok()
        .filter(|v| validate_provider_envelope(v).is_ok());
    let mid = valid_result
        .and_then(|v| v.get("tracks"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|t| t.get("mid"))
        .and_then(Value::as_str)
        .ok_or(ProviderError::Rejected)
        .map_err(ApiError::from);
    let Ok(mid) = mid else {
        // Do not retry tracks/new after an ambiguous response. Discover and close any MID
        // the provider allocated before invalidating this connection's capability.
        if let Ok(info) = s.provider.session_tracks(&s.config, &session).await
            && let Some(tracks) = info.get("tracks").and_then(Value::as_array)
        {
            for track in tracks {
                if let Some(mid) = track.get("mid").and_then(Value::as_str) {
                    enqueue_cleanup(&s, session.clone(), mid.to_owned()).await;
                }
            }
        }
        remove_participant(&s, me).await;
        return Err(mid.unwrap_err());
    };
    let mid = mid.to_owned();
    let result = result.unwrap();
    let has_offer = result
        .pointer("/sessionDescription/type")
        .and_then(Value::as_str)
        == Some("offer")
        && result
            .pointer("/sessionDescription/sdp")
            .and_then(Value::as_str)
            .is_some();
    let pending = result
        .get("requiresImmediateRenegotiation")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if pending != has_offer {
        enqueue_cleanup(&s, session, mid).await;
        remove_participant(&s, me).await;
        return Err(ProviderError::Rejected.into());
    }
    let mut r = s.registry.lock().await;
    if !r
        .participants
        .values()
        .any(|p| p.tracks.values().any(|track| track.id == i.track_id))
    {
        enqueue_cleanup_locked(&mut r, session, mid);
        drop(r);
        remove_participant(&s, me).await;
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "source left during subscription",
        ));
    }
    let Some(p) = r.participants.get_mut(&me) else {
        enqueue_cleanup_locked(&mut r, session, mid);
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "session expired"));
    };
    p.operation = false;
    p.subscriptions.insert(mid, i.track_id);
    p.pending_offer = pending;
    Ok(Json(result))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Negotiate {
    session_description: Sdp,
}
async fn negotiate(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<Negotiate>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    if !matches!(i.session_description.ty, SdpType::Answer)
        || !valid_text(&i.session_description.sdp, 200_000)
    {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid request"));
    }
    let token = bearer(&headers)?;
    let (id, session) = {
        let mut r = s.registry.lock().await;
        let id = authenticate(&r, token)?;
        let p = r.participants.get_mut(&id).unwrap();
        begin_operation(p)?;
        if !p.pending_offer {
            p.operation = false;
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "no negotiation pending",
            ));
        }
        (id, p.session.clone())
    };
    let result = s
        .provider
        .negotiate(
            &s.config,
            &session,
            json!({"sessionDescription":{"type":"answer","sdp":i.session_description.sdp}}),
        )
        .await;
    if !matches!(result.as_ref(), Ok(v) if validate_provider_envelope(v).is_ok()) {
        remove_participant(&s, id).await;
        return Err(result.err().unwrap_or(ProviderError::Rejected).into());
    }
    let result = result.unwrap();
    let mut r = s.registry.lock().await;
    let p = r
        .participants
        .get_mut(&id)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "session expired"))?;
    p.operation = false;
    p.pending_offer = false;
    Ok(Json(result))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Close {
    mid: String,
}
async fn close(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<Close>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    if !valid_text(&i.mid, 64) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid request"));
    }
    let token = bearer(&headers)?;
    let (id, session, source) = {
        let mut r = s.registry.lock().await;
        let id = authenticate(&r, token)?;
        let p = r.participants.get(&id).unwrap();
        if !p.tracks.contains_key(&i.mid) && !p.subscriptions.contains_key(&i.mid) {
            return Err(ApiError::new(StatusCode::NOT_FOUND, "track not found"));
        }
        let source = p.tracks.get(&i.mid).map(|t| t.id);
        let p = r.participants.get_mut(&id).unwrap();
        begin_operation(p)?;
        (id, p.session.clone(), source)
    };
    let result = s.provider.close(&s.config, &session, &i.mid).await;
    if !matches!(result.as_ref(), Ok(v) if validate_provider_envelope(v).is_ok()) {
        enqueue_cleanup(&s, session, i.mid).await;
        remove_participant(&s, id).await;
        return Err(result.err().unwrap_or(ProviderError::Rejected).into());
    }
    let result = result.unwrap();
    let mut r = s.registry.lock().await;
    let Some(p) = r.participants.get_mut(&id) else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "track not found"));
    };
    p.operation = false;
    p.tracks.remove(&i.mid);
    p.subscriptions.remove(&i.mid);
    if source.is_some() && p.monitor.is_none() {
        s.events.send_replace(());
    }
    if let Some(source) = source {
        drop(r);
        close_dependents(&s, source).await;
    }
    Ok(Json(result))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateBody {
    muted: bool,
    deafened: bool,
}
async fn update_state(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<StateBody>,
) -> Result<StatusCode, ApiError> {
    ensure_enabled(&s)?;
    let token = bearer(&headers)?;
    let mut r = s.registry.lock().await;
    let id = authenticate(&r, token)?;
    let p = r.participants.get_mut(&id).unwrap();
    let changed = p.muted != i.muted || p.deafened != i.deafened;
    p.muted = i.muted;
    p.deafened = i.deafened;
    if changed && p.monitor.is_none() {
        s.events.send_replace(());
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn leave(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(_): Json<Empty>,
) -> Result<StatusCode, ApiError> {
    ensure_enabled(&s)?;
    let token = bearer(&headers)?;
    let id = {
        let r = s.registry.lock().await;
        authenticate(&r, token)?
    };
    remove_participant(&s, id).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn close_dependents(s: &AppState, source: Uuid) {
    let jobs = {
        let mut r = s.registry.lock().await;
        r.participants
            .values_mut()
            .flat_map(|p| {
                let mids: Vec<_> = p
                    .subscriptions
                    .iter()
                    .filter(|(_, v)| **v == source)
                    .map(|(m, _)| m.clone())
                    .collect();
                for m in &mids {
                    p.subscriptions.remove(m);
                }
                mids.into_iter()
                    .map(|m| (p.session.clone(), m))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    };
    run_cleanup_jobs(s, jobs).await;
}
async fn remove_participant(s: &AppState, id: Uuid) {
    let removed = {
        let mut r = s.registry.lock().await;
        let public_removed = r.participants.get(&id).is_some_and(|p| p.monitor.is_none());
        let mut ids = vec![id];
        if r.participants.get(&id).is_some_and(|p| p.monitor.is_none()) {
            ids.extend(r.participants.values().filter_map(|p| {
                p.monitor
                    .filter(|monitor| monitor.parent == id)
                    .map(|_| p.id)
            }));
        }
        let removed = ids
            .into_iter()
            .filter_map(|id| {
                let p = r.participants.remove(&id)?;
                r.tokens.remove(&p.token);
                Some(p)
            })
            .collect::<Vec<_>>();
        if public_removed && !removed.is_empty() {
            s.events.send_replace(());
        }
        removed
    };
    for p in removed {
        revoke_participant_turn(s, &p).await;
        let sources: Vec<_> = p.tracks.values().map(|t| t.id).collect();
        let jobs = p
            .tracks
            .keys()
            .chain(p.subscriptions.keys())
            .map(|mid| (p.session.clone(), mid.clone()))
            .collect();
        run_cleanup_jobs(s, jobs).await;
        for source in sources {
            close_dependents(s, source).await;
        }
    }
}
async fn revoke_participant_turn(s: &AppState, p: &Participant) {
    for username in &p.turn_usernames {
        if s.provider.revoke_turn(&s.config, username).await.is_err() {
            tracing::warn!("TURN revocation failed; credential remains bounded by expiry");
        }
    }
}
fn enqueue_cleanup_locked(r: &mut Registry, session: String, mid: String) {
    if r.cleanup.len() >= MAX_CLEANUP_BACKLOG {
        tracing::error!("cleanup backlog full; dropping oldest job");
        r.cleanup.pop_front();
    }
    if !r
        .cleanup
        .iter()
        .any(|j| j.session == session && j.mid == mid)
    {
        r.cleanup.push_back(CleanupJob {
            session,
            mid,
            attempts: 0,
        });
    }
}
async fn enqueue_cleanup(s: &AppState, session: String, mid: String) {
    let mut registry = s.registry.lock().await;
    enqueue_cleanup_locked(&mut registry, session, mid);
}
async fn run_cleanup_jobs(s: &AppState, jobs: Vec<(String, String)>) {
    let mut set = tokio::task::JoinSet::new();
    for (session, mid) in jobs {
        let state = s.clone();
        set.spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(12),
                state.provider.close(&state.config, &session, &mid),
            )
            .await;
            if !matches!(result, Ok(Ok(ref value)) if validate_provider_envelope(value).is_ok()) {
                enqueue_cleanup(&state, session, mid).await;
            }
        });
    }
    while set.join_next().await.is_some() {}
}
async fn retry_backlog(s: &AppState) {
    let jobs = {
        let mut r = s.registry.lock().await;
        r.cleanup.drain(..).collect::<Vec<_>>()
    };
    for mut job in jobs {
        let result = tokio::time::timeout(
            Duration::from_secs(12),
            s.provider.close(&s.config, &job.session, &job.mid),
        )
        .await;
        if !matches!(result, Ok(Ok(ref value)) if validate_provider_envelope(value).is_ok()) {
            job.attempts = job.attempts.saturating_add(1);
            tracing::warn!(attempts=job.attempts, session=%job.session, mid=%job.mid, "provider cleanup retry failed");
            let mut r = s.registry.lock().await;
            if r.cleanup.len() < MAX_CLEANUP_BACKLOG {
                r.cleanup.push_back(job);
            }
        }
    }
}
pub fn spawn_cleanup(s: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        loop {
            tick.tick().await;
            let expired = {
                let mut r = s.registry.lock().await;
                let mut expired = r
                    .participants
                    .values()
                    .filter(|p| {
                        p.lease.elapsed() >= LEASE || p.joined.elapsed() >= MAX_CALL_DURATION
                    })
                    .map(|p| p.id)
                    .collect::<HashSet<_>>();
                let expired_parents = expired.clone();
                expired.extend(r.participants.values().filter_map(|p| {
                    p.monitor
                        .filter(|monitor| expired_parents.contains(&monitor.parent))
                        .map(|_| p.id)
                }));
                let public_removed = expired
                    .iter()
                    .any(|id| r.participants.get(id).is_some_and(|p| p.monitor.is_none()));
                // Remove under the same lock used for the expiry decision, so a concurrent
                // heartbeat cannot refresh a participant between checking and removal.
                let removed = expired
                    .into_iter()
                    .filter_map(|id| {
                        let p = r.participants.remove(&id)?;
                        r.tokens.remove(&p.token);
                        Some(p)
                    })
                    .collect::<Vec<_>>();
                if public_removed && !removed.is_empty() {
                    s.events.send_replace(());
                }
                removed
            };
            for p in expired {
                revoke_participant_turn(&s, &p).await;
                let sources = p.tracks.values().map(|t| t.id).collect::<Vec<_>>();
                run_cleanup_jobs(
                    &s,
                    p.tracks
                        .keys()
                        .chain(p.subscriptions.keys())
                        .map(|mid| (p.session.clone(), mid.clone()))
                        .collect(),
                )
                .await;
                for source in sources {
                    close_dependents(&s, source).await;
                }
            }
            retry_backlog(&s).await;
        }
    });
}

/// Close every registered provider track before process termination.
pub async fn shutdown_cleanup(s: &AppState) {
    let ids = {
        s.registry
            .lock()
            .await
            .participants
            .keys()
            .copied()
            .collect::<Vec<_>>()
    };
    let _ = tokio::time::timeout(Duration::from_secs(20), async {
        for id in ids {
            remove_participant(s, id).await;
        }
        retry_backlog(s).await;
    })
    .await;
}

#[cfg(test)]
mod tests;
