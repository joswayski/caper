//! Public demo commands. Persist before publishing; all future content rules
//! belong on this path, never in a gateway or a delete/recreate bot.
use crate::{
    ApiError, AppState, RuntimeEnvironment, account_token,
    auth::random_id,
    spaces::{channel_access, session_user},
};
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
// Separate from durable events: older gateways require a sequence on that topic.
pub(crate) const TYPING_TOPIC: &str = "caper:chat:v1:typing";
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
    sqlx::query("INSERT INTO public.channels (external_id, space_id, name) SELECT $1, id, 'general' FROM public.spaces WHERE demo AND deleted_at IS NULL AND NOT EXISTS (SELECT 1 FROM public.channels c WHERE c.space_id=spaces.id AND lower(c.name)='general' AND c.deleted_at IS NULL)")
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
        .route("/api/chat/channels/{channel}/typing", post(typing))
}

fn enabled(state: &AppState) -> Result<&Chat, ApiError> {
    state.chat.as_ref().ok_or_else(unavailable)
}

async fn general(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let (space, name, channel, channel_name): (String, String, String, String) = sqlx::query_as(
        "SELECT s.external_id, s.name, c.external_id, lower(c.name) FROM public.spaces s JOIN public.channels c ON c.space_id = s.id WHERE s.demo AND lower(c.name) = 'general'")
        .fetch_one(&chat.pool).await.map_err(database_error)?;
    let mut result = history_page(&chat.pool, &channel, None, None).await?;
    // Keep the original demo endpoint's exact metadata source and shape.
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
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let before = query.before.as_deref().map(cursor).transpose()?;
    let user = request_user(&chat.pool, &headers).await?;
    Ok(Json(
        history_page(&chat.pool, &channel, before, user).await?,
    ))
}

async fn request_user(pool: &PgPool, headers: &HeaderMap) -> Result<Option<i64>, ApiError> {
    match account_token(headers) {
        Some(token) => Ok(Some(
            session_user(pool, Sha256::digest(token.as_bytes()).as_slice()).await?,
        )),
        None => Ok(None),
    }
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
    user: Option<i64>,
) -> Result<Value, ApiError> {
    let access = channel_access(pool, channel, user).await?;
    let (space, space_name, channel_name): (String, String, String) = sqlx::query_as(
        "SELECT s.external_id,s.name,lower(c.name) FROM public.channels c JOIN public.spaces s ON s.id=c.space_id WHERE c.id=$1 AND s.id=$2 AND s.demo=$3 AND c.deleted_at IS NULL AND s.deleted_at IS NULL",
    )
    .bind(access.id)
    .bind(access.space_id)
    .bind(access.demo)
    .fetch_optional(pool)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    // Bound by the captured committed head. Later commits are replayed by WS.
    let mut rows: Vec<Value> = sqlx::query_scalar("SELECT payload FROM public.messages WHERE channel_id = $1 AND channel_seq <= $2 AND ($3::bigint IS NULL OR channel_seq < $3) ORDER BY channel_seq DESC LIMIT $4")
        .bind(access.id).bind(access.last_seq).bind(before).bind(PAGE + 1).fetch_all(pool).await.map_err(database_error)?;
    let more = rows.len() > PAGE as usize;
    rows.truncate(PAGE as usize);
    rows.reverse();
    Ok(
        json!({"messages":rows,"cursor":access.last_seq.to_string(),"hasMore":more,
        "space":{"id":space,"name":space_name},"channel":{"id":channel,"name":channel_name}}),
    )
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
    let payload = persist(
        &chat.pool,
        &channel,
        sender_token(&headers)?,
        input.client_message_id,
        &input.text,
    )
    .await?;
    chat.wake.notify_one();
    Ok(Json(payload))
}

fn sender_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get("x-caper-chat-token")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() <= 128)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "guest session required"))
}

async fn authorize_sender(
    connection: &mut sqlx::PgConnection,
    token: &str,
) -> Result<(i64, String, String, Option<i64>), ApiError> {
    sqlx::query_as(
        "SELECT s.id, COALESCE(u.external_id, s.external_id), COALESCE(u.display_name, s.name), s.user_id FROM public.chat_sessions s LEFT JOIN public.users u ON u.id = s.user_id WHERE s.token_hash = $1 AND s.expires_at > now() AND (s.user_id IS NULL OR (u.deleted_at IS NULL AND EXISTS (SELECT 1 FROM public.account_sessions a WHERE a.token_hash = s.account_session_hash AND a.user_id = s.user_id AND a.revoked_at IS NULL AND a.expires_at > now())))")
        .bind(Sha256::digest(token.as_bytes()).as_slice()).fetch_optional(connection).await.map_err(database_error)?
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "guest session expired"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TypingInput {
    typing: bool,
}

async fn typing(
    State(state): State<AppState>,
    Path(channel): Path<String>,
    headers: HeaderMap,
    Json(input): Json<TypingInput>,
) -> Result<StatusCode, ApiError> {
    publish_typing(
        enabled(&state)?,
        &channel,
        sender_token(&headers)?,
        input.typing,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn publish_typing(
    chat: &Chat,
    channel: &str,
    token: &str,
    typing: bool,
) -> Result<(), ApiError> {
    let (_, author_id, name, user_id) = {
        let mut connection = chat.pool.acquire().await.map_err(database_error)?;
        authorize_sender(&mut connection, token).await?
    };
    channel_access(&chat.pool, channel, user_id).await?;
    let event = json!({"type":"typing.updated","channelId":channel,"author":{"id":author_id,"name":name,"isGuest":user_id.is_none()},"typing":typing});
    // Atomic shared limits and publication. No draft text, DB write, outbox, or
    // sequence allocation. Broker time orders duplicate/overlapping streams;
    // keep microseconds as a string rather than rounding through Lua/JS numbers.
    let script = redis::Script::new(
        r#"
        local personal = redis.call('INCR', KEYS[1])
        if personal == 1 then redis.call('EXPIRE', KEYS[1], 1) end
        if personal > 4 then return 0 end
        local global = redis.call('INCR', KEYS[2])
        if global == 1 then redis.call('EXPIRE', KEYS[2], 1) end
        if global > 120 then return 0 end
        local event = cjson.decode(ARGV[2])
        local clock = redis.call('TIME')
        event.revision = clock[1] .. string.format('%06d', tonumber(clock[2]))
        redis.call('PUBLISH', ARGV[1], cjson.encode(event))
        return 1
    "#,
    );
    let published = tokio::time::timeout(Duration::from_secs(2), async {
        let mut connection = chat.broker.get_multiplexed_async_connection().await?;
        // Channel-local counters share a hash slot. Unrelated spaces do not
        // compete for one global typing budget or receive each other's traffic.
        script
            .key(format!("{{{TYPING_TOPIC}:{channel}}}:rate:{author_id}"))
            .key(format!("{{{TYPING_TOPIC}:{channel}}}:rate:channel"))
            .arg(format!("{TYPING_TOPIC}:{channel}"))
            .arg(event.to_string())
            .invoke_async::<i64>(&mut connection)
            .await
    })
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?;
    if published == 0 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "typing updates too frequent",
        ));
    }
    Ok(())
}

async fn persist(
    pool: &PgPool,
    channel: &str,
    token: &str,
    client_id: Uuid,
    text: &str,
) -> Result<Value, ApiError> {
    let content = prepare_text(text)?;
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (session_id, author_id, name, user_id) = authorize_sender(&mut tx, token).await?;
    // Membership mutations lock the space first. Take that lock in a separate
    // statement so the access query gets a fresh READ COMMITTED snapshot after
    // waiting; a predicate in the locking query can see pre-removal grants.
    sqlx::query("SELECT s.id FROM public.spaces s JOIN public.channels c ON c.space_id=s.id WHERE c.external_id=$1 FOR UPDATE OF s")
        .bind(channel)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    let row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT c.id,c.last_seq FROM public.channels c JOIN public.spaces s ON s.id=c.space_id
         WHERE c.external_id=$1 AND c.deleted_at IS NULL AND s.deleted_at IS NULL
           AND ((s.demo AND lower(c.name)='general') OR
                ($2::bigint IS NOT NULL
                 AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$2)
                 AND (s.owner_id=$2 OR NOT c.private OR EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=c.id AND cm.user_id=$2))))
         FOR UPDATE OF c",
    )
    .bind(channel)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    let (channel_id, head) =
        row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
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

/// API replicas claim disjoint outbox batches. Broker arrival order is not an
/// ordering authority: gateways merge/replay the committed channel sequences.
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
    let rows: Vec<(i64, i64, Value)> = sqlx::query_as("SELECT channel_id, seq, payload FROM public.channel_events WHERE published_at IS NULL ORDER BY channel_id, seq LIMIT 64 FOR UPDATE SKIP LOCKED")
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
                .arg(format!(
                    "{TOPIC}:{}",
                    event["channelId"].as_str().ok_or(())?
                ))
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
