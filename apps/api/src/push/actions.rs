//! Reply and Mark as read from a system notification (push or desktop toast),
//! by message ID alone, with the account session the device already saved.
//! Both recheck access the same way the conversation's own read and send
//! paths do: blocks, membership and private-channel grants all apply. Like
//! the in-app inbox, neither accepts a pending message request.
use super::{database_error, pool};
use crate::{ApiError, AppState, auth::Principal};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/notifications/messages/{message}/read", post(read))
        .route("/api/notifications/messages/{message}/reply", post(reply))
}

/// A message the account can read, and where it lives.
#[derive(sqlx::FromRow)]
struct Target {
    id: i64,
    seq: i64,
    channel_id: i64,
    channel: String,
    space_id: Option<i64>,
    /// The thread it replies in, when it is a thread reply.
    root_id: Option<i64>,
    root: Option<String>,
    broadcast: bool,
}

async fn target(pool: &PgPool, user: i64, message: &str) -> Result<Target, ApiError> {
    if message.is_empty() || message.len() > 64 {
        return Err(not_found());
    }
    sqlx::query_as(
        "SELECT m.id,m.channel_seq AS seq,c.id AS channel_id,c.external_id AS channel,c.space_id,
                m.thread_root_id AS root_id,root.external_id AS root,m.broadcast
         FROM public.messages m
         JOIN public.channels c ON c.id=m.channel_id AND c.deleted_at IS NULL
         LEFT JOIN public.spaces s ON s.id=c.space_id
         LEFT JOIN public.messages root ON root.id=m.thread_root_id
         WHERE m.external_id=$1 AND (
           (c.space_id IS NOT NULL AND s.deleted_at IS NULL AND NOT s.demo
             AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$2 AND sm.deleted_at IS NULL)
             AND (NOT c.private OR s.owner_id=$2 OR EXISTS(SELECT 1 FROM public.channel_members cm
                  WHERE cm.channel_id=c.id AND cm.user_id=$2 AND cm.deleted_at IS NULL)))
           OR (c.space_id IS NULL AND EXISTS(SELECT 1 FROM public.direct_conversations d
                WHERE d.channel_id=c.id AND $2 IN (d.low_user_id,d.high_user_id) AND d.accepted_at IS NOT NULL))
           -- The feedback team answers reports where they were filed.
           OR ((s.id IS NULL OR (s.deleted_at IS NULL AND NOT s.demo)) AND public.feedback_access(c.id,$2)))",
    )
    .bind(message)
    .bind(user)
    .fetch_optional(pool)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)
}

fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "message not found")
}

/// Reads the conversation up to and including `target`: the thread for a
/// reply, the channel or DM for anything shown there. Later messages stay
/// unread, and a read position never moves back.
async fn mark_read(pool: &PgPool, user: i64, target: &Target) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(database_error)?;
    if let Some(root) = target.root_id {
        sqlx::query(
            "INSERT INTO public.thread_reads(thread_root_id,user_id,seq) VALUES($1,$2,$3)
             ON CONFLICT(thread_root_id,user_id) DO UPDATE SET seq=GREATEST(thread_reads.seq,EXCLUDED.seq)",
        )
        .bind(root)
        .bind(user)
        .bind(target.seq)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    }
    if target.root_id.is_none() || target.broadcast {
        let table = if target.space_id.is_some() {
            "channel_reads"
        } else {
            "direct_reads"
        };
        sqlx::query(&format!(
            "INSERT INTO public.{table}(channel_id,user_id,seq) VALUES($1,$2,$3)
             ON CONFLICT(channel_id,user_id) DO UPDATE SET seq=GREATEST({table}.seq,EXCLUDED.seq)"
        ))
        .bind(target.channel_id)
        .bind(user)
        .bind(target.seq)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    }
    // Its inbox entry too, whatever the read positions say about it.
    sqlx::query(
        "INSERT INTO public.notification_inbox_reads(user_id,item_id) VALUES($1,'m:' || $2::bigint)
         ON CONFLICT(user_id,item_id) DO UPDATE SET read_at=now(),revoked_at=NULL
         WHERE notification_inbox_reads.revoked_at IS NOT NULL",
    )
    .bind(user)
    .bind(target.id)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    tx.commit().await.map_err(database_error)
}

async fn badge(pool: &PgPool, user: i64) -> Result<Value, ApiError> {
    Ok(json!(
        super::badge::count(pool, user)
            .await
            .map_err(database_error)?
    ))
}

async fn read(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(message): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let pool = pool(&state)?;
    let user = principal.user.id;
    mark_read(pool, user, &target(pool, user, &message).await?).await?;
    Ok(Json(json!({"badgeCount": badge(pool, user).await?})))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplyInput {
    client_message_id: Uuid,
    text: String,
}

/// Answers where the in-app Notifications inbox does: in the thread for a
/// thread reply, otherwise as a reply to the message. Sending reads the
/// conversation up to the message answered.
async fn reply(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(message): Path<String>,
    Json(input): Json<ReplyInput>,
) -> Result<Json<Value>, ApiError> {
    let chat = state.chat.as_ref().ok_or_else(crate::chat::unavailable)?;
    let pool = &chat.pool;
    let user = &principal.user;
    let target = target(pool, user.id, &message).await?;
    let (root, reply_to) = match &target.root {
        Some(root) => (Some(root.as_str()), None),
        None => (None, Some(message.as_str())),
    };
    // A notification action may be retried after a reply already committed
    // (the response was lost). Each attempt mints its own chat session, so
    // recognize this account's earlier send of the same text here.
    let sent: Option<String> = sqlx::query_scalar(
        "SELECT m.external_id FROM public.messages m JOIN public.chat_sessions s ON s.id=m.session_id
         WHERE m.channel_id=$1 AND m.client_message_id=$2 AND s.user_id=$3 AND m.request_hash=$4",
    )
    .bind(target.channel_id)
    .bind(input.client_message_id)
    .bind(user.id)
    .bind(Sha256::digest(input.text.as_bytes()).as_slice())
    .fetch_optional(pool)
    .await
    .map_err(database_error)?;
    let id = match sent {
        Some(id) => id,
        None => {
            let Some(name) = user
                .display_name
                .as_deref()
                .map(str::trim)
                .filter(|name| user.onboarded() && !name.is_empty())
            else {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "complete profile required",
                ));
            };
            let (token, _) =
                crate::chat::mint_session(pool, user.id, name, principal.token_hash.clone())
                    .await?;
            let payload = crate::chat::send_message_with_gif(
                pool,
                &target.channel,
                &token,
                input.client_message_id,
                &input.text,
                &[],
                &[],
                root,
                false,
                reply_to,
                None,
                false,
            )
            .await?;
            chat.wake.notify_one();
            payload["id"].as_str().unwrap_or_default().to_owned()
        }
    };
    mark_read(pool, user.id, &target).await?;
    Ok(Json(
        json!({"messageId": id, "badgeCount": badge(pool, user.id).await?}),
    ))
}
