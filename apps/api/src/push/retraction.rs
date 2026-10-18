//! Takes back phone notifications of deleted messages. Deleting a message
//! writes one row per device its notification reached; this sends each one
//! like a delivery, with the same lease, backoff and provider outcomes.
use super::{Alert, Conversation, Outcome, Platform, Workers, delivery::backoff};
use chrono::{DateTime, TimeDelta, Utc};
use futures_util::{StreamExt, stream};

const BATCH: i64 = 32;
const CONCURRENCY: usize = 8;
/// The original expires from devices and providers a day after it was sent.
const LIFETIME: TimeDelta = TimeDelta::hours(24);

#[derive(sqlx::FromRow)]
struct Claimed {
    id: i64,
    attempts: i16,
    created_at: DateTime<Utc>,
    device_id: i64,
    transport: String,
    address: String,
    device_live: bool,
    kind: String,
    message_id: String,
    channel_id: String,
    channel_name: String,
    space_id: Option<String>,
    space_name: Option<String>,
    sender_id: String,
    sender: String,
    sender_avatar_id: Option<i16>,
}

impl Claimed {
    /// The original's title and conversation, so iPhone replaces it in place.
    fn alert(&self) -> Alert {
        let conversation = match (&self.space_id, &self.space_name) {
            (Some(space_id), Some(space_name)) => Conversation::Channel {
                space_id: space_id.clone(),
                channel_id: self.channel_id.clone(),
                title: format!("#{} ({space_name})", self.channel_name),
                recipient_count: 0,
            },
            _ => Conversation::Direct {
                id: self.channel_id.clone(),
            },
        };
        Alert::new(
            &self.kind,
            &self.message_id,
            conversation,
            &self.sender,
            &self.sender_id,
            self.sender_avatar_id,
            "",
        )
        .retraction()
    }
}

pub(super) async fn retract_pending(workers: &Workers) -> Result<bool, ()> {
    let claimed: Vec<Claimed> = sqlx::query_as(
        "WITH due AS (
             SELECT id FROM public.notification_retractions
             WHERE sent_at IS NULL AND abandoned_at IS NULL AND available_at <= now()
               AND (locked_at IS NULL OR locked_at < now() - interval '2 minutes')
             ORDER BY available_at, id LIMIT $1 FOR UPDATE SKIP LOCKED
         ), claimed AS (
             UPDATE public.notification_retractions r SET locked_at=now(), attempts=LEAST(r.attempts+1, 32767)
             FROM due WHERE r.id=due.id
             RETURNING r.id, r.attempts, r.created_at, r.notification_id, r.device_id
         )
         SELECT c.id, c.attempts, c.created_at, c.device_id, dev.transport, dev.address,
                (dev.revoked_at IS NULL) AS device_live,
                n.kind, m.external_id AS message_id,
                ch.external_id AS channel_id, ch.name AS channel_name,
                s.external_id AS space_id, s.name AS space_name,
                COALESCE(author.external_id, m.payload->'author'->>'id', '') AS sender_id,
                COALESCE(author.display_name, m.payload->'author'->>'name', '') AS sender,
                author.avatar_id AS sender_avatar_id
         FROM claimed c
         JOIN public.notifications n ON n.id=c.notification_id
         JOIN public.notification_devices dev ON dev.id=c.device_id
         JOIN public.messages m ON m.id=n.message_id
         JOIN public.channels ch ON ch.id=m.channel_id
         LEFT JOIN public.spaces s ON s.id=ch.space_id
         LEFT JOIN public.users author ON author.id=n.actor_id AND author.deleted_at IS NULL
         ORDER BY c.id",
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
            let outcome = attempt(workers, &claimed).await;
            record(workers, &claimed, outcome).await;
        })
        .await;
    Ok(true)
}

async fn attempt(workers: &Workers, claimed: &Claimed) -> Result<Outcome, &'static str> {
    if !claimed.device_live {
        return Err("device revoked");
    }
    if Utc::now() >= claimed.created_at + LIFETIME {
        return Err("expired");
    }
    let platform = Platform::parse(&claimed.transport).ok_or("platform unavailable")?;
    Ok(workers
        .push
        .send(platform, &claimed.address, &claimed.alert())
        .await)
}

async fn record(workers: &Workers, claimed: &Claimed, result: Result<Outcome, &'static str>) {
    let pool = &workers.pool;
    let error = match result {
        Ok(Outcome::Delivered) => {
            let _ = sqlx::query(
                "UPDATE public.notification_retractions SET sent_at=now(), locked_at=NULL, last_error=NULL WHERE id=$1",
            )
            .bind(claimed.id)
            .execute(pool)
            .await;
            tracing::info!(
                event_name = "push_retracted",
                transport = claimed.transport,
                "push retraction sent"
            );
            return;
        }
        Ok(Outcome::Retry { error, after }) => {
            let wait = backoff(claimed.attempts).max(after.unwrap_or_default());
            let next = Utc::now() + TimeDelta::from_std(wait).unwrap_or(LIFETIME);
            if next < claimed.created_at + LIFETIME {
                let _ = sqlx::query(
                    "UPDATE public.notification_retractions
                     SET locked_at=NULL, available_at=now()+make_interval(secs => $2), last_error=$3
                     WHERE id=$1",
                )
                .bind(claimed.id)
                .bind(wait.as_secs_f64())
                .bind(&error)
                .execute(pool)
                .await;
                return;
            }
            error
        }
        Ok(Outcome::Revoke(error)) => {
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
        Ok(Outcome::Abandon(error)) => error,
        Err(reason) => reason.to_owned(),
    };
    let _ = sqlx::query(
        "UPDATE public.notification_retractions SET abandoned_at=now(), locked_at=NULL, last_error=$2 WHERE id=$1",
    )
    .bind(claimed.id)
    .bind(&error)
    .execute(pool)
    .await;
}
