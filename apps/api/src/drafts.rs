//! Unsent composer text, synced between an account's devices. One draft per
//! conversation (a space channel or a DM) and per thread reply, updated in
//! place: clearing it stores empty text, so a cleared draft reaches devices
//! that still show the old one. Draft text is never logged.
use crate::{ApiError, AppState, auth::Principal, chat::PROFILE_TOPIC};
use axum::{Extension, Json, Router, extract::State, http::StatusCode, routing::get};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;

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

fn draft(channel: &str, root: Option<&str>, text: &str, at: DateTime<Utc>) -> Value {
    json!({"channelId": channel, "threadRootId": root, "text": text, "updatedAt": timestamp(at)})
}

/// `{"drafts": [...]}`: the account's drafts in conversations it can still
/// read, newest first, including cleared ones (empty `text`).
async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(String, Option<String>, String, DateTime<Utc>)> = sqlx::query_as(&format!(
        "SELECT c.external_id, root.external_id, d.text, d.updated_at
         FROM public.message_drafts d
         JOIN public.channels c ON c.id=d.channel_id
         LEFT JOIN public.messages root ON root.id=d.thread_root_id
         WHERE d.user_id=$1 AND {READABLE}
           AND (d.thread_root_id IS NULL OR root.deleted_at IS NULL)
         ORDER BY d.updated_at DESC LIMIT $2"
    ))
    .bind(principal.user.id)
    .bind(LIMIT)
    .fetch_all(pool(&state)?)
    .await
    .map_err(database_error)?;
    let drafts: Vec<Value> = rows
        .iter()
        .map(|(channel, root, text, at)| draft(channel, root.as_deref(), text, *at))
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
}

/// `{"channelId","threadRootId"?,"text"}` → the saved draft. Empty text
/// clears it. The account's other devices hear it as `draft.updated`.
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
    let not_found = || ApiError::new(StatusCode::NOT_FOUND, "conversation not found");
    let saved: Option<DateTime<Utc>> = sqlx::query_scalar(&format!(
        "WITH target AS (
           SELECT c.id AS channel_id, root.id AS root_id
           FROM public.channels c
           LEFT JOIN public.messages root ON root.channel_id=c.id AND root.external_id=$3
             AND root.thread_root_id IS NULL AND root.deleted_at IS NULL
           WHERE c.external_id=$2 AND {READABLE} AND ($3::text IS NULL OR root.id IS NOT NULL))
         INSERT INTO public.message_drafts(user_id,channel_id,thread_root_id,text,updated_at)
         SELECT $1,channel_id,root_id,$4,clock_timestamp() FROM target
         ON CONFLICT (user_id,channel_id,thread_root_id)
         DO UPDATE SET text=EXCLUDED.text, updated_at=clock_timestamp()
         RETURNING updated_at"
    ))
    .bind(principal.user.id)
    .bind(&input.channel_id)
    .bind(input.thread_root_id.as_deref())
    .bind(&input.text)
    .fetch_optional(pool(&state)?)
    .await
    .map_err(database_error)?;
    let at = saved.ok_or_else(not_found)?;
    let draft = draft(
        &input.channel_id,
        input.thread_root_id.as_deref(),
        &input.text,
        at,
    );
    // Best effort: devices also reload every draft when they reconnect.
    if let Some(chat) = &state.chat {
        chat.publish(
            PROFILE_TOPIC,
            &json!({"audience":[principal.user.id],"event":{"type":"draft.updated","draft":draft}}),
        )
        .await;
    }
    Ok(Json(draft))
}
