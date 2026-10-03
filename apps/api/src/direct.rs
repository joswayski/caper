//! Account-wide 1:1 conversations. Space owners never acquire DM access.
use crate::{
    ApiError, AppState,
    auth::{Principal, random_id},
    chat,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/dms", get(list).post(create))
        .route("/api/dms/{conversation}/read", post(read))
}

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.database.as_ref().ok_or_else(chat::unavailable)
}

pub(crate) async fn conversations(pool: &PgPool, user: i64) -> Result<Vec<Value>, ApiError> {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('id',c.external_id,'peer',jsonb_build_object('id',u.external_id,'username',u.username,'displayName',u.display_name),'lastSeq',c.last_seq::text,'readSeq',COALESCE(r.seq,0)::text)
         FROM public.direct_conversations d JOIN public.channels c ON c.id=d.channel_id
         JOIN public.users u ON u.id=CASE WHEN d.low_user_id=$1 THEN d.high_user_id ELSE d.low_user_id END
         LEFT JOIN public.direct_reads r ON r.channel_id=c.id AND r.user_id=$1
         WHERE $1 IN (d.low_user_id,d.high_user_id) AND c.deleted_at IS NULL AND u.deleted_at IS NULL
         ORDER BY COALESCE((SELECT max(m.created_at) FROM public.messages m WHERE m.channel_id=c.id),d.created_at) DESC,c.id DESC",
    ).bind(user).fetch_all(pool).await.map_err(|_| chat::unavailable())
}

async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"conversations":conversations(pool(&state)?,principal.user.id).await?}),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateInput {
    username: String,
}

async fn create(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<CreateInput>,
) -> Result<Json<Value>, ApiError> {
    let id = create_conversation(pool(&state)?, principal.user.id, &input.username).await?;
    let conversation = conversations(pool(&state)?, principal.user.id)
        .await?
        .into_iter()
        .find(|c| c["id"] == id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "account not found"))?;
    Ok(Json(conversation))
}

async fn create_conversation(pool: &PgPool, user: i64, username: &str) -> Result<String, ApiError> {
    let username = username.trim().trim_start_matches('@').to_ascii_lowercase();
    if username.is_empty()
        || username.len() > 32
        || !username
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_')
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "enter an exact username",
        ));
    }
    let peer: i64 = sqlx::query_scalar("SELECT id FROM public.users WHERE username=$1 AND display_name IS NOT NULL AND deleted_at IS NULL")
        .bind(username).fetch_optional(pool).await.map_err(|_|chat::unavailable())?
        .ok_or_else(||ApiError::new(StatusCode::NOT_FOUND,"account not found"))?;
    if peer == user {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "choose another person",
        ));
    }
    let low = user.min(peer);
    let high = user.max(peer);
    let mut tx = pool.begin().await.map_err(|_| chat::unavailable())?;
    // A canonical lock order serializes both directions and per-account limits.
    let users: Vec<i64> = sqlx::query_scalar("SELECT id FROM public.users WHERE id IN ($1,$2) AND deleted_at IS NULL ORDER BY id FOR UPDATE")
        .bind(low).bind(high).fetch_all(&mut *tx).await.map_err(|_|chat::unavailable())?;
    if users.len() != 2 {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "account not found"));
    }
    if let Some(id)=sqlx::query_scalar::<_,String>("SELECT c.external_id FROM public.direct_conversations d JOIN public.channels c ON c.id=d.channel_id WHERE d.low_user_id=$1 AND d.high_user_id=$2")
        .bind(low).bind(high).fetch_optional(&mut *tx).await.map_err(|_|chat::unavailable())? { return Ok(id); }
    let (total,recent): (i64,i64)=sqlx::query_as("SELECT count(*),count(*) FILTER (WHERE created_at>now()-interval '1 minute') FROM public.direct_conversations WHERE $1 IN (low_user_id,high_user_id)")
        .bind(user).fetch_one(&mut *tx).await.map_err(|_|chat::unavailable())?;
    if total >= 1000 || recent >= 20 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too many conversations; try again later",
        ));
    }
    let id = random_id(12);
    let channel: i64=sqlx::query_scalar("INSERT INTO public.channels (external_id,name,private) VALUES ($1,'direct',true) RETURNING id")
        .bind(&id).fetch_one(&mut *tx).await.map_err(|_|chat::unavailable())?;
    sqlx::query("INSERT INTO public.direct_conversations (channel_id,low_user_id,high_user_id) VALUES ($1,$2,$3)")
        .bind(channel).bind(low).bind(high).execute(&mut *tx).await.map_err(|_|chat::unavailable())?;
    tx.commit().await.map_err(|_| chat::unavailable())?;
    Ok(id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    seq: String,
}

async fn read(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(conversation): Path<String>,
    Json(input): Json<ReadInput>,
) -> Result<StatusCode, ApiError> {
    mark_read(
        pool(&state)?,
        principal.user.id,
        &conversation,
        chat::cursor(&input.seq)?,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn mark_read(pool: &PgPool, user: i64, conversation: &str, seq: i64) -> Result<(), ApiError> {
    let result=sqlx::query("INSERT INTO public.direct_reads (channel_id,user_id,seq) SELECT c.id,$2,LEAST($3,c.last_seq) FROM public.direct_conversations d JOIN public.channels c ON c.id=d.channel_id WHERE c.external_id=$1 AND $2 IN (d.low_user_id,d.high_user_id) ON CONFLICT (channel_id,user_id) DO UPDATE SET seq=GREATEST(direct_reads.seq,EXCLUDED.seq)")
        .bind(conversation).bind(user).bind(seq).execute(pool).await.map_err(|_|chat::unavailable())?;
    if result.rows_affected() == 0 {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "conversation not found",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
