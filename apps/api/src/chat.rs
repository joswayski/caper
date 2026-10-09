//! Channel and direct-message commands. Persist before publishing; content rules
//! belong on this path, never in a gateway or a delete/recreate bot.
#[cfg(test)]
use crate::spaces::channel_access;
use crate::{
    ApiError, AppState, RuntimeEnvironment, account_token,
    auth::random_id,
    mentions::{self, Mention},
    spaces::{channel_participation, session_user},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post, put},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use rand::Rng;
use redis::aio::MultiplexedConnection;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{
    sync::{Arc, LazyLock},
    time::Duration,
};
use tokio::sync::{Mutex, Notify};
use uuid::Uuid;

mod editing;
mod forwarding;

pub(crate) const TOPIC: &str = "caper:chat:v1:events";
// Separate from durable events: older gateways require a sequence on that topic.
pub(crate) const TYPING_TOPIC: &str = "caper:chat:v1:typing";
const PAGE: i64 = 50;
const BROKER_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub(crate) struct Chat {
    pub pool: PgPool,
    pub broker: redis::Client,
    pub wake: Arc<Notify>,
    /// One multiplexed broker connection shared by outbox publishes and typing
    /// updates, instead of a new TCP/TLS handshake per message. A failed call
    /// drops it so the next caller reconnects.
    connection: Arc<Mutex<Option<MultiplexedConnection>>>,
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
        Ok(Some(Self::new(pool, broker)))
    }

    pub(crate) fn new(pool: PgPool, broker: redis::Client) -> Self {
        Self {
            pool,
            broker,
            wake: Arc::new(Notify::new()),
            connection: Arc::new(Mutex::new(None)),
        }
    }

    async fn broker_connection(&self) -> redis::RedisResult<MultiplexedConnection> {
        let mut cached = self.connection.lock().await;
        if let Some(connection) = cached.as_ref() {
            return Ok(connection.clone());
        }
        let connection = self
            .broker
            .get_multiplexed_async_connection_with_config(
                &redis::AsyncConnectionConfig::new()
                    .set_connection_timeout(BROKER_TIMEOUT)
                    .set_response_timeout(BROKER_TIMEOUT),
            )
            .await?;
        *cached = Some(connection.clone());
        Ok(connection)
    }

    /// Never replays the failed command; only the next caller reconnects.
    async fn discard_broker_connection(&self) {
        *self.connection.lock().await = None;
    }
}

/// Serialized startup seeding, with random external IDs generated only once.
#[cfg(test)]
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
        .route("/api/chat/session", post(session))
        .route(
            "/api/chat/channels/{channel}/messages",
            get(history).post(send),
        )
        .route(
            "/api/chat/channels/{channel}/messages/{message}",
            get(editing::message).put(editing::edit),
        )
        .route(
            "/api/chat/channels/{channel}/messages/{message}/versions",
            get(editing::versions),
        )
        .route(
            "/api/chat/channels/{channel}/messages/{message}/reactions",
            get(reactors).put(set_reaction),
        )
        .route(
            "/api/chat/channels/{channel}/messages/{message}/pin",
            put(set_pin),
        )
        .route(
            "/api/chat/channels/{channel}/messages/{message}/thread",
            get(thread_history),
        )
        .route("/api/chat/channels/{channel}/typing", post(typing))
        .merge(forwarding::routes())
}

fn enabled(state: &AppState) -> Result<&Chat, ApiError> {
    state.chat.as_ref().ok_or_else(unavailable)
}

#[derive(Default, Deserialize)]
struct HistoryQuery {
    before: Option<String>,
    after: Option<String>,
    around: Option<String>,
}
impl HistoryQuery {
    fn validate(&self) -> Result<(), ApiError> {
        if [
            self.before.is_some(),
            self.after.is_some(),
            self.around.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
            > 1
        {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "use one history anchor",
            ));
        }
        self.before.as_deref().map(cursor).transpose()?;
        self.after.as_deref().map(cursor).transpose()?;
        Ok(())
    }
}
async fn history(
    State(state): State<AppState>,
    Path(channel): Path<String>,
    Query(query): Query<HistoryQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    query.validate()?;
    let user = request_user(&chat.pool, &headers).await?;
    Ok(Json(
        conversation_window(&chat.pool, &channel, &query, user, None).await?,
    ))
}

async fn thread_history(
    State(state): State<AppState>,
    Path((channel, root)): Path<(String, String)>,
    Query(query): Query<HistoryQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    query.validate()?;
    let user = request_user(&chat.pool, &headers).await?;
    Ok(Json(
        conversation_window(&chat.pool, &channel, &query, user, Some(&root)).await?,
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
#[cfg(test)]
async fn history_page(
    pool: &PgPool,
    channel: &str,
    before: Option<i64>,
    user: Option<i64>,
) -> Result<Value, ApiError> {
    conversation_page(pool, channel, before, user, None).await
}

#[cfg(test)]
async fn conversation_page(
    pool: &PgPool,
    channel: &str,
    before: Option<i64>,
    user: Option<i64>,
    root: Option<&str>,
) -> Result<Value, ApiError> {
    conversation_window(
        pool,
        channel,
        &HistoryQuery {
            before: before.map(|value| value.to_string()),
            ..HistoryQuery::default()
        },
        user,
        root,
    )
    .await
}

async fn conversation_window(
    pool: &PgPool,
    channel: &str,
    query: &HistoryQuery,
    user: Option<i64>,
    root: Option<&str>,
) -> Result<Value, ApiError> {
    query.validate()?;
    // Hold a shared channel lock from captured head through payload reads. A
    // reaction commit takes the exclusive lock, so a page can never contain a
    // reactionSeq newer than its cursor (nor omit a reaction at that cursor).
    let mut tx = pool.begin().await.map_err(database_error)?;
    // Membership edits lock the space first. Read permissions only after this
    // lock is granted so a waiting history request cannot use revoked grants.
    let space_id: Option<i64> =
        sqlx::query_scalar("SELECT space_id FROM public.channels WHERE external_id=$1")
            .bind(channel)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    if let Some(space_id) = space_id {
        sqlx::query("SELECT id FROM public.spaces WHERE id=$1 FOR SHARE")
            .bind(space_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    }
    let access: Option<(i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT c.id,c.last_seq,c.space_id FROM public.channels c LEFT JOIN public.spaces s ON s.id=c.space_id
         WHERE c.external_id=$1 AND c.deleted_at IS NULL AND $2::bigint IS NOT NULL
           AND ((s.deleted_at IS NULL AND NOT s.demo
                 AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$2 AND sm.deleted_at IS NULL)
                 AND (s.owner_id=$2 OR NOT c.private OR EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=c.id AND cm.user_id=$2 AND cm.deleted_at IS NULL)))
                OR (c.space_id IS NULL AND EXISTS(SELECT 1 FROM public.direct_conversations d
                    JOIN public.users lo ON lo.id=d.low_user_id JOIN public.users hi ON hi.id=d.high_user_id
                    WHERE d.channel_id=c.id AND $2 IN (d.low_user_id,d.high_user_id) AND lo.deleted_at IS NULL AND hi.deleted_at IS NULL)))
         FOR SHARE OF c")
        .bind(channel).bind(user).fetch_optional(&mut *tx).await.map_err(database_error)?;
    let (channel_id, head, space_id) =
        access.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    let (space, space_name, channel_name): (String, String, String) = sqlx::query_as(
        "SELECT s.external_id,s.name,lower(c.name) FROM public.channels c JOIN public.spaces s ON s.id=c.space_id WHERE c.id=$1 AND s.id=$2 AND NOT s.demo AND c.deleted_at IS NULL AND s.deleted_at IS NULL
         UNION ALL SELECT '', 'Direct messages',u.display_name FROM public.direct_conversations d JOIN public.users u ON u.id=CASE WHEN d.low_user_id=$3 THEN d.high_user_id ELSE d.low_user_id END WHERE d.channel_id=$1 AND $3 IN (d.low_user_id,d.high_user_id) AND u.deleted_at IS NULL",
    )
    .bind(channel_id)
    .bind(space_id)
    .bind(user)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    let thread_root: Option<(i64, Value, Option<i16>, Option<String>)> = if let Some(root) = root {
        Some(sqlx::query_as("SELECT m.id,m.payload,u.avatar_id,u.display_name FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.channel_id=$1 AND m.external_id=$2 AND m.thread_root_id IS NULL")
            .bind(channel_id).bind(root).fetch_optional(&mut *tx).await.map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "thread not found"))?)
    } else {
        None
    };
    let root_id = thread_root.as_ref().map(|row| row.0);
    // Anchors are resolved only inside this authorized conversation. Sequence
    // numbers include edits/reactions, so context limits count rows, not events.
    let anchor: Option<i64> = if let Some(message) = &query.around {
        Some(sqlx::query_scalar("SELECT channel_seq FROM public.messages WHERE channel_id=$1 AND external_id=$2 AND (($3::bigint IS NULL AND (thread_root_id IS NULL OR broadcast)) OR thread_root_id=$3)")
            .bind(channel_id).bind(message).bind(root_id).fetch_optional(&mut *tx).await.map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "message not found"))?)
    } else {
        None
    };
    let before = query.before.as_deref().map(cursor).transpose()?;
    let after = query.after.as_deref().map(cursor).transpose()?;
    let limit = if anchor.is_some() { 32 } else { PAGE + 1 };
    // Bound by the captured committed head. Later commits are replayed by WS.
    let mut rows: Vec<(Value, Option<i16>, Option<String>)> = sqlx::query_as("SELECT m.payload,u.avatar_id,u.display_name FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.channel_id=$1 AND m.channel_seq <= $2 AND ($3::bigint IS NULL OR m.channel_seq < $3) AND ($4::bigint IS NULL OR m.channel_seq > $4) AND ($5::bigint IS NULL OR m.channel_seq <= $5) AND (($6::bigint IS NULL AND (m.thread_root_id IS NULL OR m.broadcast)) OR m.thread_root_id=$6) ORDER BY CASE WHEN $4::bigint IS NOT NULL THEN m.channel_seq END ASC, m.channel_seq DESC LIMIT $7")
        .bind(channel_id).bind(head).bind(before).bind(after).bind(anchor).bind(root_id).bind(limit).fetch_all(&mut *tx).await.map_err(database_error)?;
    let page_size = if anchor.is_some() { 31 } else { PAGE as usize };
    let overflow = rows.len() > page_size;
    rows.truncate(page_size);
    let more = after.is_none() && overflow;
    let mut newer = after.is_some() && overflow;
    if after.is_none() {
        rows.reverse();
    }
    if let Some(anchor) = anchor {
        let mut following: Vec<(Value, Option<i16>, Option<String>)> = sqlx::query_as("SELECT m.payload,u.avatar_id,u.display_name FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.channel_id=$1 AND m.channel_seq <= $2 AND m.channel_seq > $3 AND (($4::bigint IS NULL AND (m.thread_root_id IS NULL OR m.broadcast)) OR m.thread_root_id=$4) ORDER BY m.channel_seq ASC LIMIT 31")
            .bind(channel_id).bind(head).bind(anchor).bind(root_id).fetch_all(&mut *tx).await.map_err(database_error)?;
        newer = following.len() > 30;
        following.truncate(30);
        rows.extend(following);
    }
    let mut rows: Vec<Value> = rows
        .into_iter()
        .map(|(payload, avatar, name)| enrich_author(payload, avatar, name.as_deref()))
        .collect();
    forwarding::hydrate(&mut tx, &mut rows).await?;
    if let Some((_, root, avatar, name)) = thread_root {
        let mut roots = vec![enrich_author(root, avatar, name.as_deref())];
        forwarding::hydrate(&mut tx, &mut roots).await?;
        return Ok(
            json!({"root":roots[0],"messages":rows,"cursor":head.to_string(),"hasMore":more,"hasNewer":newer}),
        );
    }
    // All pins are returned independently of the history page. The shared
    // channel lock also makes their revisions consistent with this cursor.
    // Older pages only extend the timeline; every client keeps the pins from
    // the first page and live events, so skip rebuilding them per scroll.
    let mut pins: Vec<Value> = Vec::new();
    if before.is_none() {
        let rows: Vec<(Value, Option<i16>, Option<String>)> = sqlx::query_as("SELECT m.payload,u.avatar_id,u.display_name FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.channel_id=$1 AND jsonb_typeof(m.payload->'pin')='object' ORDER BY (m.payload->>'pinSeq')::bigint DESC LIMIT 100")
            .bind(channel_id).fetch_all(&mut *tx).await.map_err(database_error)?;
        pins = rows
            .into_iter()
            .map(|(payload, avatar, name)| enrich_author(payload, avatar, name.as_deref()))
            .collect();
        forwarding::hydrate(&mut tx, &mut pins).await?;
    }
    let mut channel_identity = json!({"id":channel,"name":channel_name});
    if space_id.is_none() {
        channel_identity["direct"] = json!(true);
    }
    Ok(
        json!({"messages":rows,"pinnedMessages":pins,"cursor":head.to_string(),"hasMore":more,"hasNewer":newer,
        "space":{"id":space,"name":space_name},"channel":channel_identity}),
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
    let token = account_token(&headers)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "sign in required"))?;
    let account = state.auth.authenticate(token, Some(&chat.pool)).await?.user;
    // Messages always carry the account's own profile name; the requested name
    // is kept in the contract for older clients but never shown to anyone.
    let _ = input.name;
    let Some(name) = account
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|_| account.onboarded())
    else {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "complete profile required",
        ));
    };
    if name.is_empty() || name.chars().count() > 64 || name.chars().any(char::is_control) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid name"));
    }
    let token = URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 32]>());
    let id = random_id(12);
    let mut tx = chat.pool.begin().await.map_err(database_error)?;
    // Sessions require an account, so limit each account rather than the whole
    // service: clients mint one per conversation they open, and a site-wide
    // budget let a few busy (or retrying) accounts lock everyone out of chat.
    sqlx::query("SELECT pg_advisory_xact_lock(731905, hashtext($1::bigint::text))")
        .bind(account.id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.chat_sessions WHERE user_id = $1 AND created_at > now() - interval '1 minute'",
    )
    .bind(account.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if recent >= 60 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "opening conversations too quickly; try again shortly",
        ));
    }
    sqlx::query("INSERT INTO public.chat_sessions (external_id, token_hash, user_id, name, account_session_hash) VALUES ($1, $2, $3, $4, $5)")
        .bind(&id).bind(Sha256::digest(token.as_bytes()).as_slice()).bind(account.id).bind(name)
        .bind(account_token(&headers).map(|token| Sha256::digest(token.as_bytes()).to_vec()))
        .execute(&mut *tx).await.map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok(Json(
        json!({"token":token,"author":{"id":account.external_id,"name":name,"isGuest":false,"avatarId":account.avatar_id}}),
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendInput {
    client_message_id: Uuid,
    text: String,
    thread_root_id: Option<String>,
    #[serde(default)]
    broadcast: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReactionInput {
    emoji: String,
    active: bool,
}

fn normalized_emoji(value: &str) -> Result<&'static str, ApiError> {
    emojis::get(value)
        // Keep the API and the bundled web artwork on the same emoji version.
        .filter(|emoji| emoji.emoji_version() <= emojis::EmojiVersion::new(15, 0))
        .map(|emoji| emoji.as_str())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "emoji must be one supported Unicode emoji (through Emoji 15.0)",
            )
        })
}

async fn set_reaction(
    State(state): State<AppState>,
    Path((channel, message)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<ReactionInput>,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let event = persist_reaction(
        &chat.pool,
        &channel,
        &message,
        sender_token(&headers)?,
        normalized_emoji(&input.emoji)?,
        input.active,
    )
    .await?;
    chat.wake.notify_one();
    Ok(Json(event))
}

async fn reactors(
    State(state): State<AppState>,
    Path((channel, message)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let user = request_user(&chat.pool, &headers).await?;
    Ok(Json(
        reactor_list(&chat.pool, &channel, &message, user).await?,
    ))
}

/// Emoji, then the reactor's public ID, username, display name and avatar.
type ReactorRow = (String, String, Option<String>, Option<String>, i16);

/// Who reacted, for hover text and long-press sheets. Snapshots carry only
/// public IDs; names and avatars load on demand with the same read access as
/// history. Emoji keep snapshot order; people are listed in reaction order.
async fn reactor_list(
    pool: &PgPool,
    channel: &str,
    message: &str,
    user: Option<i64>,
) -> Result<Value, ApiError> {
    let access = crate::spaces::channel_access(pool, channel, user).await?;
    let (message_id, reaction_seq): (i64, Option<String>) = sqlx::query_as(
        "SELECT id,payload->>'reactionSeq' FROM public.messages WHERE channel_id=$1 AND external_id=$2",
    )
    .bind(access.id)
    .bind(message)
    .fetch_optional(pool)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "message not found"))?;
    let rows: Vec<ReactorRow> = sqlx::query_as(
        "SELECT r.emoji,u.external_id,u.username,u.display_name,u.avatar_id
         FROM public.message_reactions r JOIN public.users u ON u.id=r.user_id
         WHERE r.message_id=$1 AND r.deleted_at IS NULL ORDER BY r.emoji,r.created_at,r.id",
    )
    .bind(message_id)
    .fetch_all(pool)
    .await
    .map_err(database_error)?;
    let mut reactions: Vec<Value> = Vec::new();
    for (emoji, id, username, display_name, avatar_id) in rows {
        let author =
            json!({"id":id,"username":username,"displayName":display_name,"avatarId":avatar_id});
        match reactions.last_mut() {
            Some(last) if last["emoji"] == emoji.as_str() => last["authors"]
                .as_array_mut()
                .expect("authors")
                .push(author),
            _ => reactions.push(json!({"emoji":emoji,"authors":[author]})),
        }
    }
    Ok(
        json!({"messageId":message,"reactionSeq":reaction_seq.unwrap_or_else(|| "0".into()),"reactions":reactions}),
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinInput {
    active: bool,
}

async fn set_pin(
    State(state): State<AppState>,
    Path((channel, message)): Path<(String, String)>,
    headers: HeaderMap,
    Json(input): Json<PinInput>,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let event = persist_pin(
        &chat.pool,
        &channel,
        &message,
        sender_token(&headers)?,
        input.active,
    )
    .await?;
    chat.wake.notify_one();
    Ok(Json(event))
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
    let payload = persist_message(
        &chat.pool,
        &channel,
        sender_token(&headers)?,
        input.client_message_id,
        &input.text,
        input.thread_root_id.as_deref(),
        input.broadcast,
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
) -> Result<(i64, String, String, Option<i64>, Option<i16>), ApiError> {
    sqlx::query_as(
        "SELECT s.id, COALESCE(u.external_id, s.external_id), COALESCE(u.display_name, s.name), s.user_id, u.avatar_id FROM public.chat_sessions s LEFT JOIN public.users u ON u.id = s.user_id WHERE s.token_hash = $1 AND s.expires_at > now() AND (s.user_id IS NULL OR (u.deleted_at IS NULL AND u.username IS NOT NULL AND u.display_name IS NOT NULL AND EXISTS (SELECT 1 FROM public.account_sessions a WHERE a.token_hash = s.account_session_hash AND a.user_id = s.user_id AND a.revoked_at IS NULL AND a.expires_at > now())))")
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

static TYPING_SCRIPT: LazyLock<redis::Script> = LazyLock::new(|| {
    redis::Script::new(
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
    )
});

async fn publish_typing(
    chat: &Chat,
    channel: &str,
    token: &str,
    typing: bool,
) -> Result<(), ApiError> {
    let (_, author_id, name, user_id, avatar_id) = {
        let mut connection = chat.pool.acquire().await.map_err(database_error)?;
        authorize_sender(&mut connection, token).await?
    };
    let access = channel_participation(&chat.pool, channel, user_id).await?;
    if let (None, Some(user_id)) = (access.space_id, user_id) {
        let mut connection = chat.pool.acquire().await.map_err(database_error)?;
        crate::direct::authorize_interaction(&mut connection, access.id, user_id).await?;
    }
    let event = json!({"type":"typing.updated","channelId":channel,"author":{"id":author_id,"name":name,"isGuest":user_id.is_none(),"avatarId":avatar_id},"typing":typing});
    // Atomic shared limits and publication. No draft text, DB write, outbox, or
    // sequence allocation. Broker time orders duplicate/overlapping streams;
    // keep microseconds as a string rather than rounding through Lua/JS numbers.
    let published = tokio::time::timeout(BROKER_TIMEOUT, async {
        let mut connection = chat.broker_connection().await?;
        // Channel-local counters share a hash slot. Unrelated spaces do not
        // compete for one global typing budget or receive each other's traffic.
        TYPING_SCRIPT
            .key(format!("{{{TYPING_TOPIC}:{channel}}}:rate:{author_id}"))
            .key(format!("{{{TYPING_TOPIC}:{channel}}}:rate:channel"))
            .arg(format!("{TYPING_TOPIC}:{channel}"))
            .arg(event.to_string())
            .invoke_async::<i64>(&mut connection)
            .await
    })
    .await;
    let Ok(Ok(published)) = published else {
        chat.discard_broker_connection().await;
        return Err(unavailable());
    };
    if published == 0 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "typing updates too frequent",
        ));
    }
    Ok(())
}

#[cfg(test)]
async fn persist(
    pool: &PgPool,
    channel: &str,
    token: &str,
    client_id: Uuid,
    text: &str,
) -> Result<Value, ApiError> {
    persist_message(pool, channel, token, client_id, text, None, false).await
}

async fn persist_message(
    pool: &PgPool,
    channel: &str,
    token: &str,
    client_id: Uuid,
    text: &str,
    root: Option<&str>,
    broadcast: bool,
) -> Result<Value, ApiError> {
    let content = prepare_text(text)?;
    if broadcast && root.is_none() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "only thread replies can also be sent to the channel",
        ));
    }
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (session_id, author_id, name, user_id, avatar_id) =
        authorize_sender(&mut tx, token).await?;
    let space_id: Option<i64> =
        sqlx::query_scalar("SELECT space_id FROM public.channels WHERE external_id=$1")
            .bind(channel)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    // Membership mutations lock the space first. Take that lock in a separate
    // statement so the access query gets a fresh READ COMMITTED snapshot after
    // waiting; a predicate in the locking query can see pre-removal grants.
    // Shared, so writes in other channels of the space are not serialized
    // behind this one; the channel row lock below orders this channel.
    if let Some(space_id) = space_id {
        sqlx::query("SELECT id FROM public.spaces WHERE id=$1 FOR SHARE")
            .bind(space_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    }
    let row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT c.id,c.last_seq FROM public.channels c LEFT JOIN public.spaces s ON s.id=c.space_id
         WHERE c.external_id=$1 AND c.deleted_at IS NULL AND $2::bigint IS NOT NULL
           AND ((s.deleted_at IS NULL AND NOT s.demo
                 AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$2 AND sm.deleted_at IS NULL)
                 AND (s.owner_id=$2 OR NOT c.private OR EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=c.id AND cm.user_id=$2 AND cm.deleted_at IS NULL))
                 AND EXISTS(SELECT 1 FROM public.channel_joins cj WHERE cj.channel_id=c.id AND cj.user_id=$2 AND cj.deleted_at IS NULL))
                OR (c.space_id IS NULL AND EXISTS(SELECT 1 FROM public.direct_conversations d
                    JOIN public.users lo ON lo.id=d.low_user_id JOIN public.users hi ON hi.id=d.high_user_id
                    WHERE d.channel_id=c.id AND $2 IN (d.low_user_id,d.high_user_id) AND lo.deleted_at IS NULL AND hi.deleted_at IS NULL)))
         FOR UPDATE OF c",
    )
    .bind(channel)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    let (channel_id, head) =
        row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    if let (None, Some(user_id)) = (space_id, user_id) {
        crate::direct::authorize_send(&mut tx, channel_id, user_id).await?;
    }
    // Preserve existing normal-message retry hashes, but bind reply retries to
    // their destination and broadcast choice as well as their text.
    let hash = Sha256::digest(text.as_bytes()).to_vec();
    let existing: Option<(i64, Vec<u8>, Value)> = sqlx::query_as("SELECT session_id, request_hash, payload FROM public.messages WHERE channel_id = $1 AND client_message_id = $2")
        .bind(channel_id).bind(client_id).fetch_optional(&mut *tx).await.map_err(database_error)?;
    if let Some((sender, original, payload)) = existing {
        return if sender == session_id
            && original == hash
            && payload.get("threadRootId").and_then(Value::as_str) == root
            && payload["broadcast"].as_bool().unwrap_or(false) == broadcast
        {
            Ok(enrich_author(
                payload,
                avatar_id,
                user_id.map(|_| name.as_str()),
            ))
        } else {
            Err(ApiError::new(
                StatusCode::CONFLICT,
                "retry key already used for a different message",
            ))
        };
    }
    let thread_root: Option<(i64, Value)> = if let Some(root) = root {
        Some(sqlx::query_as("SELECT id,payload FROM public.messages WHERE channel_id=$1 AND external_id=$2 AND thread_root_id IS NULL")
            .bind(channel_id).bind(root).fetch_optional(&mut *tx).await.map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "thread not found"))?)
    } else {
        None
    };
    // The personal budget is per account, not per chat session, so extra
    // sessions can't be used to take a whole channel's budget.
    let (global, personal): (i64, i64) = sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE m.session_id = $2 OR cs.user_id = $3) FROM public.messages m JOIN public.chat_sessions cs ON cs.id = m.session_id WHERE m.channel_id = $1 AND m.created_at > now() - interval '1 minute'")
        .bind(channel_id).bind(session_id).bind(user_id).fetch_one(&mut *tx).await.map_err(database_error)?;
    if global >= 120 || personal >= 30 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "sending too quickly; try again shortly",
        ));
    }
    let content =
        with_mentions(&mut tx, content, &mentions::parse(text, space_id.is_some())).await?;
    let seq = head + 1;
    let id = random_id(15);
    let mut payload = json!({"id":id,"channelId":channel,"seq":seq.to_string(),"author":{"id":author_id,"name":name,"isGuest":user_id.is_none(),"avatarId":avatar_id},"content":content,"createdAt":Utc::now().to_rfc3339(),"clientMessageId":client_id});
    if let Some((root_id, original)) = &thread_root {
        let mut participants = original["thread"]["participants"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if let Some(participant) = participants
            .iter_mut()
            .find(|person| person["id"] == author_id)
        {
            *participant = payload["author"].clone();
        } else if participants.len() < 5 {
            participants.push(payload["author"].clone());
        }
        let summary = json!({"replyCount":original["thread"]["replyCount"].as_u64().unwrap_or(0)+1,"participants":participants,"seq":seq.to_string()});
        payload["threadRootId"] = json!(root);
        payload["broadcast"] = json!(broadcast);
        payload["thread"] = summary.clone();
        // The summary and reply/outbox share the channel lock and transaction.
        // Every reply carries this revision so replay can refresh its parent
        // without a second event or a gap in the shared channel sequence.
        sqlx::query(
            "UPDATE public.messages SET payload=jsonb_set(payload,'{thread}',$2) WHERE id=$1",
        )
        .bind(root_id)
        .bind(summary)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    }
    let message_id: i64 = sqlx::query_scalar("INSERT INTO public.messages (external_id, channel_id, session_id, client_message_id, request_hash, channel_seq, payload, thread_root_id, broadcast) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id")
        .bind(id).bind(channel_id).bind(session_id).bind(client_id).bind(hash).bind(seq).bind(&payload).bind(thread_root.as_ref().map(|row| row.0)).bind(broadcast).fetch_one(&mut *tx).await.map_err(database_error)?;
    // Notifications are decided later from this outbox row; edits never add one.
    if user_id.is_some() {
        crate::push::enqueue(&mut tx, message_id)
            .await
            .map_err(database_error)?;
    }
    if let Some(user) = user_id
        && space_id.is_none()
    {
        sqlx::query("INSERT INTO public.direct_reads (channel_id,user_id,seq) VALUES ($1,$2,$3) ON CONFLICT (channel_id,user_id) DO UPDATE SET seq=GREATEST(direct_reads.seq,EXCLUDED.seq)")
                .bind(channel_id).bind(user).bind(seq).execute(&mut *tx).await.map_err(database_error)?;
    }
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

/// Adds `content.mentions` for `@everyone`/`@here` and for any existing
/// account named, in channels and DMs alike, so people can point each other at
/// someone. Tagging is not access: notifications must check read access
/// themselves. Unknown names stay plain text and are not recorded.
async fn with_mentions(
    connection: &mut sqlx::PgConnection,
    mut content: Value,
    found: &[Mention],
) -> Result<Value, ApiError> {
    let names: Vec<&str> = found
        .iter()
        .filter_map(|mention| match mention {
            Mention::User(name) => Some(name.as_str()),
            Mention::Everyone | Mention::Here => None,
        })
        .collect();
    let accounts: Vec<(String, String)> = if names.is_empty() {
        Vec::new()
    } else {
        sqlx::query_as(
            "SELECT username,external_id FROM public.users WHERE username=ANY($1) AND deleted_at IS NULL",
        )
        .bind(&names)
        .fetch_all(&mut *connection)
        .await
        .map_err(database_error)?
    };
    let entries: Vec<Value> = found
        .iter()
        .filter_map(|mention| match mention {
            Mention::Everyone => Some(json!({"type":"everyone"})),
            Mention::Here => Some(json!({"type":"here"})),
            Mention::User(name) => accounts
                .iter()
                .find(|(username, _)| username == name)
                .map(|(username, id)| json!({"type":"user","id":id,"username":username})),
        })
        .collect();
    if !entries.is_empty() {
        content["mentions"] = Value::Array(entries);
    }
    Ok(content)
}

async fn persist_reaction(
    pool: &PgPool,
    channel: &str,
    message: &str,
    token: &str,
    emoji: &str,
    active: bool,
) -> Result<Value, ApiError> {
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (_, _, _, user_id, _) = authorize_sender(&mut tx, token).await?;
    // Match message creation's space -> channel lock order. In particular, a
    // membership revocation which won the space lock cannot be bypassed using
    // a snapshot taken while this request was waiting.
    let space_id: Option<i64> =
        sqlx::query_scalar("SELECT space_id FROM public.channels WHERE external_id=$1")
            .bind(channel)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    if let Some(space_id) = space_id {
        sqlx::query("SELECT id FROM public.spaces WHERE id=$1 FOR SHARE")
            .bind(space_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    }
    let access: Option<(i64, i64)> = sqlx::query_as(
        "SELECT c.id,c.last_seq FROM public.channels c LEFT JOIN public.spaces s ON s.id=c.space_id
         WHERE c.external_id=$1 AND c.deleted_at IS NULL AND $2::bigint IS NOT NULL
           AND ((s.deleted_at IS NULL AND NOT s.demo
                 AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$2 AND sm.deleted_at IS NULL)
                 AND (s.owner_id=$2 OR NOT c.private OR EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=c.id AND cm.user_id=$2 AND cm.deleted_at IS NULL))
                 AND EXISTS(SELECT 1 FROM public.channel_joins cj WHERE cj.channel_id=c.id AND cj.user_id=$2 AND cj.deleted_at IS NULL))
                OR (c.space_id IS NULL AND EXISTS(SELECT 1 FROM public.direct_conversations d
                    JOIN public.users lo ON lo.id=d.low_user_id JOIN public.users hi ON hi.id=d.high_user_id
                    WHERE d.channel_id=c.id AND $2 IN (d.low_user_id,d.high_user_id) AND lo.deleted_at IS NULL AND hi.deleted_at IS NULL)))
         FOR UPDATE OF c")
        .bind(channel).bind(user_id).fetch_optional(&mut *tx).await.map_err(database_error)?;
    let (channel_id, head) =
        access.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    if let (None, Some(user_id)) = (space_id, user_id) {
        crate::direct::authorize_interaction(&mut tx, channel_id, user_id).await?;
    }
    let row: Option<(i64, Value)> = sqlx::query_as(
        "SELECT id,payload FROM public.messages WHERE channel_id=$1 AND external_id=$2 FOR UPDATE",
    )
    .bind(channel_id)
    .bind(message)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    let (message_id, mut payload) =
        row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "message not found"))?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.message_reactions WHERE message_id=$1 AND emoji=$2 AND user_id=$3 AND deleted_at IS NULL)")
        .bind(message_id).bind(emoji).bind(user_id).fetch_one(&mut *tx).await.map_err(database_error)?;

    if exists != active {
        sqlx::query("DELETE FROM public.message_reaction_activity WHERE user_id=$1 AND created_at <= now()-interval '1 minute'")
            .bind(user_id).execute(&mut *tx).await.map_err(database_error)?;
        let recent: i64 = sqlx::query_scalar("SELECT count(*) FROM public.message_reaction_activity a JOIN public.messages m ON m.id=a.message_id WHERE m.channel_id=$1 AND a.user_id=$2 AND a.created_at > now()-interval '1 minute'")
            .bind(channel_id).bind(user_id).fetch_one(&mut *tx).await.map_err(database_error)?;
        if recent >= 60 {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "reaction updates too frequent",
            ));
        }
        if active {
            let (kinds, actors, total): (i64, i64, i64) = sqlx::query_as(
                "SELECT count(DISTINCT emoji),count(*) FILTER (WHERE emoji=$2),count(*) FROM public.message_reactions WHERE message_id=$1 AND deleted_at IS NULL")
                .bind(message_id).bind(emoji).fetch_one(&mut *tx).await.map_err(database_error)?;
            if (actors == 0 && kinds >= 50) || total >= 1000 {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "reaction limit reached",
                ));
            }
            sqlx::query(
                "INSERT INTO public.message_reactions(message_id,emoji,user_id) VALUES($1,$2,$3)",
            )
            .bind(message_id)
            .bind(emoji)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        } else {
            sqlx::query("UPDATE public.message_reactions SET deleted_at=now() WHERE message_id=$1 AND emoji=$2 AND user_id=$3 AND deleted_at IS NULL")
                .bind(message_id).bind(emoji).bind(user_id).execute(&mut *tx).await.map_err(database_error)?;
        }
        sqlx::query(
            "INSERT INTO public.message_reaction_activity(message_id,user_id) VALUES($1,$2)",
        )
        .bind(message_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    }

    let rows: Vec<(String, Vec<String>)> = sqlx::query_as(
        "SELECT r.emoji,array_agg(u.external_id ORDER BY u.external_id) FROM public.message_reactions r JOIN public.users u ON u.id=r.user_id WHERE r.message_id=$1 AND r.deleted_at IS NULL GROUP BY r.emoji ORDER BY r.emoji")
        .bind(message_id).fetch_all(&mut *tx).await.map_err(database_error)?;
    let reactions: Vec<Value> = rows
        .into_iter()
        .map(|(emoji, author_ids)| json!({"emoji":emoji,"authorIds":author_ids}))
        .collect();
    let seq = if exists == active {
        payload
            .get("reactionSeq")
            .and_then(Value::as_str)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    } else {
        head + 1
    };
    let event = json!({"type":"message.reactions","schemaVersion":1,"channelId":channel,"seq":seq.to_string(),"messageId":message,"reactions":reactions});
    if exists != active {
        payload["reactions"] = event["reactions"].clone();
        payload["reactionSeq"] = json!(seq.to_string());
        sqlx::query("UPDATE public.messages SET payload=$2 WHERE id=$1")
            .bind(message_id)
            .bind(payload)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("INSERT INTO public.channel_events(channel_id,seq,payload) VALUES($1,$2,$3)")
            .bind(channel_id)
            .bind(seq)
            .bind(&event)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("UPDATE public.channels SET last_seq=$2 WHERE id=$1")
            .bind(channel_id)
            .bind(seq)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
    }
    tx.commit().await.map_err(database_error)?;
    Ok(event)
}

async fn persist_pin(
    pool: &PgPool,
    channel: &str,
    message: &str,
    token: &str,
    active: bool,
) -> Result<Value, ApiError> {
    let mut tx = pool.begin().await.map_err(database_error)?;
    let (_, author_id, name, user_id, avatar_id) = authorize_sender(&mut tx, token).await?;
    // Match the space -> channel lock order of membership edits/reactions.
    // Read permissions after waiting, never from a pre-revocation snapshot.
    let space_id: Option<i64> =
        sqlx::query_scalar("SELECT space_id FROM public.channels WHERE external_id=$1")
            .bind(channel)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    if let Some(space_id) = space_id {
        sqlx::query("SELECT id FROM public.spaces WHERE id=$1 FOR SHARE")
            .bind(space_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    }
    let access: Option<(i64, i64)> = sqlx::query_as(
        "SELECT c.id,c.last_seq FROM public.channels c LEFT JOIN public.spaces s ON s.id=c.space_id
         WHERE c.external_id=$1 AND c.deleted_at IS NULL AND $2::bigint IS NOT NULL
           AND ((s.deleted_at IS NULL AND NOT s.demo
                 AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$2 AND sm.deleted_at IS NULL)
                 AND (s.owner_id=$2 OR NOT c.private OR EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=c.id AND cm.user_id=$2 AND cm.deleted_at IS NULL))
                 AND EXISTS(SELECT 1 FROM public.channel_joins cj WHERE cj.channel_id=c.id AND cj.user_id=$2 AND cj.deleted_at IS NULL))
                OR (c.space_id IS NULL AND EXISTS(SELECT 1 FROM public.direct_conversations d
                    JOIN public.users lo ON lo.id=d.low_user_id JOIN public.users hi ON hi.id=d.high_user_id
                    WHERE d.channel_id=c.id AND $2 IN (d.low_user_id,d.high_user_id) AND lo.deleted_at IS NULL AND hi.deleted_at IS NULL)))
         FOR UPDATE OF c")
        .bind(channel).bind(user_id).fetch_optional(&mut *tx).await.map_err(database_error)?;
    let (channel_id, head) =
        access.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "channel not found"))?;
    if let (None, Some(user_id)) = (space_id, user_id) {
        crate::direct::authorize_interaction(&mut tx, channel_id, user_id).await?;
    }
    let row: Option<(i64, Value, Option<i16>, Option<String>)> = sqlx::query_as(
        "SELECT m.id,m.payload,u.avatar_id,u.display_name FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.channel_id=$1 AND m.external_id=$2 FOR UPDATE OF m")
        .bind(channel_id).bind(message).fetch_optional(&mut *tx).await.map_err(database_error)?;
    let (message_id, mut payload, message_avatar, message_name) =
        row.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "message not found"))?;
    let pinned = payload.get("pin").is_some_and(Value::is_object);
    let changed = pinned != active;
    let seq = if changed {
        head + 1
    } else {
        payload
            .get("pinSeq")
            .and_then(Value::as_str)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    };
    if changed {
        sqlx::query("DELETE FROM public.message_pin_activity WHERE user_id=$1 AND created_at <= now()-interval '1 minute'")
            .bind(user_id).execute(&mut *tx).await.map_err(database_error)?;
        let recent: i64 = sqlx::query_scalar("SELECT count(*) FROM public.message_pin_activity a JOIN public.messages m ON m.id=a.message_id WHERE m.channel_id=$1 AND a.user_id=$2 AND a.created_at > now()-interval '1 minute'")
            .bind(channel_id).bind(user_id).fetch_one(&mut *tx).await.map_err(database_error)?;
        if recent >= 60 {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "pin updates too frequent",
            ));
        }
        if active {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.messages WHERE channel_id=$1 AND jsonb_typeof(payload->'pin')='object'")
                .bind(channel_id).fetch_one(&mut *tx).await.map_err(database_error)?;
            if count >= 100 {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "channel pin limit reached (100)",
                ));
            }
        }
        payload["pin"] = if active {
            json!({"author":{"id":author_id,"name":name,"isGuest":false,"avatarId":avatar_id},"createdAt":Utc::now().to_rfc3339()})
        } else {
            Value::Null
        };
        sqlx::query("INSERT INTO public.message_pin_activity(message_id,user_id) VALUES($1,$2)")
            .bind(message_id)
            .bind(user_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
    }
    payload["pinSeq"] = json!(seq.to_string());
    let event = json!({"type":"message.pin","schemaVersion":1,"channelId":channel,"seq":seq.to_string(),"message":enrich_author(payload.clone(),message_avatar,message_name.as_deref())});
    if changed {
        sqlx::query("UPDATE public.messages SET payload=$2 WHERE id=$1")
            .bind(message_id)
            .bind(payload)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("INSERT INTO public.channel_events(channel_id,seq,payload) VALUES($1,$2,$3)")
            .bind(channel_id)
            .bind(seq)
            .bind(&event)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("UPDATE public.channels SET last_seq=$2 WHERE id=$1")
            .bind(channel_id)
            .bind(seq)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
    }
    tx.commit().await.map_err(database_error)?;
    Ok(event)
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
    let rows: Vec<PendingEvent> = sqlx::query_as("SELECT e.channel_id,e.seq,e.payload,u.avatar_id,u.display_name FROM public.channel_events e LEFT JOIN public.messages m ON m.channel_id=e.channel_id AND m.external_id=e.payload->'message'->>'id' LEFT JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE e.published_at IS NULL ORDER BY e.channel_id,e.seq LIMIT 64 FOR UPDATE OF e SKIP LOCKED")
        .fetch_all(&mut *tx).await.map_err(|_| ())?;
    if rows.is_empty() {
        return Ok(false);
    }
    forwarding::project_events(&mut tx, &rows)
        .await
        .map_err(|_| ())?;
    let count = rows.len();
    let mut publishes = redis::pipe();
    let (mut channels, mut seqs) = (Vec::with_capacity(count), Vec::with_capacity(count));
    for (channel, seq, event, avatar_id, name) in rows {
        let event = enrich_author(event, avatar_id, name.as_deref());
        let topic = format!("{TOPIC}:{}", event["channelId"].as_str().ok_or(())?);
        publishes
            .cmd("PUBLISH")
            .arg(topic)
            .arg(event.to_string())
            .ignore();
        channels.push(channel);
        seqs.push(seq);
    }
    // One pipelined round trip in claim order, then one UPDATE. Any failure
    // rolls the whole claim back, exactly as a failure mid-loop did before.
    let published = tokio::time::timeout(BROKER_TIMEOUT, async {
        let mut connection = chat.broker_connection().await?;
        publishes.query_async::<()>(&mut connection).await
    })
    .await;
    if !matches!(published, Ok(Ok(()))) {
        chat.discard_broker_connection().await;
        return Err(());
    }
    sqlx::query("UPDATE public.channel_events e SET published_at = now() FROM unnest($1::bigint[], $2::bigint[]) AS p(channel_id, seq) WHERE e.channel_id = p.channel_id AND e.seq = p.seq")
        .bind(&channels)
        .bind(&seqs)
        .execute(&mut *tx)
        .await
        .map_err(|_| ())?;
    // A crash here re-publishes, deliberately. Receivers deduplicate by sequence.
    tx.commit().await.map_err(|_| ())?;
    tracing::info!(
        event_name = "chat_published",
        count,
        "outbox batch published"
    );
    Ok(true)
}

/// Outbox row: channel, sequence, payload, and the author's current avatar and
/// display name (absent for reactions, guests and deleted accounts).
type PendingEvent = (i64, i64, Value, Option<i16>, Option<String>);

/// Serves stored messages with the author's current avatar and, for accounts
/// that still exist, current display name; the stored name is the fallback.
pub(crate) fn enrich_author(
    mut payload: Value,
    avatar_id: Option<i16>,
    display_name: Option<&str>,
) -> Value {
    let author = if payload.get("message").is_some() {
        payload.pointer_mut("/message/author")
    } else {
        payload.get_mut("author")
    };
    if let Some(author) = author.and_then(Value::as_object_mut) {
        author.insert("avatarId".into(), json!(avatar_id));
        if let Some(name) = display_name {
            author.insert("name".into(), json!(name));
        }
    }
    payload
}

#[cfg(test)]
mod tests;
