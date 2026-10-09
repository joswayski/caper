use crate::{ApiError, AppState, RuntimeEnvironment, auth::Principal, auth::random_id};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};

mod joining;
pub(crate) use joining::channel_participation;
use joining::{add_channel_member, remove_channel_member};
#[cfg(test)]
mod joining_tests;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Limits {
    pub(crate) owned_spaces: i64,
    pub(crate) total_spaces: i64,
    pub(crate) channels_per_space: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            owned_spaces: 20,
            total_spaces: 100,
            channels_per_space: 100,
        }
    }
}

impl Limits {
    pub(crate) fn from_env(environment: &RuntimeEnvironment) -> Result<Self, String> {
        let defaults = Self::default();
        Ok(Self {
            owned_spaces: limit(
                environment.get("SPACE_OWNED_LIMIT"),
                defaults.owned_spaces,
                "SPACE_OWNED_LIMIT",
            )?,
            total_spaces: limit(
                environment.get("SPACE_MEMBERSHIP_LIMIT"),
                defaults.total_spaces,
                "SPACE_MEMBERSHIP_LIMIT",
            )?,
            channels_per_space: limit(
                environment.get("SPACE_CHANNEL_LIMIT"),
                defaults.channels_per_space,
                "SPACE_CHANNEL_LIMIT",
            )?,
        })
    }
}

fn limit(value: Option<String>, default: i64, name: &str) -> Result<i64, String> {
    match value {
        None => Ok(default),
        Some(value) => value
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| format!("{name} must be a positive integer")),
    }
}

#[derive(Debug)]
pub(crate) struct ChannelAccess {
    pub(crate) id: i64,
    pub(crate) last_seq: i64,
    pub(crate) space_id: Option<i64>,
}

fn database_error(_: sqlx::Error) -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "spaces unavailable")
}

pub(crate) fn not_found() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "resource not found")
}

fn conflict(message: &'static str) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, message)
}

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state
        .database
        .as_ref()
        .ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "spaces unavailable"))
}

pub(crate) async fn session_user(pool: &PgPool, token_hash: &[u8]) -> Result<i64, ApiError> {
    sqlx::query_scalar(
        "SELECT u.id FROM public.account_sessions a
         JOIN public.users u ON u.id = a.user_id
         WHERE a.token_hash = $1 AND a.revoked_at IS NULL AND a.expires_at > now()
           AND u.deleted_at IS NULL AND u.username IS NOT NULL AND u.display_name IS NOT NULL",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized"))
}

pub(crate) async fn channel_access<'e>(
    pool: impl sqlx::Executor<'e, Database = Postgres>,
    channel: &str,
    user: Option<i64>,
) -> Result<ChannelAccess, ApiError> {
    check_channel_access(pool, channel, user, false).await
}

// $1 channel, $2 user, $3 require join. A macro so the gateway can embed the
// same predicate in its combined session and channel check.
macro_rules! channel_access_sql {
    () => {
        "SELECT c.id, c.last_seq, s.id
         FROM public.channels c JOIN public.spaces s ON s.id = c.space_id
         WHERE c.external_id = $1 AND c.deleted_at IS NULL AND s.deleted_at IS NULL
           AND NOT s.demo AND $2::bigint IS NOT NULL
                 AND EXISTS (SELECT 1 FROM public.space_members sm WHERE sm.space_id = s.id AND sm.user_id = $2 AND sm.deleted_at IS NULL)
                 AND (s.owner_id = $2 OR NOT c.private OR
                      EXISTS (SELECT 1 FROM public.channel_members cm WHERE cm.channel_id = c.id AND cm.user_id = $2 AND cm.deleted_at IS NULL))
                 AND (NOT $3 OR EXISTS(SELECT 1 FROM public.channel_joins cj WHERE cj.channel_id=c.id AND cj.user_id=$2 AND cj.deleted_at IS NULL))
         UNION ALL
         SELECT c.id,c.last_seq,NULL::bigint FROM public.channels c
         JOIN public.direct_conversations d ON d.channel_id=c.id
         JOIN public.users lo ON lo.id=d.low_user_id JOIN public.users hi ON hi.id=d.high_user_id
         WHERE c.external_id=$1 AND c.space_id IS NULL AND c.deleted_at IS NULL
           AND $2 IN (d.low_user_id,d.high_user_id) AND lo.deleted_at IS NULL AND hi.deleted_at IS NULL"
    };
}

async fn check_channel_access<'e>(
    pool: impl sqlx::Executor<'e, Database = Postgres>,
    channel: &str,
    user: Option<i64>,
    require_join: bool,
) -> Result<ChannelAccess, ApiError> {
    sqlx::query_as::<_, (i64, i64, Option<i64>)>(channel_access_sql!())
        .bind(channel)
        .bind(user)
        .bind(require_join)
        .fetch_optional(pool)
        .await
        .map_err(database_error)?
        .map(|(id, last_seq, space_id)| ChannelAccess {
            id,
            last_seq,
            space_id,
        })
        .ok_or_else(not_found)
}

/// For live subscribers: whether the account session (if any) still belongs to
/// `user`, and whether `user` can read `channel`, in one round trip. Equivalent
/// to `session_user` followed by `channel_access`, for every delivered event.
pub(crate) async fn session_channel_access(
    pool: &PgPool,
    session: Option<&[u8]>,
    user: Option<i64>,
    channel: &str,
) -> Result<(bool, bool), ApiError> {
    sqlx::query_as(concat!(
        "SELECT $4::bytea IS NULL OR EXISTS (
             SELECT 1 FROM public.account_sessions a JOIN public.users u ON u.id = a.user_id
             WHERE a.token_hash = $4 AND a.user_id = $2 AND a.revoked_at IS NULL AND a.expires_at > now()
               AND u.deleted_at IS NULL AND u.username IS NOT NULL AND u.display_name IS NOT NULL),
         EXISTS (",
        channel_access_sql!(),
        ")"
    ))
    .bind(channel)
    .bind(user)
    .bind(false)
    .bind(session)
    .fetch_one(pool)
    .await
    .map_err(database_error)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Space {
    id: String,
    name: String,
    owner_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Channel {
    id: String,
    space_id: String,
    name: String,
    private: bool,
    joined: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Member {
    id: String,
    avatar_id: i16,
    username: String,
    display_name: String,
    owner: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NameInput {
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelInput {
    name: String,
    private: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MemberInput {
    username: String,
}

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/spaces", get(list_spaces).post(create_space))
        .route(
            "/api/spaces/{space}",
            get(get_space).patch(rename_space).delete(delete_space),
        )
        .route("/api/spaces/{space}/channels", post(create_channel))
        .route(
            "/api/spaces/{space}/channels/{channel}",
            patch(update_channel).delete(delete_channel),
        )
        .route(
            "/api/spaces/{space}/channels/{channel}/membership",
            post(joining::join_channel).delete(joining::leave_channel),
        )
        .route(
            "/api/spaces/{space}/channels/{channel}/invitation",
            post(joining::accept_channel_invitation).delete(joining::decline_channel_invitation),
        )
        .route(
            "/api/spaces/{space}/members",
            get(list_members).post(add_space_member),
        )
        .route(
            "/api/spaces/{space}/members/{user}",
            delete(remove_space_member),
        )
        .route("/api/spaces/{space}/invitations", get(list_invitations))
        .route(
            "/api/spaces/{space}/invitations/{user}",
            delete(cancel_invitation),
        )
        .route(
            "/api/spaces/{space}/invitation",
            post(accept_invitation).delete(decline_invitation),
        )
        .route(
            "/api/spaces/{space}/channels/{channel}/members",
            get(list_channel_members).post(add_channel_member),
        )
        .route(
            "/api/spaces/{space}/channels/{channel}/members/{user}",
            delete(remove_channel_member),
        )
}

fn space_name(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 80 || value.chars().any(char::is_control) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid space name"));
    }
    Ok(value.to_owned())
}

fn channel_name(value: &str) -> Result<&str, ApiError> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
        || value.split('-').any(|segment| segment.is_empty())
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid channel name",
        ));
    }
    Ok(value)
}

async fn list_spaces(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<Value>, ApiError> {
    let spaces: Vec<Space> = sqlx::query_as::<_, (String, String, String)>(
        "SELECT s.external_id, s.name, owner.external_id
         FROM public.space_members sm JOIN public.spaces s ON s.id = sm.space_id
         JOIN public.users owner ON owner.id = s.owner_id
         WHERE sm.user_id = $1 AND sm.deleted_at IS NULL AND s.deleted_at IS NULL AND NOT s.demo
         ORDER BY lower(s.name), s.id",
    )
    .bind(principal.user.id)
    .fetch_all(pool(&state)?)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(|(id, name, owner_id)| Space { id, name, owner_id })
    .collect();
    // Owners are the only inviters; share their public identity, never the member list.
    let invitations: Vec<Value> = sqlx::query_as::<_, (String, String, String, String, String)>(
        "SELECT s.external_id,s.name,o.external_id,o.username,o.display_name FROM public.space_invitations i
         JOIN public.spaces s ON s.id=i.space_id JOIN public.users o ON o.id=s.owner_id
         WHERE i.user_id=$1 AND i.status='pending' AND i.updated_at > now()-interval '7 days'
           AND s.deleted_at IS NULL AND NOT s.demo
           -- Blocking an owner hides their invitations, as it declines their DM requests.
           AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$1 AND b.blocked_id=s.owner_id AND b.deleted_at IS NULL)
         ORDER BY i.updated_at DESC,s.id",
    )
    .bind(principal.user.id)
    .fetch_all(pool(&state)?)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(|(id, name, owner_id, username, display_name)| {
        json!({"id":id,"name":name,"ownerId":owner_id,
            "inviter":{"username":username,"displayName":display_name}})
    })
    .collect();
    Ok(Json(
        json!({"spaces":spaces,"invitations":invitations,"limits":state.config.space_limits}),
    ))
}

async fn create_space(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Json(input): Json<NameInput>,
) -> Result<(StatusCode, Json<Space>), ApiError> {
    let name = space_name(&input.name)?;
    let pool = pool(&state)?;
    let mut tx = pool.begin().await.map_err(database_error)?;
    lock_onboarded_user(&mut tx, principal.user.id).await?;
    let (owned, memberships): (i64, i64) = sqlx::query_as(
        "SELECT
           (SELECT count(*) FROM public.spaces WHERE owner_id = $1 AND deleted_at IS NULL),
           (SELECT count(*) FROM public.space_members sm JOIN public.spaces s ON s.id = sm.space_id WHERE sm.user_id = $1 AND sm.deleted_at IS NULL AND s.deleted_at IS NULL)",
    )
    .bind(principal.user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if owned >= state.config.space_limits.owned_spaces
        || memberships >= state.config.space_limits.total_spaces
    {
        return Err(conflict("space limit reached"));
    }
    let id = random_id(12);
    let internal: i64 = sqlx::query_scalar(
        "INSERT INTO public.spaces (external_id, name, owner_id) VALUES ($1,$2,$3) RETURNING id",
    )
    .bind(&id)
    .bind(&name)
    .bind(principal.user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    sqlx::query("INSERT INTO public.space_members (space_id,user_id) VALUES ($1,$2)")
        .bind(internal)
        .bind(principal.user.id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    let general: i64 = sqlx::query_scalar("INSERT INTO public.channels (external_id,space_id,name) VALUES ($1,$2,'general') RETURNING id")
        .bind(random_id(12))
        .bind(internal)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2)")
        .bind(general)
        .bind(principal.user.id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok((
        StatusCode::CREATED,
        Json(Space {
            id,
            name,
            owner_id: principal.user.external_id,
        }),
    ))
}

async fn get_space(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let pool = pool(&state)?;
    let row: (i64, String, String, String) = sqlx::query_as(
        "SELECT s.id,s.name,s.external_id,o.external_id FROM public.spaces s
         JOIN public.users o ON o.id=s.owner_id
         WHERE s.external_id=$1 AND s.deleted_at IS NULL AND NOT s.demo
           AND EXISTS (SELECT 1 FROM public.space_members WHERE space_id=s.id AND user_id=$2 AND deleted_at IS NULL)",
    )
    .bind(&space)
    .bind(principal.user.id)
    .fetch_optional(pool)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    let channels: Vec<Channel> = sqlx::query_as::<_, (String, String, String, bool, bool)>(
        "SELECT c.external_id,s.external_id,c.name,c.private,
         EXISTS(SELECT 1 FROM public.channel_joins WHERE channel_id=c.id AND user_id=$2 AND deleted_at IS NULL)
         FROM public.channels c
         JOIN public.spaces s ON s.id=c.space_id
         WHERE c.space_id=$1 AND c.deleted_at IS NULL
           AND (s.owner_id=$2 OR NOT c.private OR EXISTS (SELECT 1 FROM public.channel_members WHERE channel_id=c.id AND user_id=$2 AND deleted_at IS NULL))
         ORDER BY c.id",
    )
    .bind(row.0)
    .bind(principal.user.id)
    .fetch_all(pool)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(|(id, space_id, name, private, joined)| Channel { id, space_id, name, private, joined })
    .collect();
    let invitations: Vec<Value> = sqlx::query_as::<_,(String,String,String,String)>(
        "SELECT c.external_id,c.name,u.username,u.display_name FROM public.channel_invitations i
         JOIN public.channels c ON c.id=i.channel_id JOIN public.spaces s ON s.id=c.space_id
         JOIN public.users u ON u.id=s.owner_id
         WHERE c.space_id=$1 AND i.user_id=$2 AND c.private AND c.deleted_at IS NULL
           AND i.status='pending' AND i.updated_at > now()-interval '7 days'
           AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$2 AND b.blocked_id=s.owner_id AND b.deleted_at IS NULL)
         ORDER BY i.updated_at,c.id"
    ).bind(row.0).bind(principal.user.id).fetch_all(pool).await.map_err(database_error)?
        .into_iter().map(|(id,name,username,display_name)|json!({"channel":Channel{id,space_id:space.clone(),name,private:true,joined:false},"inviter":{"username":username,"displayName":display_name}})).collect();
    let members = members(pool, row.0).await?;
    Ok(Json(
        json!({"space":Space{id:row.2,name:row.1,owner_id:row.3},"channels":channels,"members":members,"channelInvitations":invitations}),
    ))
}

async fn rename_space(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
    Json(input): Json<NameInput>,
) -> Result<Json<Space>, ApiError> {
    let name = space_name(&input.name)?;
    let row: Option<(String, String)> = sqlx::query_as(
        "UPDATE public.spaces SET name=$3 WHERE external_id=$1 AND owner_id=$2 AND deleted_at IS NULL AND NOT demo RETURNING external_id,(SELECT external_id FROM public.users WHERE id=$2)",
    )
    .bind(&space)
    .bind(principal.user.id)
    .bind(&name)
    .fetch_optional(pool(&state)?)
    .await
    .map_err(database_error)?;
    let (id, owner_id) = row.ok_or_else(not_found)?;
    Ok(Json(Space { id, name, owner_id }))
}

async fn delete_space(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let id = owner_space(&mut tx, &space, principal.user.id).await?;
    sqlx::query(
        "UPDATE public.channels SET deleted_at=now() WHERE space_id=$1 AND deleted_at IS NULL",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    sqlx::query("UPDATE public.spaces SET deleted_at=now() WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_channel(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
    Json(input): Json<ChannelInput>,
) -> Result<(StatusCode, Json<Channel>), ApiError> {
    let name = channel_name(&input.name)?;
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let space_id = owner_space(&mut tx, &space, principal.user.id).await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM public.channels WHERE space_id=$1 AND deleted_at IS NULL",
    )
    .bind(space_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if count >= state.config.space_limits.channels_per_space {
        return Err(conflict("channel limit reached"));
    }
    let id = random_id(12);
    let internal: i64 = sqlx::query_scalar(
        "INSERT INTO public.channels (external_id,space_id,name,private) VALUES ($1,$2,$3,$4) RETURNING id",
    )
    .bind(&id)
    .bind(space_id)
    .bind(name)
    .bind(input.private)
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| constraint_or_database(error, "channel name already exists"))?;
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2)")
        .bind(internal)
        .bind(principal.user.id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok((
        StatusCode::CREATED,
        Json(Channel {
            id,
            space_id: space,
            name: name.to_owned(),
            private: input.private,
            joined: true,
        }),
    ))
}

async fn update_channel(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
    Json(input): Json<ChannelInput>,
) -> Result<Json<Channel>, ApiError> {
    let name = channel_name(&input.name)?;
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let space_id = owner_space(&mut tx, &space, principal.user.id).await?;
    let changed=sqlx::query("UPDATE public.channels SET name=$3,private=$4 WHERE external_id=$1 AND space_id=$2 AND deleted_at IS NULL")
        .bind(&channel).bind(space_id).bind(name).bind(input.private).execute(&mut *tx).await.map_err(|error| constraint_or_database(error,"channel name already exists"))?;
    if changed.rows_affected() != 1 {
        return Err(not_found());
    }
    let joined: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.channel_joins cj JOIN public.channels c ON c.id=cj.channel_id WHERE c.external_id=$1 AND cj.user_id=$2 AND cj.deleted_at IS NULL)")
        .bind(&channel).bind(principal.user.id).fetch_one(&mut *tx).await.map_err(database_error)?;
    if !input.private {
        sqlx::query("UPDATE public.channel_invitations SET status='revoked',updated_at=now() WHERE channel_id=(SELECT id FROM public.channels WHERE external_id=$1) AND status='pending'")
            .bind(&channel).execute(&mut *tx).await.map_err(database_error)?;
    }
    tx.commit().await.map_err(database_error)?;
    Ok(Json(Channel {
        id: channel,
        space_id: space,
        name: name.to_owned(),
        private: input.private,
        joined,
    }))
}

async fn delete_channel(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let space_id = owner_space(&mut tx, &space, principal.user.id).await?;
    let changed=sqlx::query("UPDATE public.channels SET deleted_at=now() WHERE external_id=$1 AND space_id=$2 AND deleted_at IS NULL")
        .bind(channel).bind(space_id).execute(&mut *tx).await.map_err(database_error)?;
    if changed.rows_affected() != 1 {
        return Err(not_found());
    }
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_members(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id = accessible_space(pool(&state)?, &space, principal.user.id).await?;
    Ok(Json(json!({"members":members(pool(&state)?,id).await?})))
}

async fn add_space_member(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
    Json(input): Json<MemberInput>,
) -> Result<(StatusCode, Json<Member>), ApiError> {
    let pool = pool(&state)?;
    // Authorize before revealing account lookup errors. Charge attempts outside
    // the invite transaction so errors/rollbacks cannot bypass the shared limit.
    let owns: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.spaces WHERE external_id=$1 AND owner_id=$2 AND deleted_at IS NULL AND NOT demo)")
        .bind(&space).bind(principal.user.id).fetch_one(pool).await.map_err(database_error)?;
    if !owns {
        return Err(not_found());
    }
    invite_rate_limit(pool, principal.user.id).await?;
    let mut tx = pool.begin().await.map_err(database_error)?;
    let space_id = owner_space(&mut tx, &space, principal.user.id).await?;
    let member = find_user_for_update(&mut tx, &input.username).await?;
    ensure_invitee_allows(&mut tx, member.0, principal.user.id).await?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.space_members WHERE space_id=$1 AND user_id=$2 AND deleted_at IS NULL)",
    )
    .bind(space_id)
    .bind(member.0)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if exists {
        return Err(conflict("user already in space"));
    }
    let previous: Option<(String, bool, bool)> = sqlx::query_as(
        "SELECT status,updated_at > now()-interval '7 days',updated_at > now()-interval '24 hours'
         FROM public.space_invitations WHERE space_id=$1 AND user_id=$2",
    )
    .bind(space_id)
    .bind(member.0)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    if let Some((status, live, recent)) = previous {
        if status == "pending" && live {
            return Err(conflict("user already invited"));
        }
        if status != "pending" && recent {
            return Err(conflict("invitation cooldown; try again after 24 hours"));
        }
    }
    let (received,sent): (i64,i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE i.user_id=$1 AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.blocker_id=$1 AND b.blocked_id=s.owner_id AND b.deleted_at IS NULL)),
                count(*) FILTER (WHERE i.space_id=$2)
         FROM public.space_invitations i JOIN public.spaces s ON s.id=i.space_id
         WHERE i.status='pending' AND i.updated_at > now()-interval '7 days' AND s.deleted_at IS NULL
           AND (i.user_id=$1 OR i.space_id=$2)",
    ).bind(member.0).bind(space_id).fetch_one(&mut *tx).await.map_err(database_error)?;
    if received >= 50 || sent >= 100 {
        return Err(conflict("pending invitation limit reached"));
    }
    sqlx::query(
        "INSERT INTO public.space_invitations(space_id,user_id,status) VALUES($1,$2,'pending')
        ON CONFLICT(space_id,user_id) DO UPDATE SET status='pending',updated_at=now()",
    )
    .bind(space_id)
    .bind(member.0)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok((
        StatusCode::CREATED,
        Json(Member {
            id: member.1,
            avatar_id: member.2,
            username: member.3,
            display_name: member.4,
            owner: member.0 == principal.user.id,
        }),
    ))
}

async fn invite_rate_limit(pool: &PgPool, user: i64) -> Result<(), ApiError> {
    let attempts: i32 = sqlx::query_scalar(
        "INSERT INTO public.space_invite_limits(user_id) VALUES($1)
         ON CONFLICT(user_id) DO UPDATE SET
           attempts=CASE WHEN space_invite_limits.window_start <= now()-interval '10 minutes' THEN 1
                         ELSE LEAST(space_invite_limits.attempts+1,21) END,
           window_start=CASE WHEN space_invite_limits.window_start <= now()-interval '10 minutes' THEN now()
                             ELSE space_invite_limits.window_start END
         RETURNING attempts",
    ).bind(user).fetch_one(pool).await.map_err(database_error)?;
    if attempts > 20 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too many invitation attempts; try again in 10 minutes",
        ));
    }
    Ok(())
}

async fn list_invitations(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
) -> Result<Json<Value>, ApiError> {
    // A read: check ownership without the exclusive space lock that writers
    // take, which would stall every send in the space behind this request.
    let pool = pool(&state)?;
    let id: i64 = sqlx::query_scalar("SELECT id FROM public.spaces WHERE external_id=$1 AND owner_id=$2 AND deleted_at IS NULL AND NOT demo")
        .bind(&space).bind(principal.user.id).fetch_optional(pool).await.map_err(database_error)?.ok_or_else(not_found)?;
    let rows: Vec<Member> = sqlx::query_as::<_, (String, i16, String, String)>(
        "SELECT u.external_id,u.avatar_id,u.username,u.display_name FROM public.space_invitations i
         JOIN public.users u ON u.id=i.user_id WHERE i.space_id=$1 AND i.status='pending'
           AND i.updated_at > now()-interval '7 days' AND u.deleted_at IS NULL
         ORDER BY i.updated_at,u.id",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(|(id, avatar_id, username, display_name)| Member {
        id,
        avatar_id,
        username,
        display_name,
        owner: false,
    })
    .collect();
    Ok(Json(json!({"members":rows})))
}

async fn accept_invitation(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
) -> Result<Json<Space>, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    // The same space -> recipient lock order as invitation/removal operations.
    let (id,name,owner_id): (i64,String,String) = sqlx::query_as(
        "SELECT s.id,s.name,o.external_id FROM public.spaces s JOIN public.users o ON o.id=s.owner_id
         WHERE s.external_id=$1 AND s.deleted_at IS NULL AND NOT s.demo FOR UPDATE OF s",
    ).bind(&space).fetch_optional(&mut *tx).await.map_err(database_error)?.ok_or_else(not_found)?;
    // Invitation changes are serialized on the space row held above. Check for
    // one before the profile so uninvited callers can't probe space existence.
    let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.space_invitations WHERE space_id=$1 AND user_id=$2 AND status='pending' AND updated_at > now()-interval '7 days')")
        .bind(id).bind(principal.user.id).fetch_one(&mut *tx).await.map_err(database_error)?;
    if !pending {
        return Err(not_found());
    }
    lock_onboarded_user(&mut tx, principal.user.id).await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.space_members sm JOIN public.spaces s ON s.id=sm.space_id WHERE sm.user_id=$1 AND sm.deleted_at IS NULL AND s.deleted_at IS NULL")
        .bind(principal.user.id).fetch_one(&mut *tx).await.map_err(database_error)?;
    if count >= state.config.space_limits.total_spaces {
        return Err(conflict("membership limit reached"));
    }
    sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
        .bind(id)
        .bind(principal.user.id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) SELECT id,$2 FROM public.channels WHERE space_id=$1 AND NOT private AND deleted_at IS NULL ORDER BY (name='general') DESC,id LIMIT 1")
        .bind(id).bind(principal.user.id).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("UPDATE public.space_invitations SET status='accepted',updated_at=now() WHERE space_id=$1 AND user_id=$2")
        .bind(id).bind(principal.user.id).execute(&mut *tx).await.map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok(Json(Space {
        id: space,
        name,
        owner_id,
    }))
}

async fn decline_invitation(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
) -> Result<StatusCode, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let id: i64 = sqlx::query_scalar("SELECT id FROM public.spaces WHERE external_id=$1 AND deleted_at IS NULL AND NOT demo FOR UPDATE")
        .bind(space).fetch_optional(&mut *tx).await.map_err(database_error)?.ok_or_else(not_found)?;
    let changed = sqlx::query("UPDATE public.space_invitations SET status='declined',updated_at=now()
        WHERE space_id=$1 AND user_id=$2 AND status='pending' AND updated_at > now()-interval '7 days'")
        .bind(id).bind(principal.user.id).execute(&mut *tx).await.map_err(database_error)?;
    if changed.rows_affected() != 1 {
        return Err(not_found());
    }
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn cancel_invitation(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, user)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let id = owner_space(&mut tx, &space, principal.user.id).await?;
    let changed = sqlx::query(
        "UPDATE public.space_invitations i SET status='revoked',updated_at=now()
        FROM public.users u WHERE i.space_id=$1 AND i.user_id=u.id AND u.external_id=$2
        AND i.status='pending' AND i.updated_at > now()-interval '7 days'",
    )
    .bind(id)
    .bind(user)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    if changed.rows_affected() != 1 {
        return Err(not_found());
    }
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_space_member(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, user)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let (space_id, owner_id): (i64, i64) = sqlx::query_as(
        "SELECT id,owner_id FROM public.spaces WHERE external_id=$1 AND deleted_at IS NULL AND NOT demo FOR UPDATE",
    )
    .bind(&space)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    let target: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM public.users WHERE external_id=$1 AND deleted_at IS NULL",
    )
    .bind(user)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    let target = target.ok_or_else(not_found)?;
    // Authorize first so outsiders can't learn whether a space exists or who owns it.
    if principal.user.id != owner_id && principal.user.id != target {
        return Err(not_found());
    }
    if target == owner_id {
        return Err(conflict("owner cannot be removed"));
    }
    sqlx::query("UPDATE public.channel_members cm SET deleted_at=now() FROM public.channels c WHERE cm.channel_id=c.id AND c.space_id=$1 AND cm.user_id=$2 AND cm.deleted_at IS NULL")
        .bind(space_id).bind(target).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("UPDATE public.channel_joins cj SET deleted_at=now() FROM public.channels c WHERE cj.channel_id=c.id AND c.space_id=$1 AND cj.user_id=$2 AND cj.deleted_at IS NULL")
        .bind(space_id).bind(target).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query("UPDATE public.channel_invitations i SET status='revoked',updated_at=now() FROM public.channels c WHERE i.channel_id=c.id AND c.space_id=$1 AND i.user_id=$2 AND i.status IN ('pending','accepted')")
        .bind(space_id).bind(target).execute(&mut *tx).await.map_err(database_error)?;
    let changed = sqlx::query("UPDATE public.space_members SET deleted_at=now() WHERE space_id=$1 AND user_id=$2 AND deleted_at IS NULL")
        .bind(space_id)
        .bind(target)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    if changed.rows_affected() != 1 {
        return Err(not_found());
    }
    sqlx::query(
        "INSERT INTO public.space_invitations(space_id,user_id,status) VALUES($1,$2,'revoked')
        ON CONFLICT(space_id,user_id) DO UPDATE SET status='revoked',updated_at=now()",
    )
    .bind(space_id)
    .bind(target)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_channel_members(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let pool = pool(&state)?;
    let (space_id, channel_id) = owned_channel(pool, &space, &channel, principal.user.id).await?;
    let rows: Vec<Member> = sqlx::query_as::<_, (String, i16, String, String, bool)>(
        "SELECT u.external_id,u.avatar_id,u.username,u.display_name,s.owner_id=u.id
         FROM public.space_members sm JOIN public.users u ON u.id=sm.user_id
         JOIN public.spaces s ON s.id=sm.space_id
         WHERE sm.space_id=$2 AND sm.deleted_at IS NULL AND u.deleted_at IS NULL
           AND (s.owner_id=u.id OR EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=$1 AND cm.user_id=u.id AND cm.deleted_at IS NULL))
         ORDER BY (s.owner_id=u.id) DESC,lower(u.username),u.id",
    )
    .bind(channel_id)
    .bind(space_id)
    .fetch_all(pool)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(|(id, avatar_id, username, display_name, owner)| Member {
        id,
        avatar_id,
        username,
        display_name,
        owner,
    })
    .collect();
    let invitations: Vec<Value> = sqlx::query_as::<_,(String,i16,String,String)>(
        "SELECT u.external_id,u.avatar_id,u.username,u.display_name FROM public.channel_invitations i
         JOIN public.users u ON u.id=i.user_id JOIN public.space_members sm ON sm.user_id=u.id AND sm.space_id=$2 AND sm.deleted_at IS NULL
         WHERE i.channel_id=$1 AND i.status='pending' AND i.updated_at > now()-interval '7 days' AND u.deleted_at IS NULL ORDER BY i.updated_at,u.id"
    ).bind(channel_id).bind(space_id).fetch_all(pool).await.map_err(database_error)?
        .into_iter().map(|(id,avatar_id,username,display_name)|json!(Member{id,avatar_id,username,display_name,owner:false})).collect();
    Ok(Json(json!({"members":rows,"invitations":invitations})))
}

async fn lock_onboarded_user(
    tx: &mut Transaction<'_, Postgres>,
    user: i64,
) -> Result<(), ApiError> {
    let onboarded: bool = sqlx::query_scalar(
        "SELECT username IS NOT NULL AND display_name IS NOT NULL FROM public.users WHERE id=$1 AND deleted_at IS NULL FOR UPDATE",
    )
        .bind(user)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?
        .ok_or_else(not_found)?;
    if !onboarded {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "complete profile required",
        ));
    }
    Ok(())
}

async fn owner_space(
    tx: &mut Transaction<'_, Postgres>,
    space: &str,
    user: i64,
) -> Result<i64, ApiError> {
    sqlx::query_scalar("SELECT id FROM public.spaces WHERE external_id=$1 AND owner_id=$2 AND deleted_at IS NULL AND NOT demo FOR UPDATE")
        .bind(space).bind(user).fetch_optional(&mut **tx).await.map_err(database_error)?.ok_or_else(not_found)
}

async fn accessible_space(pool: &PgPool, space: &str, user: i64) -> Result<i64, ApiError> {
    sqlx::query_scalar("SELECT s.id FROM public.spaces s WHERE s.external_id=$1 AND s.deleted_at IS NULL AND NOT s.demo AND EXISTS(SELECT 1 FROM public.space_members WHERE space_id=s.id AND user_id=$2 AND deleted_at IS NULL)")
        .bind(space).bind(user).fetch_optional(pool).await.map_err(database_error)?.ok_or_else(not_found)
}

async fn channel_for_update(
    tx: &mut Transaction<'_, Postgres>,
    space: i64,
    channel: &str,
) -> Result<i64, ApiError> {
    sqlx::query_scalar("SELECT id FROM public.channels WHERE external_id=$1 AND space_id=$2 AND deleted_at IS NULL FOR UPDATE")
        .bind(channel).bind(space).fetch_optional(&mut **tx).await.map_err(database_error)?.ok_or_else(not_found)
}

async fn owned_channel(
    pool: &PgPool,
    space: &str,
    channel: &str,
    user: i64,
) -> Result<(i64, i64), ApiError> {
    sqlx::query_as("SELECT s.id,c.id FROM public.spaces s JOIN public.channels c ON c.space_id=s.id WHERE s.external_id=$1 AND c.external_id=$2 AND s.owner_id=$3 AND s.deleted_at IS NULL AND c.deleted_at IS NULL AND NOT s.demo")
        .bind(space).bind(channel).bind(user).fetch_optional(pool).await.map_err(database_error)?.ok_or_else(not_found)
}

async fn find_user_for_update(
    tx: &mut Transaction<'_, Postgres>,
    username: &str,
) -> Result<(i64, String, i16, String, String), ApiError> {
    let username = username.trim().to_ascii_lowercase();
    if !crate::accounts::valid_username(&username) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid username"));
    }
    sqlx::query_as("SELECT id,external_id,avatar_id,username,display_name FROM public.users WHERE username=$1 AND deleted_at IS NULL AND display_name IS NOT NULL FOR UPDATE")
        .bind(username).fetch_optional(&mut **tx).await.map_err(database_error)?
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "user not found"))
}

/// Someone who blocked the owner never receives their space or channel
/// invitations. Refuse as an unknown account rather than confirming the block.
async fn ensure_invitee_allows(
    tx: &mut Transaction<'_, Postgres>,
    invitee: i64,
    owner: i64,
) -> Result<(), ApiError> {
    let blocked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.user_blocks WHERE blocker_id=$1 AND blocked_id=$2 AND deleted_at IS NULL)",
    )
    .bind(invitee)
    .bind(owner)
    .fetch_one(&mut **tx)
    .await
    .map_err(database_error)?;
    if blocked {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "user not found"));
    }
    Ok(())
}

async fn members(pool: &PgPool, space: i64) -> Result<Vec<Member>, ApiError> {
    Ok(sqlx::query_as::<_,(String,i16,String,String,bool)>("SELECT u.external_id,u.avatar_id,u.username,u.display_name,s.owner_id=u.id FROM public.space_members sm JOIN public.users u ON u.id=sm.user_id JOIN public.spaces s ON s.id=sm.space_id WHERE sm.space_id=$1 AND sm.deleted_at IS NULL AND u.deleted_at IS NULL ORDER BY (s.owner_id=u.id) DESC,lower(u.username),u.id")
        .bind(space).fetch_all(pool).await.map_err(database_error)?.into_iter().map(|(id,avatar_id,username,display_name,owner)|Member{id,avatar_id,username,display_name,owner}).collect())
}

fn constraint_or_database(error: sqlx::Error, message: &'static str) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
    {
        conflict(message)
    } else {
        database_error(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cloudflare, Config, accounts::User};
    use sqlx::{
        Connection, Executor,
        postgres::{PgConnectOptions, PgPoolOptions},
    };
    use std::{str::FromStr, sync::Arc};
    use uuid::Uuid;

    #[test]
    fn limits_require_positive_integers_and_default_only_when_absent() {
        assert_eq!(limit(None, 20, "LIMIT").unwrap(), 20);
        assert_eq!(limit(Some(" 7 ".into()), 20, "LIMIT").unwrap(), 7);
        for value in ["0", "-1", "", " ", "1.5", "many", "9223372036854775808"] {
            assert_eq!(
                limit(Some(value.into()), 20, "LIMIT").unwrap_err(),
                "LIMIT must be a positive integer",
                "{value}"
            );
        }
        assert_eq!(
            serde_json::to_value(Limits::default()).unwrap(),
            json!({"ownedSpaces":20,"totalSpaces":100,"channelsPerSpace":100})
        );
    }

    #[test]
    fn validates_names_at_exact_boundaries() {
        assert_eq!(space_name("  Café  ").unwrap(), "Café");
        assert!(space_name("\0bad").is_err());
        assert!(space_name(&"🙂".repeat(80)).is_ok());
        assert!(space_name(&"🙂".repeat(81)).is_err());
        for valid in ["a", "general", "one-two", "a-b-c"] {
            assert!(channel_name(valid).is_ok());
        }
        for invalid in ["", "General", "one--two", "-one", "one-", "123", "one_2"] {
            assert!(channel_name(invalid).is_err(), "{invalid}");
        }
    }

    fn principal(id: i64, external_id: &str, username: &str) -> Principal {
        Principal {
            user: User {
                id,
                external_id: external_id.to_owned(),
                avatar_id: 42,
                email: None,
                username: Some(username.to_owned()),
                display_name: Some(username.to_owned()),
            },
            token_hash: Vec::new(),
        }
    }

    #[tokio::test]
    #[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL"]
    async fn authorization_self_leave_soft_deletion_and_quota_races() {
        let url = std::env::var("CHAT_TEST_DATABASE_URL")
            .expect("CHAT_TEST_DATABASE_URL must point at disposable Postgres");
        let options = PgConnectOptions::from_str(&url).unwrap();
        assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
        let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
        let database = format!("spaces_test_{}", Uuid::new_v4().simple());
        admin
            .execute(format!("CREATE DATABASE {database}").as_str())
            .await
            .unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(12)
            .connect_with(options.database(&database))
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        crate::chat::seed(&pool).await.unwrap();

        let mut users = Vec::new();
        for username in ["owner", "member", "outsider", "quota_owner", "target"] {
            let external = random_id(12);
            let id: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,$2,$2) RETURNING id")
                .bind(&external).bind(username).fetch_one(&pool).await.unwrap();
            users.push((id, external, username));
        }
        let owner = principal(users[0].0, &users[0].1, users[0].2);
        let member = principal(users[1].0, &users[1].1, users[1].2);
        let outsider = principal(users[2].0, &users[2].1, users[2].2);
        let quota_owner = principal(users[3].0, &users[3].1, users[3].2);
        let target = principal(users[4].0, &users[4].1, users[4].2);

        let space = random_id(12);
        let space_id: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,'Friends',$2) RETURNING id")
            .bind(&space).bind(owner.user.id).fetch_one(&pool).await.unwrap();
        for user in [owner.user.id, member.user.id, outsider.user.id] {
            sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
                .bind(space_id)
                .bind(user)
                .execute(&pool)
                .await
                .unwrap();
        }
        let public = random_id(12);
        let private = random_id(12);
        let private_id: i64 = sqlx::query_scalar("INSERT INTO public.channels(external_id,space_id,name,private) VALUES($1,$2,'private',true) RETURNING id")
            .bind(&private).bind(space_id).fetch_one(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,'public')",
        )
        .bind(&public)
        .bind(space_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO public.channel_members(channel_id,user_id) VALUES($1,$2)")
            .bind(private_id)
            .bind(member.user.id)
            .execute(&pool)
            .await
            .unwrap();

        assert!(
            channel_access(&pool, &public, Some(member.user.id))
                .await
                .is_ok()
        );
        assert!(
            channel_access(&pool, &private, Some(owner.user.id))
                .await
                .is_ok()
        );
        assert!(
            channel_access(&pool, &private, Some(member.user.id))
                .await
                .is_ok()
        );
        assert_eq!(
            channel_access(&pool, &private, Some(outsider.user.id))
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            channel_access(&pool, &public, None)
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        let demo: String = sqlx::query_scalar("SELECT c.external_id FROM public.channels c JOIN public.spaces s ON s.id=c.space_id WHERE s.demo")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(
            channel_access(&pool, &demo, Some(member.user.id))
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );

        let mut config = Config::test(false);
        config.space_limits = Limits {
            owned_spaces: 3,
            total_spaces: 7,
            channels_per_space: 5,
        };
        let state =
            AppState::with_database(config, Arc::new(Cloudflare::new()), Some(pool.clone()));
        let listed = list_spaces(State(state.clone()), Extension(owner.clone()))
            .await
            .unwrap();
        assert_eq!(
            listed.0["limits"],
            json!({"ownedSpaces":3,"totalSpaces":7,"channelsPerSpace":5})
        );
        let incomplete_external = random_id(12);
        let incomplete_id: i64 =
            sqlx::query_scalar("INSERT INTO public.users(external_id) VALUES($1) RETURNING id")
                .bind(&incomplete_external)
                .fetch_one(&pool)
                .await
                .unwrap();
        let incomplete = create_space(
            State(state.clone()),
            Extension(Principal {
                user: User {
                    id: incomplete_id,
                    external_id: incomplete_external,
                    avatar_id: 42,
                    email: None,
                    username: None,
                    display_name: None,
                },
                token_hash: Vec::new(),
            }),
            Json(NameInput {
                name: "Not allowed".into(),
            }),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(incomplete.status, StatusCode::FORBIDDEN);
        let denied = remove_space_member(
            State(state.clone()),
            Extension(outsider.clone()),
            Path((space.clone(), member.user.external_id.clone())),
        )
        .await
        .unwrap_err();
        assert_eq!(denied.status, StatusCode::NOT_FOUND);
        assert_eq!(
            remove_space_member(
                State(state.clone()),
                Extension(member.clone()),
                Path((space.clone(), member.user.external_id.clone())),
            )
            .await
            .unwrap(),
            StatusCode::NO_CONTENT
        );
        // Leaving ends access but keeps both membership records.
        let periods = |table: &'static str, column: &'static str, scope: i64| {
            let pool = pool.clone();
            let user = member.user.id;
            async move {
                sqlx::query_as::<_, (i64, i64)>(&format!(
                    "SELECT count(*), count(*) FILTER (WHERE deleted_at IS NULL) FROM public.{table}
                     WHERE {column}=$1 AND user_id=$2"
                ))
                .bind(scope)
                .bind(user)
                .fetch_one(&pool)
                .await
                .unwrap()
            }
        };
        assert_eq!(periods("space_members", "space_id", space_id).await, (1, 0));
        assert_eq!(
            periods("channel_members", "channel_id", private_id).await,
            (1, 0)
        );
        assert_eq!(
            channel_access(&pool, &public, Some(member.user.id))
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        // Rejoining through a new invitation starts a new period without
        // reviving the old private grant, and removing again keeps every period.
        // Backdate the leave tombstones past the 24-hour re-invite cooldown.
        for table in ["space_invitations", "channel_invitations"] {
            sqlx::query(&format!(
                "UPDATE public.{table} SET updated_at=now()-interval '25 hours' WHERE user_id=$1"
            ))
            .bind(member.user.id)
            .execute(&pool)
            .await
            .unwrap();
        }
        let (status, _) = add_space_member(
            State(state.clone()),
            Extension(owner.clone()),
            Path(space.clone()),
            Json(MemberInput {
                username: "member".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::CREATED);
        let accepted = accept_invitation(
            State(state.clone()),
            Extension(member.clone()),
            Path(space.clone()),
        )
        .await
        .unwrap();
        assert_eq!(accepted.0.id, space);
        assert_eq!(periods("space_members", "space_id", space_id).await, (2, 1));
        assert_eq!(
            channel_access(&pool, &private, Some(member.user.id))
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        let (status, _) = add_channel_member(
            State(state.clone()),
            Extension(owner.clone()),
            Path((space.clone(), private.clone())),
            Json(MemberInput {
                username: "member".into(),
            }),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::CREATED);
        let joined = joining::accept_channel_invitation(
            State(state.clone()),
            Extension(member.clone()),
            Path((space.clone(), private.clone())),
        )
        .await
        .unwrap();
        assert!(joined.0.joined);
        assert_eq!(
            periods("channel_members", "channel_id", private_id).await,
            (2, 1)
        );
        assert_eq!(
            periods("channel_joins", "channel_id", private_id).await.1,
            1
        );
        assert!(
            channel_access(&pool, &private, Some(member.user.id))
                .await
                .is_ok()
        );
        assert_eq!(
            remove_space_member(
                State(state.clone()),
                Extension(owner.clone()),
                Path((space.clone(), member.user.external_id.clone())),
            )
            .await
            .unwrap(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(periods("space_members", "space_id", space_id).await, (2, 0));
        assert_eq!(
            periods("channel_members", "channel_id", private_id).await,
            (2, 0)
        );
        let joins = periods("channel_joins", "channel_id", private_id).await;
        assert_eq!((joins.0 >= 1, joins.1), (true, 0));
        assert_eq!(
            remove_space_member(
                State(state.clone()),
                Extension(owner.clone()),
                Path((space.clone(), member.user.external_id.clone())),
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            remove_space_member(
                State(state.clone()),
                Extension(owner.clone()),
                Path((space.clone(), owner.user.external_id.clone())),
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::CONFLICT
        );

        sqlx::query("UPDATE public.channels SET deleted_at=now() WHERE id=$1")
            .bind(private_id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            channel_access(&pool, &private, Some(owner.user.id))
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        let replacement = create_channel(
            State(state.clone()),
            Extension(owner.clone()),
            Path(space.clone()),
            Json(ChannelInput {
                name: "private".into(),
                private: false,
            }),
        )
        .await
        .unwrap();
        assert_eq!(replacement.1.0.name, "private");

        let account_hash = b"valid-account";
        sqlx::query("INSERT INTO public.account_sessions(token_hash,user_id,expires_at) VALUES($1,$2,now()+interval '1 hour')")
            .bind(account_hash.as_slice()).bind(owner.user.id).execute(&pool).await.unwrap();
        assert_eq!(
            session_user(&pool, account_hash).await.unwrap(),
            owner.user.id
        );
        sqlx::query("UPDATE public.account_sessions SET revoked_at=now() WHERE token_hash=$1")
            .bind(account_hash.as_slice())
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            session_user(&pool, account_hash).await.unwrap_err().status,
            StatusCode::UNAUTHORIZED
        );

        // Exactly one of two concurrent creates can consume the configured third slot.
        for index in 0..2 {
            let id: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,$2,$3) RETURNING id")
                .bind(random_id(12)).bind(format!("owned {index}")).bind(quota_owner.user.id).fetch_one(&pool).await.unwrap();
            sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
                .bind(id)
                .bind(quota_owner.user.id)
                .execute(&pool)
                .await
                .unwrap();
        }
        let one = create_space(
            State(state.clone()),
            Extension(quota_owner.clone()),
            Json(NameInput {
                name: "third a".into(),
            }),
        );
        let two = create_space(
            State(state.clone()),
            Extension(quota_owner.clone()),
            Json(NameInput {
                name: "third b".into(),
            }),
        );
        let (one, two) = tokio::join!(one, two);
        assert_eq!(
            [one.is_ok(), two.is_ok()]
                .into_iter()
                .filter(|ok| *ok)
                .count(),
            1
        );

        // Channel creation locks the space, so the active count cannot pass five.
        let channel_space = random_id(12);
        let channel_space_id:i64=sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,'Channel quota',$2) RETURNING id")
            .bind(&channel_space).bind(owner.user.id).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
            .bind(channel_space_id)
            .bind(owner.user.id)
            .execute(&pool)
            .await
            .unwrap();
        for index in 0..4 {
            sqlx::query("INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,$3)")
                .bind(random_id(12))
                .bind(channel_space_id)
                .bind(format!("channel-{index}"))
                .execute(&pool)
                .await
                .unwrap();
        }
        let one = create_channel(
            State(state.clone()),
            Extension(owner.clone()),
            Path(channel_space.clone()),
            Json(ChannelInput {
                name: "last-a".into(),
                private: false,
            }),
        );
        let two = create_channel(
            State(state.clone()),
            Extension(owner.clone()),
            Path(channel_space.clone()),
            Json(ChannelInput {
                name: "last-b".into(),
                private: false,
            }),
        );
        let (one, two) = tokio::join!(one, two);
        assert_eq!(
            [one.is_ok(), two.is_ok()]
                .into_iter()
                .filter(|ok| *ok)
                .count(),
            1
        );

        // Invitations don't consume membership quota. Concurrent acceptances
        // lock the recipient, so only one can consume the seventh active slot.
        for index in 0..6 {
            let id:i64=sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,$2,$3) RETURNING id")
                .bind(random_id(12)).bind(format!("membership {index}")).bind(owner.user.id).fetch_one(&pool).await.unwrap();
            sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
                .bind(id)
                .bind(target.user.id)
                .execute(&pool)
                .await
                .unwrap();
        }
        let destinations = [random_id(12), random_id(12)];
        for destination in &destinations {
            let id:i64=sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,'Destination',$2) RETURNING id")
                .bind(destination).bind(owner.user.id).fetch_one(&pool).await.unwrap();
            sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
                .bind(id)
                .bind(owner.user.id)
                .execute(&pool)
                .await
                .unwrap();
        }
        for destination in &destinations {
            assert_eq!(
                add_space_member(
                    State(state.clone()),
                    Extension(owner.clone()),
                    Path(destination.clone()),
                    Json(MemberInput {
                        username: "target".into()
                    })
                )
                .await
                .unwrap()
                .0,
                StatusCode::CREATED
            );
        }
        let one = accept_invitation(
            State(state.clone()),
            Extension(target.clone()),
            Path(destinations[0].clone()),
        );
        let two = accept_invitation(
            State(state.clone()),
            Extension(target.clone()),
            Path(destinations[1].clone()),
        );
        let (one, two) = tokio::join!(one, two);
        assert_eq!(
            [one.is_ok(), two.is_ok()]
                .into_iter()
                .filter(|ok| *ok)
                .count(),
            1
        );

        // Consent lifecycle, privacy and duplicate races on a separate destination.
        let consent = random_id(12);
        let consent_id: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,'Consent',$2) RETURNING id")
            .bind(&consent).bind(owner.user.id).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
            .bind(consent_id)
            .bind(owner.user.id)
            .execute(&pool)
            .await
            .unwrap();
        let consent_channel = random_id(12);
        sqlx::query(
            "INSERT INTO public.channels(external_id,space_id,name) VALUES($1,$2,'general')",
        )
        .bind(&consent_channel)
        .bind(consent_id)
        .execute(&pool)
        .await
        .unwrap();
        let invite = |username: &str| {
            add_space_member(
                State(state.clone()),
                Extension(owner.clone()),
                Path(consent.clone()),
                Json(MemberInput {
                    username: username.into(),
                }),
            )
        };
        for (username, status, message) in [
            ("bad name!", StatusCode::BAD_REQUEST, "invalid username"),
            ("missing_user", StatusCode::NOT_FOUND, "user not found"),
            ("owner", StatusCode::CONFLICT, "user already in space"),
        ] {
            let error = invite(username).await.err().unwrap();
            assert_eq!((error.status, error.message), (status, message));
        }
        // A non-owner cannot use the invite endpoint as an account directory.
        assert_eq!(
            add_space_member(
                State(state.clone()),
                Extension(outsider.clone()),
                Path(consent.clone()),
                Json(MemberInput {
                    username: "missing_user".into()
                })
            )
            .await
            .err()
            .unwrap()
            .message,
            "resource not found"
        );
        let (first, duplicate) = tokio::join!(invite(" MEMBER "), invite("member"));
        assert_eq!(
            usize::from(first.is_ok()) + usize::from(duplicate.is_ok()),
            1
        );
        assert_eq!(
            first.err().or_else(|| duplicate.err()).unwrap().message,
            "user already invited"
        );
        sqlx::query("UPDATE public.users SET display_name='Space Host' WHERE id=$1")
            .bind(owner.user.id)
            .execute(&pool)
            .await
            .unwrap();
        let listed = list_spaces(State(state.clone()), Extension(member.clone()))
            .await
            .unwrap()
            .0;
        assert!(
            !listed["spaces"]
                .as_array()
                .unwrap()
                .iter()
                .any(|space| space["id"] == consent)
        );
        assert_eq!(
            listed["invitations"][0],
            json!({"id":consent,"name":"Consent","ownerId":owner.user.external_id,
                "inviter":{"username":"owner","displayName":"Space Host"}})
        );
        assert_eq!(members(&pool, consent_id).await.unwrap().len(), 1);
        assert_eq!(
            get_space(
                State(state.clone()),
                Extension(member.clone()),
                Path(consent.clone())
            )
            .await
            .err()
            .unwrap()
            .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            channel_access(&pool, &consent_channel, Some(member.user.id))
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            list_invitations(
                State(state.clone()),
                Extension(member.clone()),
                Path(consent.clone())
            )
            .await
            .err()
            .unwrap()
            .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            accept_invitation(
                State(state.clone()),
                Extension(outsider.clone()),
                Path(consent.clone())
            )
            .await
            .err()
            .unwrap()
            .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            decline_invitation(
                State(state.clone()),
                Extension(member.clone()),
                Path(consent.clone())
            )
            .await
            .unwrap(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            invite("member").await.err().unwrap().status,
            StatusCode::CONFLICT
        );
        assert_eq!(
            accept_invitation(
                State(state.clone()),
                Extension(member.clone()),
                Path(consent.clone())
            )
            .await
            .err()
            .unwrap()
            .status,
            StatusCode::NOT_FOUND
        );
        sqlx::query("UPDATE public.space_invitations SET updated_at=now()-interval '24 hours' WHERE space_id=$1")
            .bind(consent_id).execute(&pool).await.unwrap();
        assert_eq!(invite("member").await.unwrap().0, StatusCode::CREATED);
        cancel_invitation(
            State(state.clone()),
            Extension(owner.clone()),
            Path((consent.clone(), member.user.external_id.clone())),
        )
        .await
        .unwrap();
        assert_eq!(
            accept_invitation(
                State(state.clone()),
                Extension(member.clone()),
                Path(consent.clone())
            )
            .await
            .err()
            .unwrap()
            .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            invite("member").await.err().unwrap().status,
            StatusCode::CONFLICT
        );
        sqlx::query("UPDATE public.space_invitations SET updated_at=now()-interval '24 hours' WHERE space_id=$1")
            .bind(consent_id).execute(&pool).await.unwrap();
        assert_eq!(invite("member").await.unwrap().0, StatusCode::CREATED);
        // Expiry excludes the invite and blocks acceptance, but permits a fresh invite.
        sqlx::query("UPDATE public.space_invitations SET updated_at=now()-interval '7 days' WHERE space_id=$1")
            .bind(consent_id).execute(&pool).await.unwrap();
        assert_eq!(
            list_spaces(State(state.clone()), Extension(member.clone()))
                .await
                .unwrap()
                .0["invitations"],
            json!([])
        );
        assert_eq!(
            accept_invitation(
                State(state.clone()),
                Extension(member.clone()),
                Path(consent.clone())
            )
            .await
            .err()
            .unwrap()
            .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(invite("member").await.unwrap().0, StatusCode::CREATED);
        let (accepted, repeated) = tokio::join!(
            accept_invitation(
                State(state.clone()),
                Extension(member.clone()),
                Path(consent.clone())
            ),
            accept_invitation(
                State(state.clone()),
                Extension(member.clone()),
                Path(consent.clone())
            )
        );
        assert_eq!(
            usize::from(accepted.is_ok()) + usize::from(repeated.is_ok()),
            1
        );
        assert!(
            channel_access(&pool, &consent_channel, Some(member.user.id))
                .await
                .is_ok()
        );
        assert_eq!(
            invite("member").await.err().unwrap().message,
            "user already in space"
        );
        remove_space_member(
            State(state.clone()),
            Extension(owner.clone()),
            Path((consent.clone(), member.user.external_id.clone())),
        )
        .await
        .unwrap();
        assert_eq!(
            invite("member").await.err().unwrap().status,
            StatusCode::CONFLICT
        );
        assert_eq!(
            channel_access(&pool, &consent_channel, Some(member.user.id))
                .await
                .unwrap_err()
                .status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            list_spaces(State(state.clone()), Extension(member.clone()))
                .await
                .unwrap()
                .0["spaces"],
            json!([])
        );

        // A concurrent decline must not report success after acceptance commits.
        sqlx::query("UPDATE public.space_invitations SET updated_at=now()-interval '24 hours' WHERE space_id=$1")
            .bind(consent_id).execute(&pool).await.unwrap();
        sqlx::query("UPDATE public.space_invite_limits SET window_start=now()-interval '10 minutes' WHERE user_id=$1")
            .bind(owner.user.id).execute(&pool).await.unwrap();
        assert_eq!(invite("member").await.unwrap().0, StatusCode::CREATED);
        let (accepted, declined) = tokio::join!(
            accept_invitation(
                State(state.clone()),
                Extension(member.clone()),
                Path(consent.clone())
            ),
            decline_invitation(
                State(state.clone()),
                Extension(member.clone()),
                Path(consent.clone())
            )
        );
        assert_eq!(
            usize::from(accepted.is_ok()) + usize::from(declined.is_ok()),
            1
        );
        let (status, belongs): (String, bool) = sqlx::query_as(
            "SELECT i.status,EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=i.space_id AND sm.user_id=i.user_id AND sm.deleted_at IS NULL)
             FROM public.space_invitations i WHERE i.space_id=$1 AND i.user_id=$2",
        ).bind(consent_id).bind(member.user.id).fetch_one(&pool).await.unwrap();
        assert_eq!(belongs, accepted.is_ok());
        assert_eq!(
            status,
            if accepted.is_ok() {
                "accepted"
            } else {
                "declined"
            }
        );
        if accepted.is_ok() {
            remove_space_member(
                State(state.clone()),
                Extension(owner.clone()),
                Path((consent.clone(), member.user.external_id.clone())),
            )
            .await
            .unwrap();
        }

        assert_eq!(
            accepted.err().or_else(|| declined.err()).unwrap().status,
            StatusCode::NOT_FOUND
        );

        // Every failed attempt is durable, and the 20th/21st boundary is atomic.
        sqlx::query("UPDATE public.space_invite_limits SET attempts=19 WHERE user_id=$1")
            .bind(owner.user.id)
            .execute(&pool)
            .await
            .unwrap();
        let (one, two) = tokio::join!(invite("missing_one"), invite("missing_two"));
        let statuses = [one.err().unwrap().status, two.err().unwrap().status];
        assert!(statuses.contains(&StatusCode::NOT_FOUND));
        assert!(statuses.contains(&StatusCode::TOO_MANY_REQUESTS));
        assert_eq!(
            invite("missing_user").await.err().unwrap().status,
            StatusCode::TOO_MANY_REQUESTS
        );
        sqlx::query("UPDATE public.space_invite_limits SET window_start=now()-interval '10 minutes' WHERE user_id=$1")
            .bind(owner.user.id).execute(&pool).await.unwrap();
        assert_eq!(
            invite("missing_user").await.err().unwrap().status,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            sqlx::query_scalar::<_, i32>(
                "SELECT attempts FROM public.space_invite_limits WHERE user_id=$1"
            )
            .bind(owner.user.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );

        // Recipient cap serializes concurrent senders even across different spaces.
        sqlx::query("WITH destinations AS (INSERT INTO public.spaces(external_id,name,owner_id)
            SELECT 'cap'||lpad(n::text,9,'0'),'Cap fixture',$1 FROM generate_series(1,49) n RETURNING id)
            INSERT INTO public.space_invitations(space_id,user_id,status) SELECT id,$2,'pending' FROM destinations")
            .bind(owner.user.id).bind(member.user.id).execute(&pool).await.unwrap();
        sqlx::query("UPDATE public.space_invitations SET updated_at=now()-interval '24 hours' WHERE space_id=$1 AND user_id=$2")
            .bind(consent_id).bind(member.user.id).execute(&pool).await.unwrap();
        let another = add_space_member(
            State(state.clone()),
            Extension(owner.clone()),
            Path(destinations[0].clone()),
            Json(MemberInput {
                username: "member".into(),
            }),
        );
        let (one, two) = tokio::join!(invite("member"), another);
        assert_eq!(usize::from(one.is_ok()) + usize::from(two.is_ok()), 1);
        assert_eq!(
            one.err().or_else(|| two.err()).unwrap().message,
            "pending invitation limit reached"
        );

        pool.close().await;
        admin
            .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires disposable loopback CHAT_TEST_DATABASE_URL"]
    async fn blocks_stop_invitations_and_outsiders_cannot_probe_spaces() {
        let url = std::env::var("CHAT_TEST_DATABASE_URL")
            .expect("CHAT_TEST_DATABASE_URL must point at disposable Postgres");
        let options = PgConnectOptions::from_str(&url).unwrap();
        assert!(matches!(options.get_host(), "127.0.0.1" | "localhost"));
        let mut admin = sqlx::PgConnection::connect_with(&options).await.unwrap();
        let database = format!("spaces_block_test_{}", Uuid::new_v4().simple());
        admin
            .execute(format!("CREATE DATABASE {database}").as_str())
            .await
            .unwrap();
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.database(&database))
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let mut users = Vec::new();
        for username in ["owner", "invitee", "outsider"] {
            let external = random_id(12);
            let id: i64 = sqlx::query_scalar("INSERT INTO public.users(external_id,username,display_name) VALUES($1,$2,$2) RETURNING id")
                .bind(&external).bind(username).fetch_one(&pool).await.unwrap();
            users.push(principal(id, &external, username));
        }
        let (owner, invitee, outsider) = (&users[0], &users[1], &users[2]);
        let state = AppState::with_database(
            Config::test(true),
            Arc::new(Cloudflare::new()),
            Some(pool.clone()),
        );
        let space = |name: &'static str| {
            let pool = pool.clone();
            let owner = owner.user.id;
            async move {
                let external = random_id(12);
                let id: i64 = sqlx::query_scalar("INSERT INTO public.spaces(external_id,name,owner_id) VALUES($1,$2,$3) RETURNING id")
                    .bind(&external).bind(name).bind(owner).fetch_one(&pool).await.unwrap();
                sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
                    .bind(id)
                    .bind(owner)
                    .execute(&pool)
                    .await
                    .unwrap();
                (id, external)
            }
        };
        let invite = |space: String| {
            add_space_member(
                State(state.clone()),
                Extension(owner.clone()),
                Path(space),
                Json(MemberInput {
                    username: "invitee".into(),
                }),
            )
        };
        let block = |active: bool| {
            let pool = pool.clone();
            let (blocker, blocked) = (invitee.user.id, owner.user.id);
            async move {
                if active {
                    sqlx::query(
                        "INSERT INTO public.user_blocks(blocker_id,blocked_id) VALUES($1,$2)",
                    )
                    .bind(blocker)
                    .bind(blocked)
                    .execute(&pool)
                    .await
                    .unwrap();
                } else {
                    sqlx::query("UPDATE public.user_blocks SET deleted_at=now() WHERE blocker_id=$1 AND blocked_id=$2 AND deleted_at IS NULL")
                        .bind(blocker).bind(blocked).execute(&pool).await.unwrap();
                }
            }
        };
        let pending_spaces = || async {
            list_spaces(State(state.clone()), Extension(invitee.clone()))
                .await
                .unwrap()
                .0["invitations"]
                .as_array()
                .unwrap()
                .len()
        };

        // A pending invitation is hidden while its sender is blocked, and the
        // blocked owner can't send another one through a fresh space.
        let (_, first) = space("First").await;
        assert_eq!(invite(first.clone()).await.unwrap().0, StatusCode::CREATED);
        assert_eq!(pending_spaces().await, 1);
        block(true).await;
        assert_eq!(pending_spaces().await, 0);
        let (_, second) = space("Second").await;
        let refused = invite(second.clone()).await.err().unwrap();
        assert_eq!(
            (refused.status, refused.message),
            (StatusCode::NOT_FOUND, "user not found")
        );
        block(false).await;
        assert_eq!(pending_spaces().await, 1);
        assert_eq!(invite(second).await.unwrap().0, StatusCode::CREATED);
        assert_eq!(pending_spaces().await, 2);

        // Private channel invitations inside a shared space follow the same rule.
        let (shared_id, shared) = space("Shared").await;
        sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
            .bind(shared_id)
            .bind(invitee.user.id)
            .execute(&pool)
            .await
            .unwrap();
        let channel = random_id(12);
        sqlx::query("INSERT INTO public.channels(external_id,space_id,name,private) VALUES($1,$2,'quiet',true)")
            .bind(&channel).bind(shared_id).execute(&pool).await.unwrap();
        let invite_channel = || {
            add_channel_member(
                State(state.clone()),
                Extension(owner.clone()),
                Path((shared.clone(), channel.clone())),
                Json(MemberInput {
                    username: "invitee".into(),
                }),
            )
        };
        let pending_channels = || async {
            get_space(
                State(state.clone()),
                Extension(invitee.clone()),
                Path(shared.clone()),
            )
            .await
            .unwrap()
            .0["channelInvitations"]
                .as_array()
                .unwrap()
                .len()
        };
        block(true).await;
        assert_eq!(
            invite_channel().await.err().unwrap().status,
            StatusCode::NOT_FOUND
        );
        block(false).await;
        assert_eq!(invite_channel().await.unwrap().0, StatusCode::CREATED);
        assert_eq!(pending_channels().await, 1);
        block(true).await;
        assert_eq!(pending_channels().await, 0);

        // Outsiders get the same 404 for real and missing spaces.
        for space in [first.clone(), random_id(12)] {
            let removal = remove_space_member(
                State(state.clone()),
                Extension(outsider.clone()),
                Path((space.clone(), owner.user.external_id.clone())),
            )
            .await
            .unwrap_err();
            assert_eq!(removal.status, StatusCode::NOT_FOUND, "{space}");
        }
        sqlx::query("UPDATE public.users SET username=NULL,display_name=NULL WHERE id=$1")
            .bind(outsider.user.id)
            .execute(&pool)
            .await
            .unwrap();
        let accepted = accept_invitation(
            State(state.clone()),
            Extension(outsider.clone()),
            Path(first.clone()),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(accepted.status, StatusCode::NOT_FOUND);
        // The owner still can't remove themselves.
        let own = remove_space_member(
            State(state.clone()),
            Extension(owner.clone()),
            Path((first, owner.user.external_id.clone())),
        )
        .await
        .unwrap_err();
        assert_eq!(own.status, StatusCode::CONFLICT);

        pool.close().await;
        admin
            .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
            .await
            .unwrap();
    }
}
