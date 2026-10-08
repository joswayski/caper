//! Account-wide conversations and personal notes. Space owners never acquire DM access.
//!
//! People who share a space can always start a DM. Anyone else's first DM is a
//! message request: the recipient sees it under "Message requests" until they
//! accept, decline or block, and their DM privacy setting can refuse it outright.
use crate::{
    ApiError, AppState,
    auth::{Principal, random_id},
    chat,
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/dms", get(list).post(create))
        .route("/api/dms/{conversation}/read", post(read))
        .route("/api/people", get(list_people))
        .route("/api/dms/{conversation}/accept", post(accept))
        .route("/api/dms/{conversation}/decline", post(decline))
        .route("/api/blocks", get(list_blocks))
        .route("/api/blocks/{user}", put(block).delete(unblock))
        .route("/api/account/privacy", get(privacy).put(set_privacy))
}

/// Shown for both a block and a privacy setting, so neither is revealed.
fn not_accepting() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "this person isn't accepting direct messages",
    )
    .with_code("dm_not_accepted")
}

fn blocked_by_you() -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, "unblock this person to message them")
        .with_code("dm_blocked")
}

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.database.as_ref().ok_or_else(chat::unavailable)
}

/// `status` is `accepted`, `outgoing` (you asked, they haven't accepted) or
/// `incoming` (a request for you). Declined requests are hidden from the
/// recipient only; the sender keeps seeing theirs as waiting.
pub(crate) async fn conversations(pool: &PgPool, user: i64) -> Result<Vec<Value>, ApiError> {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('id',c.external_id,'peer',jsonb_build_object('id',u.external_id,'username',u.username,'displayName',u.display_name,'avatarId',u.avatar_id),'lastSeq',c.last_seq::text,'readSeq',COALESCE(r.seq,0)::text,
                'status',CASE WHEN d.accepted_at IS NOT NULL THEN 'accepted' WHEN d.requested_by=$1 THEN 'outgoing' ELSE 'incoming' END,
                'blocked',EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$1 AND b.blocked_id=u.id AND b.deleted_at IS NULL))
         FROM public.direct_conversations d JOIN public.channels c ON c.id=d.channel_id
         JOIN public.users u ON u.id=CASE WHEN d.low_user_id=$1 THEN d.high_user_id ELSE d.low_user_id END
         LEFT JOIN public.direct_reads r ON r.channel_id=c.id AND r.user_id=$1
         WHERE $1 IN (d.low_user_id,d.high_user_id) AND c.deleted_at IS NULL AND u.deleted_at IS NULL
           AND (d.accepted_at IS NOT NULL OR d.requested_by=$1 OR d.declined_at IS NULL)
         ORDER BY COALESCE((SELECT max(m.created_at) FROM public.messages m WHERE m.channel_id=c.id),d.created_at) DESC,c.id DESC",
    ).bind(user).fetch_all(pool).await.map_err(|_| chat::unavailable())
}

/// People the account already knows: anyone sharing an active space or a DM
/// they can see (declined requests stay hidden, as in the DM list).
/// Composers suggest them after `@` in DMs; it is not a directory search.
pub(crate) async fn people(pool: &PgPool, user: i64) -> Result<Vec<Value>, ApiError> {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('id',u.external_id,'username',u.username,'displayName',u.display_name,'avatarId',u.avatar_id)
         FROM public.users u
         WHERE u.id<>$1 AND u.deleted_at IS NULL AND u.username IS NOT NULL AND u.display_name IS NOT NULL
           AND (EXISTS(SELECT 1 FROM public.space_members mine
                       JOIN public.space_members theirs ON theirs.space_id=mine.space_id AND theirs.deleted_at IS NULL
                       JOIN public.spaces s ON s.id=mine.space_id AND s.deleted_at IS NULL AND NOT s.demo
                       WHERE mine.user_id=$1 AND mine.deleted_at IS NULL AND theirs.user_id=u.id)
             OR EXISTS(SELECT 1 FROM public.direct_conversations d JOIN public.channels c ON c.id=d.channel_id AND c.deleted_at IS NULL
                       WHERE ((d.low_user_id=$1 AND d.high_user_id=u.id) OR (d.high_user_id=$1 AND d.low_user_id=u.id))
                         AND (d.accepted_at IS NOT NULL OR d.requested_by=$1 OR d.declined_at IS NULL)))
         ORDER BY lower(u.username),u.id LIMIT 500",
    ).bind(user).fetch_all(pool).await.map_err(|_| chat::unavailable())
}

async fn list_people(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"people":people(pool(&state)?,principal.user.id).await?}),
    ))
}

async fn conversation(pool: &PgPool, user: i64, id: &str) -> Result<Json<Value>, ApiError> {
    conversations(pool, user)
        .await?
        .into_iter()
        .find(|c| c["id"] == id)
        .map(Json)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "conversation not found"))
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
    // Recipients identify a sender by @username; nobody may write anonymously.
    if !principal.user.onboarded() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "complete profile required",
        ));
    }
    let id = create_conversation(pool(&state)?, principal.user.id, &input.username).await?;
    conversation(pool(&state)?, principal.user.id, &id).await
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
    let low = user.min(peer);
    let high = user.max(peer);
    let mut tx = pool.begin().await.map_err(|_| chat::unavailable())?;
    // A canonical lock order serializes both directions and per-account limits.
    let users: Vec<i64> = sqlx::query_scalar("SELECT id FROM public.users WHERE id IN ($1,$2) AND deleted_at IS NULL ORDER BY id FOR UPDATE")
        .bind(low).bind(high).fetch_all(&mut *tx).await.map_err(|_|chat::unavailable())?;
    if users.len() != if low == high { 1 } else { 2 } {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "account not found"));
    }
    if let Some((id, pending_for_you)) = sqlx::query_as::<_, (String, bool)>("SELECT c.external_id,d.accepted_at IS NULL AND d.requested_by<>$3 FROM public.direct_conversations d JOIN public.channels c ON c.id=d.channel_id WHERE d.low_user_id=$1 AND d.high_user_id=$2")
        .bind(low).bind(high).bind(user).fetch_optional(&mut *tx).await.map_err(|_|chat::unavailable())? {
        // Choosing to message someone who sent you a request accepts it.
        if pending_for_you {
            accept_request(&mut tx, &id, user).await?;
        }
        tx.commit().await.map_err(|_| chat::unavailable())?;
        return Ok(id);
    }
    let accepted = if low == high {
        true
    } else {
        let (you_blocked, they_blocked, shared, policy): (bool, bool, bool, String) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM public.user_blocks WHERE blocker_id=$1 AND blocked_id=$2 AND deleted_at IS NULL),
                    EXISTS(SELECT 1 FROM public.user_blocks WHERE blocker_id=$2 AND blocked_id=$1 AND deleted_at IS NULL),
                    EXISTS(SELECT 1 FROM public.space_members a JOIN public.space_members b ON b.space_id=a.space_id JOIN public.spaces s ON s.id=a.space_id
                           WHERE a.user_id=$1 AND b.user_id=$2 AND a.deleted_at IS NULL AND b.deleted_at IS NULL AND s.deleted_at IS NULL AND NOT s.demo),
                    (SELECT dm_policy FROM public.users WHERE id=$2)",
        ).bind(user).bind(peer).fetch_one(&mut *tx).await.map_err(|_|chat::unavailable())?;
        if you_blocked {
            return Err(blocked_by_you());
        }
        if they_blocked || policy == "nobody" || (policy == "spaces" && !shared) {
            return Err(not_accepting());
        }
        shared
    };
    // Only conversations this account started count, so strangers' requests
    // can't use up someone else's quota.
    let (total,recent,requests): (i64,i64,i64)=sqlx::query_as("SELECT count(*),count(*) FILTER (WHERE created_at>now()-interval '1 minute'),count(*) FILTER (WHERE accepted_at IS NULL AND created_at>now()-interval '1 hour') FROM public.direct_conversations WHERE $1 IN (low_user_id,high_user_id) AND requested_by=$1")
        .bind(user).fetch_one(&mut *tx).await.map_err(|_|chat::unavailable())?;
    if total >= 1000 || recent >= 20 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too many conversations; try again later",
        ));
    }
    if !accepted && requests >= 10 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too many message requests; try again later",
        ));
    }
    let id = random_id(12);
    let channel: i64=sqlx::query_scalar("INSERT INTO public.channels (external_id,name,private) VALUES ($1,'direct',true) RETURNING id")
        .bind(&id).fetch_one(&mut *tx).await.map_err(|_|chat::unavailable())?;
    sqlx::query("INSERT INTO public.direct_conversations (channel_id,low_user_id,high_user_id,requested_by,accepted_at) VALUES ($1,$2,$3,$4,CASE WHEN $5 THEN now() END)")
        .bind(channel).bind(low).bind(high).bind(user).bind(accepted).execute(&mut *tx).await.map_err(|_|chat::unavailable())?;
    tx.commit().await.map_err(|_| chat::unavailable())?;
    Ok(id)
}

/// Accepts a request addressed to `user`. Returns false when there is none.
async fn accept_request(
    connection: &mut sqlx::PgConnection,
    conversation: &str,
    user: i64,
) -> Result<bool, ApiError> {
    let result = sqlx::query("UPDATE public.direct_conversations d SET accepted_at=now(),declined_at=NULL FROM public.channels c WHERE c.id=d.channel_id AND c.external_id=$1 AND $2 IN (d.low_user_id,d.high_user_id) AND d.requested_by<>$2 AND d.accepted_at IS NULL")
        .bind(conversation).bind(user).execute(connection).await.map_err(|_|chat::unavailable())?;
    Ok(result.rows_affected() > 0)
}

/// Called with the channel row locked before a message is stored in a DM.
/// Either side's block stops sending; the recipient replying accepts a request.
pub(crate) async fn authorize_send(
    connection: &mut sqlx::PgConnection,
    channel: i64,
    user: i64,
) -> Result<(), ApiError> {
    if unblocked_request_for(&mut *connection, channel, user).await? {
        sqlx::query("UPDATE public.direct_conversations SET accepted_at=now(),declined_at=NULL WHERE channel_id=$1 AND accepted_at IS NULL")
            .bind(channel)
            .execute(connection)
            .await
            .map_err(|_| chat::unavailable())?;
    }
    Ok(())
}

/// Checked before every other DM write (reactions, pins, edits, typing). A
/// block stops these too, but unlike a reply they never accept a request.
pub(crate) async fn ensure_not_blocked(
    connection: &mut sqlx::PgConnection,
    channel: i64,
    user: i64,
) -> Result<(), ApiError> {
    unblocked_request_for(connection, channel, user)
        .await
        .map(|_| ())
}

/// Refuses while either person blocks the other. Otherwise returns whether
/// the conversation is a pending request addressed to `user`.
async fn unblocked_request_for(
    connection: &mut sqlx::PgConnection,
    channel: i64,
    user: i64,
) -> Result<bool, ApiError> {
    let (you_blocked, they_blocked, pending_for_you): (bool, bool, bool) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$2 AND b.blocked_id=peer AND b.deleted_at IS NULL),
                EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=peer AND b.blocked_id=$2 AND b.deleted_at IS NULL),
                pending_for_you
         FROM (SELECT CASE WHEN low_user_id=$2 THEN high_user_id ELSE low_user_id END AS peer,
                      accepted_at IS NULL AND requested_by<>$2 AS pending_for_you
               FROM public.direct_conversations WHERE channel_id=$1) d",
    )
    .bind(channel)
    .bind(user)
    .fetch_one(connection)
    .await
    .map_err(|_| chat::unavailable())?;
    if you_blocked {
        return Err(blocked_by_you());
    }
    if they_blocked {
        return Err(not_accepting());
    }
    Ok(pending_for_you)
}

async fn accept(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let pool = pool(&state)?;
    accept_request(
        &mut *pool.acquire().await.map_err(|_| chat::unavailable())?,
        &id,
        principal.user.id,
    )
    .await?;
    // Accepting an accepted conversation is a no-op that still returns it.
    conversation(pool, principal.user.id, &id).await
}

async fn decline(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    decline_request(pool(&state)?, principal.user.id, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Hides a request from its recipient. The sender is not told.
async fn decline_request(pool: &PgPool, user: i64, conversation: &str) -> Result<(), ApiError> {
    let found: Option<bool> = sqlx::query_scalar("UPDATE public.direct_conversations d SET declined_at=COALESCE(d.declined_at,now()) FROM public.channels c WHERE c.id=d.channel_id AND c.external_id=$1 AND $2 IN (d.low_user_id,d.high_user_id) AND d.requested_by<>$2 AND d.accepted_at IS NULL RETURNING true")
        .bind(conversation).bind(user).fetch_optional(pool).await.map_err(|_|chat::unavailable())?;
    found
        .map(|_| ())
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "request not found"))
}

async fn list_blocks(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        json!({"blocks":blocks(pool(&state)?,principal.user.id).await?}),
    ))
}

/// Accounts `user` has blocked, newest first. Clients hide their messages.
pub(crate) async fn blocks(pool: &PgPool, user: i64) -> Result<Vec<Value>, ApiError> {
    sqlx::query_scalar(
        "SELECT jsonb_build_object('id',u.external_id,'username',u.username,'displayName',u.display_name,'avatarId',u.avatar_id)
         FROM public.user_blocks b JOIN public.users u ON u.id=b.blocked_id
         WHERE b.blocker_id=$1 AND b.deleted_at IS NULL AND u.deleted_at IS NULL
         ORDER BY b.created_at DESC,b.id DESC",
    )
    .bind(user)
    .fetch_all(pool)
    .await
    .map_err(|_| chat::unavailable())
}

async fn block(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(user): Path<String>,
) -> Result<StatusCode, ApiError> {
    set_block(pool(&state)?, principal.user.id, &user, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unblock(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(user): Path<String>,
) -> Result<StatusCode, ApiError> {
    set_block(pool(&state)?, principal.user.id, &user, false).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Idempotent. Blocking also declines any request they sent you.
async fn set_block(pool: &PgPool, user: i64, target: &str, blocked: bool) -> Result<(), ApiError> {
    let mut tx = pool.begin().await.map_err(|_| chat::unavailable())?;
    let target: i64 = sqlx::query_scalar(
        "SELECT id FROM public.users WHERE external_id=$1 AND deleted_at IS NULL",
    )
    .bind(target)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|_| chat::unavailable())?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "account not found"))?;
    if target == user {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "you can't block yourself",
        ));
    }
    // The same canonical lock order as creating a DM, so a block can't race a request.
    sqlx::query("SELECT id FROM public.users WHERE id IN ($1,$2) ORDER BY id FOR UPDATE")
        .bind(user)
        .bind(target)
        .execute(&mut *tx)
        .await
        .map_err(|_| chat::unavailable())?;
    if blocked {
        let (active, count): (bool, i64) = sqlx::query_as(
            "SELECT bool_or(blocked_id=$2) IS TRUE,count(*) FROM public.user_blocks WHERE blocker_id=$1 AND deleted_at IS NULL",
        )
        .bind(user)
        .bind(target)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| chat::unavailable())?;
        if !active {
            if count >= 1000 {
                return Err(ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "too many blocked accounts",
                ));
            }
            sqlx::query("INSERT INTO public.user_blocks (blocker_id,blocked_id) VALUES ($1,$2)")
                .bind(user)
                .bind(target)
                .execute(&mut *tx)
                .await
                .map_err(|_| chat::unavailable())?;
        }
        sqlx::query("UPDATE public.direct_conversations SET declined_at=COALESCE(declined_at,now()) WHERE low_user_id=LEAST($1::bigint,$2::bigint) AND high_user_id=GREATEST($1::bigint,$2::bigint) AND requested_by=$2 AND accepted_at IS NULL")
            .bind(user).bind(target).execute(&mut *tx).await.map_err(|_|chat::unavailable())?;
    } else {
        sqlx::query("UPDATE public.user_blocks SET deleted_at=now() WHERE blocker_id=$1 AND blocked_id=$2 AND deleted_at IS NULL")
            .bind(user).bind(target).execute(&mut *tx).await.map_err(|_|chat::unavailable())?;
    }
    tx.commit().await.map_err(|_| chat::unavailable())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PrivacyInput {
    direct_messages: String,
}

async fn privacy(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    let policy: String = sqlx::query_scalar("SELECT dm_policy FROM public.users WHERE id=$1")
        .bind(principal.user.id)
        .fetch_one(pool(&state)?)
        .await
        .map_err(|_| chat::unavailable())?;
    Ok(Json(json!({"directMessages":policy})))
}

/// `anyone` (as a request, the default), `spaces` (people who share a space
/// with you) or `nobody`. Conversations that already exist are unaffected.
async fn set_privacy(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<PrivacyInput>,
) -> Result<Json<Value>, ApiError> {
    if !["anyone", "spaces", "nobody"].contains(&input.direct_messages.as_str()) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "directMessages must be anyone, spaces or nobody",
        ));
    }
    sqlx::query("UPDATE public.users SET dm_policy=$2,updated_at=now() WHERE id=$1")
        .bind(principal.user.id)
        .bind(&input.direct_messages)
        .execute(pool(&state)?)
        .await
        .map_err(|_| chat::unavailable())?;
    Ok(Json(json!({"directMessages":input.direct_messages})))
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
