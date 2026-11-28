//! Unsent composer text, synced between an account's devices. One draft per
//! conversation (a space channel or a DM) and per thread reply, updated in
//! place: clearing it stores empty text, so a cleared draft reaches devices
//! that still show the old one. Draft text is never logged.
use crate::{ApiError, AppState, assets::CdnSigner, auth::Principal, chat::PROFILE_TOPIC};
use axum::{Extension, Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::collections::HashMap;

/// Room for a long message being written; sending still enforces the
/// message limit.
const MAX_CHARACTERS: usize = 16_000;
/// The most recently changed drafts a device loads.
const LIMIT: i64 = 500;

/// `c` (a channel) is one the account (`$1`) can read: a channel of a space
/// it belongs to (a private one needs ownership or a grant), or its DM.
const READABLE: &str = "c.deleted_at IS NULL AND (
    (c.space_id IS NOT NULL AND EXISTS(SELECT 1 FROM public.spaces s
        WHERE s.id=c.space_id AND s.deleted_at IS NULL AND NOT s.demo
          AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=$1 AND sm.deleted_at IS NULL)
          AND (s.owner_id=$1 OR NOT c.private OR EXISTS(SELECT 1 FROM public.channel_members cm
               WHERE cm.channel_id=c.id AND cm.user_id=$1 AND cm.deleted_at IS NULL))))
    OR (c.space_id IS NULL AND EXISTS(SELECT 1 FROM public.direct_conversations d
        JOIN public.users lo ON lo.id=d.low_user_id JOIN public.users hi ON hi.id=d.high_user_id
        WHERE d.channel_id=c.id AND $1 IN (d.low_user_id,d.high_user_id)
          AND lo.deleted_at IS NULL AND hi.deleted_at IS NULL)))";

pub(crate) fn routes() -> Router<AppState> {
    Router::new().route("/api/drafts", get(list).put(save))
}

fn unavailable() -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "drafts unavailable")
}

fn database_error(_: sqlx::Error) -> ApiError {
    tracing::error!(
        event_name = "drafts_database_failed",
        "draft database operation failed"
    );
    unavailable()
}

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.database.as_ref().ok_or_else(unavailable)
}

/// Fixed width, so clients may compare two of them as strings.
fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Files a draft may hold, as a message may.
const MAX_FILES: usize = 10;

fn draft(
    channel: &str,
    root: Option<&str>,
    text: &str,
    attachments: Vec<Value>,
    at: DateTime<Utc>,
) -> Value {
    json!({"channelId": channel, "threadRootId": root, "text": text,
           "attachments": attachments, "updatedAt": timestamp(at)})
}

/// The draft's still-staged files in order, described and signed like a
/// message's, with `spoiler: true` on marked ones.
fn files(
    staged: &HashMap<String, Value>,
    ids: &[String],
    spoilers: &[String],
    signer: Option<&CdnSigner>,
) -> Vec<Value> {
    let mut files: Vec<Value> = ids
        .iter()
        .filter_map(|id| {
            let mut file = staged.get(id)?.clone();
            if spoilers.contains(id) {
                file["spoiler"] = json!(true);
            }
            Some(file)
        })
        .collect();
    crate::assets::sign_files(&mut files, signer);
    files
}

type Listed = (
    String,
    Option<String>,
    String,
    Vec<String>,
    Vec<String>,
    DateTime<Utc>,
);

/// `{"drafts": [...]}`: the account's drafts in conversations it can still
/// read, newest first, including cleared ones (empty `text`).
async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    let pool = pool(&state)?;
    let rows: Vec<Listed> = sqlx::query_as(&format!(
        "SELECT c.external_id, root.external_id, d.text, d.attachment_ids, d.spoiler_ids, d.updated_at
         FROM public.message_drafts d
         JOIN public.channels c ON c.id=d.channel_id
         LEFT JOIN public.messages root ON root.id=d.thread_root_id
         WHERE d.user_id=$1 AND {READABLE}
           AND (d.thread_root_id IS NULL OR root.deleted_at IS NULL)
         ORDER BY d.updated_at DESC LIMIT $2"
    ))
    .bind(principal.user.id)
    .bind(LIMIT)
    .fetch_all(pool)
    .await
    .map_err(database_error)?;
    let ids: Vec<String> = rows.iter().flat_map(|row| row.3.iter().cloned()).collect();
    let staged: HashMap<String, Value> = crate::assets::staged(pool, principal.user.id, None, &ids)
        .await
        .map_err(database_error)?
        .into_iter()
        .collect();
    let signer = state.chat.as_ref().and_then(|chat| chat.cdn.as_deref());
    let drafts: Vec<Value> = rows
        .iter()
        .map(|(channel, root, text, ids, spoilers, at)| {
            draft(
                channel,
                root.as_deref(),
                text,
                files(&staged, ids, spoilers, signer),
                *at,
            )
        })
        .collect();
    Ok(Json(json!({ "drafts": drafts })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SaveInput {
    channel_id: String,
    #[serde(default)]
    thread_root_id: Option<String>,
    text: String,
    /// Replaces the draft's files when present; missing keeps them, so a
    /// client that predates file sync never drops another device's files.
    #[serde(default)]
    attachment_ids: Option<Vec<String>>,
    #[serde(default)]
    spoiler_attachment_ids: Vec<String>,
}

/// `{"channelId","threadRootId"?,"text","attachmentIds"?,"spoilerAttachmentIds"?}`
/// → the saved draft. Empty text and no files clears it. Files must be the
/// account's own ready, unsent uploads in this conversation; others (sent,
/// removed, another channel's) are left out of what is stored and returned.
/// The account's other devices hear it as `draft.updated`.
async fn save(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<SaveInput>,
) -> Result<Json<Value>, ApiError> {
    if input.text.chars().count() > MAX_CHARACTERS {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "draft must be at most 16000 characters",
        ));
    }
    if input
        .attachment_ids
        .as_ref()
        .is_some_and(|ids| ids.len() > MAX_FILES || ids.iter().any(|id| id.len() > 64))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "a draft holds at most 10 files",
        ));
    }
    let pool = pool(&state)?;
    let user = principal.user.id;
    let not_found = || ApiError::new(StatusCode::NOT_FOUND, "conversation not found");
    let target: Option<(i64, Option<i64>)> = sqlx::query_as(&format!(
        "SELECT c.id, root.id FROM public.channels c
         LEFT JOIN public.messages root ON root.channel_id=c.id AND root.external_id=$3
           AND root.thread_root_id IS NULL AND root.deleted_at IS NULL
         WHERE c.external_id=$2 AND {READABLE} AND ($3::text IS NULL OR root.id IS NOT NULL)"
    ))
    .bind(user)
    .bind(&input.channel_id)
    .bind(input.thread_root_id.as_deref())
    .fetch_optional(pool)
    .await
    .map_err(database_error)?;
    let (channel, root) = target.ok_or_else(not_found)?;
    // Only files still staged here are kept, in the order asked.
    let kept = match &input.attachment_ids {
        Some(ids) => {
            let mut unique = Vec::new();
            for id in ids {
                if !unique.contains(id) {
                    unique.push(id.clone());
                }
            }
            Some(
                crate::assets::staged(pool, user, Some(channel), &unique)
                    .await
                    .map_err(database_error)?
                    .into_iter()
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>(),
            )
        }
        None => None,
    };
    let spoilers: Option<Vec<String>> = kept.as_ref().map(|kept| {
        kept.iter()
            .filter(|id| input.spoiler_attachment_ids.contains(id))
            .cloned()
            .collect()
    });
    let (ids, spoilers, at): (Vec<String>, Vec<String>, DateTime<Utc>) = sqlx::query_as(
        "INSERT INTO public.message_drafts(user_id,channel_id,thread_root_id,text,attachment_ids,spoiler_ids,updated_at)
         VALUES($1,$2,$3,$4,COALESCE($5,'{}'),COALESCE($6,'{}'),clock_timestamp())
         ON CONFLICT (user_id,channel_id,thread_root_id)
         DO UPDATE SET text=EXCLUDED.text, updated_at=clock_timestamp(),
           attachment_ids=COALESCE($5,message_drafts.attachment_ids),
           spoiler_ids=COALESCE($6,message_drafts.spoiler_ids)
         RETURNING attachment_ids, spoiler_ids, updated_at",
    )
    .bind(user)
    .bind(channel)
    .bind(root)
    .bind(&input.text)
    .bind(kept.as_deref())
    .bind(spoilers.as_deref())
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    // Files kept from before (an older client saved text only) are
    // described as they are now.
    let staged: HashMap<String, Value> = crate::assets::staged(pool, user, Some(channel), &ids)
        .await
        .map_err(database_error)?
        .into_iter()
        .collect();
    let signer = state.chat.as_ref().and_then(|chat| chat.cdn.as_deref());
    let draft = draft(
        &input.channel_id,
        input.thread_root_id.as_deref(),
        &input.text,
        files(&staged, &ids, &spoilers, signer),
        at,
    );
    // Best effort: devices also reload every draft when they reconnect.
    if let Some(chat) = &state.chat {
        chat.publish(
            PROFILE_TOPIC,
            &json!({"audience":[user],"event":{"type":"draft.updated","draft":draft}}),
        )
        .await;
    }
    Ok(Json(draft))
}
