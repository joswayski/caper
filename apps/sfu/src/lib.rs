//! A self-hosted, audio-only SFU exposing the subset of the Cloudflare Realtime
//! SFU and TURN HTTP APIs that the Caper API uses. Pointing the API's
//! `MEDIA_PROVIDER_BASE` here replaces Cloudflare without client changes.

pub mod engine;
pub mod turn;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use engine::{ApiError, Command, Description, TrackRequest};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use subtle::ConstantTimeEq;
use tokio::sync::{mpsc, oneshot};

const BODY_LIMIT: usize = 256 * 1024;
const ENGINE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct Credentials {
    pub app_id: String,
    pub app_secret: String,
    pub turn_key_id: String,
    pub turn_api_token: String,
}

#[derive(Clone)]
pub struct AppState {
    engine: mpsc::Sender<Command>,
    credentials: Arc<Credentials>,
    turn: Arc<turn::Turn>,
}

impl AppState {
    #[must_use]
    pub fn new(engine: mpsc::Sender<Command>, credentials: Credentials, turn: turn::Turn) -> Self {
        Self {
            engine,
            credentials: Arc::new(credentials),
            turn: Arc::new(turn),
        }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(|| async { StatusCode::NO_CONTENT }))
        .route("/v1/apps/{app}/stats", get(stats))
        .route("/v1/apps/{app}/sessions/new", post(create_session))
        .route("/v1/apps/{app}/sessions/{session}", get(get_session))
        .route("/v1/apps/{app}/sessions/{session}/tracks/new", post(tracks_new))
        .route(
            "/v1/apps/{app}/sessions/{session}/renegotiate",
            put(renegotiate),
        )
        .route(
            "/v1/apps/{app}/sessions/{session}/tracks/close",
            put(close_tracks),
        )
        .route(
            "/v1/turn/keys/{key}/credentials/generate-ice-servers",
            post(generate_ice_servers),
        )
        .route(
            "/v1/turn/keys/{key}/credentials/{username}/revoke",
            post(revoke),
        )
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state)
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::BAD_REQUEST);
        (
            status,
            Json(json!({"errorCode": self.code, "errorDescription": self.description})),
        )
            .into_response()
    }
}

fn bearer_matches(headers: &HeaderMap, secret: &str) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| bool::from(token.as_bytes().ct_eq(secret.as_bytes())))
}

fn authorize_app(s: &AppState, headers: &HeaderMap, app: &str) -> Result<(), ApiError> {
    if app == s.credentials.app_id && bearer_matches(headers, &s.credentials.app_secret) {
        Ok(())
    } else {
        Err(unauthorized())
    }
}

fn authorize_turn(s: &AppState, headers: &HeaderMap, key: &str) -> Result<(), ApiError> {
    if key == s.credentials.turn_key_id && bearer_matches(headers, &s.credentials.turn_api_token) {
        Ok(())
    } else {
        Err(unauthorized())
    }
}

fn unauthorized() -> ApiError {
    ApiError {
        status: 401,
        code: "authentication_error",
        description: "invalid credentials".into(),
    }
}

async fn call<T>(
    s: &AppState,
    command: impl FnOnce(oneshot::Sender<Result<T, ApiError>>) -> Command,
) -> Result<T, ApiError> {
    let unavailable = || ApiError {
        status: 503,
        code: "unavailable_error",
        description: "media engine unavailable".into(),
    };
    let (reply, receive) = oneshot::channel();
    s.engine
        .send(command(reply))
        .await
        .map_err(|_| unavailable())?;
    tokio::time::timeout(ENGINE_TIMEOUT, receive)
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?
}

/// Parses an optional JSON body; session creation has none.
fn body<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<Option<T>, ApiError> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    serde_json::from_slice(bytes).map(Some).map_err(|error| ApiError {
        status: 400,
        code: "invalid_request_error",
        description: error.to_string(),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionBody {
    session_description: Option<Description>,
}

async fn create_session(
    State(s): State<AppState>,
    Path(app): Path<String>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize_app(&s, &headers, &app)?;
    let offer = body::<SessionBody>(&bytes)?
        .and_then(|b| b.session_description)
        .map(|d| {
            if d.kind == "offer" {
                Ok(d.sdp)
            } else {
                Err(ApiError {
                    status: 400,
                    code: "invalid_request_error",
                    description: "sessionDescription must be an offer".into(),
                })
            }
        })
        .transpose()?;
    let value = call(&s, |reply| Command::Create { offer, reply }).await?;
    Ok((StatusCode::CREATED, Json(value)))
}

async fn get_session(
    State(s): State<AppState>,
    Path((app, session)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    authorize_app(&s, &headers, &app)?;
    call(&s, |reply| Command::Get { session, reply })
        .await
        .map(Json)
}

async fn stats(
    State(s): State<AppState>,
    Path(app): Path<String>,
    headers: HeaderMap,
) -> Result<Json<engine::Stats>, ApiError> {
    authorize_app(&s, &headers, &app)?;
    let (reply, receive) = oneshot::channel();
    let _ = s.engine.send(Command::Stats { reply }).await;
    tokio::time::timeout(ENGINE_TIMEOUT, receive)
        .await
        .ok()
        .and_then(Result::ok)
        .map(Json)
        .ok_or(ApiError {
            status: 503,
            code: "unavailable_error",
            description: "media engine unavailable".into(),
        })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TracksBody {
    session_description: Option<Description>,
    #[serde(default)]
    tracks: Vec<TrackRequest>,
}

async fn tracks_new(
    State(s): State<AppState>,
    Path((app, session)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<TracksBody>,
) -> Result<Json<Value>, ApiError> {
    authorize_app(&s, &headers, &app)?;
    call(&s, |reply| Command::Tracks {
        session,
        offer: input.session_description,
        tracks: input.tracks,
        reply,
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenegotiateBody {
    session_description: Description,
}

async fn renegotiate(
    State(s): State<AppState>,
    Path((app, session)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<RenegotiateBody>,
) -> Result<Json<Value>, ApiError> {
    authorize_app(&s, &headers, &app)?;
    call(&s, |reply| Command::Renegotiate {
        session,
        answer: input.session_description,
        reply,
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct CloseTrack {
    mid: String,
}

#[derive(Deserialize)]
struct CloseBody {
    tracks: Vec<CloseTrack>,
}

async fn close_tracks(
    State(s): State<AppState>,
    Path((app, session)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<CloseBody>,
) -> Result<Json<Value>, ApiError> {
    authorize_app(&s, &headers, &app)?;
    let mids = input.tracks.into_iter().map(|t| t.mid).collect();
    call(&s, |reply| Command::Close {
        session,
        mids,
        reply,
    })
    .await
    .map(Json)
}

#[derive(Deserialize)]
struct TurnBody {
    ttl: Option<u64>,
}

async fn generate_ice_servers(
    State(s): State<AppState>,
    Path(key): Path<String>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    authorize_turn(&s, &headers, &key)?;
    let ttl = body::<TurnBody>(&bytes)?.and_then(|b| b.ttl);
    Ok((
        StatusCode::CREATED,
        Json(json!({ "iceServers": s.turn.ice_servers(ttl) })),
    ))
}

/// TURN REST credentials cannot be revoked before they expire; accept the
/// request so the API's cleanup completes.
async fn revoke(
    State(s): State<AppState>,
    Path((key, _username)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    authorize_turn(&s, &headers, &key)?;
    Ok(StatusCode::NO_CONTENT)
}
