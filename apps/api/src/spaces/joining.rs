//! Persistent participation and recipient consent, separate from access grants.
use super::*;

pub(crate) async fn channel_participation(
    pool: &PgPool,
    channel: &str,
    user: Option<i64>,
) -> Result<ChannelAccess, ApiError> {
    check_channel_access(pool, channel, user, true).await
}

// Lock in the same space -> channel order as sends, grants and removal. Check
// membership separately after waiting so a revoked member cannot use old state.
async fn member_channel(
    tx: &mut Transaction<'_, Postgres>,
    space: &str,
    channel: &str,
    user: i64,
) -> Result<(i64, i64, String, bool, bool), ApiError> {
    let (space_id, owner): (i64,i64) = sqlx::query_as("SELECT id,owner_id FROM public.spaces WHERE external_id=$1 AND deleted_at IS NULL AND NOT demo FOR UPDATE")
        .bind(space).fetch_optional(&mut **tx).await.map_err(database_error)?.ok_or_else(not_found)?;
    let row: (i64,String,bool) = sqlx::query_as("SELECT c.id,c.name,c.private FROM public.channels c WHERE c.space_id=$1 AND c.external_id=$2 AND c.deleted_at IS NULL AND EXISTS(SELECT 1 FROM public.space_members WHERE space_id=$1 AND user_id=$3 AND deleted_at IS NULL) FOR UPDATE OF c")
        .bind(space_id).bind(channel).bind(user).fetch_optional(&mut **tx).await.map_err(database_error)?.ok_or_else(not_found)?;
    Ok((space_id, row.0, row.1, row.2, owner == user))
}

pub(super) async fn join_channel(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
) -> Result<Json<Channel>, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let (_, id, name, private, owner) =
        member_channel(&mut tx, &space, &channel, principal.user.id).await?;
    if private && !owner {
        let granted: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.channel_members WHERE channel_id=$1 AND user_id=$2 AND deleted_at IS NULL)")
            .bind(id).bind(principal.user.id).fetch_one(&mut *tx).await.map_err(database_error)?;
        if !granted {
            return Err(not_found());
        }
    }
    sqlx::query(
        "INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(principal.user.id)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok(Json(Channel {
        id: channel,
        space_id: space,
        name,
        private,
        joined: true,
    }))
}

pub(super) async fn leave_channel(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let (_, id, _, private, owner) =
        member_channel(&mut tx, &space, &channel, principal.user.id).await?;
    if private && !owner {
        let granted: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM public.channel_members WHERE channel_id=$1 AND user_id=$2 AND deleted_at IS NULL)")
            .bind(id).bind(principal.user.id).fetch_one(&mut *tx).await.map_err(database_error)?;
        if !granted {
            return Err(not_found());
        }
    }
    sqlx::query("UPDATE public.channel_joins SET deleted_at=now() WHERE channel_id=$1 AND user_id=$2 AND deleted_at IS NULL")
        .bind(id)
        .bind(principal.user.id)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    if private && !owner {
        sqlx::query("UPDATE public.channel_members SET deleted_at=now() WHERE channel_id=$1 AND user_id=$2 AND deleted_at IS NULL")
            .bind(id)
            .bind(principal.user.id)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
        // Keep a tombstone even for legacy grants so leaving cannot be undone
        // by immediately sending another unsolicited invitation.
        sqlx::query("INSERT INTO public.channel_invitations(channel_id,user_id,status) VALUES($1,$2,'revoked') ON CONFLICT(channel_id,user_id) DO UPDATE SET status='revoked',updated_at=now()")
            .bind(id).bind(principal.user.id).execute(&mut *tx).await.map_err(database_error)?;
    }
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn add_channel_member(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
    Json(input): Json<MemberInput>,
) -> Result<(StatusCode, Json<Member>), ApiError> {
    let pool = pool(&state)?;
    owned_channel(pool, &space, &channel, principal.user.id).await?;
    // Share the owner's invitation-attempt budget across spaces and channels.
    invite_rate_limit(pool, principal.user.id).await?;
    let mut tx = pool.begin().await.map_err(database_error)?;
    let space_id = owner_space(&mut tx, &space, principal.user.id).await?;
    let id = channel_for_update(&mut tx, space_id, &channel).await?;
    let private: bool = sqlx::query_scalar("SELECT private FROM public.channels WHERE id=$1")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
    if !private {
        return Err(conflict("public channels are self-joined"));
    }
    let member = find_user_for_update(&mut tx, &input.username).await?;
    let belongs: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.space_members WHERE space_id=$1 AND user_id=$2 AND deleted_at IS NULL)",
    )
    .bind(space_id)
    .bind(member.0)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if !belongs {
        return Err(conflict("user must join the space first"));
    }
    let granted: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM public.channel_members WHERE channel_id=$1 AND user_id=$2 AND deleted_at IS NULL)",
    )
    .bind(id)
    .bind(member.0)
    .fetch_one(&mut *tx)
    .await
    .map_err(database_error)?;
    if granted || member.0 == principal.user.id {
        return Err(conflict("user already in channel"));
    }
    let previous: Option<(String,bool,bool)> = sqlx::query_as("SELECT status,updated_at > now()-interval '7 days',updated_at > now()-interval '24 hours' FROM public.channel_invitations WHERE channel_id=$1 AND user_id=$2")
        .bind(id).bind(member.0).fetch_optional(&mut *tx).await.map_err(database_error)?;
    if let Some((status, live, recent)) = previous {
        if status == "pending" && live {
            return Err(conflict("user already invited"));
        }
        if status != "pending" && recent {
            return Err(conflict("invitation cooldown; try again after 24 hours"));
        }
    }
    let (received,sent): (i64,i64) = sqlx::query_as("SELECT count(*) FILTER(WHERE i.user_id=$1),count(*) FILTER(WHERE i.channel_id=$2) FROM public.channel_invitations i JOIN public.channels c ON c.id=i.channel_id JOIN public.spaces s ON s.id=c.space_id WHERE i.status='pending' AND i.updated_at > now()-interval '7 days' AND c.deleted_at IS NULL AND s.deleted_at IS NULL AND (i.user_id=$1 OR i.channel_id=$2)")
        .bind(member.0).bind(id).fetch_one(&mut *tx).await.map_err(database_error)?;
    if received >= 50 || sent >= 100 {
        return Err(conflict("pending invitation limit reached"));
    }
    sqlx::query("INSERT INTO public.channel_invitations(channel_id,user_id,status) VALUES($1,$2,'pending') ON CONFLICT(channel_id,user_id) DO UPDATE SET status='pending',updated_at=now()")
        .bind(id).bind(member.0).execute(&mut *tx).await.map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok((
        StatusCode::CREATED,
        Json(Member {
            id: member.1,
            avatar_id: member.2,
            username: member.3,
            display_name: member.4,
            owner: false,
        }),
    ))
}

pub(super) async fn remove_channel_member(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel, user)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let space_id = owner_space(&mut tx, &space, principal.user.id).await?;
    let id = channel_for_update(&mut tx, space_id, &channel).await?;
    let target: i64 = sqlx::query_scalar(
        "SELECT id FROM public.users WHERE external_id=$1 AND deleted_at IS NULL FOR UPDATE",
    )
    .bind(user)
    .fetch_optional(&mut *tx)
    .await
    .map_err(database_error)?
    .ok_or_else(not_found)?;
    if target == principal.user.id {
        return Err(conflict("owner cannot be removed"));
    }
    let changed =
        sqlx::query("UPDATE public.channel_members SET deleted_at=now() WHERE channel_id=$1 AND user_id=$2 AND deleted_at IS NULL")
            .bind(id)
            .bind(target)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
    let pending = sqlx::query("UPDATE public.channel_invitations SET status='revoked',updated_at=now() WHERE channel_id=$1 AND user_id=$2 AND status='pending'")
        .bind(id).bind(target).execute(&mut *tx).await.map_err(database_error)?;
    if changed.rows_affected() == 0 && pending.rows_affected() == 0 {
        return Err(not_found());
    }
    sqlx::query("UPDATE public.channel_joins SET deleted_at=now() WHERE channel_id=$1 AND user_id=$2 AND deleted_at IS NULL")
        .bind(id)
        .bind(target)
        .execute(&mut *tx)
        .await
        .map_err(database_error)?;
    if changed.rows_affected() > 0 {
        sqlx::query("INSERT INTO public.channel_invitations(channel_id,user_id,status) VALUES($1,$2,'revoked') ON CONFLICT(channel_id,user_id) DO UPDATE SET status='revoked',updated_at=now()")
            .bind(id).bind(target).execute(&mut *tx).await.map_err(database_error)?;
    }
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn accept_channel_invitation(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
) -> Result<Json<Channel>, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let (_, id, name, private, _) =
        member_channel(&mut tx, &space, &channel, principal.user.id).await?;
    if !private {
        return Err(not_found());
    }
    let changed = sqlx::query("UPDATE public.channel_invitations SET status='accepted',updated_at=now() WHERE channel_id=$1 AND user_id=$2 AND status='pending' AND updated_at > now()-interval '7 days'")
        .bind(id).bind(principal.user.id).execute(&mut *tx).await.map_err(database_error)?;
    if changed.rows_affected() != 1 {
        return Err(not_found());
    }
    sqlx::query("INSERT INTO public.channel_members(channel_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING")
        .bind(id).bind(principal.user.id).execute(&mut *tx).await.map_err(database_error)?;
    sqlx::query(
        "INSERT INTO public.channel_joins(channel_id,user_id) VALUES($1,$2) ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(principal.user.id)
    .execute(&mut *tx)
    .await
    .map_err(database_error)?;
    tx.commit().await.map_err(database_error)?;
    Ok(Json(Channel {
        id: channel,
        space_id: space,
        name,
        private,
        joined: true,
    }))
}

pub(super) async fn decline_channel_invitation(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((space, channel)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let mut tx = pool(&state)?.begin().await.map_err(database_error)?;
    let (_, id, _, _, _) = member_channel(&mut tx, &space, &channel, principal.user.id).await?;
    let changed = sqlx::query("UPDATE public.channel_invitations SET status='declined',updated_at=now() WHERE channel_id=$1 AND user_id=$2 AND status='pending' AND updated_at > now()-interval '7 days'")
        .bind(id).bind(principal.user.id).execute(&mut *tx).await.map_err(database_error)?;
    if changed.rows_affected() != 1 {
        return Err(not_found());
    }
    tx.commit().await.map_err(database_error)?;
    Ok(StatusCode::NO_CONTENT)
}
