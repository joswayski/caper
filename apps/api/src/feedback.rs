//! `@feedback` (or `@caper`) asks the Caper team to look at a conversation.
//!
//! The message stays where it was written; nothing is copied. Reporting it
//! lets feedback team members open that channel, thread or DM without joining
//! (see `feedback_access` in the migration), read it for context and reply in
//! place as themselves with an ADMIN badge, so everyone there sees the answer.
//! Team members are notified like an @mention and list every report here.
use crate::{
    ApiError, AppState,
    auth::Principal,
    chat::{self, unavailable},
};
use axum::{
    Extension, Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool};

const PAGE: i64 = 50;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/feedback", get(list))
        .route("/api/feedback/archive", put(archive))
}

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.database.as_ref().ok_or_else(unavailable)
}

/// Whether `user` is on the feedback team. Their messages carry `author.admin`.
pub(crate) async fn team_member(
    connection: &mut PgConnection,
    user: Option<i64>,
) -> Result<bool, ApiError> {
    let Some(user) = user else {
        return Ok(false);
    };
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.feedback_team WHERE user_id=$1 AND deleted_at IS NULL)",
    )
    .bind(user)
    .fetch_one(connection)
    .await
    .map_err(|_| unavailable())
}

/// Whether `channel` is a DM with the Caper account.
pub(crate) async fn team_conversation(
    connection: &mut PgConnection,
    channel: i64,
) -> Result<bool, ApiError> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.direct_conversations d JOIN public.users u
             ON u.id IN (d.low_user_id,d.high_user_id) WHERE d.channel_id=$1 AND u.team_account)",
    )
    .bind(channel)
    .fetch_one(connection)
    .await
    .map_err(|_| unavailable())
}

/// Records a report in the send transaction. Every team member's access to
/// the conversation starts with its first report.
pub(crate) async fn record(
    connection: &mut PgConnection,
    message: i64,
    channel: i64,
    thread_root: i64,
    reporter: i64,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO public.feedback_reports (message_id,channel_id,thread_root_id,reporter_id)
         VALUES ($1,$2,$3,$4) ON CONFLICT (message_id) DO NOTHING",
    )
    .bind(message)
    .bind(channel)
    .bind(thread_root)
    .bind(reporter)
    .execute(connection)
    .await
    .map_err(|_| unavailable())?;
    tracing::info!(event_name = "feedback_reported", "feedback reported");
    Ok(())
}

/// Whether a message was reported when it was sent. Edits keep its
/// `@feedback` highlighted but never report it (edits never notify).
pub(crate) async fn reported(
    connection: &mut PgConnection,
    message: i64,
) -> Result<bool, ApiError> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.feedback_reports WHERE message_id=$1)")
        .bind(message)
        .fetch_one(connection)
        .await
        .map_err(|_| unavailable())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    before: Option<String>,
    /// The archived threads instead of the inbox.
    #[serde(default)]
    archived: bool,
}

/// Every report, newest first, for team members only (`?archived=true`: the
/// archived ones). Each opens its conversation at `messageId`, in the thread
/// `threadRootId`.
async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let pool = pool(&state)?;
    let user = principal.user.id;
    if !team_member(
        &mut *pool.acquire().await.map_err(|_| unavailable())?,
        Some(user),
    )
    .await?
    {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not found"));
    }
    let before = query.before.as_deref().map(chat::cursor).transpose()?;
    Ok(Json(json!(page(pool, before, query.archived).await?)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ArchiveInput {
    channel_id: String,
    thread_root_id: String,
    archived: bool,
}

/// Hides a reported thread from every team member's inbox, or brings it back.
/// Anyone but the team posting in the thread afterwards brings it back.
async fn archive(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<ArchiveInput>,
) -> Result<StatusCode, ApiError> {
    let pool = pool(&state)?;
    if !team_member(
        &mut *pool.acquire().await.map_err(|_| unavailable())?,
        Some(principal.user.id),
    )
    .await?
    {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "not found"));
    }
    let updated = sqlx::query(
        "UPDATE public.feedback_reports r
         SET archived_at = CASE WHEN $3 THEN now() END
         FROM public.channels c, public.messages root
         WHERE c.id=r.channel_id AND root.id=r.thread_root_id
           AND c.external_id=$1 AND root.external_id=$2",
    )
    .bind(&input.channel_id)
    .bind(&input.thread_root_id)
    .bind(input.archived)
    .execute(pool)
    .await
    .map_err(|_| unavailable())?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "report not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn page(
    pool: &PgPool,
    before: Option<i64>,
    archived: bool,
) -> Result<Value, ApiError> {
    let mut rows: Vec<(i64, Value)> = sqlx::query_as(
        "SELECT r.id, jsonb_build_object(
                'id', m.external_id, 'messageId', m.external_id, 'threadRootId', root.external_id,
                'createdAt', r.created_at,
                'text', left(COALESCE(m.payload->'content'->>'text', ''), 280),
                'attachments', COALESCE(jsonb_array_length(m.payload->'content'->'attachments'), 0),
                'reporter', jsonb_build_object('id', u.external_id, 'username', u.username,
                    'displayName', u.display_name, 'avatarId', u.avatar_id),
                'channel', jsonb_build_object('id', c.external_id, 'direct', c.space_id IS NULL,
                    'name', CASE WHEN c.space_id IS NOT NULL THEN lower(c.name)
                                 WHEN d.low_user_id=d.high_user_id THEN lo.display_name
                                 WHEN lo.team_account THEN hi.display_name || ' → ' || lo.display_name
                                 WHEN hi.team_account THEN lo.display_name || ' → ' || hi.display_name
                                 ELSE lo.display_name || ' & ' || hi.display_name END),
                'space', CASE WHEN s.id IS NULL THEN NULL ELSE jsonb_build_object('id', s.external_id, 'name', s.name) END,
                'replyCount', COALESCE((root.payload->'thread'->>'replyCount')::bigint, 0),
                'archived', z.archived,
                -- Answered while the newest message in its thread is the
                -- team's; anyone else replying makes it wait again.
                'answered', COALESCE((SELECT EXISTS(SELECT 1 FROM public.feedback_team t
                        WHERE t.user_id=cs.user_id AND t.deleted_at IS NULL)
                    FROM public.messages a JOIN public.chat_sessions cs ON cs.id=a.session_id
                    WHERE a.channel_id=r.channel_id AND (a.thread_root_id=r.thread_root_id OR a.id=r.thread_root_id)
                    ORDER BY a.channel_seq DESC LIMIT 1), false))
         FROM public.feedback_reports r
         JOIN public.messages m ON m.id=r.message_id
         JOIN public.messages root ON root.id=r.thread_root_id
         JOIN public.channels c ON c.id=r.channel_id
         JOIN public.users u ON u.id=r.reporter_id
         LEFT JOIN public.spaces s ON s.id=c.space_id
         LEFT JOIN public.direct_conversations d ON d.channel_id=c.id
         LEFT JOIN public.users lo ON lo.id=d.low_user_id
         LEFT JOIN public.users hi ON hi.id=d.high_user_id
         -- Archived until someone other than the team posts in the thread.
         CROSS JOIN LATERAL (SELECT r.archived_at IS NOT NULL AND NOT EXISTS(
             SELECT 1 FROM public.messages a JOIN public.chat_sessions cs ON cs.id=a.session_id
             WHERE a.channel_id=r.channel_id AND a.thread_root_id=r.thread_root_id AND a.created_at>r.archived_at
               AND NOT EXISTS(SELECT 1 FROM public.feedback_team t WHERE t.user_id=cs.user_id AND t.deleted_at IS NULL)
         ) AS archived) z
         WHERE ($1::bigint IS NULL OR r.id<$1) AND z.archived = $3
           AND c.deleted_at IS NULL AND u.deleted_at IS NULL
           AND (s.id IS NULL OR (s.deleted_at IS NULL AND NOT s.demo))
           AND (d.channel_id IS NULL OR (lo.deleted_at IS NULL AND hi.deleted_at IS NULL))
         ORDER BY r.id DESC LIMIT $2",
    )
    .bind(before)
    .bind(PAGE + 1)
    .bind(archived)
    .fetch_all(pool)
    .await
    .map_err(|_| unavailable())?;
    let more = rows.len() > PAGE as usize;
    rows.truncate(PAGE as usize);
    let cursor = more
        .then(|| rows.last().map(|row| row.0.to_string()))
        .flatten();
    Ok(json!({
        "reports": rows.into_iter().map(|row| row.1).collect::<Vec<_>>(),
        "cursor": cursor,
    }))
}
