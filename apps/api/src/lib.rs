use async_trait::async_trait;
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response, Sse, sse::Event},
    routing::{get, post},
};
use futures_util::{StreamExt, stream};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    collections::{HashMap, VecDeque},
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::sync::{Mutex, Notify, watch};
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};
use uuid::Uuid;

const LEASE: Duration = Duration::from_secs(45);
// Cloudflare disconnects an unused session after 10-15 s (live-measured
// September 25, 2026), so a prepared session is only offered for this long.
const PREPARED_TTL: Duration = Duration::from_secs(8);
// Revoke an unused prepared TURN credential once it can no longer be taken.
const PREPARED_REVOKE_AFTER: Duration = Duration::from_secs(12);
// A warm session pair is created and connected by a signed-in member's browser
// before any Join, and kept connected while the app is open: Cloudflare does not
// expire a connected session without media. A join may adopt it for this long;
// the browser replaces it sooner (after 20 minutes).
const WARM_ADOPT_WINDOW: Duration = Duration::from_secs(25 * 60);
// Its TURN credentials are revoked then unless a join adopted them.
const WARM_REVOKE_AFTER: Duration = Duration::from_secs(30 * 60);
// Per account, per room: bounds provider calls from repeated warm requests.
const WARM_MIN_INTERVAL: Duration = Duration::from_secs(10);
// Cloudflare's maximum credential lifetime, not an application call-age limit.
// Revoke on leave/lease expiry. The browser renews halfway through this lifetime
// using ICE restart; setConfiguration alone does not renew allocations.
const TURN_TTL: u64 = 48 * 60 * 60;
const MAX_PARTICIPANTS: usize = 12;
const MAX_TRACKS: usize = 1;
const MAX_SUBSCRIPTIONS: usize = MAX_PARTICIPANTS - 1;
const JOIN_LIMIT_PER_MINUTE: usize = 30;
const OP_LIMIT_PER_MINUTE: usize = 120;
const MAX_CLEANUP_BACKLOG: usize = 512;
const CLEANUP_RECONCILE_INTERVAL: Duration = Duration::from_secs(5);
const BODY_LIMIT: usize = 256 * 1024;
const RESERVATION: Duration = Duration::from_secs(30);

pub mod accounts;
mod auth;
mod channel_media;
mod chat;
mod db;
mod email;
mod environment;
pub mod gateway;
mod media_store;
mod notifications;
mod presence;
mod spaces;
use media_store::Timestamp;

pub use db::{connect_database, connect_runtime_database, migrate_database};
pub use environment::RuntimeEnvironment;

#[derive(Clone)]
pub struct Config {
    pub enabled: bool,
    pub bind: SocketAddr,
    space_limits: spaces::Limits,
    app_id: Option<String>,
    app_secret: Option<String>,
    turn_key_id: Option<String>,
    turn_token: Option<String>,
    provider_base: String,
    #[cfg(test)]
    auth_fixture: bool,
}

impl Config {
    pub fn from_env(environment: &RuntimeEnvironment) -> Result<Self, String> {
        let enabled = environment
            .get("MEDIA_ENABLED")
            .is_some_and(|v| v == "true" || v == "1");
        let get = |key| environment.get(key).filter(|v| !v.trim().is_empty());
        let config = Self {
            enabled,
            space_limits: spaces::Limits::from_env(environment)?,
            bind: environment
                .get("MEDIA_BIND")
                .unwrap_or_else(|| "0.0.0.0:3001".into())
                .parse()
                .map_err(|_| "MEDIA_BIND must be a socket address")?,
            app_id: get("CF_SFU_APP_ID"),
            app_secret: get("CF_SFU_APP_SECRET"),
            turn_key_id: get("CF_TURN_KEY_ID"),
            turn_token: get("CF_TURN_API_TOKEN"),
            provider_base: "https://rtc.live.cloudflare.com/v1".into(),
            #[cfg(test)]
            auth_fixture: false,
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
            space_limits: spaces::Limits::default(),
            app_id: Some("app".into()),
            app_secret: Some("secret".into()),
            turn_key_id: Some("turn".into()),
            turn_token: Some("token".into()),
            provider_base: "mock".into(),
            auth_fixture: true,
        }
    }
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("provider unavailable")]
    Unavailable,
    #[error("provider rejected request")]
    Rejected,
    #[error("provider request failed")]
    Request(Box<ProviderFailure>),
}

// Bounded diagnostic fields only: never reqwest errors (which contain URLs),
// SDP, response descriptions/bodies, or provider credentials.
#[derive(Debug)]
pub struct ProviderFailure {
    id: Uuid,
    operation: &'static str,
    kind: &'static str,
    status: Option<u16>,
    ray: Option<String>,
    code: Option<String>,
    elapsed_ms: Option<u128>,
}
impl ProviderError {
    fn invalid_response(operation: &'static str) -> Self {
        Self::Request(Box::new(ProviderFailure {
            id: Uuid::new_v4(),
            operation,
            kind: "invalid_response",
            status: None,
            ray: None,
            code: None,
            elapsed_ms: None,
        }))
    }

    fn transient(&self) -> bool {
        match self {
            Self::Unavailable => true,
            Self::Rejected => false,
            Self::Request(f) => {
                matches!(f.kind, "timeout" | "transport")
                    || f.status
                        .is_some_and(|s| matches!(s, 408 | 429 | 500 | 502 | 503 | 504))
            }
        }
    }
}
fn diagnostic_token(value: &str) -> Option<String> {
    (!value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b)))
    .then(|| value.to_owned())
}
fn provider_code(value: &Value) -> Option<String> {
    let code = value
        .get("errorCode")
        .filter(|v| !v.is_null())
        .or_else(|| {
            value
                .get("tracks")?
                .as_array()?
                .iter()
                .find_map(|t| t.get("errorCode").filter(|v| !v.is_null()))
        })
        .or_else(|| value.pointer("/errors/0/code"))?;
    if let Some(code) = code.as_str() {
        diagnostic_token(code)
    } else if code.is_number() {
        diagnostic_token(&code.to_string())
    } else {
        None
    }
}

#[async_trait]
pub trait Provider: Send + Sync {
    async fn create_session(&self, config: &Config) -> Result<String, ProviderError>;
    /// Creates a session from the browser's offer and returns its answer, so a
    /// session can be connected before it has any track (a warm session).
    async fn connect_session(
        &self,
        _config: &Config,
        _offer: &str,
    ) -> Result<(String, Value), ProviderError> {
        Err(ProviderError::Rejected)
    }
    async fn turn(&self, config: &Config) -> Result<Vec<IceServer>, ProviderError>;
    async fn revoke_turn(&self, config: &Config, username: &str) -> Result<(), ProviderError>;
    async fn session_tracks(&self, config: &Config, session: &str) -> Result<Value, ProviderError>;
    async fn tracks_new(
        &self,
        config: &Config,
        session: &str,
        body: Value,
    ) -> Result<Value, ProviderError>;
    /// Remote pulls whose per-track results the caller classifies, however
    /// many there are: per-track refusals are partial success, not errors.
    async fn pull_batch(
        &self,
        config: &Config,
        session: &str,
        body: Value,
    ) -> Result<Value, ProviderError> {
        self.tracks_new(config, session, body).await
    }
    async fn restart_ice(
        &self,
        config: &Config,
        session: &str,
        body: Value,
    ) -> Result<Value, ProviderError> {
        self.tracks_new(config, session, body).await
    }
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
                "create_session",
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
    async fn connect_session(
        &self,
        c: &Config,
        offer: &str,
    ) -> Result<(String, Value), ProviderError> {
        // The only form Cloudflare accepts for a session without tracks: tracks/new
        // requires at least one track (live-checked September 25, 2026).
        let value = self
            .request(
                "connect_session",
                reqwest::Method::POST,
                c,
                &format!("apps/{}/sessions/new", required(&c.app_id)),
                json!({"sessionDescription":{"type":"offer","sdp":offer}}),
            )
            .await?;
        let session = value
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(ProviderError::Rejected)?;
        Ok((session, value["sessionDescription"].clone()))
    }
    async fn turn(&self, c: &Config) -> Result<Vec<IceServer>, ProviderError> {
        let value = self
            .execute(
                "turn_issue",
                self.client
                    .post(format!(
                        "{}/turn/keys/{}/credentials/generate-ice-servers",
                        c.provider_base,
                        required(&c.turn_key_id)
                    ))
                    .bearer_auth(required(&c.turn_token))
                    .json(&json!({"ttl": TURN_TTL})),
            )
            .await?;
        #[derive(Deserialize)]
        struct Turn {
            #[serde(rename = "iceServers")]
            ice_servers: Vec<IceServer>,
        }
        let mut servers = serde_json::from_value::<Turn>(value)
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
        self.execute(
            "turn_revoke",
            self.client.post(url).bearer_auth(required(&c.turn_token)),
        )
        .await?;
        Ok(())
    }
    async fn session_tracks(&self, c: &Config, session: &str) -> Result<Value, ProviderError> {
        self.request(
            "session_tracks",
            reqwest::Method::GET,
            c,
            &format!("apps/{}/sessions/{session}", required(&c.app_id)),
            json!({}),
        )
        .await
    }
    async fn tracks_new(&self, c: &Config, s: &str, body: Value) -> Result<Value, ProviderError> {
        self.request(
            match (
                body["tracks"][0]["location"].as_str(),
                body["tracks"].as_array().map(Vec::len),
            ) {
                (Some("remote"), Some(1)) => "subscribe",
                (Some("remote"), _) => "subscribe_batch",
                _ => "publish",
            },
            reqwest::Method::POST,
            c,
            &format!("apps/{}/sessions/{s}/tracks/new", required(&c.app_id)),
            body,
        )
        .await
    }
    async fn pull_batch(&self, c: &Config, s: &str, body: Value) -> Result<Value, ProviderError> {
        self.request(
            "subscribe_batch",
            reqwest::Method::POST,
            c,
            &format!("apps/{}/sessions/{s}/tracks/new", required(&c.app_id)),
            body,
        )
        .await
    }
    async fn restart_ice(&self, c: &Config, s: &str, body: Value) -> Result<Value, ProviderError> {
        self.request(
            "restart_ice",
            reqwest::Method::POST,
            c,
            &format!("apps/{}/sessions/{s}/tracks/new", required(&c.app_id)),
            body,
        )
        .await
    }
    async fn negotiate(&self, c: &Config, s: &str, body: Value) -> Result<Value, ProviderError> {
        self.request(
            "negotiate",
            reqwest::Method::PUT,
            c,
            &format!("apps/{}/sessions/{s}/renegotiate", required(&c.app_id)),
            body,
        )
        .await
    }
    async fn close(&self, c: &Config, s: &str, mid: &str) -> Result<Value, ProviderError> {
        self.request(
            "close",
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
        operation: &'static str,
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
        // Retries belong to the bounded cleanup queue, never to this transport:
        // creates and SDP mutations can succeed despite an ambiguous response.
        self.execute(operation, request).await
    }

    async fn execute(
        &self,
        operation: &'static str,
        request: reqwest::RequestBuilder,
    ) -> Result<Value, ProviderError> {
        let started = Instant::now();
        let id = Uuid::new_v4();
        let failure = |kind, status, ray: Option<String>, code: Option<String>| {
            let elapsed_ms = started.elapsed().as_millis();
            tracing::warn!(%id, operation, kind, status, ray, code, elapsed_ms,
                "Cloudflare operation failed");
            ProviderError::Request(Box::new(ProviderFailure {
                id,
                operation,
                kind,
                status,
                ray,
                code,
                elapsed_ms: Some(elapsed_ms),
            }))
        };
        let response = request.send().await.map_err(|error| {
            failure(
                if error.is_timeout() {
                    "timeout"
                } else {
                    "transport"
                },
                None,
                None,
                None,
            )
        })?;
        let status = response.status();
        let ray = response
            .headers()
            .get("cf-ray")
            .and_then(|v| v.to_str().ok())
            .and_then(diagnostic_token);
        let mut response = response;
        let read_body = async {
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if bytes.len() + chunk.len() > BODY_LIMIT {
                    return Ok(None);
                }
                bytes.extend_from_slice(&chunk);
            }
            Ok::<_, reqwest::Error>(Some(bytes))
        };
        // Once HTTP failure is known, diagnostic collection must not turn an
        // immediate error into a ten-second wait for a broken upstream body.
        let bytes = if status.is_success() {
            read_body.await
        } else {
            tokio::time::timeout(Duration::from_millis(250), read_body)
                .await
                .map_err(|_| {
                    failure(
                        "http_body_timeout",
                        Some(status.as_u16()),
                        ray.clone(),
                        None,
                    )
                })?
        };
        let bytes = bytes
            .map_err(|error| {
                failure(
                    if error.is_timeout() {
                        "timeout"
                    } else {
                        "response_body"
                    },
                    Some(status.as_u16()),
                    ray.clone(),
                    None,
                )
            })?
            .ok_or_else(|| {
                failure(
                    "response_too_large",
                    Some(status.as_u16()),
                    ray.clone(),
                    None,
                )
            })?;
        // Revocation may legitimately return 204/empty; if a body is present,
        // still preserve provider error envelopes rather than assuming success.
        let value = if operation == "turn_revoke" && bytes.is_empty() {
            Ok(json!({}))
        } else {
            serde_json::from_slice::<Value>(&bytes)
        };
        if !status.is_success() {
            return Err(failure(
                "http",
                Some(status.as_u16()),
                ray,
                value.as_ref().ok().and_then(provider_code),
            ));
        }
        let value =
            value.map_err(|_| failure("invalid_json", Some(status.as_u16()), ray.clone(), None))?;
        if operation == "subscribe" && rejected_remote_track(&value) {
            return Err(failure(
                "track_unavailable",
                Some(status.as_u16()),
                ray,
                provider_code(&value),
            ));
        }
        let envelope = if operation == "subscribe_batch" {
            // Per-track errors are partial success here, classified by the handler.
            pull_batch_envelope(&value)
        } else {
            validate_provider_envelope(&value).is_ok()
        };
        if !envelope {
            return Err(failure(
                "provider_error",
                Some(status.as_u16()),
                ray,
                provider_code(&value),
            ));
        }
        let shape_valid =
            match operation {
                "create_session" => value
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .is_some_and(|v| !v.is_empty()),
                "connect_session" => {
                    value
                        .get("sessionId")
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty())
                        && value["sessionDescription"]["type"] == "answer"
                        && value["sessionDescription"]["sdp"].is_string()
                }
                "turn_issue" => {
                    status == StatusCode::CREATED
                        && value.get("iceServers").is_some_and(|v| {
                            serde_json::from_value::<Vec<IceServer>>(v.clone()).is_ok()
                        })
                }
                "session_tracks" => value.get("tracks").and_then(Value::as_array).is_some(),
                "publish" | "subscribe" => value
                    .get("tracks")
                    .and_then(Value::as_array)
                    .is_some_and(|v| {
                        !v.is_empty()
                            && v.iter()
                                .all(|t| t.get("mid").and_then(Value::as_str).is_some())
                    }),
                _ => true,
            };
        if !shape_valid {
            return Err(failure(
                "invalid_response",
                Some(status.as_u16()),
                ray,
                provider_code(&value),
            ));
        }
        // Info level: join latency is a chain of these calls, so production
        // needs every successful elapsed time, not only failures.
        tracing::info!(%id, operation, status = status.as_u16(), elapsed_ms = started.elapsed().as_millis(), "Cloudflare operation succeeded");
        Ok(value)
    }
}
// A single rejected pull without a MID or SDP did not change the listener's
// negotiation. Never infer this from an HTTP error, timeout, or partial offer.
fn rejected_remote_track(value: &Value) -> bool {
    value.get("errorCode").is_none_or(Value::is_null)
        && value.get("success").and_then(Value::as_bool) != Some(false)
        && value
            .get("errors")
            .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty))
        && value.get("sessionDescription").is_none_or(Value::is_null)
        && value
            .get("requiresImmediateRenegotiation")
            .and_then(Value::as_bool)
            == Some(false)
        && value
            .get("tracks")
            .and_then(Value::as_array)
            .is_some_and(|tracks| {
                tracks.len() == 1
                    && tracks[0]
                        .get("mid")
                        .is_none_or(|v| v.is_null() || v.as_str() == Some(""))
                    && matches!(
                        tracks[0].get("errorCode").and_then(Value::as_str),
                        Some("not_found_track_error" | "empty_track_error" | "track_error")
                    )
            })
}
fn validate_provider_envelope(value: &Value) -> Result<(), ProviderError> {
    if !value.is_object()
        || value.get("success").and_then(Value::as_bool) == Some(false)
        || value
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty())
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
    channel_registries: Arc<Mutex<HashMap<String, Registry>>>,
    media_channel: Option<String>,
    media_session: Option<Vec<u8>>,
    store: Option<Arc<media_store::ValkeyStore>>,
    database: Option<PgPool>,
    chat: Option<chat::Chat>,
    events: watch::Sender<()>,
    room_events: Arc<std::sync::Mutex<HashMap<Option<String>, watch::Sender<()>>>>,
    room_interest: Arc<Notify>,
    shutting_down: watch::Sender<bool>,
    cleanup_wakeup: Arc<Notify>,
    cleanup_lock: Arc<Mutex<()>>,
    expiry_lock: Arc<Mutex<()>>,
    auth: auth::AuthVerifier,
    debug_users: accounts::DebugUsers,
    notifications_webhook: notifications::NotificationsWebhook,
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
        let (shutting_down, _) = watch::channel(false);
        let auth = auth::AuthVerifier::new();
        #[cfg(test)]
        let auth = if config.auth_fixture {
            auth::AuthVerifier::test_bypass()
        } else {
            auth
        };
        Self {
            config,
            provider,
            registry: Arc::new(Mutex::new(Registry::default())),
            channel_registries: Arc::new(Mutex::new(HashMap::new())),
            media_channel: None,
            media_session: None,
            store: None,
            database,
            chat: None,
            events,
            room_events: Arc::new(std::sync::Mutex::new(HashMap::new())),
            room_interest: Arc::new(Notify::new()),
            shutting_down,
            cleanup_wakeup: Arc::new(Notify::new()),
            cleanup_lock: Arc::new(Mutex::new(())),
            expiry_lock: Arc::new(Mutex::new(())),
            auth,
            debug_users: accounts::DebugUsers::default(),
            notifications_webhook: notifications::NotificationsWebhook::from_env(
                &RuntimeEnvironment::default(),
            ),
        }
    }

    /// End long-lived event streams so HTTP draining can complete on deployment.
    pub fn begin_shutdown(&self) {
        self.shutting_down.send_replace(true);
    }

    #[must_use]
    pub fn database(&self) -> Option<&PgPool> {
        self.database.as_ref()
    }

    pub async fn enable_chat(&mut self, environment: &RuntimeEnvironment) -> Result<(), String> {
        self.chat = chat::Chat::from_env(self.database.as_ref(), environment).await?;
        if let Some(chat) = self.chat.clone() {
            chat::spawn_publisher(chat);
        }
        Ok(())
    }

    pub async fn enable_accounts_from_env(
        &mut self,
        environment: &RuntimeEnvironment,
    ) -> Result<(), String> {
        self.auth = auth::AuthVerifier::from_env(environment).await?;
        self.debug_users = accounts::DebugUsers::from_env(environment);
        self.notifications_webhook = notifications::NotificationsWebhook::from_env(environment);
        Ok(())
    }
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Registry {
    participants: HashMap<Uuid, Participant>,
    tokens: HashMap<String, Uuid>,
    joins: VecDeque<Timestamp>,
    cleanup: VecDeque<CleanupJob>,
    reservations: HashMap<Uuid, JoinReservation>,
    revision: u64,
    /// Provider sessions and TURN created shortly before a signed-in member joins.
    #[serde(default)]
    prepared: Vec<PreparedJoin>,
    /// Recent warm session issues, only to rate-limit them per account.
    #[serde(default)]
    warmed: Vec<WarmIssue>,
}
#[derive(Clone, Serialize, Deserialize)]
struct WarmIssue {
    account: Vec<u8>,
    issued: Timestamp,
}
#[derive(Clone, Serialize, Deserialize)]
struct PreparedJoin {
    /// The account session hash that may take it, in this room only.
    account: Vec<u8>,
    session: String,
    ice_servers: Vec<IceServer>,
    issued: Timestamp,
    /// Created when others were publishing, for the join's receive-only pulls.
    #[serde(default)]
    receive_session: Option<String>,
}
impl PreparedJoin {
    fn usernames(&self) -> impl Iterator<Item = &String> {
        self.ice_servers
            .iter()
            .filter_map(|server| server.username.as_ref())
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct JoinReservation {
    started: Timestamp,
    monitor: Option<Monitor>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Participant {
    id: Uuid,
    token: String,
    #[serde(default)]
    account_session: Option<Vec<u8>>,
    name: String,
    country_code: Option<String>,
    session: String,
    /// A second, receive-only provider session holding every subscription once
    /// created. Cloudflare answers a pull into a session with no negotiated
    /// PeerConnection at once, but holds one into a negotiated, unconnected
    /// session until it connects (then 425). A joining browser asks for this on
    /// its first pull, so it can subscribe while its microphone transport is
    /// still connecting. Created lazily because an unused provider session is
    /// disconnected within 30 s. Absent for monitors, native and older clients.
    #[serde(default)]
    receive_session: Option<String>,
    turn_usernames: Vec<String>,
    #[serde(default)]
    turn: Option<TurnCache>,
    #[serde(default)]
    turn_retired: Vec<RetiredTurn>,
    #[serde(default)]
    turn_claim: Option<Claim>,
    #[serde(default)]
    turn_attempts: VecDeque<Timestamp>,
    muted: bool,
    deafened: bool,
    #[serde(default)]
    state_sequence: u64,
    lease: Timestamp,
    joined: Timestamp,
    tracks: HashMap<String, Track>,
    subscriptions: HashMap<String, Uuid>,
    pending_offer: bool,
    operation: bool,
    operation_started: Option<Timestamp>,
    #[serde(default)]
    restart: Option<PendingRestart>,
    #[serde(default)]
    restart_acknowledged: u64,
    operations: VecDeque<Timestamp>,
    monitor: Option<Monitor>,
    events: Option<Uuid>,
}
impl Participant {
    /// The provider session that holds this participant's subscriptions.
    fn pull_session(&self) -> &String {
        self.receive_session.as_ref().unwrap_or(&self.session)
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Claim {
    nonce: Uuid,
    started: Timestamp,
}
#[derive(Clone, Serialize, Deserialize)]
struct TurnCache {
    generation: Uuid,
    ice_servers: Vec<IceServer>,
    issued: Timestamp,
    expires: Timestamp,
    revoke_after: Timestamp,
}
#[derive(Clone, Serialize, Deserialize)]
struct RetiredTurn {
    usernames: Vec<String>,
    expires: Timestamp,
}
#[derive(Clone, Serialize, Deserialize)]
struct PendingRestart {
    generation: Uuid,
    sequence: u64,
    hash: String,
    claim: Option<Claim>,
}
#[derive(Clone, Copy, Serialize, Deserialize)]
struct Monitor {
    parent: Uuid,
    role: MonitorRole,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
enum MonitorRole {
    Sender,
    Receiver,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
enum CleanupAction {
    Close { session: String, mid: String },
    Discover { session: String },
    Revoke { username: String },
}
impl CleanupAction {
    fn operation(&self) -> &'static str {
        match self {
            Self::Close { .. } => "close",
            Self::Discover { .. } => "session_tracks",
            Self::Revoke { .. } => "turn_revoke",
        }
    }
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CleanupJob {
    action: CleanupAction,
    attempts: u8,
    not_before: Timestamp,
    claim: Option<Uuid>,
}
#[derive(Clone, Serialize, Deserialize)]
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
    code: Option<&'static str>,
    error_id: Option<Uuid>,
    attempts_remaining: Option<u8>,
}
impl ApiError {
    fn new(status: StatusCode, message: &'static str) -> Self {
        Self {
            status,
            message,
            code: None,
            error_id: None,
            attempts_remaining: None,
        }
    }

    fn with_attempts_remaining(mut self, attempts_remaining: u8) -> Self {
        self.attempts_remaining = Some(attempts_remaining);
        self
    }

    fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = match self.attempts_remaining {
            Some(attempts_remaining) => {
                json!({"error": self.message, "attemptsRemaining": attempts_remaining})
            }
            None => json!({"error": self.message}),
        };
        if let Some(code) = self.code {
            body["code"] = json!(code);
        }
        let mut response = (self.status, Json(body)).into_response();
        if let Some(id) = self.error_id {
            response
                .headers_mut()
                .insert("x-caper-error-id", id.to_string().parse().unwrap());
        }
        response
    }
}
impl From<ProviderError> for ApiError {
    fn from(error: ProviderError) -> Self {
        let id = match error {
            ProviderError::Request(f) => {
                tracing::warn!(id = %f.id, operation = f.operation, kind = f.kind,
                    upstream_status = f.status, ray = f.ray, code = f.code,
                    provider_elapsed_ms = f.elapsed_ms, "media request failed");
                f.id
            }
            error => {
                let id = Uuid::new_v4();
                tracing::warn!(%id, kind = %error, "media response validation failed");
                id
            }
        };
        // Preserve the response contract for old browser tabs/native clients.
        Self {
            error_id: Some(id),
            ..Self::new(StatusCode::BAD_GATEWAY, "media provider unavailable")
        }
    }
}

pub fn app(state: AppState) -> Router {
    let protected = Router::new()
        .route("/api/account/me", get(account_me))
        .route("/api/account/profile", post(account_profile))
        .route("/api/auth/logout", post(auth_logout))
        .merge(spaces::routes())
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            account_auth,
        ));
    let account_login = Router::new()
        .route("/api/auth/email/request", post(auth_email_request))
        .route("/api/auth/email/verify", post(auth_email_verify));
    let media = media_routes();
    Router::new()
        .route("/health", get(|| async { StatusCode::NO_CONTENT }))
        .route("/readyz", get(ready))
        .route("/api/health", get(|| async { StatusCode::NO_CONTENT }))
        .merge(account_login)
        .merge(protected)
        .merge(media)
        .merge(channel_media::routes())
        .merge(chat::routes())
        .layer(DefaultBodyLimit::disable())
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &axum::http::Request<axum::body::Body>| {
                    // Only router-owned paths: never log query strings, headers,
                    // arbitrary unmatched URLs, SDP, or request bodies.
                    let route = request
                        .extensions()
                        .get::<axum::extract::MatchedPath>()
                        .map_or("unmatched", axum::extract::MatchedPath::as_str);
                    tracing::info_span!("http_request", http_method = %request.method(),
                        http_route = route, request_id = %Uuid::new_v4())
                })
                .on_response(
                    |response: &Response, latency: Duration, _span: &tracing::Span| {
                        tracing::info!(
                            event_name = "http_response",
                            status = response.status().as_u16(),
                            duration_ms = latency.as_secs_f64() * 1000.0,
                            "HTTP response completed"
                        );
                    },
                ),
        )
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

fn media_routes() -> Router<AppState> {
    Router::new()
        .route("/api/media/status", get(status))
        .route("/api/media/presence", get(presence))
        .route("/api/media/presence/events", get(presence_events))
        .route("/api/media/join", post(join))
        .route("/api/media/prepare", post(prepare))
        .route("/api/media/warm", post(warm))
        .route("/api/media/turn", post(turn))
        .route("/api/media/restart-ice", post(restart_ice))
        .route("/api/media/restart-ice-ack", post(restart_ice_ack))
        .route("/api/media/snapshot", post(snapshot))
        .route("/api/media/events", get(events))
        .route("/api/media/publish", post(publish))
        .route("/api/media/subscribe", post(subscribe))
        .route("/api/media/negotiate", post(negotiate))
        .route("/api/media/close", post(close))
        .route("/api/media/state", post(update_state))
        .route("/api/media/leave", post(leave))
}

async fn account_auth(
    State(state): State<AppState>,
    mut request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<Response, ApiError> {
    let token = account_token(request.headers());
    #[cfg(test)]
    let token = if state.auth.is_test_bypass() {
        token.or(Some("test-fixture"))
    } else {
        token
    };
    let principal = state
        .auth
        .authenticate(
            token.ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"))?,
            state.database.as_ref(),
        )
        .await?;
    request.extensions_mut().insert(principal);
    Ok(next.run(request).await)
}

fn account_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .or_else(|| session_cookie(headers))
}

fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|cookie| cookie.strip_prefix("caper_session="))
        .filter(|token| !token.is_empty())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailRequestInput {
    email: String,
}

async fn auth_email_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<EmailRequestInput>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let ip = headers
        .get("cf-connecting-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED));
    let challenge_id = state
        .auth
        .request_code(state.database.as_ref(), &input.email, ip)
        .await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({"challengeId": challenge_id})),
    ))
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum TokenTransport {
    Cookie,
    Bearer,
}

fn cookie_transport() -> TokenTransport {
    TokenTransport::Cookie
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct EmailVerifyInput {
    challenge_id: Uuid,
    code: String,
    #[serde(default = "cookie_transport")]
    token_transport: TokenTransport,
}

async fn auth_email_verify(
    State(state): State<AppState>,
    Json(input): Json<EmailVerifyInput>,
) -> Result<Response, ApiError> {
    let session = state
        .auth
        .verify_code(state.database.as_ref(), input.challenge_id, &input.code)
        .await?;
    if session.user_created {
        state
            .notifications_webhook
            .notify(notifications::NotificationEvent::user_created(
                &session.user,
            ));
    }
    let mut response = match input.token_transport {
        TokenTransport::Cookie => {
            Json(json!({"account": session.user.own(&state.debug_users)})).into_response()
        }
        TokenTransport::Bearer => {
            Json(json!({"account": session.user.own(&state.debug_users), "token": session.token}))
                .into_response()
        }
    };
    if matches!(input.token_transport, TokenTransport::Cookie) {
        response.headers_mut().insert(
            "set-cookie",
            format!(
                "caper_session={}; Path=/; Max-Age=2592000; HttpOnly; Secure; SameSite=Lax",
                session.token
            )
            .parse()
            .expect("session token makes a valid cookie"),
        );
    }
    Ok(response)
}

async fn auth_logout(
    State(state): State<AppState>,
    Extension(principal): Extension<auth::Principal>,
) -> Result<Response, ApiError> {
    state
        .auth
        .logout(state.database.as_ref(), &principal.token_hash)
        .await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response.headers_mut().insert(
        "set-cookie",
        axum::http::HeaderValue::from_static(
            "caper_session=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax",
        ),
    );
    Ok(response)
}

async fn account_me(
    State(state): State<AppState>,
    Extension(principal): Extension<auth::Principal>,
) -> Json<Value> {
    Json(serde_json::to_value(principal.user.own(&state.debug_users)).expect("account serializes"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ProfileInput {
    username: String,
    display_name: String,
}

async fn account_profile(
    State(state): State<AppState>,
    Extension(principal): Extension<auth::Principal>,
    Json(input): Json<ProfileInput>,
) -> Result<Json<Value>, ApiError> {
    let username = input.username.trim().to_ascii_lowercase();
    let display_name = input.display_name.trim();
    if !(3..=32).contains(&username.len())
        || !username
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        || !(1..=64).contains(&display_name.chars().count())
        || display_name.chars().any(char::is_control)
    {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid profile"));
    }
    let pool = state.database.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "account service unavailable",
        )
    })?;
    match accounts::set_profile(pool, principal.user.id, &username, display_name).await {
        Ok(Some(user)) => Ok(Json(
            serde_json::to_value(user.own(&state.debug_users)).expect("account serializes"),
        )),
        Ok(None) => Err(ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized")),
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            Err(ApiError::new(StatusCode::CONFLICT, "username unavailable"))
        }
        Err(_) => {
            tracing::error!("profile update failed");
            Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "account service unavailable",
            ))
        }
    }
}
async fn status(State(s): State<AppState>) -> Json<Value> {
    Json(json!({"enabled":s.config.enabled}))
}
async fn ready(State(s): State<AppState>) -> Result<StatusCode, ApiError> {
    if *s.shutting_down.borrow() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "API is draining",
        ));
    }
    s.read(|_| Ok(StatusCode::NO_CONTENT)).await
}
fn ensure_enabled(s: &AppState) -> Result<(), ApiError> {
    if *s.shutting_down.borrow() {
        return Err(
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "API is draining")
                .with_code("api_draining"),
        );
    }
    s.config
        .enabled
        .then_some(())
        .ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "media disabled"))
}
fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("x-caper-media-token")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"))
}
fn token_hash(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
fn authenticate(r: &Registry, token: &str) -> Result<Uuid, ApiError> {
    let token = token_hash(token);
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
    if p.lease.elapsed() >= LEASE {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "session expired"));
    }
    if let Some(monitor) = p.monitor {
        let parent = r
            .participants
            .get(&monitor.parent)
            .filter(|parent| parent.monitor.is_none())
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "parent session ended"))?;
        if parent.lease.elapsed() >= LEASE {
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
    token: Option<String>,
    updates: watch::Receiver<()>,
    connection: Uuid,
    shutdown: watch::Receiver<bool>,
    heartbeat: tokio::time::Interval,
    first: bool,
    snapshots: bool,
    initial_snapshot: bool,
    done: bool,
    revision: Option<u64>,
    handoff: bool,
    handoff_deadline: Option<tokio::time::Instant>,
}

#[derive(Deserialize, Default)]
struct EventQuery {
    snapshots: Option<u8>,
    handoff: Option<u8>,
}

async fn events(
    State(s): State<AppState>,
    Query(query): Query<EventQuery>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    ensure_enabled(&s)?;
    event_stream(
        s,
        Some(bearer(&headers)?.to_owned()),
        query.snapshots == Some(1),
        query.handoff == Some(1) && query.snapshots == Some(1),
    )
    .await
}

async fn presence_events(
    State(s): State<AppState>,
    Query(query): Query<EventQuery>,
) -> Result<impl IntoResponse, ApiError> {
    ensure_enabled(&s)?;
    event_stream(s, None, true, query.handoff == Some(1)).await
}

async fn event_stream(
    s: AppState,
    token: Option<String>,
    snapshots: bool,
    handoff: bool,
) -> Result<Response, ApiError> {
    let shutdown = s.shutting_down.subscribe();
    if *shutdown.borrow() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "API is draining",
        ));
    }
    let connection = Uuid::new_v4();
    let updates = {
        // Listen before registration; the initial snapshot reads current shared state.
        let mut updates = s.room_updates();
        if let Some(token) = &token {
            s.update(|r| {
                let id = authenticate(r, token)?;
                let participant = r.participants.get_mut(&id).unwrap();
                if participant.monitor.is_some() {
                    return Err(ApiError::new(
                        StatusCode::FORBIDDEN,
                        "monitor sessions cannot receive public events",
                    ));
                }
                participant.events = Some(connection);
                Ok(())
            })
            .await?;
        } else {
            // Public spectators never create a participant or replace its event connection.
            s.read(|_| Ok(())).await?;
        }
        updates.borrow_and_update();
        updates
    };
    let mut heartbeat = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(10),
        Duration::from_secs(10),
    );
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let stream = stream::unfold(
        EventStreamState {
            state: s,
            token,
            updates,
            connection,
            shutdown,
            heartbeat,
            first: true,
            snapshots,
            initial_snapshot: snapshots,
            done: false,
            revision: None,
            handoff,
            handoff_deadline: None,
        },
        |mut stream| async move {
            loop {
                if stream.done {
                    return None;
                }
                let event = if *stream.shutdown.borrow() && stream.handoff_deadline.is_none() {
                    "draining"
                } else if stream.first {
                    stream.first = false;
                    "ready"
                } else if stream.initial_snapshot {
                    stream.initial_snapshot = false;
                    "snapshot"
                } else {
                    tokio::select! {
                        _ = stream.shutdown.changed(), if stream.handoff_deadline.is_none() => "draining",
                        () = tokio::time::sleep_until(stream.handoff_deadline.unwrap_or_else(tokio::time::Instant::now)), if stream.handoff_deadline.is_some() => return None,
                        changed = stream.updates.changed() => {
                            if changed.is_err() { return None; }
                            "changed"
                        }
                        _ = stream.heartbeat.tick() => "heartbeat",
                    }
                };
                // A heartbeat/update can become ready at the same time as the
                // deadline. Do not let select's branch order extend the overlap.
                if stream
                    .handoff_deadline
                    .is_some_and(|deadline| tokio::time::Instant::now() >= deadline)
                {
                    return None;
                }
                if event == "draining" {
                    if !stream.snapshots || stream.first {
                        return None;
                    }
                    let event = if stream.handoff {
                        // Keep forwarding shared-state updates while the client opens
                        // its replacement. This fits inside the 30s HTTP drain budget.
                        stream.handoff_deadline =
                            Some(tokio::time::Instant::now() + Duration::from_secs(10));
                        "migrating"
                    } else {
                        stream.done = true;
                        "draining"
                    };
                    return Some((
                        Ok::<_, std::convert::Infallible>(Event::default().event(event).data("{}")),
                        stream,
                    ));
                }
                // Heartbeats do not renew the lease. They do bound expiry/revocation detection
                // even when the cleanup sweep is not running.
                stream.state.check_media_access().await.ok()?;
                let snapshot = stream
                    .state
                    .read(|r| {
                        if let Some(token) = &stream.token {
                            let id = authenticate(r, token)?;
                            // During bounded shutdown overlap, replacement registration
                            // must not retire this stream before its snapshot arrives.
                            // The capability is still authenticated on every emission.
                            if stream.handoff_deadline.is_none()
                                && r.participants.get(&id).and_then(|p| p.events)
                                    != Some(stream.connection)
                            {
                                return Err(ApiError::new(
                                    StatusCode::UNAUTHORIZED,
                                    "unauthorized",
                                ));
                            }
                            Ok((r.revision, public_snapshot(r)))
                        } else {
                            Ok((r.revision, presence_snapshot(r)))
                        }
                    })
                    .await
                    .ok()?;
                let (revision, snapshot) = snapshot;
                // A delayed/lost Pub/Sub wake must not let a heartbeat acknowledge
                // a new revision without delivering it. Heartbeats also repair state.
                let event = if event == "heartbeat" && stream.revision != Some(revision) {
                    "changed"
                } else {
                    event
                };
                // Connection replacement also wakes streams. Do not expose it as
                // a roster mutation to unrelated participants.
                if event == "changed" && stream.revision == Some(revision) {
                    continue;
                }
                stream.revision = Some(revision);
                let (name, data) =
                    if event == "snapshot" || (stream.snapshots && event == "changed") {
                        ("snapshot", snapshot)
                    } else {
                        (event, json!({}))
                    };
                return Some((
                    Ok::<_, std::convert::Infallible>(
                        Event::default().event(name).data(data.to_string()),
                    ),
                    stream,
                ));
            }
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
    if p.operation {
        return Err(ApiError::new(StatusCode::CONFLICT, "operation pending"));
    }
    record_operation(p)?;
    p.operation = true;
    p.operation_started = Some(Timestamp::now());
    Ok(())
}

fn record_operation(p: &mut Participant) -> Result<(), ApiError> {
    let now = Timestamp::now();
    while p
        .operations
        .front()
        .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(60))
    {
        p.operations.pop_front();
    }
    if p.operations.len() >= OP_LIMIT_PER_MINUTE {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate limit exceeded",
        ));
    }
    p.operations.push_back(now);
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Join {
    #[serde(default)]
    name: Option<String>,
    monitor: Option<MonitorRole>,
    #[serde(default)]
    muted: bool,
    #[serde(default)]
    deafened: bool,
    /// The browser's first microphone offer. Publishing it inside join saves a
    /// signaling round trip; the same handler state machine runs either way.
    #[serde(default)]
    publish: Option<InitialPublish>,
    /// With `publish`: also pull everyone already publishing, into a separate
    /// receive-only session, so both connections come up together.
    #[serde(default)]
    receive: bool,
    /// A ticket from `warm`: publish into and pull into its already-connected
    /// sessions instead of creating new ones.
    #[serde(default)]
    warm: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct InitialPublish {
    mid: String,
    session_description: Sdp,
}

fn country_code(headers: &HeaderMap) -> Option<String> {
    let code = headers.get("cf-ipcountry")?.to_str().ok()?;
    (code.len() == 2 && code != "XX" && code.bytes().all(|byte| byte.is_ascii_uppercase()))
        .then(|| code.to_owned())
}

async fn join(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Join>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    let submitted_name = input.name.as_deref().unwrap_or_default().trim();
    let account_name = if let Some(token) = account_token(&headers) {
        s.auth
            .authenticate(token, s.database.as_ref())
            .await
            .ok()
            .and_then(|principal| principal.user.display_name)
    } else {
        None
    };
    let name = account_name.as_deref().unwrap_or(submitted_name).trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid name"));
    }
    if input
        .publish
        .as_ref()
        .is_some_and(|p| !valid_offer(&p.mid, &p.session_description))
    {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid request"));
    }
    // Verified before anything is reserved. Expired, foreign or malformed: the
    // browser discards it and joins the ordinary way.
    let warm = match &input.warm {
        None => None,
        Some(ticket) => {
            let verified = match (&s.media_session, &input.monitor, &input.publish) {
                (Some(account), None, Some(_)) => verify_warm(&s.config, account, ticket),
                _ => None,
            };
            Some(
                verified
                    .filter(|t| t.issued.elapsed() < WARM_ADOPT_WINDOW)
                    .ok_or_else(|| {
                        ApiError::new(StatusCode::CONFLICT, "warm session unavailable")
                            .with_code("warm_unavailable")
                    })?,
            )
        }
    };
    let country_code = country_code(&headers);
    let reservation = Uuid::new_v4();
    let (monitor, pulls) = s
        .update(|r| {
            let now = Timestamp::now();
            r.reservations
                .retain(|_, reservation| reservation.started.elapsed() < Duration::from_secs(30));
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
            if r.participants.len() + r.reservations.len() >= MAX_PARTICIPANTS {
                return Err(ApiError::new(StatusCode::CONFLICT, "lobby full"));
            }
            let monitor = if let Some(role) = input.monitor {
                let parent = authenticate(r, bearer(&headers)?)?;
                if r.participants.get(&parent).unwrap().monitor.is_some() {
                    return Err(ApiError::new(
                        StatusCode::FORBIDDEN,
                        "monitor sessions cannot create monitors",
                    ));
                }
                if r.reservations.values().any(|reservation| {
                    reservation
                        .monitor
                        .is_some_and(|m| m.parent == parent && m.role == role)
                }) || r.participants.values().any(|p| {
                    p.monitor
                        .is_some_and(|m| m.parent == parent && m.role == role)
                }) {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        "monitor role already active",
                    ));
                }
                Some(Monitor { parent, role })
            } else {
                None
            };
            // A ticket adopts its sessions once.
            if let Some(warm) = &warm
                && r.participants.values().any(|p| {
                    p.session == warm.session || p.receive_session.as_ref() == Some(&warm.receive)
                })
            {
                return Err(
                    ApiError::new(StatusCode::CONFLICT, "warm session unavailable")
                        .with_code("warm_unavailable"),
                );
            }
            r.joins.push_back(now);
            r.reservations.insert(
                reservation,
                JoinReservation {
                    started: now,
                    monitor,
                },
            );
            let pulls = if input.receive && input.publish.is_some() && monitor.is_none() {
                public_sources_locked(r)
            } else {
                vec![]
            };
            Ok((monitor, pulls))
        })
        .await?;
    let wants_receive = !pulls.is_empty();
    if let Some(warm) = &warm {
        // Adopted: its credentials now live and die with this participant.
        let mut origin = s.clone();
        origin.media_channel = warm.channel.clone();
        let usernames: Vec<_> = warm
            .ice_servers
            .iter()
            .filter_map(|server| server.username.as_ref())
            .collect();
        let cancelled = origin
            .update(|r| {
                r.cleanup.retain(|job| {
                    !matches!(&job.action, CleanupAction::Revoke { username } if usernames.contains(&username))
                });
                Ok(())
            })
            .await;
        if let Err(error) = cancelled {
            release_join_reservation(&s, reservation).await;
            return Err(error);
        }
    }
    let prepared = match (&warm, &s.media_session, monitor) {
        (Some(warm), _, _) => Some(PreparedJoin {
            account: vec![],
            session: warm.session.clone(),
            ice_servers: warm.ice_servers.clone(),
            issued: warm.issued,
            receive_session: Some(warm.receive.clone()),
        }),
        (None, Some(account), None) => s.update(|r| Ok(take_prepared_locked(r, account))).await?,
        _ => None,
    };
    // These independent provider requests run together. Session creation is intentionally
    // never retried: an ambiguous create could orphan a session.
    let receiving = async {
        if wants_receive {
            Some(s.provider.create_session(&s.config).await)
        } else {
            None
        }
    };
    let provisioning = async {
        if let Some(prepared) = prepared {
            let receive = match prepared.receive_session {
                // A warm receive session is connected: keep it even with nobody to pull yet.
                Some(session) if wants_receive || warm.is_some() => Some(Ok(session)),
                _ => receiving.await,
            };
            (
                prepared.issued,
                Ok(prepared.session),
                Ok(prepared.ice_servers),
                receive,
            )
        } else {
            let issued = Timestamp::now();
            let (session, ice, receive) = tokio::join!(
                s.provider.create_session(&s.config),
                s.provider.turn(&s.config),
                receiving
            );
            (issued, session, ice, receive)
        }
    };
    // The access re-check is independent of provisioning; its result is still
    // applied before the participant is committed or any capability returned.
    let ((turn_issued, session, ice, receive_session), access) =
        tokio::join!(provisioning, s.check_media_access());
    // Without a receive session, pulls simply happen after connecting, as
    // before. An unused provider session holds nothing and needs no cleanup.
    let receive_session = receive_session.and_then(Result::ok);
    let session = match session {
        Ok(session) => session,
        Err(error) => {
            release_join_reservation(&s, reservation).await;
            if let Ok(servers) = ice {
                for username in servers
                    .iter()
                    .filter_map(|server| server.username.as_deref())
                {
                    enqueue_action(
                        &s,
                        CleanupAction::Revoke {
                            username: username.into(),
                        },
                    )
                    .await;
                }
            }
            return Err(error.into());
        }
    };
    let ice = match ice {
        Ok(ice) => ice,
        Err(error) => {
            release_join_reservation(&s, reservation).await;
            return Err(error.into());
        }
    };
    let id = Uuid::new_v4();
    let generation = Uuid::new_v4();
    let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let p = Participant {
        id,
        token: token_hash(&token),
        account_session: s.media_session.clone(),
        name: name.into(),
        country_code,
        session,
        // Later pulls go to a warm receive session from the start.
        receive_session: warm.as_ref().map(|warm| warm.receive.clone()),
        turn_usernames: ice
            .iter()
            .filter_map(|server| server.username.clone())
            .collect(),
        turn: Some(TurnCache {
            generation,
            ice_servers: ice.clone(),
            issued: turn_issued,
            expires: turn_issued + Duration::from_secs(TURN_TTL),
            revoke_after: Timestamp::now() + Duration::from_secs(TURN_TTL),
        }),
        turn_retired: vec![],
        turn_claim: None,
        turn_attempts: VecDeque::new(),
        muted: input.muted,
        deafened: input.deafened,
        state_sequence: 0,
        lease: Timestamp::now(),
        joined: Timestamp::now(),
        tracks: HashMap::new(),
        subscriptions: HashMap::new(),
        pending_offer: false,
        operation: false,
        operation_started: None,
        restart: None,
        restart_acknowledged: 0,
        operations: VecDeque::new(),
        monitor,
        events: None,
    };
    if let Err(error) = access {
        s.update(|r| {
            r.reservations.remove(&reservation);
            cleanup_participant_locked(r, &p);
            Ok(())
        })
        .await?;
        return Err(error);
    }
    s.update(|r| {
        let reserved = r
            .reservations
            .remove(&reservation)
            .is_some_and(|r| r.started.elapsed() < Duration::from_secs(30));
        let parent_valid = monitor.is_none_or(|monitor| {
            r.participants
                .get(&monitor.parent)
                .is_some_and(|parent| parent.monitor.is_none() && parent.lease.elapsed() < LEASE)
        });
        if !reserved || !parent_valid {
            cleanup_participant_locked(r, &p);
            return Ok(Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "join reservation or parent expired",
            )));
        }
        r.tokens.insert(p.token.clone(), id);
        r.participants.insert(id, p.clone());
        Ok(Ok(()))
    })
    .await??;
    let mut response = json!({"token":token,"id":id,"iceServers":ice,"turn":turn_metadata(p.turn.as_ref().unwrap())});
    if warm.is_some() {
        response["warm"] = json!(true);
    }
    // The capability was never returned, so nobody else can use this
    // participant yet: publication and the pulls run together without the
    // per-participant operation lock, and are committed afterwards.
    let publishing = async {
        match &input.publish {
            Some(publish) => Some(
                publish_track(&s, &token, &publish.mid, &publish.session_description.sdp).await,
            ),
            None => None,
        }
    };
    let pulling = async {
        match &receive_session {
            Some(session) if !pulls.is_empty() => Some(pull_tracks(&s, session, &pulls).await),
            _ => None,
        }
    };
    let (published, pulled) = tokio::join!(publishing, pulling);
    let pulled = receive_session.zip(pulled);
    match published {
        Some(Ok(publication)) => response["publish"] = publication,
        Some(Err(error)) => {
            // Any failure removes the participant instead of leaving a half-join.
            if let Some((session, pulled)) = pulled {
                abandon_pull(&s, &session, pulled).await;
            }
            remove_participant(&s, id).await;
            return Err(error);
        }
        None => {}
    }
    if let Some((session, pulled)) = pulled {
        match pulled {
            // The browser needs an offer to build its receive connection; without
            // one (nothing allocated, no renegotiation) pull after connecting.
            Ok(outcome) if outcome.offer.is_none() => abandon_pull(&s, &session, Ok(outcome)).await,
            Ok(outcome) => {
                let committed = s
                    .update(|r| {
                        let Some(p) = r.participants.get_mut(&id) else {
                            return Ok(false);
                        };
                        p.receive_session = Some(session.clone());
                        for (mid, track_id) in &outcome.allocated {
                            p.subscriptions.insert(mid.clone(), *track_id);
                        }
                        p.pending_offer = outcome.pending;
                        Ok(true)
                    })
                    .await?;
                if !committed {
                    abandon_pull(&s, &session, Ok(outcome)).await;
                    return Err(ApiError::new(StatusCode::UNAUTHORIZED, "session expired"));
                }
                response["receive"] = outcome.response([]);
            }
            // Uncertain: clean up whatever may exist and pull after connecting.
            Err(error) => abandon_pull(&s, &session, Err(error)).await,
        }
    }
    Ok(Json(response))
}

/// Releases a join-time pull that will not be used, whatever it allocated.
async fn abandon_pull(
    s: &AppState,
    session: &str,
    pulled: Result<PullOutcome, (ProviderError, Vec<String>)>,
) {
    let (mids, uncertain) = match pulled {
        Ok(outcome) => (
            outcome.allocated.into_iter().map(|(mid, _)| mid).collect(),
            false,
        ),
        Err((_, mids)) => (mids, true),
    };
    let _ = s
        .update(|r| {
            for mid in &mids {
                enqueue_cleanup_locked(r, session.to_owned(), mid.clone());
            }
            if uncertain {
                enqueue_action_locked(
                    r,
                    CleanupAction::Discover {
                        session: session.to_owned(),
                    },
                );
            }
            Ok(())
        })
        .await;
}

/// Removes and returns this account's prepared join while it is still usable,
/// cancelling the scheduled revocation of its TURN credentials.
fn take_prepared_locked(r: &mut Registry, account: &[u8]) -> Option<PreparedJoin> {
    r.prepared.retain(|p| p.issued.elapsed() < PREPARED_TTL);
    let index = r.prepared.iter().position(|p| p.account == account)?;
    let prepared = r.prepared.swap_remove(index);
    let usernames: Vec<_> = prepared.usernames().cloned().collect();
    r.cleanup.retain(|job| {
        !matches!(&job.action, CleanupAction::Revoke { username } if usernames.contains(username))
    });
    Some(prepared)
}

/// Creates a provider session and TURN credentials moments before a signed-in
/// member joins this channel, so their join skips both provider calls. An
/// unused session is disconnected by Cloudflare within 10-15 s and needs no
/// cleanup; its TURN credentials are revoked on a schedule set here.
async fn prepare(State(s): State<AppState>, Json(_): Json<Empty>) -> Result<StatusCode, ApiError> {
    ensure_enabled(&s)?;
    // Signed-in account channels only (the channel route sets this after its
    // membership check); the public demo stays create-on-join.
    let Some(account) = s.media_session.clone() else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not available"));
    };
    let (wanted, others_publishing) = s
        .update(|r| {
            r.prepared.retain(|p| p.issued.elapsed() < PREPARED_TTL);
            // A fresh one is still worth taking; this also bounds provider calls.
            let fresh = r
                .prepared
                .iter()
                .any(|p| p.account == account && p.issued.elapsed() < PREPARED_TTL / 2);
            Ok((
                !fresh && r.prepared.len() < MAX_PARTICIPANTS,
                !public_sources_locked(r).is_empty(),
            ))
        })
        .await?;
    if !wanted {
        return Ok(StatusCode::NO_CONTENT);
    }
    let issued = Timestamp::now();
    let (session, ice, receive_session) = tokio::join!(
        s.provider.create_session(&s.config),
        s.provider.turn(&s.config),
        async {
            if others_publishing {
                s.provider.create_session(&s.config).await.ok()
            } else {
                None
            }
        }
    );
    // Preparation is best effort; join creates whatever is missing as usual.
    let Ok(ice_servers) = ice else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let usernames: Vec<_> = ice_servers
        .iter()
        .filter_map(|server| server.username.clone())
        .collect();
    s.update(|r| {
        // Even when session creation failed, these credentials exist until revoked.
        for username in &usernames {
            enqueue_action_at_locked(
                r,
                CleanupAction::Revoke {
                    username: username.clone(),
                },
                issued + PREPARED_REVOKE_AFTER,
            );
        }
        if let Ok(session) = &session {
            r.prepared.retain(|p| p.account != account);
            r.prepared.push(PreparedJoin {
                account: account.clone(),
                session: session.clone(),
                ice_servers: ice_servers.clone(),
                issued,
                receive_session: receive_session.clone(),
            });
        }
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// What a warm ticket carries. It is signed for one account and never stored:
/// any room that account joins can adopt it, and the issuing room holds the
/// scheduled revocation of its TURN credentials.
#[derive(Serialize, Deserialize)]
struct WarmTicket {
    channel: Option<String>,
    session: String,
    receive: String,
    ice_servers: Vec<IceServer>,
    issued: Timestamp,
}

fn warm_mac(c: &Config, account: &[u8], payload: &[u8]) -> hmac::Hmac<Sha256> {
    use hmac::Mac;
    // A dedicated key derived from the provider secret every media pod already holds.
    let key = Sha256::new()
        .chain_update(b"caper-warm-ticket-v1\0")
        .chain_update(required(&c.app_secret).as_bytes())
        .finalize();
    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(&key).expect("HMAC accepts any key length");
    mac.update(&(account.len() as u64).to_be_bytes());
    mac.update(account);
    mac.update(payload);
    mac
}

fn sign_warm(c: &Config, account: &[u8], ticket: &WarmTicket) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use hmac::Mac;
    let payload = serde_json::to_vec(ticket).expect("a warm ticket serializes");
    let tag = warm_mac(c, account, &payload).finalize().into_bytes();
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&payload),
        URL_SAFE_NO_PAD.encode(tag)
    )
}

/// The ticket, if this account's own and unmodified. Age is checked by the caller.
fn verify_warm(c: &Config, account: &[u8], ticket: &str) -> Option<WarmTicket> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use hmac::Mac;
    if ticket.len() > 16_384 {
        return None;
    }
    let (payload, tag) = ticket.split_once('.')?;
    let payload = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let tag = URL_SAFE_NO_PAD.decode(tag).ok()?;
    warm_mac(c, account, &payload).verify_slice(&tag).ok()?;
    serde_json::from_slice(&payload).ok()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WarmInput {
    main: Sdp,
    receive: Sdp,
}

/// Creates a session pair from a signed-in member's two browser offers, before
/// any Join, and returns their answers. The browser connects both and keeps them
/// connected; Join then publishes and pulls into them, skipping session
/// creation, TURN issuance and both connection handshakes. Nothing is stored:
/// the returned ticket is signed for this account. Unadopted TURN credentials
/// are revoked after WARM_REVOKE_AFTER; closing the browser's connections ends
/// the provider sessions.
async fn warm(
    State(s): State<AppState>,
    Json(input): Json<WarmInput>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    let Some(account) = s.media_session.clone() else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not available"));
    };
    if [&input.main, &input.receive]
        .iter()
        .any(|d| !matches!(d.ty, SdpType::Offer) || !valid_text(&d.sdp, 200_000))
    {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid request"));
    }
    s.update(|r| {
        r.warmed.retain(|w| w.issued.elapsed() < WARM_MIN_INTERVAL);
        if r.warmed.len() >= 4 * MAX_PARTICIPANTS || r.warmed.iter().any(|w| w.account == account) {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate limit exceeded",
            ));
        }
        r.warmed.push(WarmIssue {
            account: account.clone(),
            issued: Timestamp::now(),
        });
        Ok(())
    })
    .await?;
    let issued = Timestamp::now();
    let (main, receive, ice) = tokio::join!(
        s.provider.connect_session(&s.config, &input.main.sdp),
        s.provider.connect_session(&s.config, &input.receive.sdp),
        s.provider.turn(&s.config),
    );
    let usable = main.is_ok() && receive.is_ok() && ice.is_ok();
    if let Ok(servers) = &ice {
        // Whatever else failed, these credentials exist until revoked.
        let revoke_at = if usable {
            issued + WARM_REVOKE_AFTER
        } else {
            issued
        };
        s.update(|r| {
            for username in servers.iter().filter_map(|server| server.username.as_ref()) {
                enqueue_action_at_locked(
                    r,
                    CleanupAction::Revoke {
                        username: username.clone(),
                    },
                    revoke_at,
                );
            }
            Ok(())
        })
        .await?;
    }
    // A provider session that is never connected needs no cleanup.
    let ((session, main_answer), (receive_session, receive_answer), ice_servers) =
        match (main, receive, ice) {
            (Ok(main), Ok(receive), Ok(ice)) => (main, receive, ice),
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => {
                return Err(error.into());
            }
        };
    let ticket = sign_warm(
        &s.config,
        &account,
        &WarmTicket {
            channel: s.media_channel.clone(),
            session,
            receive: receive_session,
            ice_servers: ice_servers.clone(),
            issued,
        },
    );
    Ok(Json(json!({
        "ticket": ticket,
        "main": main_answer,
        "receive": receive_answer,
        "iceServers": ice_servers,
        "adoptWithinMs": WARM_ADOPT_WINDOW.as_millis() as u64,
    })))
}

fn turn_metadata(cache: &TurnCache) -> Value {
    let now = Timestamp::now();
    json!({"generation":cache.generation,
        "refreshAfterMs":(cache.issued + Duration::from_secs(TURN_TTL / 2)).duration_since(now).as_millis() as u64,
        "expiresInMs":cache.expires.duration_since(now).as_millis() as u64})
}

fn prune_turn(p: &mut Participant) {
    let now = Timestamp::now();
    // Records written before credential caching contain only the original names.
    if p.turn.is_none() && p.turn_retired.is_empty() && !p.turn_usernames.is_empty() {
        p.turn_retired.push(RetiredTurn {
            usernames: p.turn_usernames.clone(),
            expires: p.joined + Duration::from_secs(TURN_TTL),
        });
    }
    if p.turn.as_ref().is_some_and(|cache| cache.expires <= now) {
        let cache = p.turn.take().unwrap();
        p.turn_retired.push(RetiredTurn {
            usernames: cache
                .ice_servers
                .iter()
                .filter_map(|v| v.username.clone())
                .collect(),
            expires: cache.revoke_after,
        });
    }
    p.turn_retired.retain(|retired| retired.expires > now);
    p.turn_usernames = p
        .turn_retired
        .iter()
        .flat_map(|r| r.usernames.iter().cloned())
        .chain(
            p.turn
                .iter()
                .flat_map(|c| c.ice_servers.iter().filter_map(|v| v.username.clone())),
        )
        .collect();
    p.turn_usernames.sort();
    p.turn_usernames.dedup();
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TurnInput {
    generation: Option<Uuid>,
}

enum TurnDecision {
    Cached(TurnCache),
    Mint {
        id: Uuid,
        session: String,
        claim: Uuid,
    },
}

async fn turn(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<TurnInput>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    let token = bearer(&headers)?;
    let decision = s
        .update(|r| {
            let id = authenticate(r, token)?;
            let p = r.participants.get_mut(&id).unwrap();
            let now = Timestamp::now();
            prune_turn(p);
            if let Some(cache) = &p.turn {
                let due = now.duration_since(cache.issued) >= Duration::from_secs(TURN_TTL / 2);
                if input.generation != Some(cache.generation) || !due {
                    return Ok(TurnDecision::Cached(cache.clone()));
                }
            }
            if p.restart.is_some() {
                return Err(ApiError::new(StatusCode::CONFLICT, "ICE restart pending")
                    .with_code("ice_restart_pending"));
            }
            if p.turn_retired.len() + usize::from(p.turn.is_some()) >= 3 {
                return Err(ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "credential generations still active",
                ));
            }
            if p.turn_claim
                .as_ref()
                .is_some_and(|c| c.started.elapsed() < RESERVATION)
            {
                return Err(ApiError::new(StatusCode::CONFLICT, "TURN issuance pending")
                    .with_code("ice_restart_pending"));
            }
            while p
                .turn_attempts
                .front()
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(60))
            {
                p.turn_attempts.pop_front();
            }
            if p.turn_attempts.len() >= 2 {
                return Err(ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate limit exceeded",
                ));
            }
            p.turn_attempts.push_back(now);
            let claim = Uuid::new_v4();
            p.turn_claim = Some(Claim {
                nonce: claim,
                started: now,
            });
            Ok(TurnDecision::Mint {
                id,
                session: p.session.clone(),
                claim,
            })
        })
        .await?;
    if let TurnDecision::Cached(cache) = decision {
        return Ok(Json(
            json!({"iceServers":cache.ice_servers,"turn":turn_metadata(&cache)}),
        ));
    }
    let TurnDecision::Mint { id, session, claim } = decision else {
        unreachable!()
    };
    let issued = Timestamp::now();
    let ice = match s.provider.turn(&s.config).await {
        Ok(ice) => ice,
        Err(error) => {
            let _ = s
                .update(|r| {
                    if let Some(p) = r.participants.get_mut(&id)
                        && p.turn_claim.as_ref().is_some_and(|c| c.nonce == claim)
                    {
                        p.turn_claim = None;
                    }
                    Ok(())
                })
                .await;
            return Err(error.into());
        }
    };
    let generation = Uuid::new_v4();
    let expires = issued + Duration::from_secs(TURN_TTL);
    let cache = TurnCache {
        generation,
        ice_servers: ice.clone(),
        issued,
        expires,
        revoke_after: Timestamp::now() + Duration::from_secs(TURN_TTL),
    };
    let committed = s
        .update(|r| {
            let parent_live = r.participants.get(&id).is_some_and(|participant| {
                participant.monitor.is_none_or(|monitor| {
                    r.participants
                        .get(&monitor.parent)
                        .is_some_and(|parent| parent.lease.elapsed() < LEASE)
                })
            });
            let Some(p) = r.participants.get_mut(&id) else {
                return Ok(false);
            };
            if p.turn_claim.as_ref().is_none_or(|c| c.nonce != claim)
                || p.session != session
                || p.lease.elapsed() >= LEASE
                || !parent_live
            {
                return Ok(false);
            }
            p.turn_claim = None;
            if let Some(previous) = p.turn.take() {
                p.turn_retired.push(RetiredTurn {
                    usernames: previous
                        .ice_servers
                        .iter()
                        .filter_map(|v| v.username.clone())
                        .collect(),
                    expires: previous.revoke_after,
                });
            }
            p.turn = Some(cache.clone());
            prune_turn(p);
            Ok(true)
        })
        .await?;
    if !committed {
        for username in ice.iter().filter_map(|server| server.username.as_deref()) {
            enqueue_action(
                &s,
                CleanupAction::Revoke {
                    username: username.into(),
                },
            )
            .await;
        }
        // A newer issuance claim may have won without ending this call.
        s.read(|r| authenticate(r, token)).await?;
        return Err(
            ApiError::new(StatusCode::CONFLICT, "TURN issuance superseded")
                .with_code("ice_restart_pending"),
        );
    }
    Ok(Json(json!({"iceServers":ice,"turn":turn_metadata(&cache)})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RestartInput {
    generation: Uuid,
    sequence: u64,
    session_description: Sdp,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RestartAck {
    generation: Uuid,
    sequence: u64,
}

fn sdp_restart_valid(sdp: &str, p: &Participant) -> bool {
    if !valid_text(sdp, 200_000) {
        return false;
    }
    let mut mids = std::collections::HashSet::new();
    let mut section: Option<(Option<&str>, Option<&str>)> = None;
    let mut sections = vec![];
    for line in sdp.lines().map(|l| l.trim_end_matches('\r')) {
        if line.starts_with("m=") {
            if let Some(v) = section.take() {
                sections.push(v);
            }
            if !line.starts_with("m=audio ") {
                return false;
            }
            section = Some((None, None));
        } else if let Some(current) = section.as_mut() {
            if let Some(mid) = line.strip_prefix("a=mid:") {
                if current.0.is_some() {
                    return false;
                }
                current.0 = Some(mid);
            }
            if matches!(
                line,
                "a=sendonly" | "a=recvonly" | "a=sendrecv" | "a=inactive"
            ) {
                if current.1.is_some() {
                    return false;
                }
                current.1 = Some(&line[2..]);
            }
        }
    }
    if let Some(v) = section {
        sections.push(v);
    }
    if sections.is_empty() {
        return false;
    }
    sections.into_iter().all(|(mid, direction)| {
        let Some(mid) = mid else { return false };
        let Some(direction) = direction else {
            return false;
        };
        if !valid_text(mid, 64) || !mids.insert(mid) {
            return false;
        }
        if direction == "sendrecv" {
            return false;
        }
        let sends = direction == "sendonly";
        !sends
            || (p.tracks.contains_key(mid)
                && !p.monitor.is_some_and(|m| m.role == MonitorRole::Receiver))
    })
}

async fn restart_ice(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<RestartInput>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    if i.sequence == 0 || !matches!(i.session_description.ty, SdpType::Offer) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid request"));
    }
    let hash = format!("{:x}", Sha256::digest(i.session_description.sdp.as_bytes()));
    let claim = Uuid::new_v4();
    let token = bearer(&headers)?;
    let (id, session) = s
        .update(|r| {
            let id = authenticate(r, token)?;
            let p = r.participants.get_mut(&id).unwrap();
            if !sdp_restart_valid(&i.session_description.sdp, p) {
                return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid request"));
            }
            if p.turn.as_ref().is_none_or(|t| t.generation != i.generation) {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "TURN generation mismatch",
                ));
            }
            if i.sequence <= p.restart_acknowledged {
                return Err(ApiError::new(StatusCode::CONFLICT, "stale ICE restart"));
            }
            if p.pending_offer || (p.operation && p.restart.is_none()) {
                return Err(ApiError::new(StatusCode::CONFLICT, "operation pending")
                    .with_code("ice_restart_pending"));
            }
            record_operation(p)?;
            if let Some(pending) = &mut p.restart {
                if pending.generation != i.generation
                    || pending.sequence != i.sequence
                    || pending.hash != hash
                {
                    return Err(ApiError::new(StatusCode::CONFLICT, "ICE restart mismatch"));
                }
                if pending
                    .claim
                    .as_ref()
                    .is_some_and(|c| c.started.elapsed() < RESERVATION)
                {
                    return Err(ApiError::new(StatusCode::CONFLICT, "ICE restart pending")
                        .with_code("ice_restart_pending"));
                }
                pending.claim = Some(Claim {
                    nonce: claim,
                    started: Timestamp::now(),
                });
            } else {
                if i.sequence != p.restart_acknowledged + 1 {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        "ICE restart sequence mismatch",
                    ));
                }
                p.operation = true;
                p.operation_started = None;
                p.restart = Some(PendingRestart {
                    generation: i.generation,
                    sequence: i.sequence,
                    hash: hash.clone(),
                    claim: Some(Claim {
                        nonce: claim,
                        started: Timestamp::now(),
                    }),
                });
            }
            Ok((id, p.session.clone()))
        })
        .await?;
    let result = s.provider.restart_ice(&s.config, &session, json!({"sessionDescription":{"type":"offer","sdp":i.session_description.sdp},"autoDiscover":true})).await;
    let valid = result.as_ref().is_ok_and(|v| {
        validate_provider_envelope(v).is_ok()
            && v.get("tracks")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
            && v.pointer("/sessionDescription/type")
                .and_then(Value::as_str)
                == Some("answer")
            && v.pointer("/sessionDescription/sdp")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
    });
    s.update(|r| {
        authenticate(r, token)?;
        if let Some(p) = r.participants.get_mut(&id).filter(|p| p.session == session)
            && let Some(x) = &mut p.restart
            && x.generation == i.generation
            && x.sequence == i.sequence
            && x.hash == hash
            && x.claim.as_ref().is_some_and(|c| c.nonce == claim)
        {
            x.claim = None;
            return Ok(());
        }
        Err(
            ApiError::new(StatusCode::CONFLICT, "ICE restart superseded")
                .with_code("ice_restart_pending"),
        )
    })
    .await?;
    if !valid {
        if let Err(error) = result {
            // Provider authentication failures are not expired user capabilities.
            let retry = error.transient()
                || matches!(&error, ProviderError::Request(f) if matches!(f.status, Some(401 | 403)));
            return Err(ApiError::from(error).with_code(if retry {
                "ice_restart_retry"
            } else {
                "ice_restart_invalid"
            }));
        }
        return Err(
            ApiError::new(StatusCode::BAD_GATEWAY, "invalid ICE restart answer")
                .with_code("ice_restart_invalid"),
        );
    }
    Ok(Json(result.unwrap()))
}

async fn restart_ice_ack(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<RestartAck>,
) -> Result<StatusCode, ApiError> {
    ensure_enabled(&s)?;
    let token = bearer(&headers)?;
    s.update(|r| {
        let id = authenticate(r, token)?;
        let p = r.participants.get_mut(&id).unwrap();
        if i.sequence <= p.restart_acknowledged {
            return Ok(());
        }
        if p.restart
            .as_ref()
            .is_none_or(|x| x.generation != i.generation || x.sequence != i.sequence)
        {
            return Err(ApiError::new(StatusCode::CONFLICT, "ICE restart mismatch"));
        }
        p.restart_acknowledged = i.sequence;
        p.restart = None;
        p.operation = false;
        p.operation_started = None;
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn release_join_reservation(s: &AppState, reservation: Uuid) {
    let _ = s
        .update(|r| {
            r.reservations.remove(&reservation);
            Ok(())
        })
        .await;
}

#[derive(Serialize)]
struct View<'a> {
    id: Uuid,
    name: &'a str,
    #[serde(rename = "countryCode", skip_serializing_if = "Option::is_none")]
    country_code: Option<&'a str>,
    muted: bool,
    deafened: bool,
    tracks: Vec<TrackView>,
}
#[derive(Serialize)]
struct TrackView {
    id: Uuid,
    kind: Kind,
}
fn public_snapshot(r: &Registry) -> Value {
    let mut participants: Vec<_> = r
        .participants
        .values()
        .filter(|p| p.monitor.is_none())
        .collect();
    participants.sort_by_key(|p| p.id);
    let participants: Vec<_> = participants
        .into_iter()
        .map(|p| {
            let mut tracks: Vec<_> = p
                .tracks
                .values()
                .map(|t| TrackView {
                    id: t.id,
                    kind: t.kind,
                })
                .collect();
            tracks.sort_by_key(|t| t.id);
            View {
                id: p.id,
                name: &p.name,
                country_code: p.country_code.as_deref(),
                muted: p.muted,
                deafened: p.deafened,
                tracks,
            }
        })
        .collect();
    json!({"participants":participants,"revision":r.revision})
}
#[derive(Serialize)]
struct PresenceView<'a> {
    id: Uuid,
    name: &'a str,
    #[serde(rename = "countryCode", skip_serializing_if = "Option::is_none")]
    country_code: Option<&'a str>,
    muted: bool,
    deafened: bool,
}
async fn presence(State(s): State<AppState>) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    s.read(|r| Ok(Json(presence_snapshot(r)))).await
}
fn presence_snapshot(r: &Registry) -> Value {
    let mut participants: Vec<_> = r
        .participants
        .values()
        .filter(|p| p.monitor.is_none())
        .map(|p| PresenceView {
            id: p.id,
            name: &p.name,
            country_code: p.country_code.as_deref(),
            muted: p.muted,
            deafened: p.deafened,
        })
        .collect();
    participants.sort_by_key(|p| p.id);
    json!({"participants":participants,"revision":r.revision})
}
async fn snapshot(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(_): Json<Empty>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    let token = bearer(&headers)?;
    s.update(|r| {
        let id = authenticate(r, token)?;
        r.participants.get_mut(&id).unwrap().lease = Timestamp::now();
        if r.participants.get(&id).unwrap().monitor.is_some() {
            return Ok(Json(json!({"participants":[]})));
        }
        Ok(Json(public_snapshot(r)))
    })
    .await
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
    if i.kind != "microphone" || !valid_offer(&i.mid, &i.session_description) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid request"));
    }
    publish_track(&s, bearer(&headers)?, &i.mid, &i.session_description.sdp)
        .await
        .map(Json)
}

fn valid_offer(mid: &str, description: &Sdp) -> bool {
    matches!(description.ty, SdpType::Offer)
        && valid_text(mid, 64)
        && valid_text(&description.sdp, 200_000)
}

async fn publish_track(s: &AppState, token: &str, mid: &str, sdp: &str) -> Result<Value, ApiError> {
    let (id, session) = s
        .update(|r| {
            let id = authenticate(r, token)?;
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
            if p.tracks.len() >= MAX_TRACKS || p.tracks.contains_key(mid) {
                p.operation = false;
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "track limit or duplicate track",
                ));
            }
            Ok((id, p.session.clone()))
        })
        .await?;
    let track_id = Uuid::new_v4();
    let provider_name = format!("caper-{track_id}");
    let body = json!({"sessionDescription":{"type":"offer","sdp":sdp},"tracks":[{"location":"local","mid":mid,"trackName":provider_name,"kind":"audio"}]});
    let result = s.provider.tracks_new(&s.config, &session, body).await;
    let valid = result.as_ref().is_ok_and(|v| {
        validate_provider_envelope(v).is_ok()
            && v.get("tracks").and_then(Value::as_array).is_some_and(|a| {
                a.iter()
                    .any(|t| t.get("mid").and_then(Value::as_str) == Some(mid))
            })
            && v.pointer("/sessionDescription/type")
                .and_then(Value::as_str)
                == Some("answer")
            && v.pointer("/sessionDescription/sdp")
                .and_then(Value::as_str)
                .is_some()
    });
    if !valid {
        enqueue_cleanup(s, session.clone(), mid.to_owned()).await;
        remove_participant(s, id).await;
        return Err(ApiError::from(
            result
                .err()
                .unwrap_or_else(|| ProviderError::invalid_response("publish")),
        ));
    }
    let result = result.unwrap();
    s.update(|r| {
        let Some(p) = r.participants.get_mut(&id) else {
            enqueue_cleanup_locked(r, session.clone(), mid.to_owned());
            return Ok(Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "session expired",
            )));
        };
        p.operation = false;
        p.operation_started = None;
        p.tracks.insert(
            mid.to_owned(),
            Track {
                id: track_id,
                kind: Kind::Microphone,
                provider_name: provider_name.clone(),
            },
        );
        Ok(Ok(()))
    })
    .await??;
    let mut result = result;
    if let Some(object) = result.as_object_mut() {
        object.insert("trackId".into(), json!(track_id));
    }
    Ok(result)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Subscribe {
    track_id: Option<Uuid>,
    /// Several publications pulled with one provider request and one SDP
    /// exchange. The single `trackId` form and its response are unchanged.
    track_ids: Option<Vec<Uuid>>,
}
async fn subscribe(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<Subscribe>,
) -> Result<Json<Value>, ApiError> {
    ensure_enabled(&s)?;
    let token = bearer(&headers)?;
    match (i.track_id, i.track_ids) {
        (Some(track_id), None) => subscribe_one(&s, token, track_id).await,
        (None, Some(ids)) if !ids.is_empty() && ids.len() <= MAX_SUBSCRIPTIONS => {
            subscribe_many(&s, token, ids).await
        }
        _ => Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid request")),
    }
}

async fn subscribe_one(s: &AppState, token: &str, track_id: Uuid) -> Result<Json<Value>, ApiError> {
    let (me, session, source) = s
        .update(|r| {
            let me = authenticate(r, token)?;
            let source = r
                .participants
                .values()
                .find_map(|p| {
                    p.tracks
                        .values()
                        .find(|t| t.id == track_id)
                        .map(|t| (p.id, p.session.clone(), t.provider_name.clone(), t.kind))
                })
                .ok_or_else(|| {
                    ApiError::new(StatusCode::NOT_FOUND, "track not found").with_code("track_gone")
                })?;
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
                || p.subscriptions.values().any(|id| *id == track_id)
            {
                p.operation = false;
                return Err(ApiError::new(StatusCode::CONFLICT, "subscription limit"));
            }
            Ok((me, p.pull_session().clone(), source))
        })
        .await?;
    let result = s
        .provider
        .tracks_new(
            &s.config,
            &session,
            json!({"tracks":[{"location":"remote","sessionId":source.1,"trackName":source.2}]}),
        )
        .await;
    if matches!(&result, Err(ProviderError::Request(f)) if f.kind == "track_unavailable") {
        s.update(|r| {
            let id = authenticate(r, token)?;
            let p = r.participants.get_mut(&id).unwrap();
            p.operation = false;
            p.operation_started = None;
            Ok(())
        })
        .await?;
        return Err(ApiError::new(StatusCode::NOT_FOUND, "track not found").with_code("track_gone"));
    }
    let valid_result = result
        .as_ref()
        .ok()
        .filter(|v| validate_provider_envelope(v).is_ok());
    let mid = valid_result
        .and_then(|v| v.get("tracks"))
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|t| t.get("mid"))
        .and_then(Value::as_str);
    let Some(mid) = mid else {
        // The mutation may have succeeded. Invalidate locally now, then discover
        // orphan MIDs in the worker; never replace the original provider failure.
        remove_participant(s, me).await;
        enqueue_action(s, CleanupAction::Discover { session }).await;
        return Err(result
            .err()
            .unwrap_or_else(|| ProviderError::invalid_response("subscribe"))
            .into());
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
        enqueue_cleanup(s, session, mid).await;
        remove_participant(s, me).await;
        return Err(ProviderError::invalid_response("subscribe").into());
    }
    s.update(|r| {
        // The provider already created this MID and possibly an SDP offer.
        // Even if the source left meanwhile, finish that negotiation rather
        // than invalidating the listener's entire call. Its next roster omits
        // the source, so the browser closes this MID; lease cleanup covers a
        // browser that disappears before doing so.
        let Some(p) = r.participants.get_mut(&me) else {
            enqueue_cleanup_locked(r, session.clone(), mid.clone());
            return Ok(Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "session expired",
            )));
        };
        p.operation = false;
        p.operation_started = None;
        p.subscriptions.insert(mid.clone(), track_id);
        p.pending_offer = pending;
        Ok(Ok(()))
    })
    .await??;
    Ok(Json(result))
}

/// One requested pull: the listener-facing track ID and its provider locator.
struct Pull {
    track_id: Uuid,
    session: String,
    name: String,
}

enum PullResult {
    Allocated(String),
    Refused,
}

/// Classify every requested pull from a batch response, or None if any result
/// is unaccounted for. Cloudflare reports partial success per track: a result
/// with a MID is allocated; one with a documented unavailable-source code and
/// no MID allocated nothing. Anything else leaves the outcome uncertain.
fn classify_pulls(pulls: &[Pull], value: &Value) -> Option<Vec<PullResult>> {
    if !pull_batch_envelope(value) {
        return None;
    }
    let results = value.get("tracks")?.as_array()?;
    if results.len() != pulls.len() {
        return None;
    }
    pulls
        .iter()
        .enumerate()
        .map(|(index, pull)| {
            // Match by locator; fall back to request order only when the
            // provider omits the echo.
            let result = results
                .iter()
                .find(|t| {
                    t.get("sessionId").and_then(Value::as_str) == Some(pull.session.as_str())
                        && t.get("trackName").and_then(Value::as_str) == Some(pull.name.as_str())
                })
                .or_else(|| {
                    let t = &results[index];
                    (t.get("trackName").is_none() && t.get("sessionId").is_none()).then_some(t)
                })?;
            let mid = result
                .get("mid")
                .and_then(Value::as_str)
                .filter(|mid| !mid.is_empty());
            match (mid, result.get("errorCode").filter(|v| !v.is_null())) {
                (Some(mid), None) => Some(PullResult::Allocated(mid.to_owned())),
                (None, Some(code)) if unavailable_track_code(code) => Some(PullResult::Refused),
                _ => None,
            }
        })
        .collect()
}
fn unavailable_track_code(code: &Value) -> bool {
    matches!(
        code.as_str(),
        Some("not_found_track_error" | "empty_track_error" | "track_error")
    )
}
/// Request-level validity for a remote batch. Per-track errors are allowed
/// here and classified by `classify_pulls`.
fn pull_batch_envelope(value: &Value) -> bool {
    value.is_object()
        && value.get("success").and_then(Value::as_bool) != Some(false)
        && value
            .get("errors")
            .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty))
        && value.get("errorCode").is_none_or(Value::is_null)
        && value
            .get("tracks")
            .and_then(Value::as_array)
            .is_some_and(|tracks| !tracks.is_empty())
}

/// The provider may have allocated MIDs. Invalidate locally and discover
/// orphans, exactly like an uncertain single subscription.
async fn uncertain_pull(s: &AppState, me: Uuid, session: &str, allocated: Vec<String>) {
    for mid in allocated {
        enqueue_cleanup(s, session.to_owned(), mid).await;
    }
    remove_participant(s, me).await;
    enqueue_action(
        s,
        CleanupAction::Discover {
            session: session.to_owned(),
        },
    )
    .await;
}

/// A classified batch pull: allocated (MID, track) pairs, refused tracks, and
/// the provider offer to answer, if any.
struct PullOutcome {
    allocated: Vec<(String, Uuid)>,
    refused: Vec<Uuid>,
    offer: Option<Value>,
    pending: bool,
}

/// Pulls `pulls` into `session` with one provider request and classifies the
/// result. No registry state is touched. On error the outcome is uncertain:
/// the provider may have allocated MIDs, which are returned for cleanup.
async fn pull_tracks(
    s: &AppState,
    session: &str,
    pulls: &[Pull],
) -> Result<PullOutcome, (ProviderError, Vec<String>)> {
    let tracks = pulls
        .iter()
        .map(|pull| json!({"location":"remote","sessionId":pull.session,"trackName":pull.name}))
        .collect::<Vec<_>>();
    let result = s
        .provider
        .pull_batch(&s.config, session, json!({ "tracks": tracks }))
        .await;
    let classified = result
        .as_ref()
        .ok()
        .and_then(|value| classify_pulls(pulls, value));
    let Some(classified) = classified else {
        return Err((
            result
                .err()
                .unwrap_or_else(|| ProviderError::invalid_response("subscribe")),
            vec![],
        ));
    };
    let result = result.unwrap();
    let mut allocated = vec![];
    let mut refused = vec![];
    for (pull, outcome) in pulls.iter().zip(classified) {
        match outcome {
            PullResult::Allocated(mid) => allocated.push((mid, pull.track_id)),
            PullResult::Refused => refused.push(pull.track_id),
        }
    }
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
    // Allocations need an offer. An offer without allocations is Cloudflare
    // renegotiating anyway: a pull into a session with no negotiated
    // PeerConnection returns an inactive placeholder m-line even when every
    // source is refused (live-captured September 25, 2026). Answer it.
    if pending != has_offer || (!allocated.is_empty() && !has_offer) {
        return Err((
            ProviderError::invalid_response("subscribe"),
            allocated.into_iter().map(|(mid, _)| mid).collect(),
        ));
    }
    Ok(PullOutcome {
        allocated,
        refused,
        offer: has_offer.then(|| result["sessionDescription"].clone()),
        pending,
    })
}

impl PullOutcome {
    fn response(&self, departed: impl IntoIterator<Item = Uuid>) -> Value {
        let mut response = json!({
            "tracks": self.allocated.iter().map(|(mid, track_id)| json!({"trackId":track_id,"mid":mid})).collect::<Vec<_>>(),
            "gone": self.refused.iter().copied().chain(departed).collect::<Vec<_>>(),
            "requiresImmediateRenegotiation": self.pending,
        });
        if let Some(offer) = &self.offer {
            response["sessionDescription"] = offer.clone();
        }
        response
    }
}

/// Every non-monitor publication in the room: what a public participant may pull.
fn public_sources_locked(r: &Registry) -> Vec<Pull> {
    r.participants
        .values()
        .filter(|p| p.monitor.is_none())
        .flat_map(|p| {
            p.tracks.values().map(|t| Pull {
                track_id: t.id,
                session: p.session.clone(),
                name: t.provider_name.clone(),
            })
        })
        .take(MAX_SUBSCRIPTIONS)
        .collect()
}

async fn subscribe_many(
    s: &AppState,
    token: &str,
    ids: Vec<Uuid>,
) -> Result<Json<Value>, ApiError> {
    // Deduplicate while keeping the caller's order.
    let mut seen = std::collections::HashSet::new();
    let ids = ids
        .into_iter()
        .filter(|id| seen.insert(*id))
        .collect::<Vec<_>>();
    let (me, session, pulls, departed) = s
        .update(|r| {
            let me = authenticate(r, token)?;
            let subscriber_monitor = r.participants.get(&me).unwrap().monitor;
            let mut pulls = vec![];
            let mut departed = vec![];
            for &track_id in &ids {
                let Some((owner, owner_session, name, owner_monitor)) =
                    r.participants.values().find_map(|p| {
                        p.tracks
                            .values()
                            .find(|t| t.id == track_id)
                            .map(|t| (p.id, p.session.clone(), t.provider_name.clone(), p.monitor))
                    })
                else {
                    departed.push(track_id);
                    continue;
                };
                if owner == me {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        "cannot subscribe to own track",
                    ));
                }
                let authorized = match (subscriber_monitor, owner_monitor) {
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
                pulls.push(Pull {
                    track_id,
                    session: owner_session,
                    name,
                });
            }
            if pulls.is_empty() {
                return Err(
                    ApiError::new(StatusCode::NOT_FOUND, "track not found").with_code("track_gone")
                );
            }
            let p = r.participants.get_mut(&me).unwrap();
            begin_operation(p)?;
            if p.pending_offer {
                p.operation = false;
                return Err(ApiError::new(StatusCode::CONFLICT, "negotiation pending"));
            }
            if p.subscriptions.len() + pulls.len() > MAX_SUBSCRIPTIONS
                || pulls
                    .iter()
                    .any(|pull| p.subscriptions.values().any(|id| *id == pull.track_id))
            {
                p.operation = false;
                return Err(ApiError::new(StatusCode::CONFLICT, "subscription limit"));
            }
            Ok((me, p.pull_session().clone(), pulls, departed))
        })
        .await?;
    let outcome = match pull_tracks(s, &session, &pulls).await {
        Ok(outcome) => outcome,
        Err((error, allocated)) => {
            uncertain_pull(s, me, &session, allocated).await;
            return Err(error.into());
        }
    };
    s.update(|r| {
        let Some(p) = r.participants.get_mut(&me) else {
            for (mid, _) in &outcome.allocated {
                enqueue_cleanup_locked(r, session.clone(), mid.clone());
            }
            return Ok(Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                "session expired",
            )));
        };
        p.operation = false;
        p.operation_started = None;
        for (mid, track_id) in &outcome.allocated {
            p.subscriptions.insert(mid.clone(), *track_id);
        }
        p.pending_offer = outcome.pending;
        Ok(Ok(()))
    })
    .await??;
    Ok(Json(outcome.response(departed)))
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
    let (id, session) = s
        .update(|r| {
            let id = authenticate(r, token)?;
            let p = r.participants.get_mut(&id).unwrap();
            begin_operation(p)?;
            if !p.pending_offer {
                p.operation = false;
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "no negotiation pending",
                ));
            }
            // Only subscriptions leave a provider offer pending.
            Ok((id, p.pull_session().clone()))
        })
        .await?;
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
        return Err(result
            .err()
            .unwrap_or_else(|| ProviderError::invalid_response("negotiate"))
            .into());
    }
    let result = result.unwrap();
    s.update(|r| {
        let p = r
            .participants
            .get_mut(&id)
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "session expired"))?;
        p.operation = false;
        p.operation_started = None;
        p.pending_offer = false;
        Ok(())
    })
    .await?;
    Ok(Json(result))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Close {
    mid: String,
    /// With a receive session, publication and subscription MIDs belong to
    /// different provider sessions and may be equal; this names the latter.
    #[serde(default)]
    subscription: bool,
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
    s.update(|r| {
        let id = authenticate(r, token)?;
        let p = r.participants.get_mut(&id).unwrap();
        let separate = p.receive_session.is_some();
        // Without a receive session one MID space holds both, as before.
        let publication = !i.subscription && p.tracks.contains_key(&i.mid);
        let subscription = (i.subscription || !separate) && p.subscriptions.contains_key(&i.mid);
        if !publication && !subscription {
            return Err(ApiError::new(StatusCode::NOT_FOUND, "track not found"));
        }
        begin_operation(p)?;
        p.operation = false;
        p.operation_started = None;
        let session = if publication {
            p.session.clone()
        } else {
            p.pull_session().clone()
        };
        let source = if publication {
            p.tracks.remove(&i.mid).map(|t| t.id)
        } else {
            None
        };
        if subscription {
            p.subscriptions.remove(&i.mid);
        }
        // force:true stops only this MID's data flow, without changing SDP.
        // Commit removal and cleanup together; a provider timeout/rejection must
        // never revoke the caller's unrelated tracks or call capability.
        enqueue_cleanup_locked(r, session, i.mid.clone());
        if let Some(source) = source {
            close_dependents_locked(r, source);
        }
        Ok(())
    })
    .await?;
    Ok(Json(json!({})))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StateBody {
    muted: bool,
    deafened: bool,
    sequence: Option<u64>,
}
async fn update_state(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(i): Json<StateBody>,
) -> Result<StatusCode, ApiError> {
    ensure_enabled(&s)?;
    let token = bearer(&headers)?;
    s.update(|r| {
        let id = authenticate(r, token)?;
        let p = r.participants.get_mut(&id).unwrap();
        if let Some(sequence) = i.sequence {
            // A client timeout does not cancel an admitted request on another pod.
            // Preserve newer intent even if an old write or replay commits later.
            if sequence <= p.state_sequence {
                return Ok(StatusCode::NO_CONTENT);
            }
            p.state_sequence = sequence;
        }
        p.muted = i.muted;
        p.deafened = i.deafened;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
}
async fn leave(
    State(s): State<AppState>,
    headers: HeaderMap,
    Json(_): Json<Empty>,
) -> Result<StatusCode, ApiError> {
    ensure_enabled(&s)?;
    let token = bearer(&headers)?;
    s.update(|r| {
        let id = authenticate(r, token)?;
        remove_participant_locked(r, id);
        Ok(())
    })
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

fn close_dependents_locked(r: &mut Registry, source: Uuid) {
    let jobs = r
        .participants
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
                .map(|m| (p.pull_session().clone(), m))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    for (session, mid) in jobs {
        enqueue_cleanup_locked(r, session, mid);
    }
}
async fn remove_participant(s: &AppState, id: Uuid) {
    if s.update(|r| {
        remove_participant_locked(r, id);
        Ok(())
    })
    .await
    .is_err()
    {
        tracing::warn!("media removal deferred to shared lease expiry");
    }
}
fn remove_participant_locked(r: &mut Registry, id: Uuid) {
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
    for p in removed {
        cleanup_participant_locked(r, &p);
        for source in p.tracks.values().map(|t| t.id) {
            close_dependents_locked(r, source);
        }
    }
}
fn cleanup_participant_locked(r: &mut Registry, p: &Participant) {
    for username in &p.turn_usernames {
        enqueue_action_locked(
            r,
            CleanupAction::Revoke {
                username: username.clone(),
            },
        );
    }
    for mid in p.tracks.keys() {
        enqueue_cleanup_locked(r, p.session.clone(), mid.clone());
    }
    for mid in p.subscriptions.keys() {
        enqueue_cleanup_locked(r, p.pull_session().clone(), mid.clone());
    }
    if p.operation {
        enqueue_action_locked(
            r,
            CleanupAction::Discover {
                session: p.session.clone(),
            },
        );
        if let Some(session) = &p.receive_session {
            enqueue_action_locked(
                r,
                CleanupAction::Discover {
                    session: session.clone(),
                },
            );
        }
    }
}
fn enqueue_action_locked(r: &mut Registry, action: CleanupAction) {
    enqueue_action_at_locked(r, action, Timestamp::now());
}
fn enqueue_action_at_locked(r: &mut Registry, action: CleanupAction, not_before: Timestamp) {
    if r.cleanup.iter().any(|job| job.action == action) {
        return;
    }
    if r.cleanup.len() >= MAX_CLEANUP_BACKLOG {
        tracing::error!(
            operation = action.operation(),
            "cleanup backlog full; dropping new job; provider resources may remain"
        );
        return;
    }
    r.cleanup.push_back(CleanupJob {
        action,
        attempts: 0,
        not_before,
        claim: None,
    });
}
async fn enqueue_action(s: &AppState, action: CleanupAction) {
    let _ = s
        .update(|r| {
            enqueue_action_locked(r, action.clone());
            Ok(())
        })
        .await;
}
fn enqueue_cleanup_locked(r: &mut Registry, session: String, mid: String) {
    enqueue_action_locked(r, CleanupAction::Close { session, mid });
}
async fn enqueue_cleanup(s: &AppState, session: String, mid: String) {
    enqueue_action(s, CleanupAction::Close { session, mid }).await;
}
async fn execute_cleanup(s: &AppState, action: &CleanupAction) -> Result<(), ProviderError> {
    match action {
        // force:true only stops this MID's data flow; no SDP mutation or resource
        // creation. Caper never reuses these MIDs via tracks/update.
        CleanupAction::Close { session, mid } => {
            let value = s.provider.close(&s.config, session, mid).await?;
            validate_provider_envelope(&value)
        }
        CleanupAction::Revoke { username } => s.provider.revoke_turn(&s.config, username).await,
        CleanupAction::Discover { session } => {
            let info = s.provider.session_tracks(&s.config, session).await?;
            validate_provider_envelope(&info)?;
            let tracks = info
                .get("tracks")
                .and_then(Value::as_array)
                .ok_or(ProviderError::Rejected)?;
            s.update(|r| {
                for track in tracks {
                    if let Some(mid) = track.get("mid").and_then(Value::as_str) {
                        enqueue_cleanup_locked(r, session.clone(), mid.into());
                    }
                }
                Ok(())
            })
            .await
            .map_err(|_| ProviderError::Unavailable)
        }
    }
}
async fn retry_backlog(s: &AppState) -> Duration {
    // One batch at a time, including during shutdown. At most four provider
    // requests in flight and 512 queued; an outage cannot spawn unbounded tasks.
    let _guard = s.cleanup_lock.lock().await;
    let claim = Uuid::new_v4();
    let (jobs, next) = s
        .update(|r| {
            let mut jobs = vec![];
            let now = Timestamp::now();
            for job in &mut r.cleanup {
                if jobs.len() < 4 && job.not_before <= now {
                    job.claim = Some(claim);
                    job.not_before = now + Duration::from_secs(30);
                    jobs.push(job.clone());
                }
            }
            let next = r
                .cleanup
                .iter()
                .filter(|job| job.claim != Some(claim))
                .map(|job| job.not_before)
                .min();
            Ok((jobs, next))
        })
        .await
        .unwrap_or_default();
    stream::iter(jobs)
        .for_each_concurrent(4, |mut job| async move {
            let result =
                tokio::time::timeout(Duration::from_secs(12), execute_cleanup(s, &job.action))
                    .await;
            let error = match result {
                Ok(Ok(())) => {
                    let _ = s
                        .update(|r| {
                            r.cleanup.retain(|queued| {
                                queued.claim != Some(claim) || queued.action != job.action
                            });
                            Ok(())
                        })
                        .await;
                    return;
                }
                Ok(Err(error)) => error,
                Err(_) => ProviderError::Unavailable,
            };
            job.attempts += 1;
            if !error.transient() || job.attempts >= 5 {
                tracing::warn!(
                    operation = job.action.operation(),
                    attempts = job.attempts,
                    "provider cleanup abandoned; resource may remain until provider expiry"
                );
                let _ = s
                    .update(|r| {
                        r.cleanup.retain(|queued| {
                            queued.claim != Some(claim) || queued.action != job.action
                        });
                        Ok(())
                    })
                    .await;
                return;
            }
            // Delayed, jittered retries only for reads, force-close and revocation.
            // Never repeat auth/validation failures or ambiguous creation/SDP mutations.
            let delay_ms =
                1_000 * (1u64 << job.attempts) + u64::from(Uuid::new_v4().as_bytes()[0]) * 4;
            job.not_before = Timestamp::now() + Duration::from_millis(delay_ms);
            job.claim = None;
            tracing::warn!(
                operation = job.action.operation(),
                attempts = job.attempts,
                delay_ms,
                "provider cleanup retry scheduled"
            );
            let _ = s
                .update(|r| {
                    if let Some(queued) = r
                        .cleanup
                        .iter_mut()
                        .find(|queued| queued.claim == Some(claim) && queued.action == job.action)
                    {
                        *queued = job.clone();
                    }
                    Ok(())
                })
                .await;
        })
        .await;
    next.map_or(CLEANUP_RECONCILE_INTERVAL, |deadline| {
        deadline
            .duration_since(Timestamp::now())
            .min(CLEANUP_RECONCILE_INTERVAL)
    })
}
async fn expire_sessions(s: &AppState) -> Result<(), ApiError> {
    s.update(|r| {
        r.reservations
            .retain(|_, reservation| reservation.started.elapsed() < Duration::from_secs(30));
        let expired: Vec<_> = r
            .participants
            .values()
            .filter(|p| {
                p.lease.elapsed() >= LEASE
                    || p.operation_started
                        .is_some_and(|t| t.elapsed() >= Duration::from_secs(30))
            })
            .map(|p| p.id)
            .collect();
        for id in expired {
            remove_participant_locked(r, id);
        }
        for participant in r.participants.values_mut() {
            prune_turn(participant);
        }
        Ok(())
    })
    .await
}
pub fn spawn_cleanup(s: AppState) {
    let worker = s.clone();
    tokio::spawn(async move {
        let mut shutdown = worker.shutting_down.subscribe();
        while !*shutdown.borrow() {
            let mut wait = CLEANUP_RECONCILE_INTERVAL;
            if let Ok(rooms) = worker.media_rooms().await {
                for room in rooms {
                    wait = wait.min(retry_backlog(&room).await);
                }
            }
            // Local queue changes wake this worker immediately. The slower poll
            // recovers shared work left by a replica that exited after enqueueing.
            tokio::select! {
                _ = shutdown.changed() => break,
                () = worker.cleanup_wakeup.notified() => {},
                () = tokio::time::sleep(wait) => {},
            };
        }
    });
    // Provider cleanup must not delay lease/capability expiry.
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut shutdown = s.shutting_down.subscribe();
        while !*shutdown.borrow() {
            tokio::select! {
                _ = shutdown.changed() => break,
                _ = tick.tick() => {},
            }
            let _expiry_guard = s.expiry_lock.lock().await;
            if *shutdown.borrow() {
                break;
            }
            if let Ok(rooms) = s.media_rooms().await {
                for room in rooms {
                    let _ = room.revoke_media_access().await;
                    let _ = expire_sessions(&room).await;
                }
            }
        }
    });
}

/// Close every registered provider track before process termination.
pub async fn shutdown_cleanup(s: &AppState) {
    s.begin_shutdown();
    // Shared state and cleanup work belong to the service, not this pod. A
    // rollout must not tear down healthy Cloudflare sessions.
    if s.store.is_some() {
        return;
    }
    // Finish any local expiry/enqueue pass before deciding the queue is drained.
    let _expiry_guard = s.expiry_lock.lock().await;
    let rooms = s.media_rooms().await.unwrap_or_else(|_| vec![s.clone()]);
    let result = tokio::time::timeout(Duration::from_secs(20), async {
        for room in &rooms {
            let ids = room
                .read(|r| Ok(r.participants.keys().copied().collect::<Vec<_>>()))
                .await
                .unwrap_or_default();
            for id in ids {
                remove_participant(room, id).await;
            }
        }
        loop {
            let mut remaining = 0;
            for room in &rooms {
                retry_backlog(room).await;
                remaining += room.read(|r| Ok(r.cleanup.len())).await.unwrap_or(1);
            }
            if remaining == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await;
    if result.is_err() {
        tracing::warn!(
            "shutdown cleanup deadline reached; queued or in-flight resources may remain"
        );
    }
}

#[cfg(test)]
mod request_logging_tests;
#[cfg(test)]
mod tests;
