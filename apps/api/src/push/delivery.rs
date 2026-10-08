//! Sends due deliveries. Claiming and recording are single short statements;
//! the provider call holds no database connection (the pool has five).
use super::{Alert, Conversation, Outcome, Platform, Workers, expansion};
use chrono::{DateTime, TimeDelta, Utc};
use futures_util::{StreamExt, stream};
use std::time::Duration;

const BATCH: i64 = 32;
const CONCURRENCY: usize = 8;
/// APNs expiration and FCM TTL: retries stop a day after the message.
const LIFETIME: TimeDelta = TimeDelta::hours(24);
const FIRST_RETRY: Duration = Duration::from_secs(15);
const LONGEST_RETRY: Duration = Duration::from_secs(30 * 60);

/// 15 s, 30 s, 1 min, … up to 30 minutes.
pub(super) fn backoff(attempts: i16) -> Duration {
    let doublings = u32::try_from(attempts.max(1) - 1).unwrap_or(0).min(16);
    FIRST_RETRY
        .saturating_mul(2u32.saturating_pow(doublings))
        .min(LONGEST_RETRY)
}

#[derive(sqlx::FromRow)]
struct Claimed {
    id: i64,
    attempts: i16,
    held: bool,
    device_id: i64,
    transport: String,
    address: String,
    device_tag: String,
    session_valid: bool,
    conversation_live: bool,
    message_at: DateTime<Utc>,
    read: bool,
    kind: String,
    message_id: String,
    text: Option<String>,
    channel_id: String,
    channel_name: String,
    space_id: Option<String>,
    space_name: Option<String>,
    sender_id: String,
    sender: String,
    recipient: String,
    mobile: String,
}

impl Claimed {
    fn alert(&self) -> Alert {
        let conversation = match (&self.space_id, &self.space_name) {
            (Some(space_id), Some(space_name)) => Conversation::Channel {
                space_id: space_id.clone(),
                channel_id: self.channel_id.clone(),
                title: format!("#{} ({space_name})", self.channel_name),
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
            self.text.as_deref().unwrap_or_default(),
        )
    }
}

/// What happened to one claimed delivery.
#[derive(Debug, PartialEq, Eq)]
enum Attempt {
    /// Not sent: the session ended, the message expired or was read, or the
    /// recipient is still active elsewhere.
    Dropped(&'static str),
    Sent(Outcome),
}

pub(super) async fn deliver_pending(workers: &Workers) -> Result<bool, ()> {
    // One statement claims a batch under a lease and reads what sending needs,
    // including the send-time checks: session, conversation, DM read cursor.
    let claimed: Vec<Claimed> = sqlx::query_as(
        "WITH due AS (
             SELECT id FROM public.notification_deliveries
             WHERE delivered_at IS NULL AND abandoned_at IS NULL AND available_at <= now()
               AND (locked_at IS NULL OR locked_at < now() - interval '2 minutes')
             ORDER BY available_at, id LIMIT $1 FOR UPDATE SKIP LOCKED
         ), claimed AS (
             UPDATE public.notification_deliveries d SET locked_at=now(), attempts=LEAST(d.attempts+1, 32767)
             FROM due WHERE d.id=due.id
             RETURNING d.id, d.attempts, d.held, d.device_id, d.notification_id
         )
         SELECT c.id, c.attempts, c.held, c.device_id, dev.transport, dev.address,
                encode(substring(dev.account_session_hash FROM 1 FOR 8), 'hex') AS device_tag,
                (dev.revoked_at IS NULL AND a.revoked_at IS NULL AND a.expires_at > now()
                 AND a.user_id=dev.user_id) AS session_valid,
                (ch.deleted_at IS NULL AND (ch.space_id IS NULL OR s.deleted_at IS NULL)) AS conversation_live,
                m.created_at AS message_at,
                (ch.space_id IS NULL AND COALESCE(r.seq, 0) >= m.channel_seq) AS read,
                n.kind, m.external_id AS message_id, m.payload->'content'->>'text' AS text,
                ch.external_id AS channel_id, ch.name AS channel_name,
                s.external_id AS space_id, s.name AS space_name,
                COALESCE(author.external_id, m.payload->'author'->>'id', '') AS sender_id,
                COALESCE(author.display_name, m.payload->'author'->>'name', '') AS sender,
                recipient.external_id AS recipient,
                COALESCE(ns.mobile, 'whenInactive') AS mobile
         FROM claimed c
         JOIN public.notifications n ON n.id=c.notification_id
         JOIN public.notification_devices dev ON dev.id=c.device_id
         JOIN public.account_sessions a ON a.token_hash=dev.account_session_hash
         JOIN public.messages m ON m.id=n.message_id
         JOIN public.channels ch ON ch.id=n.channel_id
         LEFT JOIN public.spaces s ON s.id=ch.space_id
         JOIN public.users recipient ON recipient.id=n.user_id
         LEFT JOIN public.users author ON author.id=n.actor_id AND author.deleted_at IS NULL
         LEFT JOIN public.notification_settings ns ON ns.user_id=n.user_id
         LEFT JOIN public.direct_reads r ON r.channel_id=n.channel_id AND r.user_id=n.user_id
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
            let result = attempt(workers, &claimed).await;
            record(workers, &claimed, result).await;
        })
        .await;
    Ok(true)
}

async fn attempt(workers: &Workers, claimed: &Claimed) -> Attempt {
    if !claimed.session_valid {
        return Attempt::Dropped("session ended");
    }
    if !claimed.conversation_live {
        return Attempt::Dropped("conversation deleted");
    }
    if Utc::now() >= claimed.message_at + LIFETIME {
        return Attempt::Dropped("expired");
    }
    if claimed.read {
        return Attempt::Dropped("read");
    }
    if claimed.held && claimed.mobile != "always" {
        // Still active on another session: that client already showed it.
        // Without presence the push goes out rather than being lost.
        let users = [claimed.recipient.clone()];
        if let Ok(active) = expansion::active_sessions(workers, &users).await
            && active
                .get(&claimed.recipient)
                .is_some_and(|tags| tags.iter().any(|tag| *tag != claimed.device_tag))
        {
            return Attempt::Dropped("active elsewhere");
        }
    }
    let Some(platform) = Platform::parse(&claimed.transport) else {
        return Attempt::Dropped("platform unavailable");
    };
    Attempt::Sent(
        workers
            .push
            .send(platform, &claimed.address, &claimed.alert())
            .await,
    )
}

async fn record(workers: &Workers, claimed: &Claimed, result: Attempt) {
    let pool = &workers.pool;
    let (done, error) = match result {
        Attempt::Sent(Outcome::Delivered) => {
            let _ = sqlx::query(
                "UPDATE public.notification_deliveries SET delivered_at=now(), locked_at=NULL, last_error=NULL WHERE id=$1",
            )
            .bind(claimed.id)
            .execute(pool)
            .await;
            tracing::info!(
                event_name = "push_delivered",
                transport = claimed.transport,
                "push delivered"
            );
            return;
        }
        Attempt::Sent(Outcome::Retry { error, after }) => {
            let wait = backoff(claimed.attempts).max(after.unwrap_or_default());
            let next = Utc::now() + TimeDelta::from_std(wait).unwrap_or(LIFETIME);
            if next < claimed.message_at + LIFETIME {
                let _ = sqlx::query(
                    "UPDATE public.notification_deliveries
                     SET locked_at=NULL, available_at=now()+make_interval(secs => $2), last_error=$3
                     WHERE id=$1",
                )
                .bind(claimed.id)
                .bind(wait.as_secs_f64())
                .bind(&error)
                .execute(pool)
                .await;
                tracing::warn!(
                    event_name = "push_delivery_retry",
                    transport = claimed.transport,
                    error,
                    attempts = claimed.attempts,
                    "push delivery will retry"
                );
                return;
            }
            (true, error)
        }
        Attempt::Sent(Outcome::Revoke(error)) => {
            // The device is dead for every pending delivery; those see a
            // revoked registration when claimed and are dropped.
            let _ = sqlx::query(
                "UPDATE public.notification_devices SET revoked_at=now(), revoked_reason=$2, updated_at=now()
                 WHERE id=$1 AND revoked_at IS NULL",
            )
            .bind(claimed.device_id)
            .bind(&error)
            .execute(pool)
            .await;
            (true, error)
        }
        Attempt::Sent(Outcome::Abandon(error)) => (true, error),
        Attempt::Dropped(reason) => (false, reason.to_owned()),
    };
    let _ = sqlx::query(
        "UPDATE public.notification_deliveries SET abandoned_at=now(), locked_at=NULL, last_error=$2 WHERE id=$1",
    )
    .bind(claimed.id)
    .bind(&error)
    .execute(pool)
    .await;
    if done {
        tracing::warn!(
            event_name = "push_delivery_abandoned",
            transport = claimed.transport,
            error,
            "push delivery abandoned"
        );
    }
}

/// Finished deliveries and expanded jobs are operational state: keep a week.
pub(super) async fn prune(pool: &sqlx::PgPool) {
    for statement in [
        "DELETE FROM public.notification_deliveries WHERE id IN (
             SELECT id FROM public.notification_deliveries
             WHERE delivered_at < now() - interval '7 days' OR abandoned_at < now() - interval '7 days'
             LIMIT 1000)",
        "DELETE FROM public.notification_jobs WHERE id IN (
             SELECT id FROM public.notification_jobs WHERE expanded_at < now() - interval '7 days'
             LIMIT 1000)",
    ] {
        let _ = sqlx::query(statement).execute(pool).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_fifteen_seconds_to_half_an_hour() {
        assert_eq!(backoff(1), Duration::from_secs(15));
        assert_eq!(backoff(2), Duration::from_secs(30));
        assert_eq!(backoff(4), Duration::from_secs(120));
        assert_eq!(backoff(8), Duration::from_secs(30 * 60));
        assert_eq!(backoff(i16::MAX), Duration::from_secs(30 * 60));
        assert_eq!(backoff(0), Duration::from_secs(15));
    }
}
