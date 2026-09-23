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
    pub(crate) space_id: i64,
    pub(crate) demo: bool,
}

fn database_error(_: sqlx::Error) -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "spaces unavailable")
}

fn not_found() -> ApiError {
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

pub(crate) async fn channel_access(
    pool: &PgPool,
    channel: &str,
    user: Option<i64>,
) -> Result<ChannelAccess, ApiError> {
    sqlx::query_as::<_, (i64, i64, i64, bool)>(
        "SELECT c.id, c.last_seq, s.id, s.demo
         FROM public.channels c JOIN public.spaces s ON s.id = c.space_id
         WHERE c.external_id = $1 AND c.deleted_at IS NULL AND s.deleted_at IS NULL
           AND ((s.demo AND c.name = 'General') OR
                ($2::bigint IS NOT NULL
                 AND EXISTS (SELECT 1 FROM public.space_members sm WHERE sm.space_id = s.id AND sm.user_id = $2)
                 AND (s.owner_id = $2 OR NOT c.private OR
                      EXISTS (SELECT 1 FROM public.channel_members cm WHERE cm.channel_id = c.id AND cm.user_id = $2))))",
    )
    .bind(channel)
    .bind(user)
    .fetch_optional(pool)
    .await
    .map_err(database_error)?
    .map(|(id, last_seq, space_id, demo)| ChannelAccess {
        id,
        last_seq,
        space_id,
        demo,
    })
    .ok_or_else(not_found)
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
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Member {
    id: String,
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
            "/api/spaces/{space}/members",
            get(list_members).post(add_space_member),
        )
        .route(
            "/api/spaces/{space}/members/{user}",
            delete(remove_space_member),
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
         WHERE sm.user_id = $1 AND s.deleted_at IS NULL AND NOT s.demo
         ORDER BY lower(s.name), s.id",
    )
    .bind(principal.user.id)
    .fetch_all(pool(&state)?)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(|(id, name, owner_id)| Space { id, name, owner_id })
    .collect();
    Ok(Json(
        json!({"spaces":spaces,"limits":state.config.space_limits}),
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
           (SELECT count(*) FROM public.space_members sm JOIN public.spaces s ON s.id = sm.space_id WHERE sm.user_id = $1 AND s.deleted_at IS NULL)",
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
    sqlx::query("INSERT INTO public.channels (external_id,space_id,name) VALUES ($1,$2,'general')")
        .bind(random_id(12))
        .bind(internal)
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
           AND EXISTS (SELECT 1 FROM public.space_members WHERE space_id=s.id AND user_id=$2)",
    )
    .bind(&space)
    .bind(principal.user.id)
    .fetch_optional(pool)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    let channels: Vec<Channel> = sqlx::query_as::<_, (String, String, String, bool)>(
        "SELECT c.external_id,s.external_id,c.name,c.private FROM public.channels c
         JOIN public.spaces s ON s.id=c.space_id
         WHERE c.space_id=$1 AND c.deleted_at IS NULL
           AND (s.owner_id=$2 OR NOT c.private OR EXISTS (SELECT 1 FROM public.channel_members WHERE channel_id=c.id AND user_id=$2))
         ORDER BY c.id",
    )
    .bind(row.0)
    .bind(principal.user.id)
    .fetch_all(pool)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(|(id, space_id, name, private)| Channel { id, space_id, name, private })
    .collect();
    let members = members(pool, row.0).await?;
    Ok(Json(
        json!({"space":Space{id:row.2,name:row.1,owner_id:row.3},"channels":channels,"members":members}),
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
    sqlx::query(
        "INSERT INTO public.channels (external_id,space_id,name,private) VALUES ($1,$2,$3,$4)",
    )
    .bind(&id)
    .bind(space_id)
    .bind(name)
    .bind(input.private)
    .execute(&mut *tx)
    .await
    .map_err(|error| constraint_or_database(error, "channel name already exists"))?;
    tx.commit().await.map_err(database_error)?;
    Ok((
        StatusCode::CREATED,
        Json(Channel {
            id,
            space_id: space,
            name: name.to_owned(),
            private: input.private,
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
    tx.commit().await.map_err(database_error)?;
    Ok(Json(Channel {
        id: channel,
        space_id: space,
        name: name.to_owned(),
        private: input.private,
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
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let space_id = owner_space(&mut tx, &space, principal.user.id).await?;
    let member = find_user_for_update(&mut tx, &input.username).await?;
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM public.space_members sm JOIN public.spaces s ON s.id=sm.space_id WHERE sm.user_id=$1 AND s.deleted_at IS NULL")
        .bind(member.0).fetch_one(&mut *tx).await.map_err(database_error)?;
    let exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.space_members WHERE space_id=$1 AND user_id=$2)",
    )
    .bind(space_id)
    .bind(member.0)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if !exists && count >= state.config.space_limits.total_spaces {
        return Err(conflict("membership limit reached"));
    }
    if !exists {
        sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
            .bind(space_id)
            .bind(member.0)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
    }
    tx.commit().await.map_err(database_error)?;
    Ok((
        if exists {
            StatusCode::OK
        } else {
            StatusCode::CREATED
        },
        Json(Member {
            id: member.1,
            username: member.2,
            display_name: member.3,
            owner: member.0 == principal.user.id,
        }),
    ))
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
    if target == owner_id {
        return Err(conflict("owner cannot be removed"));
    }
    if principal.user.id != owner_id && principal.user.id != target {
        return Err(not_found());
    }
    sqlx::query("DELETE FROM public.channel_members cm USING public.channels c WHERE cm.channel_id=c.id AND c.space_id=$1 AND cm.user_id=$2")
        .bind(space_id).bind(target).execute(&mut *tx).await.map_err(database_error)?;
    let changed = sqlx::query("DELETE FROM public.space_members WHERE space_id=$1 AND user_id=$2")
        .bind(space_id)
        .bind(target)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    if changed.rows_affected() != 1 {
        return Err(not_found());
    }
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
    let rows: Vec<Member> = sqlx::query_as::<_, (String, String, String, bool)>(
        "SELECT u.external_id,u.username,u.display_name,s.owner_id=u.id
         FROM public.space_members sm JOIN public.users u ON u.id=sm.user_id
         JOIN public.spaces s ON s.id=sm.space_id
         WHERE sm.space_id=$2 AND u.deleted_at IS NULL
           AND (s.owner_id=u.id OR EXISTS(SELECT 1 FROM public.channel_members cm WHERE cm.channel_id=$1 AND cm.user_id=u.id))
         ORDER BY (s.owner_id=u.id) DESC,lower(u.username),u.id",
    )
    .bind(channel_id)
    .bind(space_id)
    .fetch_all(pool)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(|(id, username, display_name, owner)| Member {
        id,
        username,
        display_name,
        owner,
    })
    .collect();
    Ok(Json(json!({"members":rows})))
}

async fn add_channel_member(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
    Json(input): Json<MemberInput>,
) -> Result<(StatusCode, Json<Member>), ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let space_id = owner_space(&mut tx, &space, principal.user.id).await?;
    let channel_id = channel_for_update(&mut tx, space_id, &channel).await?;
    let member = find_user_for_update(&mut tx, &input.username).await?;
    let belongs: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.space_members WHERE space_id=$1 AND user_id=$2)",
    )
    .bind(space_id)
    .bind(member.0)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if !belongs {
        return Err(not_found());
    }
    let changed=sqlx::query("INSERT INTO public.channel_members(channel_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(channel_id).bind(member.0).execute(&mut *tx).await.map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok((
        if changed.rows_affected() == 1 {
            StatusCode::CREATED
        } else {
            StatusCode::OK
        },
        Json(Member {
            id: member.1,
            username: member.2,
            display_name: member.3,
            owner: member.0 == principal.user.id,
        }),
    ))
}

async fn remove_channel_member(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel, user)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let space_id = owner_space(&mut tx, &space, principal.user.id).await?;
    let channel_id = channel_for_update(&mut tx, space_id, &channel).await?;
    let target: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM public.users WHERE external_id=$1 AND deleted_at IS NULL",
    )
    .bind(user)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    let target = target.ok_or_else(not_found)?;
    if target == principal.user.id {
        return Err(conflict("owner cannot be removed"));
    }
    let changed =
        sqlx::query("DELETE FROM public.channel_members WHERE channel_id=$1 AND user_id=$2")
            .bind(channel_id)
            .bind(target)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
    if changed.rows_affected() != 1 {
        return Err(not_found());
    }
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
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
    sqlx::query_scalar("SELECT s.id FROM public.spaces s WHERE s.external_id=$1 AND s.deleted_at IS NULL AND NOT s.demo AND EXISTS(SELECT 1 FROM public.space_members WHERE space_id=s.id AND user_id=$2)")
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
) -> Result<(i64, String, String, String), ApiError> {
    sqlx::query_as("SELECT id,external_id,username,display_name FROM public.users WHERE username=$1 AND deleted_at IS NULL AND display_name IS NOT NULL FOR UPDATE")
        .bind(username).fetch_optional(&mut **tx).await.map_err(database_error)?.ok_or_else(not_found)
}

async fn members(pool: &PgPool, space: i64) -> Result<Vec<Member>, ApiError> {
    Ok(sqlx::query_as::<_,(String,String,String,bool)>("SELECT u.external_id,u.username,u.display_name,s.owner_id=u.id FROM public.space_members sm JOIN public.users u ON u.id=sm.user_id JOIN public.spaces s ON s.id=sm.space_id WHERE sm.space_id=$1 AND u.deleted_at IS NULL ORDER BY (s.owner_id=u.id) DESC,lower(u.username),u.id")
        .bind(space).fetch_all(pool).await.map_err(database_error)?.into_iter().map(|(id,username,display_name,owner)|Member{id,username,display_name,owner}).collect())
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
        assert!(channel_access(&pool, &demo, None).await.unwrap().demo);

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
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM public.channel_members WHERE channel_id=$1 AND user_id=$2"
            )
            .bind(private_id)
            .bind(member.user.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            channel_access(&pool, &public, Some(member.user.id))
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

        // Membership creation locks the target user, so only one seventh active
        // membership can be added by concurrent owners.
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
        let one = add_space_member(
            State(state.clone()),
            Extension(owner.clone()),
            Path(destinations[0].clone()),
            Json(MemberInput {
                username: "target".into(),
            }),
        );
        let two = add_space_member(
            State(state),
            Extension(owner),
            Path(destinations[1].clone()),
            Json(MemberInput {
                username: "target".into(),
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

        pool.close().await;
        admin
            .execute(format!("DROP DATABASE {database} WITH (FORCE)").as_str())
            .await
            .unwrap();
    }
}
