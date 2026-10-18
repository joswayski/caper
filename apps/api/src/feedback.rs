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

// Listing and navigation counts share visibility, archive and reply semantics.
const REPORTS: &str = "SELECT r.id, jsonb_build_object(
                'id', m.external_id, 'messageId', m.external_id, 'threadRootId', root.external_id,
                'createdAt', r.created_at,
                'text', left(COALESCE(m.payload->'content'->>'text', ''), 280),
                'attachments', COALESCE(jsonb_array_length(m.payload->'content'->'attachments'), 0),
                'reporter', jsonb_build_object('id', u.external_id, 'username', u.username,
                    'displayName', u.display_name, 'avatarId', u.avatar_id, 'avatarAsset', u.avatar_asset),
                'channel', jsonb_build_object('id', c.external_id, 'direct', c.space_id IS NULL,
                    'name', CASE WHEN c.space_id IS NOT NULL THEN lower(c.name)
                                 WHEN d.low_user_id=d.high_user_id THEN lo.display_name
                                 WHEN lo.team_account THEN hi.display_name || ' → ' || lo.display_name
                                 WHEN hi.team_account THEN lo.display_name || ' → ' || hi.display_name
                                 ELSE lo.display_name || ' & ' || hi.display_name END),
                'space', CASE WHEN s.id IS NULL THEN NULL ELSE jsonb_build_object('id', s.external_id, 'name', s.name, 'iconAsset', s.icon_asset) END,
                'replyCount', COALESCE((root.payload->'thread'->>'replyCount')::bigint, 0),
                'archived', z.archived,
                -- Answered while the newest message in its thread (in the
                -- Caper DM, also the DM itself since) is the team's; anyone
                -- else replying makes it wait again. The newest reply, the
                -- root and the newest later DM message are separate indexed
                -- lookups (thread and channel timelines); the latest wins.
                'answered', COALESCE((SELECT EXISTS(SELECT 1 FROM public.feedback_team t
                        WHERE t.user_id=cs.user_id AND t.deleted_at IS NULL)
                    FROM (
                        (SELECT a.session_id, a.channel_seq FROM public.messages a
                         WHERE a.thread_root_id=r.thread_root_id AND a.channel_id=r.channel_id
                         ORDER BY a.channel_seq DESC LIMIT 1)
                        UNION ALL
                        SELECT root.session_id, root.channel_seq WHERE root.channel_id=r.channel_id
                        UNION ALL
                        (SELECT a.session_id, a.channel_seq FROM public.messages a
                         WHERE COALESCE(lo.team_account OR hi.team_account, false)
                           AND a.channel_id=r.channel_id AND a.thread_root_id IS NULL
                           AND a.channel_seq>m.channel_seq AND a.id>r.message_id
                         ORDER BY a.channel_seq DESC LIMIT 1)
                    ) a JOIN public.chat_sessions cs ON cs.id=a.session_id
                    ORDER BY a.channel_seq DESC LIMIT 1), false)) AS report
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
         WHERE ($1::bigint IS NULL OR r.id<$1) AND z.archived = $2
           AND c.deleted_at IS NULL AND u.deleted_at IS NULL
           AND (s.id IS NULL OR (s.deleted_at IS NULL AND NOT s.demo))
           AND (d.channel_id IS NULL OR (lo.deleted_at IS NULL AND hi.deleted_at IS NULL))";

/// All waiting conversations, not just the first page of reports. Match the
/// inbox's newest-report-per-thread grouping; only the team may see the count.
pub(crate) async fn waiting_count(pool: &PgPool, user: i64) -> Result<i64, ApiError> {
    if !team_member(
        &mut *pool.acquire().await.map_err(|_| unavailable())?,
        Some(user),
    )
    .await?
    {
        return Ok(0);
    }
    sqlx::query_scalar(&format!(
        "SELECT count(*) FROM (
            SELECT DISTINCT ON (report->'channel'->>'id', report->>'threadRootId') report
            FROM ({REPORTS}) reports
            ORDER BY report->'channel'->>'id', report->>'threadRootId', id DESC
         ) threads WHERE NOT (report->>'answered')::boolean"
    ))
    .bind(None::<i64>)
    .bind(false)
    .fetch_one(pool)
    .await
    .map_err(|_| unavailable())
}

pub(crate) async fn page(
    pool: &PgPool,
    before: Option<i64>,
    archived: bool,
) -> Result<Value, ApiError> {
    let mut rows: Vec<(i64, Value)> =
        sqlx::query_as(&format!("{REPORTS} ORDER BY r.id DESC LIMIT $3"))
            .bind(before)
            .bind(archived)
            .bind(PAGE + 1)
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

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
    use tower::ServiceExt;

    #[sqlx::test(migrations = "./migrations")]
    #[ignore = "requires disposable Postgres DATABASE_URL"]
    async fn waiting_navigation_count_is_team_only_and_not_page_or_read_limited(pool: PgPool) {
        for name in ["team", "reporter"] {
            let user: i64 = sqlx::query_scalar("INSERT INTO users(external_id,username,display_name) VALUES($1,$1,$1) RETURNING id")
                .bind(name).fetch_one(&pool).await.unwrap();
            let hash = Sha256::digest(name.as_bytes()).to_vec();
            sqlx::query("INSERT INTO account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 day')")
                .bind(&hash).bind(user).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO chat_sessions(external_id,token_hash,name,user_id,account_session_hash) VALUES($1,$2,$1,$3,$2)")
                .bind(name).bind(hash).bind(user).execute(&pool).await.unwrap();
        }
        sqlx::raw_sql(
            "INSERT INTO feedback_team(user_id) SELECT id FROM users WHERE external_id='team';
             INSERT INTO spaces(external_id,name,owner_id) SELECT 'feedback-space','Feedback',id FROM users WHERE external_id='reporter';
             INSERT INTO channels(external_id,name,space_id) SELECT 'feedback-channel','general',id FROM spaces WHERE external_id='feedback-space';
             INSERT INTO messages(external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload)
             SELECT 'report-'||n,c.id,s.id,gen_random_uuid(),'x'::bytea,n,'{\"content\":{\"text\":\"@feedback help\"}}'::jsonb
             FROM generate_series(1,56) n,channels c,chat_sessions s
             WHERE c.external_id='feedback-channel' AND s.external_id='reporter';
             UPDATE messages SET thread_root_id=(SELECT id FROM messages WHERE external_id='report-55') WHERE external_id='report-56';
             INSERT INTO feedback_reports(message_id,channel_id,thread_root_id,reporter_id)
             SELECT m.id,m.channel_id,COALESCE(m.thread_root_id,m.id),s.user_id FROM messages m
             JOIN chat_sessions s ON s.id=m.session_id WHERE s.external_id='reporter' ORDER BY m.channel_seq;",
        ).execute(&pool).await.unwrap();
        let mut config = crate::Config::test(false);
        config.auth_fixture = false;
        let app = crate::app(AppState::with_database(
            config,
            Arc::new(crate::Cloudflare::new()),
            Some(pool.clone()),
        ));
        let request =
            |account: &'static str, method: &'static str, path: &'static str, body: Value| {
                let app = app.clone();
                async move {
                    let response = app
                        .oneshot(
                            axum::http::Request::builder()
                                .method(method)
                                .uri(path)
                                .header("authorization", format!("Bearer {account}"))
                                .header("content-type", "application/json")
                                .body(axum::body::Body::from(body.to_string()))
                                .unwrap(),
                        )
                        .await
                        .unwrap();
                    let status = response.status();
                    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                        .await
                        .unwrap();
                    (
                        status,
                        serde_json::from_slice::<Value>(&bytes).unwrap_or(Value::Null),
                    )
                }
            };
        let count = || request("team", "GET", "/api/spaces/unread", Value::Null);
        let initial = count().await;
        assert_eq!(initial.0, StatusCode::OK);
        assert_eq!(initial.1["feedbackWaiting"], 55);
        assert_eq!(
            request("reporter", "GET", "/api/spaces/unread", Value::Null)
                .await
                .1["feedbackWaiting"],
            0
        );
        assert_eq!(
            request("invalid", "GET", "/api/spaces/unread", Value::Null)
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
        let inbox = request("team", "GET", "/api/feedback", Value::Null).await;
        assert_eq!(inbox.0, StatusCode::OK);
        assert_eq!(inbox.1["reports"].as_array().unwrap().len(), 50);
        assert!(inbox.1["cursor"].is_string());
        assert_eq!(
            count().await.1["feedbackWaiting"],
            55,
            "opening the inbox does not acknowledge the backlog"
        );
        assert_eq!(
            request(
                "team",
                "PUT",
                "/api/feedback/archive",
                json!({
                    "channelId":"feedback-channel", "threadRootId":"report-1", "archived":true
                })
            )
            .await
            .0,
            StatusCode::NO_CONTENT
        );
        assert_eq!(count().await.1["feedbackWaiting"], 54);
        let archived = request("team", "GET", "/api/feedback?archived=true", Value::Null).await;
        assert_eq!(archived.1["reports"].as_array().unwrap().len(), 1);
        // The team's answer removes one; a later reporter reply brings it
        // back, including when the conversation had been archived.
        for (seq, author, root, expected) in [
            (57, "team", "report-2", 53),
            (58, "reporter", "report-2", 54),
            (59, "reporter", "report-1", 55),
        ] {
            sqlx::query("INSERT INTO messages(external_id,channel_id,session_id,client_message_id,request_hash,channel_seq,payload,thread_root_id)
                SELECT $1,c.id,s.id,gen_random_uuid(),'x'::bytea,$2,'{}'::jsonb,m.id FROM channels c,chat_sessions s,messages m
                WHERE c.external_id='feedback-channel' AND s.external_id=$3 AND m.external_id=$4")
                .bind(format!("reply-{seq}")).bind(seq as i64).bind(author).bind(root).execute(&pool).await.unwrap();
            assert_eq!(count().await.1["feedbackWaiting"], expected);
        }
        sqlx::query("UPDATE channels SET deleted_at=now() WHERE external_id='feedback-channel'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(count().await.1["feedbackWaiting"], 0);
    }
}
