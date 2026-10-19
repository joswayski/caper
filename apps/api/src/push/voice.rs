//! Successful joins serialize per space, not per room. Provisional joins and
//! monitors never count. Occupancy stays in the media store; Postgres owns the
//! cooldown and durable fanout. No notification side effects run in CAS closures.
use crate::{ApiError, AppState, LEASE};
use uuid::Uuid;

pub(crate) async fn joined(state: &AppState, participant: Uuid) -> Result<(), ApiError> {
    let (Some(pool), Some(channel)) = (&state.database, &state.media_channel) else {
        return Ok(());
    };
    let source: Option<(i64, i64)> = sqlx::query_as(
        "SELECT c.id,s.id FROM public.channels c JOIN public.spaces s ON s.id=c.space_id
         WHERE c.external_id=$1 AND c.deleted_at IS NULL AND s.deleted_at IS NULL AND NOT s.demo",
    )
    .bind(channel)
    .fetch_optional(pool)
    .await
    .map_err(super::database_error)?;
    let Some((channel_id, space)) = source else {
        return Ok(());
    };
    let actor = state
        .read(|r| {
            Ok(r.participants
                .get(&participant)
                .filter(|p| p.monitor.is_none() && p.lease.elapsed() < LEASE)
                .and_then(|p| p.user_id))
        })
        .await?;
    let Some(actor) = actor else {
        return Ok(());
    };
    let mut tx = pool.begin().await.map_err(super::database_error)?;
    sqlx::query(
        "INSERT INTO public.voice_notification_state(space_id) VALUES ($1) ON CONFLICT DO NOTHING",
    )
    .bind(space)
    .execute(&mut *tx)
    .await
    .map_err(super::database_error)?;
    let cooled: bool = sqlx::query_scalar(
        "SELECT notified_at IS NULL OR notified_at<=clock_timestamp()-interval '5 minutes'
         FROM public.voice_notification_state WHERE space_id=$1 FOR UPDATE",
    )
    .bind(space)
    .fetch_one(&mut *tx)
    .await
    .map_err(super::database_error)?;
    let channels: Vec<String> = sqlx::query_scalar(
        "SELECT external_id FROM public.channels WHERE space_id=$1 AND deleted_at IS NULL",
    )
    .bind(space)
    .fetch_all(&mut *tx)
    .await
    .map_err(super::database_error)?;
    let mut occupied = false;
    for channel in channels {
        let mut room = state.clone();
        room.media_channel = Some(channel);
        if room
            .read(|r| {
                Ok(r.participants.values().any(|p| {
                    p.id != participant
                        && p.monitor.is_none()
                        && !p.voice_join_pending
                        && p.lease.elapsed() < LEASE
                }))
            })
            .await?
        {
            occupied = true;
            break;
        }
    }
    // A successful join becomes visible to the next space-serialized join only
    // here, after provider setup, even when no notification can be sent.
    let activated = state
        .update(|r| {
            let Some(p) = r.participants.get_mut(&participant) else {
                return Ok(false);
            };
            let pending = p.voice_join_pending && p.lease.elapsed() < LEASE;
            p.voice_join_pending = false;
            Ok(pending)
        })
        .await?;
    if activated && !occupied && cooled {
        let recipients: Vec<i64> = sqlx::query_scalar(
            "SELECT u.id FROM public.users u JOIN public.space_members sm ON sm.user_id=u.id
             WHERE sm.space_id=$1 AND sm.deleted_at IS NULL
               AND public.voice_notification_allowed(u.id,$2,$3)
             ORDER BY u.id FOR NO KEY UPDATE OF u",
        )
        .bind(space)
        .bind(actor)
        .bind(channel_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(super::database_error)?;
        for recipient in recipients {
            super::consent(
                &mut tx,
                recipient,
                actor,
                "voice.started",
                Some(space),
                Some(channel_id),
            )
            .await
            .map_err(super::database_error)?;
        }
        sqlx::query("UPDATE public.voice_notification_state SET notified_at=clock_timestamp() WHERE space_id=$1")
            .bind(space).execute(&mut *tx).await.map_err(super::database_error)?;
    }
    tx.commit().await.map_err(super::database_error)
}
