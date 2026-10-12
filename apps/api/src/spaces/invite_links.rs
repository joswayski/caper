//! Shareable invite links, modeled on Discord invites. Owners create a code
//! with an optional expiry, use cap and landing channel; anyone holding a live
//! code can preview the space and join it. Codes are random and unguessable,
//! so the public preview reveals only what the owner chose to share.
use super::*;
use chrono::{DateTime, Utc};

const CODE_LENGTH: usize = 10;
/// Expiry from one minute to a year, in seconds; `None` never expires.
const MAX_AGE: std::ops::RangeInclusive<i64> = 60..=31_536_000;
/// Spaces have no member cap, so this bound is a practical one.
const MAX_USES: std::ops::RangeInclusive<i32> = 1..=10_000;
const ACTIVE_LINKS_PER_SPACE: i64 = 100;
const LIVE: &str = "l.revoked_at IS NULL AND (l.expires_at IS NULL OR l.expires_at > now())
     AND (l.max_uses IS NULL OR l.uses < l.max_uses)";

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct LinkInput {
    #[serde(default)]
    channel_id: Option<String>,
    /// Seconds until expiry; omitted or null never expires.
    #[serde(default)]
    max_age: Option<i64>,
    /// Omitted or null allows unlimited joins.
    #[serde(default)]
    max_uses: Option<i32>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LinkChannel {
    id: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Person {
    username: String,
    display_name: String,
    avatar_id: i16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InviteLink {
    code: String,
    channel: Option<LinkChannel>,
    created_by: Person,
    uses: i32,
    max_uses: Option<i32>,
    expires_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

type LinkRow = (
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    i16,
    i32,
    Option<i32>,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
);

fn link(
    (
        code,
        channel_id,
        channel_name,
        username,
        display_name,
        avatar_id,
        uses,
        max_uses,
        expires_at,
        created_at,
    ): LinkRow,
) -> InviteLink {
    InviteLink {
        code,
        channel: channel_id
            .zip(channel_name)
            .map(|(id, name)| LinkChannel { id, name }),
        created_by: Person {
            username,
            display_name,
            avatar_id,
        },
        uses,
        max_uses,
        expires_at,
        created_at,
    }
}

// Landing channels are shown only while still public; otherwise joins use the
// starter channel, so a link never reveals a private channel's name.
const LINK_SELECT: &str =
    "SELECT l.code,c.external_id,c.name,u.username,u.display_name,u.avatar_id,
        l.uses,l.max_uses,l.expires_at,l.created_at
     FROM public.space_invite_links l JOIN public.users u ON u.id=l.created_by
     LEFT JOIN public.channels c ON c.id=l.channel_id AND c.deleted_at IS NULL AND NOT c.private";

pub(super) fn valid_code(code: &str) -> bool {
    code.len() == CODE_LENGTH && code.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn validate(input: &LinkInput) -> Result<(), ApiError> {
    if input.max_age.is_some_and(|age| !MAX_AGE.contains(&age)) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid maxAge"));
    }
    if input.max_uses.is_some_and(|uses| !MAX_USES.contains(&uses)) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid maxUses"));
    }
    Ok(())
}

pub(super) async fn list_links(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let pool = pool(&state)?;
    let id: i64 = sqlx::query_scalar("SELECT id FROM public.spaces WHERE external_id=$1 AND owner_id=$2 AND deleted_at IS NULL AND NOT demo")
        .bind(&space).bind(principal.user.id).fetch_optional(pool).await.map_err(database_error)?.ok_or_else(not_found)?;
    let links: Vec<InviteLink> = sqlx::query_as::<_, LinkRow>(&format!(
        "{LINK_SELECT} WHERE l.space_id=$1 AND {LIVE} ORDER BY l.created_at DESC,l.id DESC"
    ))
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(link)
    .collect();
    Ok(Json(json!({"links":links})))
}

pub(super) async fn create_link(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(space): Path<String>,
    Json(input): Json<LinkInput>,
) -> Result<(StatusCode, Json<InviteLink>), ApiError> {
    validate(&input)?;
    let pool = pool(&state)?;
    let owns: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.spaces WHERE external_id=$1 AND owner_id=$2 AND deleted_at IS NULL AND NOT demo)")
        .bind(&space).bind(principal.user.id).fetch_one(pool).await.map_err(database_error)?;
    if !owns {
        return Err(not_found());
    }
    // Share the owner's invitation budget so links cannot be minted in bulk.
    invite_rate_limit(pool, principal.user.id).await?;
    let mut tx = pool.begin().await.map_err(database_error)?;
    let space_id = owner_space(&mut tx, &space, principal.user.id).await?;
    let channel_id = match &input.channel_id {
        None => None,
        Some(channel) => {
            let (id, private): (i64, bool) = sqlx::query_as("SELECT id,private FROM public.channels WHERE external_id=$1 AND space_id=$2 AND deleted_at IS NULL")
                .bind(channel).bind(space_id).fetch_optional(&mut *tx).await.map_err(database_error)?.ok_or_else(not_found)?;
            if private {
                return Err(conflict("invite links cannot target private channels"));
            }
            Some(id)
        }
    };
    let active: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM public.space_invite_links l WHERE l.space_id=$1 AND {LIVE}"
    ))
    .bind(space_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if active >= ACTIVE_LINKS_PER_SPACE {
        return Err(conflict("invite link limit reached"));
    }
    let code = random_id(CODE_LENGTH);
    sqlx::query(
        "INSERT INTO public.space_invite_links(code,space_id,channel_id,created_by,max_uses,expires_at)
         VALUES($1,$2,$3,$4,$5,now()+make_interval(secs=>$6::double precision))",
    )
    .bind(&code)
    .bind(space_id)
    .bind(channel_id)
    .bind(principal.user.id)
    .bind(input.max_uses)
    .bind(input.max_age.map(|age| age as f64))
    .execute(&mut *tx)
    .await
    .map_err(|error| constraint_or_database(error, "invite link code collision; try again"))?;
    let created = sqlx::query_as::<_, LinkRow>(&format!("{LINK_SELECT} WHERE l.code=$1"))
        .bind(&code)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok((StatusCode::CREATED, Json(link(created))))
}

pub(super) async fn revoke_link(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, code)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let id = owner_space(&mut tx, &space, principal.user.id).await?;
    let changed = sqlx::query(
        "UPDATE public.space_invite_links SET revoked_at=now()
         WHERE space_id=$1 AND code=$2 AND revoked_at IS NULL",
    )
    .bind(id)
    .bind(code)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    if changed.rows_affected() != 1 {
        return Err(not_found());
    }
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

type PreviewRow = (
    String,
    Option<String>,
    String,
    String,
    i16,
    i64,
    Option<DateTime<Utc>>,
);

/// Public, unauthenticated preview for the invite page and native join sheet.
/// Shares only the space name, landing channel, inviter and member count.
pub(crate) async fn preview(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if !valid_code(&code) {
        return Err(not_found());
    }
    let row: Option<PreviewRow> =
        sqlx::query_as(&format!(
            "SELECT s.name,c.name,u.username,u.display_name,u.avatar_id,
                (SELECT count(*) FROM public.space_members sm JOIN public.users mu ON mu.id=sm.user_id
                 WHERE sm.space_id=s.id AND sm.deleted_at IS NULL AND mu.deleted_at IS NULL),
                l.expires_at
             FROM public.space_invite_links l JOIN public.spaces s ON s.id=l.space_id
             JOIN public.users u ON u.id=l.created_by
             LEFT JOIN public.channels c ON c.id=l.channel_id AND c.deleted_at IS NULL AND NOT c.private
             WHERE l.code=$1 AND s.deleted_at IS NULL AND NOT s.demo AND {LIVE}"
        ))
        .bind(&code)
        .fetch_optional(pool(&state)?)
        .await
        .map_err(database_error)?;
    let (space, channel, username, display_name, avatar_id, members, expires_at) =
        row.ok_or_else(invalid_link)?;
    Ok(Json(json!({
        "code": code,
        "space": {"name": space},
        "channel": channel.map(|name| json!({"name": name})),
        "inviter": Person { username, display_name, avatar_id },
        "memberCount": members,
        "expiresAt": expires_at,
    })))
}

fn invalid_link() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "invite link is invalid or has expired",
    )
}

/// Joins the space behind a live link. Members who already belong get the same
/// response without consuming a use, so links double as space shortcuts.
pub(crate) async fn accept(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(code): Path<String>,
) -> Result<Json<Value>, ApiError> {
    if !valid_code(&code) {
        return Err(invalid_link());
    }
    let user = principal.user.id;
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    // Space first, matching the space -> recipient lock order used by
    // invitation and removal paths; then the link row for its use counter.
    let (space_id, space, name, owner_id, owner): (i64, String, String, String, i64) =
        sqlx::query_as(
            "SELECT s.id,s.external_id,s.name,o.external_id,s.owner_id FROM public.spaces s
         JOIN public.users o ON o.id=s.owner_id
         WHERE s.id=(SELECT space_id FROM public.space_invite_links WHERE code=$1)
           AND s.deleted_at IS NULL AND NOT s.demo FOR UPDATE OF s",
        )
        .bind(&code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_error)?
        .ok_or_else(invalid_link)?;
    let live: Option<(i64, Option<i64>, bool)> = sqlx::query_as(&format!(
        "SELECT l.id,c.id,{LIVE} FROM public.space_invite_links l
         LEFT JOIN public.channels c ON c.id=l.channel_id AND c.deleted_at IS NULL AND NOT c.private
         WHERE l.code=$1 FOR UPDATE OF l"
    ))
    .bind(&code)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?;
    let (link_id, landing, live) = live.ok_or_else(invalid_link)?;
    let member: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.space_members WHERE space_id=$1 AND user_id=$2 AND deleted_at IS NULL)")
        .bind(space_id).bind(user).fetch_one(&mut *tx).await.map_err(database_error)?;
    let starter = "SELECT id FROM public.channels WHERE space_id=$1 AND NOT private AND deleted_at IS NULL ORDER BY (name='general') DESC,id LIMIT 1";
    let channel_id = match landing {
        Some(id) => Some(id),
        None => sqlx::query_scalar(starter)
            .bind(space_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?,
    };
    if !member {
        if !live {
            return Err(invalid_link());
        }
        let removed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.space_invitations WHERE space_id=$1 AND user_id=$2 AND status='removed')")
            .bind(space_id).bind(user).fetch_one(&mut *tx).await.map_err(database_error)?;
        // An owner who blocked this account has excluded them as surely as a
        // removal; answer identically so neither is confirmed to the joiner.
        let blocked: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.user_blocks WHERE blocker_id=$1 AND blocked_id=$2 AND deleted_at IS NULL)")
            .bind(owner).bind(user).fetch_one(&mut *tx).await.map_err(database_error)?;
        if removed || blocked {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "you can't join this space with a link; ask the owner to invite you",
            ));
        }
        lock_onboarded_user(&mut tx, user).await?;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM public.space_members sm JOIN public.spaces s ON s.id=sm.space_id WHERE sm.user_id=$1 AND sm.deleted_at IS NULL AND s.deleted_at IS NULL")
            .bind(user).fetch_one(&mut *tx).await.map_err(database_error)?;
        if count >= state.config.space_limits.total_spaces {
            return Err(conflict("membership limit reached"));
        }
        sqlx::query("INSERT INTO public.space_members(space_id,user_id) VALUES($1,$2)")
            .bind(space_id)
            .bind(user)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("UPDATE public.space_invite_links SET uses=uses+1 WHERE id=$1")
            .bind(link_id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        sqlx::query("INSERT INTO public.space_invite_link_uses(link_id,user_id) VALUES($1,$2)")
            .bind(link_id)
            .bind(user)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        // A pending username invitation is fulfilled by joining.
        sqlx::query("UPDATE public.space_invitations SET status='accepted',updated_at=now() WHERE space_id=$1 AND user_id=$2 AND status='pending'")
            .bind(space_id).bind(user).execute(&mut *tx).await.map_err(database_error)?;
    }
    if let Some(channel) = channel_id {
        sqlx::query("INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
            .bind(channel).bind(user).execute(&mut *tx).await.map_err(database_error)?;
    }
    let channel: Option<String> = match channel_id {
        Some(id) => Some(
            sqlx::query_scalar("SELECT external_id FROM public.channels WHERE id=$1")
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(database_error)?,
        ),
        None => None,
    };
    tx.commit().await.map_err(database_error)?;
    Ok(Json(json!({
        "space": Space { id: space, name, owner_id },
        "channelId": channel,
        "joined": !member,
    })))
}
