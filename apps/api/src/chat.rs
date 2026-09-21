//! Public demo commands. Persist before publishing; all future content rules
//! belong on this path, never in a gateway or a delete/recreate bot.
use crate::{ApiError, AppState, RuntimeEnvironment, account_token, auth::random_id};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use rand::Rng;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;
use uuid::Uuid;

pub(crate) const TOPIC: &str = "caper:chat:v1:events";
const PAGE: i64 = 50;

#[derive(Clone)]
pub(crate) struct Chat {
    pub pool: PgPool,
    pub broker: redis::Client,
    pub wake: Arc<Notify>,
}

pub(crate) fn unavailable() -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "messages unavailable")
}
fn database_error(_: sqlx::Error) -> ApiError {
    unavailable()
}

impl Chat {
    pub async fn from_env(
        pool: Option<&PgPool>,
        environment: &RuntimeEnvironment,
    ) -> Result<Option<Self>, String> {
        if !environment
            .get("CHAT_ENABLED")
            .is_some_and(|v| v == "true" || v == "1")
        {
            return Ok(None);
        }
        let pool = pool.ok_or("CHAT_ENABLED requires DATABASE_URL")?.clone();
        let url = environment
            .get("VALKEY_URL")
            .ok_or("CHAT_ENABLED requires VALKEY_URL")?;
        let insecure =
            std::env::var("VALKEY_ALLOW_INSECURE").is_ok_and(|v| v == "true" || v == "1");
        crate::media_store::validate_url(&url, insecure).map_err(|_| "invalid chat VALKEY_URL")?;
        let broker = redis::Client::open(url).map_err(|_| "invalid chat VALKEY_URL")?;
        seed(&pool)
            .await
            .map_err(|_| "failed to initialize public chat")?;
        Ok(Some(Self {
            pool,
            broker,
            wake: Arc::new(Notify::new()),
        }))
    }
}

/// Serialized startup seeding, with random external IDs generated only once.
pub(crate) async fn seed(pool: &PgPool) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(731902, 1)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO public.spaces (external_id, name, demo) SELECT $1, 'Public demo', true WHERE NOT EXISTS (SELECT 1 FROM public.spaces WHERE demo)")
        .bind(random_id(12)).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO public.channels (external_id, space_id, name) SELECT $1, id, 'General' FROM public.spaces WHERE demo ON CONFLICT (space_id, name) DO NOTHING")
        .bind(random_id(12)).execute(&mut *tx).await?;
    tx.commit().await
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/chat/general", get(general))
        .route("/api/chat/session", post(session))
        .route(
            "/api/chat/channels/{channel}/messages",
            get(history).post(send),
        )
}

fn enabled(state: &AppState) -> Result<&Chat, ApiError> {
    state.chat.as_ref().ok_or_else(unavailable)
}

// This is deliberately the ONLY authorization policy for this throwaway public
// demo. Future private channels must change both command and gateway checks.
pub(crate) async fn public_channel(pool: &PgPool, id: &str) -> Result<(i64, i64), ApiError> {
    sqlx::query_as("SELECT c.id, c.last_seq FROM public.channels c JOIN public.spaces s ON s.id = c.space_id WHERE c.external_id = $1 AND s.demo AND c.name = 'General'")
        .bind(id).fetch_optional(pool).await.map_err(database_error)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))
}

async fn general(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let (space, name, channel, channel_name): (String, String, String, String) = sqlx::query_as(
        "SELECT s.external_id, s.name, c.external_id, c.name FROM public.spaces s JOIN public.channels c ON c.space_id = s.id WHERE s.demo AND c.name = 'General'")
        .fetch_one(&chat.pool).await.map_err(database_error)?;
    let mut result = history_page(&chat.pool, &channel, None).await?;
    result["space"] = json!({"id":space,"name":name});
    result["channel"] = json!({"id":channel,"name":channel_name});
    Ok(Json(result))
}

#[derive(Deserialize)]
struct HistoryQuery {
    before: Option<String>,
}
async fn history(
    State(state): State<AppState>,
    Path(channel): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<Value>, ApiError> {
    let before = query.before.as_deref().map(cursor).transpose()?;
    Ok(Json(
        history_page(&enabled(&state)?.pool, &channel, before).await?,
    ))
}
pub(crate) fn cursor(value: &str) -> Result<i64, ApiError> {
    value
        .parse::<i64>()
        .ok()
        .filter(|v| *v >= 0)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid cursor"))
}
async fn history_page(
    pool: &PgPool,
    channel: &str,
    before: Option<i64>,
) -> Result<Value, ApiError> {
    let (id, head) = public_channel(pool, channel).await?;
    // Bound by the captured committed head. Later commits are replayed by WS.
    let mut rows: Vec<Value> = sqlx::query_scalar("SELECT payload FROM public.messages WHERE channel_id = $1 AND channel_seq <= $2 AND ($3::bigint IS NULL OR channel_seq < $3) ORDER BY channel_seq DESC LIMIT $4")
        .bind(id).bind(head).bind(before).bind(PAGE + 1).fetch_all(pool).await.map_err(database_error)?;
    let more = rows.len() > PAGE as usize;
    rows.truncate(PAGE as usize);
    rows.reverse();
    Ok(json!({"messages":rows,"cursor":head.to_string(),"hasMore":more}))
}

#[derive(Deserialize)]
struct SessionInput {
    name: String,
}
async fn session(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<SessionInput>,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let account = if let Some(token) = account_token(&headers) {
        Some(state.auth.authenticate(token, Some(&chat.pool)).await?.user)
    } else {
        None
    };
    let name = account
        .as_ref()
        .and_then(|u| u.display_name.as_deref())
        .unwrap_or(&input.name)
        .trim();
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid name"));
    }
    let token = URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 32]>());
    let id = random_id(12);
    let mut tx = chat.pool.begin().await.map_err(database_error)?;
    sqlx::query("SELECT pg_advisory_xact_lock(731902, 2)")
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.chat_sessions WHERE created_at > now() - interval '1 minute'",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if recent >= 60 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "guest creation busy; try again shortly",
        ));
    }
    sqlx::query("INSERT INTO public.chat_sessions (external_id, token_hash, user_id, name, account_session_hash) VALUES ($1, $2, $3, $4, $5)")
        .bind(&id).bind(Sha256::digest(token.as_bytes()).as_slice()).bind(account.as_ref().map(|u| u.id)).bind(name)
        .bind(account_token(&headers).map(|token| Sha256::digest(token.as_bytes()).to_vec()))
        .execute(&mut *tx).await.map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok(Json(
        json!({"token":token,"author":{"id":account.as_ref().map_or(id.as_str(), |u| u.external_id.as_str()),"name":name,"isGuest":account.is_none()}}),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendInput {
    client_message_id: Uuid,
    text: String,
}

/// Single pre-publication boundary. Future replacement rules run here before
/// size validation, persistence, outbox creation, or notifications. No BO2 rule
/// is hard-coded: that was an example, not the demo's policy.
fn prepare_text(text: &str) -> Result<Value, ApiError> {
    if text.trim().is_empty()
        || text.chars().count() > 4000
        || text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "text must contain 1–4000 characters",
        ));
    }
    Ok(json!({"version":1,"type":"text","text":text}))
}

async fn send(
    State(state): State<AppState>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    Json(input): Json<SendInput>,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let token = headers
        .get("x-caper-chat-token")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() <= 128)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "guest session required"))?;
    let payload = persist(
        &chat.pool,
        &channel,
        token,
        input.client_message_id,
        &input.text,
    )
    .await?;
    chat.wake.notify_one();
    Ok(Json(payload))
}

async fn persist(
    pool: &PgPool,
    channel: &str,
    token: &str,
    client_id: Uuid,
    text: &str,
) -> Result<Value, ApiError> {
    let content = prepare_text(text)?;
    let (channel_id, _) = public_channel(pool, channel).await?;
    let mut tx = pool.begin().await.map_err(database_error)?;
    // Channel lock serializes commit order, rate checks, and duplicate requests.
    let head: i64 =
        sqlx::query_scalar("SELECT last_seq FROM public.channels WHERE id = $1 FOR UPDATE")
            .bind(channel_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(database_error)?;
    let author: Option<(i64, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT s.id, COALESCE(u.external_id, s.external_id), COALESCE(u.display_name, s.name), s.user_id FROM public.chat_sessions s LEFT JOIN public.users u ON u.id = s.user_id WHERE s.token_hash = $1 AND s.expires_at > now() AND (s.user_id IS NULL OR (u.deleted_at IS NULL AND EXISTS (SELECT 1 FROM public.account_sessions a WHERE a.token_hash = s.account_session_hash AND a.user_id = s.user_id AND a.revoked_at IS NULL AND a.expires_at > now())))")
        .bind(Sha256::digest(token.as_bytes()).as_slice()).fetch_optional(&mut *tx).await.map_err(database_error)?;
    let (session_id, author_id, name, user_id) =
        author.ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "guest session expired"))?;
    let hash = Sha256::digest(text.as_bytes()).to_vec();
    let existing: Option<(i64, Vec<u8>, Value)> = sqlx::query_as("SELECT session_id, request_hash, payload FROM public.messages WHERE channel_id = $1 AND client_message_id = $2")
        .bind(channel_id).bind(client_id).fetch_optional(&mut *tx).await.map_err(database_error)?;
    if let Some((sender, original, payload)) = existing {
        return if sender == session_id && original == hash {
            Ok(payload)
        } else {
            Err(ApiError::new(
                StatusCode::CONFLICT,
                "retry key already used for different text",
            ))
        };
    }
    let (global, personal): (i64, i64) = sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE session_id = $2) FROM public.messages WHERE channel_id = $1 AND created_at > now() - interval '1 minute'")
        .bind(channel_id).bind(session_id).fetch_one(&mut *tx).await.map_err(database_error)?;
    if global >= 120 || personal >= 30 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "sending too quickly; try again shortly",
        ));
    }
    let seq = head + 1;
    let id = random_id(15);
    let payload = json!({"id":id,"channelId":channel,"seq":seq.to_string(),"author":{"id":author_id,"name":name,"isGuest":user_id.is_none()},"content":content,"createdAt":Utc::now().to_rfc3339(),"clientMessageId":client_id});
    sqlx::query("INSERT INTO public.messages (external_id, channel_id, session_id, client_message_id, request_hash, channel_seq, payload) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(id).bind(channel_id).bind(session_id).bind(client_id).bind(hash).bind(seq).bind(&payload).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("INSERT INTO public.channel_events (channel_id, seq, payload) VALUES ($1,$2,$3)")
        .bind(channel_id).bind(seq).bind(json!({"type":"message.created","schemaVersion":1,"channelId":channel,"seq":seq.to_string(),"message":payload})).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("UPDATE public.channels SET last_seq = $2 WHERE id = $1")
        .bind(channel_id)
        .bind(seq)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    tracing::info!(event_name = "chat_committed", "message committed");
    Ok(payload)
}

/// Every API replica can publish. A transaction-scoped advisory lock prevents
/// concurrent publishers reordering this demo's events; it never locks sends.
pub(crate) fn spawn_publisher(chat: Chat) {
    tokio::spawn(async move {
        loop {
            match publish_pending(&chat).await {
                Ok(true) => continue,
                Ok(false) => {}
                Err(()) => tracing::warn!(
                    event_name = "chat_publish_retry",
                    "outbox publish will retry"
                ),
            }
            tokio::select! { _ = chat.wake.notified() => {}, _ = tokio::time::sleep(Duration::from_secs(1)) => {} }
        }
    });
}
async fn publish_pending(chat: &Chat) -> Result<bool, ()> {
    let mut tx = chat.pool.begin().await.map_err(|_| ())?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(731902, 3)")
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| ())?;
    if !locked {
        return Ok(false);
    }
    let rows: Vec<(i64, i64, Value)> = sqlx::query_as("SELECT channel_id, seq, payload FROM public.channel_events WHERE published_at IS NULL ORDER BY channel_id, seq LIMIT 64")
        .fetch_all(&mut *tx).await.map_err(|_| ())?;
    if rows.is_empty() {
        return Ok(false);
    }
    let mut connection = tokio::time::timeout(
        Duration::from_secs(2),
        chat.broker.get_multiplexed_async_connection(),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())?;
    for (channel, seq, event) in &rows {
        tokio::time::timeout(
            Duration::from_secs(2),
            redis::cmd("PUBLISH")
                .arg(TOPIC)
                .arg(event.to_string())
                .query_async::<i64>(&mut connection),
        )
        .await
        .map_err(|_| ())?
        .map_err(|_| ())?;
        sqlx::query("UPDATE public.channel_events SET published_at = now() WHERE channel_id = $1 AND seq = $2").bind(channel).bind(seq).execute(&mut *tx).await.map_err(|_| ())?;
    }
    // A crash here re-publishes, deliberately. Receivers deduplicate by sequence.
    tx.commit().await.map_err(|_| ())?;
    tracing::info!(
        event_name = "chat_published",
        count = rows.len(),
        "outbox batch published"
    );
    Ok(true)
}

#[cfg(test)]
mod tests;
