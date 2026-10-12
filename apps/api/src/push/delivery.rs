//! Sends due deliveries. Claiming and recording are single short statements;
//! the provider call holds no database connection (the pool has five).
use super::{Alert, Conversation, Level, Outcome, Platform, Workers, expansion};
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
    notification_id: i64,
    attempts: i16,
    held: bool,
    device_id: i64,
    transport: String,
    address: String,
    device_tag: String,
    session_valid: bool,
    conversation_live: bool,
    recipient_eligible: bool,
    account_level: Option<String>,
    paused: bool,
    channel_level: Option<String>,
    channel_muted: bool,
    space_level: Option<String>,
    space_muted: bool,
    message_at: DateTime<Utc>,
    read: bool,
    kind: String,
    message_id: String,
    message: serde_json::Value,
    channel_id: String,
    channel_name: String,
    space_id: Option<String>,
    space_name: Option<String>,
    sender_id: String,
    sender: String,
    sender_avatar_id: Option<i16>,
    recipient_count: i64,
    recipient: String,
    mobile: String,
}

impl Claimed {
    fn notifies(&self) -> bool {
        use expansion::{Kind, Preferences};
        let kind = match self.kind.as_str() {
            "direct.message" => Kind::DirectMessage,
            "channel.message" => Kind::ChannelMessage,
            "mention.user" => Kind::MentionUser,
            "mention.everyone" => Kind::MentionEveryone,
            "space.invitation" | "channel.invitation" => Kind::Invitation,
            "direct.request" => Kind::DirectRequest,
            _ => return false,
        };
        let level = |value: &Option<String>| value.as_deref().and_then(Level::parse);
        expansion::notifies(
            kind,
            &Preferences {
                account: level(&self.account_level),
                paused: self.paused,
                channel: level(&self.channel_level),
                channel_muted: self.channel_muted,
                space: level(&self.space_level),
                space_muted: self.space_muted,
            },
        )
    }

    fn alert(&self, cdn: Option<&crate::assets::CdnSigner>) -> Alert {
        if self.kind == "direct.request" {
            return Alert::request(
                self.notification_id,
                &self.channel_id,
                &self.sender,
                &self.sender_id,
                self.sender_avatar_id,
            );
        }
        if self.kind == "space.invitation" || self.kind == "channel.invitation" {
            return Alert::invitation(
                self.notification_id,
                self.space_id.as_deref().unwrap_or_default(),
                self.space_name.as_deref().unwrap_or_default(),
                (self.kind == "channel.invitation")
                    .then_some((self.channel_id.as_str(), self.channel_name.as_str())),
                &self.sender,
                &self.sender_id,
                self.sender_avatar_id,
            );
        }
        let conversation = match (&self.space_id, &self.space_name) {
            (Some(space_id), Some(space_name)) => Conversation::Channel {
                space_id: space_id.clone(),
                channel_id: self.channel_id.clone(),
                title: format!("#{} ({space_name})", self.channel_name),
                recipient_count: self.recipient_count,
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
            self.message
                .pointer("/content/text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
        )
        .with_content(&self.message, cdn)
    }
}

/// What happened to one claimed delivery.
#[derive(Debug, PartialEq, Eq)]
enum Attempt {
    /// Not sent: the session ended, the message expired or was read, or the
    /// recipient is still active elsewhere.
    Dropped(&'static str),
    Hold,
    Sent(Outcome),
}

pub(super) async fn deliver_pending(workers: &Workers) -> Result<bool, ()> {
    // One statement claims a batch under a lease and reads what sending needs,
    // including current access, blocks, preferences and the DM read cursor.
    // Holds and provider retries must not preserve revoked notification consent.
    let mut claimed: Vec<Claimed> = sqlx::query_as(
        "WITH due AS (
             SELECT id FROM public.notification_deliveries
             WHERE delivered_at IS NULL AND abandoned_at IS NULL AND available_at <= now()
               AND (locked_at IS NULL OR locked_at < now() - interval '2 minutes')
             ORDER BY available_at, id LIMIT $1 FOR UPDATE SKIP LOCKED
         ), claimed AS (
             UPDATE public.notification_deliveries d SET locked_at=now(), attempts=LEAST(d.attempts+1, 32767)
             FROM due WHERE d.id=due.id
             RETURNING d.id, d.attempts, d.held, d.device_id, d.notification_id
         ), audience AS (
             -- One message fans out to a delivery per recipient device: count
             -- each channel's audience once per batch, not once per delivery.
             SELECT p.channel_id, p.actor_id,
                    (SELECT count(*) FROM public.space_members sm JOIN public.users u ON u.id=sm.user_id
                     WHERE sm.space_id=ch.space_id AND sm.deleted_at IS NULL AND u.deleted_at IS NULL AND sm.user_id<>p.actor_id
                     AND (NOT ch.private OR s.owner_id=sm.user_id OR EXISTS(SELECT 1 FROM public.channel_members cm
                          WHERE cm.channel_id=ch.id AND cm.user_id=sm.user_id AND cm.deleted_at IS NULL))) AS recipient_count
             FROM (SELECT DISTINCT n.channel_id, n.actor_id FROM claimed c
                   JOIN public.notifications n ON n.id=c.notification_id) p
             JOIN public.channels ch ON ch.id=p.channel_id
             LEFT JOIN public.spaces s ON s.id=ch.space_id
         )
         SELECT c.id, n.id AS notification_id, c.attempts, c.held, c.device_id, dev.transport, dev.address,
                encode(substring(dev.account_session_hash FROM 1 FOR 8), 'hex') AS device_tag,
                (dev.revoked_at IS NULL AND a.revoked_at IS NULL AND a.expires_at > now()
                 AND a.user_id=dev.user_id AND dev.user_id=n.user_id) AS session_valid,
                (s.deleted_at IS NULL AND (n.message_id IS NULL OR ch.deleted_at IS NULL)) AS conversation_live,
                CASE WHEN n.kind='direct.request' THEN COALESCE(req.eligible,false)
                WHEN n.message_id IS NULL THEN COALESCE(inv.eligible,false)
                ELSE (recipient.deleted_at IS NULL AND author.id IS NOT NULL
                 AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.deleted_at IS NULL
                     AND ((b.blocker_id=n.user_id AND b.blocked_id=n.actor_id)
                         OR (ch.space_id IS NULL AND b.blocker_id=n.actor_id AND b.blocked_id=n.user_id)))
                 AND CASE WHEN ch.space_id IS NULL THEN
                     EXISTS(SELECT 1 FROM public.direct_conversations d WHERE d.channel_id=ch.id
                         AND d.accepted_at IS NOT NULL AND d.low_user_id<>d.high_user_id
                         AND n.user_id IN(d.low_user_id,d.high_user_id) AND n.actor_id IN(d.low_user_id,d.high_user_id))
                 ELSE
                     NOT s.demo
                     AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=n.user_id AND sm.deleted_at IS NULL)
                     AND (NOT ch.private OR s.owner_id=n.user_id OR EXISTS(SELECT 1 FROM public.channel_members cm
                         WHERE cm.channel_id=ch.id AND cm.user_id=n.user_id AND cm.deleted_at IS NULL))
                     AND (n.kind='mention.user' OR EXISTS(SELECT 1 FROM public.channel_joins cj
                         WHERE cj.channel_id=ch.id AND cj.user_id=n.user_id AND cj.deleted_at IS NULL))
                 END) END AS recipient_eligible,
                ns.default_level AS account_level, COALESCE(ns.paused_until > now(), false) AS paused,
                co.level AS channel_level, COALESCE(co.muted_until > now(), false) AS channel_muted,
                so.level AS space_level, COALESCE(so.muted_until > now(), false) AS space_muted,
                COALESCE(m.created_at,n.created_at) AS message_at,
                COALESCE(ch.space_id IS NULL AND COALESCE(r.seq, 0) >= m.channel_seq,false) AS read,
                n.kind, COALESCE(m.external_id,'') AS message_id, COALESCE(m.payload,'{}'::jsonb) AS message,
                COALESCE(ch.external_id,'') AS channel_id, COALESCE(ch.name,'') AS channel_name,
                s.external_id AS space_id, s.name AS space_name,
                COALESCE(author.external_id, m.payload->'author'->>'id', '') AS sender_id,
                COALESCE(author.display_name, m.payload->'author'->>'name', '') AS sender,
                author.avatar_id AS sender_avatar_id,
                COALESCE(rc.recipient_count, 0) AS recipient_count,
                recipient.external_id AS recipient,
                COALESCE(ns.mobile, 'whenInactive') AS mobile
         FROM claimed c
         JOIN public.notifications n ON n.id=c.notification_id
         JOIN public.notification_devices dev ON dev.id=c.device_id
         JOIN public.account_sessions a ON a.token_hash=dev.account_session_hash
         LEFT JOIN public.messages m ON m.id=n.message_id
         LEFT JOIN public.channels ch ON ch.id=n.channel_id
         LEFT JOIN public.spaces s ON s.id=n.space_id
         LEFT JOIN public.notification_invitation_sources inv ON inv.id=n.id
         LEFT JOIN public.notification_request_sources req ON req.id=n.id
         JOIN public.users recipient ON recipient.id=n.user_id
         LEFT JOIN public.users author ON author.id=n.actor_id AND author.deleted_at IS NULL
         LEFT JOIN public.notification_settings ns ON ns.user_id=n.user_id
         LEFT JOIN public.notification_overrides co ON co.user_id=n.user_id AND co.channel_id=ch.id
         LEFT JOIN public.notification_overrides so ON so.user_id=n.user_id AND so.space_id=ch.space_id
         LEFT JOIN public.direct_reads r ON r.channel_id=n.channel_id AND r.user_id=n.user_id
         LEFT JOIN audience rc ON rc.channel_id=n.channel_id AND rc.actor_id IS NOT DISTINCT FROM n.actor_id
         ORDER BY c.id",
    )
    .bind(BATCH)
    .fetch_all(&workers.pool)
    .await
    .map_err(|_| ())?;
    if claimed.is_empty() {
        return Ok(false);
    }
    let mut messages: Vec<_> = claimed.iter().map(|row| row.message.clone()).collect();
    crate::assets::notification_content(&workers.pool, &mut messages)
        .await
        .map_err(|_| ())?;
    for (row, message) in claimed.iter_mut().zip(messages) {
        row.message = message;
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
    if !claimed.recipient_eligible {
        return Attempt::Dropped("recipient ineligible");
    }
    if !claimed.notifies() {
        return Attempt::Dropped("notifications disabled");
    }
    if Utc::now() >= claimed.message_at + LIFETIME {
        return Attempt::Dropped("expired");
    }
    if claimed.read {
        return Attempt::Dropped("read");
    }
    let consent = matches!(
        claimed.kind.as_str(),
        "space.invitation" | "channel.invitation" | "direct.request"
    );
    if (claimed.held || consent) && claimed.mobile != "always" {
        // Still active on another session: that client already showed it.
        // Without presence the push goes out rather than being lost.
        let users = [claimed.recipient.clone()];
        if let Ok(active) = expansion::active_sessions(workers, &users).await
            && active
                .get(&claimed.recipient)
                .is_some_and(|tags| tags.iter().any(|tag| *tag != claimed.device_tag))
        {
            return if claimed.held {
                Attempt::Dropped("active elsewhere")
            } else {
                Attempt::Hold
            };
        }
    }
    let Some(platform) = Platform::parse(&claimed.transport) else {
        return Attempt::Dropped("platform unavailable");
    };
    Attempt::Sent(
        workers
            .push
            .send(
                platform,
                &claimed.address,
                &claimed.alert(workers.cdn.as_deref()),
            )
            .await,
    )
}

async fn record(workers: &Workers, claimed: &Claimed, result: Attempt) {
    let pool = &workers.pool;
    let (done, error) = match result {
        Attempt::Hold => {
            let _ = sqlx::query(
                "UPDATE public.notification_deliveries SET held=true,locked_at=NULL,
                 available_at=now()+interval '60 seconds' WHERE id=$1",
            )
            .bind(claimed.id)
            .execute(pool)
            .await;
            return;
        }
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

/// Rows per prune statement: short locks and modest WAL per batch.
const PRUNE_BATCH: i64 = 5_000;
/// Batches per table per pass (every 10 minutes): about 14 million rows a day,
/// far above one delivery per recipient device per message today.
const PRUNE_BATCHES: usize = 20;

/// Finished deliveries and expanded jobs are operational state: keep a week.
/// Each table is drained in batches until nothing week-old remains or the pass
/// reaches its cap; the next pass continues. Returns the rows deleted.
pub(super) async fn prune(pool: &sqlx::PgPool) -> u64 {
    // Oldest first through the partial indexes notification_deliveries_finished
    // and notification_jobs_expanded; `= ANY(ARRAY(...))` keeps the delete on
    // primary-key lookups instead of a hash join over the whole table.
    let mut total = 0;
    for (table, statement) in [
        (
            "notification_deliveries",
            "DELETE FROM public.notification_deliveries WHERE id = ANY(ARRAY(
                 SELECT id FROM public.notification_deliveries
                 WHERE (delivered_at IS NOT NULL OR abandoned_at IS NOT NULL)
                   AND LEAST(delivered_at, abandoned_at) < now() - interval '7 days'
                 ORDER BY LEAST(delivered_at, abandoned_at) LIMIT $1))",
        ),
        (
            "notification_jobs",
            "DELETE FROM public.notification_jobs WHERE id = ANY(ARRAY(
                 SELECT id FROM public.notification_jobs
                 WHERE expanded_at IS NOT NULL AND expanded_at < now() - interval '7 days'
                 ORDER BY expanded_at LIMIT $1))",
        ),
    ] {
        let mut deleted = 0;
        let mut drained = false;
        for _ in 0..PRUNE_BATCHES {
            let Ok(result) = sqlx::query(statement).bind(PRUNE_BATCH).execute(pool).await else {
                break;
            };
            deleted += result.rows_affected();
            if result.rows_affected() < PRUNE_BATCH.unsigned_abs() {
                drained = true;
                break;
            }
            // Let sends and deliveries interleave with a long backlog.
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if deleted > 0 {
            tracing::info!(
                event_name = "push_pruned",
                table,
                deleted,
                drained,
                "operational rows pruned"
            );
        }
        total += deleted;
    }
    total
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
