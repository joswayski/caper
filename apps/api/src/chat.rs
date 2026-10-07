//! Channel and direct-message commands. Persist before publishing; content rules
//! belong on this path, never in a gateway or a delete/recreate bot.
#[cfg(test)]
use crate::spaces::channel_access;
use crate::{
    ApiError, AppState, RuntimeEnvironment, account_token,
    assets::{self, CdnSigner},
    auth::random_id,
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
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use tokio::sync::Notify;
use uuid::Uuid;

mod editing;
mod forwarding;

pub(crate) const TOPIC: &str = "caper:chat:v1:events";
// Separate from durable events: older gateways require a sequence on that topic.
pub(crate) const TYPING_TOPIC: &str = "caper:chat:v1:typing";
const PAGE: i64 = 50;

/// Unsequenced broker events: typing and media processing progress. Gateways
/// forward them without touching the durable delivery cursor.
pub(crate) fn ephemeral(event: &Value) -> bool {
    matches!(
        event["type"].as_str(),
        Some("typing.updated" | "attachment.progress")
    )
}

#[derive(Clone)]
pub(crate) struct Chat {
    pub pool: PgPool,
    pub broker: redis::Client,
    pub wake: Arc<Notify>,
    /// Signs attachment delivery URLs; absent until the CDN is configured.
    pub cdn: Option<Arc<CdnSigner>>,
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
        Ok(Some(Self {
            pool,
            broker,
            wake: Arc::new(Notify::new()),
            cdn: CdnSigner::from_env(environment)?,
        }))
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
        history_with(&chat.pool, &channel, before, user, chat.cdn.as_deref()).await?,
    ))
}

async fn thread_history(
    State(state): State<AppState>,
    Path((channel, root)): Path<(String, String)>,
    Query(query): Query<HistoryQuery>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let chat = enabled(&state)?;
    let before = query.before.as_deref().map(cursor).transpose()?;
    let user = request_user(&chat.pool, &headers).await?;
    Ok(Json(
        page(
            &chat.pool,
            &channel,
            before,
            user,
            Some(&root),
            chat.cdn.as_deref(),
        )
        .await?,
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
    page(pool, channel, before, user, root, None).await
}

pub(crate) async fn history_with(
    pool: &PgPool,
    channel: &str,
    before: Option<i64>,
    user: Option<i64>,
    cdn: Option<&CdnSigner>,
) -> Result<Value, ApiError> {
    page(pool, channel, before, user, None, cdn).await
}

/// Attachment state and fresh delivery URLs for messages leaving the server:
/// history pages, thread roots and replies, and pins.
async fn deliver(
    pool: &PgPool,
    cdn: Option<&CdnSigner>,
    messages: &mut [Value],
) -> Result<(), ApiError> {
    assets::mark_deleted(pool, messages).await?;
    for message in messages.iter_mut() {
        *message = assets::sign_attachments(std::mem::take(message), cdn);
    }
    Ok(())
}

async fn page(
    pool: &PgPool,
    channel: &str,
    before: Option<i64>,
    user: Option<i64>,
    root: Option<&str>,
    cdn: Option<&CdnSigner>,
) -> Result<Value, ApiError> {
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
    let thread_root: Option<(i64, Value, Option<i16>)> = if let Some(root) = root {
        Some(sqlx::query_as("SELECT m.id,m.payload,u.avatar_id FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.channel_id=$1 AND m.external_id=$2 AND m.thread_root_id IS NULL")
            .bind(channel_id).bind(root).fetch_optional(&mut *tx).await.map_err(database_error)?
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "thread not found"))?)
    } else {
        None
    };
    // Bound by the captured committed head. Later commits are replayed by WS.
    let mut rows: Vec<(Value, Option<i16>)> = sqlx::query_as("SELECT m.payload,u.avatar_id FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.channel_id = $1 AND m.channel_seq <= $2 AND ($3::bigint IS NULL OR m.channel_seq < $3) AND (($5::bigint IS NULL AND (m.thread_root_id IS NULL OR m.broadcast)) OR m.thread_root_id=$5) ORDER BY m.channel_seq DESC LIMIT $4")
        .bind(channel_id).bind(head).bind(before).bind(PAGE + 1).bind(thread_root.as_ref().map(|row| row.0)).fetch_all(&mut *tx).await.map_err(database_error)?;
    let more = rows.len() > PAGE as usize;
    rows.truncate(PAGE as usize);
    rows.reverse();
    let mut rows: Vec<Value> = rows
        .into_iter()
        .map(|(payload, avatar)| enrich_author(payload, avatar))
        .collect();
    forwarding::hydrate(&mut tx, &mut rows).await?;
    deliver(pool, cdn, &mut rows).await?;
    if let Some((_, root, avatar)) = thread_root {
        let mut roots = vec![enrich_author(root, avatar)];
        forwarding::hydrate(&mut tx, &mut roots).await?;
        deliver(pool, cdn, &mut roots).await?;
        return Ok(
            json!({"root":roots[0],"messages":rows,"cursor":head.to_string(),"hasMore":more}),
        );
    }
    // All pins are returned independently of the history page. The shared
    // channel lock also makes their revisions consistent with this cursor.
    let pins: Vec<(Value, Option<i16>)> = sqlx::query_as("SELECT m.payload,u.avatar_id FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.channel_id=$1 AND jsonb_typeof(m.payload->'pin')='object' ORDER BY (m.payload->>'pinSeq')::bigint DESC LIMIT 100")
        .bind(channel_id).fetch_all(&mut *tx).await.map_err(database_error)?;
    let mut pins: Vec<Value> = pins
        .into_iter()
        .map(|(payload, avatar)| enrich_author(payload, avatar))
        .collect();
    forwarding::hydrate(&mut tx, &mut pins).await?;
    deliver(pool, cdn, &mut pins).await?;
    let mut channel_identity = json!({"id":channel,"name":channel_name});
    if space_id.is_none() {
        channel_identity["direct"] = json!(true);
    }
    Ok(
        json!({"messages":rows,"pinnedMessages":pins,"cursor":head.to_string(),"hasMore":more,
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
    let name = account
        .display_name
        .as_deref()
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
    #[serde(default)]
    text: String,
    /// Uploaded files from `POST /api/assets`, in display order.
    #[serde(default)]
    attachment_ids: Vec<String>,
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
    prepare_content(text, false)
}

/// Text may be empty only when the message carries attachments.
fn prepare_content(text: &str, has_attachments: bool) -> Result<Value, ApiError> {
    if (text.trim().is_empty() && !has_attachments)
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
    let payload = send_message(
        &chat.pool,
        &channel,
        sender_token(&headers)?,
        input.client_message_id,
        &input.text,
        &input.attachment_ids,
        input.thread_root_id.as_deref(),
        input.broadcast,
    )
    .await?;
    chat.wake.notify_one();
    Ok(Json(assets::sign_attachments(payload, chat.cdn.as_deref())))
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
        "SELECT s.id, COALESCE(u.external_id, s.external_id), COALESCE(u.display_name, s.name), s.user_id, u.avatar_id FROM public.chat_sessions s LEFT JOIN public.users u ON u.id = s.user_id WHERE s.token_hash = $1 AND s.expires_at > now() AND (s.user_id IS NULL OR (u.deleted_at IS NULL AND EXISTS (SELECT 1 FROM public.account_sessions a WHERE a.token_hash = s.account_session_hash AND a.user_id = s.user_id AND a.revoked_at IS NULL AND a.expires_at > now())))")
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
    let (_, author_id, name, user_id, avatar_id) = {
        let mut connection = chat.pool.acquire().await.map_err(database_error)?;
        authorize_sender(&mut connection, token).await?
    };
    channel_participation(&chat.pool, channel, user_id).await?;
    let event = json!({"type":"typing.updated","channelId":channel,"author":{"id":author_id,"name":name,"isGuest":user_id.is_none(),"avatarId":avatar_id},"typing":typing});
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

#[cfg(test)]
async fn persist(
    pool: &PgPool,
    channel: &str,
    token: &str,
    client_id: Uuid,
    text: &str,
) -> Result<Value, ApiError> {
    send_message(pool, channel, token, client_id, text, &[], None, false).await
}

#[cfg(test)]
async fn persist_message(
    pool: &PgPool,
    channel: &str,
    token: &str,
    client_id: Uuid,
    text: &str,
    root: Option<&str>,
    broadcast: bool,
) -> Result<Value, ApiError> {
    send_message(pool, channel, token, client_id, text, &[], root, broadcast).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn send_message(
    pool: &PgPool,
    channel: &str,
    token: &str,
    client_id: Uuid,
    text: &str,
    attachment_ids: &[String],
    root: Option<&str>,
    broadcast: bool,
) -> Result<Value, ApiError> {
    if attachment_ids.len() > assets::MAX_PER_MESSAGE
        || attachment_ids
            .iter()
            .enumerate()
            .any(|(i, id)| attachment_ids[..i].contains(id))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "attach up to 10 different files",
        ));
    }
    let mut content = prepare_content(text, !attachment_ids.is_empty())?;
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
    if let Some(space_id) = space_id {
        sqlx::query("SELECT id FROM public.spaces WHERE id=$1 FOR UPDATE")
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
    // Preserve existing text-only retry hashes; replies are also bound to their
    // destination and broadcast choice below, and files to their ids.
    let hash = if attachment_ids.is_empty() {
        Sha256::digest(text.as_bytes()).to_vec()
    } else {
        Sha256::digest(format!("{text}\0{}", attachment_ids.join(",")).as_bytes()).to_vec()
    };
    let existing: Option<(i64, Vec<u8>, Value)> = sqlx::query_as("SELECT session_id, request_hash, payload FROM public.messages WHERE channel_id = $1 AND client_message_id = $2")
        .bind(channel_id).bind(client_id).fetch_optional(&mut *tx).await.map_err(database_error)?;
    if let Some((sender, original, payload)) = existing {
        return if sender == session_id
            && original == hash
            && payload.get("threadRootId").and_then(Value::as_str) == root
            && payload["broadcast"].as_bool().unwrap_or(false) == broadcast
        {
            Ok(enrich_author(payload, avatar_id))
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
    let (global, personal): (i64, i64) = sqlx::query_as("SELECT count(*), count(*) FILTER (WHERE session_id = $2) FROM public.messages WHERE channel_id = $1 AND created_at > now() - interval '1 minute'")
        .bind(channel_id).bind(session_id).fetch_one(&mut *tx).await.map_err(database_error)?;
    if global >= 120 || personal >= 30 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "sending too quickly; try again shortly",
        ));
    }
    if !attachment_ids.is_empty() {
        let owner =
            user_id.ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "sign in to send files"))?;
        content["attachments"] =
            Value::Array(assets::attach(&mut tx, attachment_ids, owner, channel_id).await?);
    }
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
    assets::link(&mut tx, attachment_ids, message_id).await?;
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
        sqlx::query("SELECT id FROM public.spaces WHERE id=$1 FOR UPDATE")
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
        sqlx::query("SELECT id FROM public.spaces WHERE id=$1 FOR UPDATE")
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
    let row: Option<(i64, Value, Option<i16>)> = sqlx::query_as(
        "SELECT m.id,m.payload,u.avatar_id FROM public.messages m JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE m.channel_id=$1 AND m.external_id=$2 FOR UPDATE OF m")
        .bind(channel_id).bind(message).fetch_optional(&mut *tx).await.map_err(database_error)?;
    let (message_id, mut payload, message_avatar) =
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
    let event = json!({"type":"message.pin","schemaVersion":1,"channelId":channel,"seq":seq.to_string(),"message":enrich_author(payload.clone(),message_avatar)});
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
    let rows: Vec<(i64, i64, Value, Option<i16>)> = sqlx::query_as("SELECT e.channel_id,e.seq,e.payload,u.avatar_id FROM public.channel_events e LEFT JOIN public.messages m ON m.channel_id=e.channel_id AND m.external_id=e.payload->'message'->>'id' LEFT JOIN public.chat_sessions cs ON cs.id=m.session_id LEFT JOIN public.users u ON u.id=cs.user_id AND u.deleted_at IS NULL WHERE e.published_at IS NULL ORDER BY e.channel_id,e.seq LIMIT 64 FOR UPDATE OF e SKIP LOCKED")
        .fetch_all(&mut *tx).await.map_err(|_| ())?;
    if rows.is_empty() {
        return Ok(false);
    }
    forwarding::project_events(&mut tx, &rows)
        .await
        .map_err(|_| ())?;
    let mut connection = tokio::time::timeout(
        Duration::from_secs(2),
        chat.broker.get_multiplexed_async_connection(),
    )
    .await
    .map_err(|_| ())?
    .map_err(|_| ())?;
    for (channel, seq, event, avatar_id) in &rows {
        let event = enrich_author(event.clone(), *avatar_id);
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

pub(crate) fn enrich_author(mut payload: Value, avatar_id: Option<i16>) -> Value {
    let author = if payload.get("message").is_some() {
        payload.pointer_mut("/message/author")
    } else {
        payload.get_mut("author")
    };
    if let Some(author) = author.and_then(Value::as_object_mut) {
        author.insert("avatarId".into(), json!(avatar_id));
    }
    payload
}

#[cfg(test)]
mod tests;
