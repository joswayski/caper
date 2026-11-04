//! Durable activity is derived from accessible sources, not best-effort push jobs.
//! Listing never acknowledges anything; read receipts apply only to explicit IDs.
use super::{bad_request, database_error, pool};
use crate::{ApiError, AppState, auth::Principal};
use axum::{
    Extension, Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;

const SOURCES: &str = include_str!("inbox.sql");
const PAGE_SIZE: usize = 40;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/notifications/inbox", get(list))
        .route("/api/notifications/inbox/read", post(read))
}

pub(crate) async fn count(pool: &PgPool, user: i64) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "{SOURCES} SELECT count(*) FROM activity WHERE is_unread"
    ))
    .bind(user)
    .fetch_one(pool)
    .await
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Filter {
    #[default]
    All,
    Dms,
    Mentions,
    Threads,
    Reactions,
    Invitations,
}

impl Filter {
    fn condition(&self) -> &'static str {
        match self {
            Self::All => "true",
            Self::Dms => "a.space_id IS NULL",
            Self::Mentions => "a.kind='mention'",
            Self::Threads => "m.thread_root_id IS NOT NULL AND a.kind<>'reaction'",
            Self::Reactions => "a.kind='reaction'",
            Self::Invitations => {
                "a.kind IN ('space.invitation','channel.invitation','direct.request')"
            }
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    #[serde(default)]
    filter: Filter,
    #[serde(default)]
    unread: bool,
    cursor: Option<String>,
}

async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let cursor: Option<(DateTime<Utc>, String)> = query
        .cursor
        .as_deref()
        .map(|cursor| {
            if cursor.len() > 512 {
                return Err(bad_request("invalid inbox cursor"));
            }
            serde_json::from_str(cursor).map_err(|_| bad_request("invalid inbox cursor"))
        })
        .transpose()?;
    let pool = pool(&state)?;
    // Count and page share one statement/snapshot, including concurrent reads.
    let result: Value = sqlx::query_scalar(&format!("{SOURCES}
        SELECT jsonb_build_object('unreadCount',(SELECT count(*) FROM activity WHERE is_unread),
          'items',COALESCE((SELECT jsonb_agg(item ORDER BY created_at DESC,id DESC) FROM (
            SELECT a.created_at,a.id,jsonb_build_object(
              'id',a.id,'kind',a.kind,'createdAt',a.created_at,'unread',a.is_unread,
              'actorId',actor.external_id,'actorName',COALESCE(actor.display_name,actor.username,'Caper member'),'avatarId',actor.avatar_id,
              'body',left(COALESCE(m.payload->'content'->>'text',''),280),'emoji',a.emoji,
              'spaceId',s.external_id,'spaceName',s.name,
              'channelId',CASE WHEN a.space_id IS NOT NULL THEN c.external_id END,'channelName',c.name,
              'conversationId',CASE WHEN a.space_id IS NULL THEN c.external_id END,
              'messageId',m.external_id,'threadRootId',root.external_id) AS item
            FROM activity a JOIN public.users actor ON actor.id=a.actor_id
            LEFT JOIN public.messages m ON m.id=a.message_id
            LEFT JOIN public.messages root ON root.id=m.thread_root_id
            LEFT JOIN public.channels c ON c.id=a.channel_id
            LEFT JOIN public.spaces s ON s.id=a.space_id
            WHERE ({}) AND (NOT $2 OR a.is_unread)
              AND ($3::timestamptz IS NULL OR (a.created_at,a.id)<($3,$4::text))
            ORDER BY a.created_at DESC,a.id DESC LIMIT 41
          ) page),'[]'::jsonb))",query.filter.condition()))
        .bind(principal.user.id).bind(query.unread)
        .bind(cursor.as_ref().map(|c| c.0)).bind(cursor.as_ref().map(|c| &c.1))
        .fetch_one(pool).await.map_err(database_error)?;
    let mut result = result;
    let items = result["items"]
        .as_array_mut()
        .expect("SQL returns an array");
    let next = if items.len() > PAGE_SIZE {
        items.truncate(PAGE_SIZE);
        let last = &items[PAGE_SIZE - 1];
        Some(
            serde_json::to_string(&(last["createdAt"].as_str(), last["id"].as_str()))
                .expect("cursor serializes"),
        )
    } else {
        None
    };
    result["nextCursor"] = json!(next);
    Ok(Json(result))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    ids: Vec<String>,
}

async fn read(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<ReadInput>,
) -> Result<Json<Value>, ApiError> {
    if input.ids.is_empty() || input.ids.len() > 100 || input.ids.iter().any(|id| id.len() > 200) {
        return Err(bad_request("provide between 1 and 100 inbox IDs"));
    }
    let pool = pool(&state)?;
    // Recheck visibility; opaque IDs from another account cannot acknowledge its
    // activity. Do not advance a channel past other messages the reader never saw.
    sqlx::query(&format!(
        "{SOURCES}
        INSERT INTO public.notification_inbox_reads(user_id,item_id)
        SELECT $1,id FROM activity WHERE id=ANY($2) ON CONFLICT DO NOTHING"
    ))
    .bind(principal.user.id)
    .bind(input.ids)
    .execute(pool)
    .await
    .map_err(database_error)?;
    Ok(Json(
        json!({"unreadCount":count(pool,principal.user.id).await.map_err(database_error)?}),
    ))
}
