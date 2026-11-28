//! Every app-icon count has a corresponding accessible inbox entry.
//!
//! Badge sync: a read on one device asks each of the account's phones for a
//! fresh count (`notification_badge_syncs`, one row per device). The worker
//! waits a moment so a burst of reads sends once, and skips phones that were
//! last given the same count.
use super::{Outcome, Platform, Workers, delivery::backoff};
use chrono::{DateTime, TimeDelta, Utc};
use futures_util::{StreamExt, stream};
use sqlx::PgPool;

pub(crate) use super::inbox::count;

const BATCH: i64 = 32;
const CONCURRENCY: usize = 8;
/// A refresh stops retrying an hour after the read that asked for it.
const LIFETIME: TimeDelta = TimeDelta::hours(1);

/// After a read that may change `user`'s count: every live phone of the
/// account refreshes about two seconds later. Best effort; a failure only
/// leaves phones on their previous count until the next alert or open.
pub(crate) async fn request(pool: &PgPool, user: i64) {
    let requested = sqlx::query(
        "INSERT INTO public.notification_badge_syncs(device_id,pending,requested_at,available_at,attempts)
         SELECT d.id,true,now(),now()+interval '2 seconds',0
         FROM public.notification_devices d
         JOIN public.account_sessions a ON a.token_hash=d.account_session_hash
         WHERE d.user_id=$1 AND d.revoked_at IS NULL AND a.revoked_at IS NULL AND a.expires_at>now()
         ON CONFLICT (device_id) DO UPDATE SET requested_at=now(),
           available_at=CASE WHEN notification_badge_syncs.pending THEN notification_badge_syncs.available_at
                             ELSE EXCLUDED.available_at END,
           attempts=CASE WHEN notification_badge_syncs.pending THEN notification_badge_syncs.attempts ELSE 0 END,
           pending=true",
    )
    .bind(user)
    .execute(pool)
    .await;
    if requested.is_err() {
        tracing::warn!(
            event_name = "badge_sync_request_failed",
            "badge sync request failed"
        );
    }
}

/// The phone now shows `count` (an alert or retraction carried it, or the
/// app read it itself); `None` when it is unknown, so the next refresh sends.
pub(crate) async fn given(pool: &PgPool, device: i64, count: Option<i64>) {
    let _ = sqlx::query(
        "INSERT INTO public.notification_badge_syncs(device_id,badge_count) VALUES($1,$2)
         ON CONFLICT (device_id) DO UPDATE SET badge_count=EXCLUDED.badge_count",
    )
    .bind(device)
    .bind(count)
    .execute(pool)
    .await;
}

/// `GET /api/spaces/unread` from a phone: its session's registration now
/// shows `count`.
pub(crate) async fn seen(pool: &PgPool, session: &[u8], count: i64) {
    let _ = sqlx::query(
        "INSERT INTO public.notification_badge_syncs(device_id,badge_count)
         SELECT id,$2 FROM public.notification_devices WHERE account_session_hash=$1 AND revoked_at IS NULL
         ON CONFLICT (device_id) DO UPDATE SET badge_count=EXCLUDED.badge_count",
    )
    .bind(session)
    .bind(count)
    .execute(pool)
    .await;
}

#[derive(sqlx::FromRow)]
struct Claimed {
    device_id: i64,
    attempts: i16,
    requested_at: DateTime<Utc>,
    locked_at: DateTime<Utc>,
    badge_count: Option<i64>,
    transport: String,
    address: String,
    user_id: i64,
    live: bool,
}

pub(super) async fn sync_pending(workers: &Workers) -> Result<bool, ()> {
    let claimed: Vec<Claimed> = sqlx::query_as(
        "WITH due AS (
             SELECT device_id FROM public.notification_badge_syncs
             WHERE pending AND available_at <= now()
               AND (locked_at IS NULL OR locked_at < now() - interval '2 minutes')
             ORDER BY available_at LIMIT $1 FOR UPDATE SKIP LOCKED
         ), claimed AS (
             UPDATE public.notification_badge_syncs b SET locked_at=clock_timestamp(), attempts=LEAST(b.attempts+1, 32767)
             FROM due WHERE b.device_id=due.device_id
             RETURNING b.device_id, b.attempts, b.requested_at, b.locked_at, b.badge_count
         )
         SELECT c.device_id, c.attempts, c.requested_at, c.locked_at, c.badge_count,
                d.transport, d.address, d.user_id,
                (d.revoked_at IS NULL AND a.revoked_at IS NULL AND a.expires_at>now()) AS live
         FROM claimed c
         JOIN public.notification_devices d ON d.id=c.device_id
         JOIN public.account_sessions a ON a.token_hash=d.account_session_hash",
    )
    .bind(BATCH)
    .fetch_all(&workers.pool)
    .await
    .map_err(|_| ())?;
    if claimed.is_empty() {
        return Ok(false);
    }
    stream::iter(claimed)
        .for_each_concurrent(CONCURRENCY, |claimed| async move {
            sync(workers, &claimed).await;
        })
        .await;
    Ok(true)
}

async fn sync(workers: &Workers, claimed: &Claimed) {
    let pool = &workers.pool;
    let platform = Platform::parse(&claimed.transport);
    let (outcome, count) = match (claimed.live, platform) {
        (true, Some(platform)) => match count(pool, claimed.user_id).await {
            // Already showing it: nothing to send.
            Ok(count) if Some(count) == claimed.badge_count => (Outcome::Delivered, Some(count)),
            Ok(count) => (
                workers
                    .push
                    .send_badge(platform, &claimed.address, count)
                    .await,
                Some(count),
            ),
            Err(_) => (
                Outcome::Retry {
                    error: "badge count unavailable".into(),
                    after: None,
                },
                None,
            ),
        },
        _ => (Outcome::Abandon("device unavailable".into()), None),
    };
    // A read that arrived after this claim asks again.
    let finished = |error: Option<String>| {
        sqlx::query(
            "UPDATE public.notification_badge_syncs SET pending=requested_at>$2, locked_at=NULL, attempts=0,
               available_at=now()+interval '2 seconds', badge_count=COALESCE($3,badge_count), last_error=$4
             WHERE device_id=$1",
        )
        .bind(claimed.device_id)
        .bind(claimed.locked_at)
        .bind(count)
        .bind(error)
        .execute(pool)
    };
    let error = match outcome {
        Outcome::Delivered => {
            let _ = finished(None).await;
            return;
        }
        Outcome::Retry { error, after } => {
            let wait = backoff(claimed.attempts).max(after.unwrap_or_default());
            let next = Utc::now() + TimeDelta::from_std(wait).unwrap_or(LIFETIME);
            if next < claimed.requested_at + LIFETIME {
                let _ = sqlx::query(
                    "UPDATE public.notification_badge_syncs
                     SET locked_at=NULL, available_at=now()+make_interval(secs => $2), last_error=$3
                     WHERE device_id=$1",
                )
                .bind(claimed.device_id)
                .bind(wait.as_secs_f64())
                .bind(&error)
                .execute(pool)
                .await;
                return;
            }
            error
        }
        Outcome::Revoke(error) => {
            let _ = sqlx::query(
                "UPDATE public.notification_devices SET revoked_at=now(), revoked_reason=$2, updated_at=now()
                 WHERE id=$1 AND revoked_at IS NULL",
            )
            .bind(claimed.device_id)
            .bind(&error)
            .execute(pool)
            .await;
            error
        }
        Outcome::Abandon(error) => error,
    };
    // Not sent: the phone keeps whatever it showed, so the next read sends.
    let _ = sqlx::query(
        "UPDATE public.notification_badge_syncs SET pending=requested_at>$2, locked_at=NULL, attempts=0,
           available_at=now()+interval '2 seconds', badge_count=NULL, last_error=$3
         WHERE device_id=$1",
    )
    .bind(claimed.device_id)
    .bind(claimed.locked_at)
    .bind(&error)
    .execute(pool)
    .await;
}
