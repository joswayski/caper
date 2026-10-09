//! Outbox expansion: who is notified about one message, and on which devices.
//! The rules are "Who gets a notification" in docs/notifications.md.
use super::{Level, Platform, Workers};
use crate::presence::Presence;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

const BATCH: i64 = 16;
/// Presence decides `@here` and the phone hold. After this many failed
/// attempts a job goes out without it rather than never.
const PRESENCE_ATTEMPTS: i16 = 3;
const PRESENCE_CHUNK: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    DirectMessage,
    ChannelMessage,
    MentionUser,
    MentionEveryone,
}

impl Kind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::DirectMessage => "direct.message",
            Self::ChannelMessage => "channel.message",
            Self::MentionUser => "mention.user",
            Self::MentionEveryone => "mention.everyone",
        }
    }
}

/// A recipient's settings for one conversation. For a DM, `channel` is the DM
/// override and the space fields are unset.
#[derive(Clone, Debug, Default)]
pub(super) struct Preferences {
    pub(super) account: Option<Level>,
    pub(super) paused: bool,
    pub(super) channel: Option<Level>,
    pub(super) channel_muted: bool,
    pub(super) space: Option<Level>,
    pub(super) space_muted: bool,
}

/// Resolution steps 1–4: account off or paused, then mutes (which a direct
/// @mention ignores), then the first set level of channel, space and account.
pub(super) fn notifies(kind: Kind, preferences: &Preferences) -> bool {
    if preferences.account == Some(Level::Nothing) || preferences.paused {
        return false;
    }
    if (preferences.channel_muted || preferences.space_muted) && kind != Kind::MentionUser {
        return false;
    }
    if kind == Kind::DirectMessage {
        return preferences.channel != Some(Level::Nothing);
    }
    let level = preferences
        .channel
        .or(preferences.space)
        .or(preferences.account)
        .unwrap_or(Level::All);
    match kind {
        Kind::ChannelMessage => level == Level::All,
        _ => level != Level::Nothing,
    }
}

#[derive(sqlx::FromRow)]
struct Source {
    channel_id: i64,
    space_id: Option<i64>,
    author: Option<i64>,
    thread_reply: bool,
    mentions: Option<Value>,
    sendable: bool,
}

#[derive(sqlx::FromRow)]
struct Candidate {
    id: i64,
    external_id: String,
    /// Joined the channel (always true for DMs). Joining is consent to
    /// receive, so only joined readers hear about ordinary messages.
    joined: bool,
    account_level: Option<String>,
    paused: bool,
    mobile: String,
    channel_level: Option<String>,
    channel_muted: bool,
    space_level: Option<String>,
    space_muted: bool,
}

impl Candidate {
    fn preferences(&self) -> Preferences {
        let level = |value: &Option<String>| value.as_deref().and_then(Level::parse);
        Preferences {
            account: level(&self.account_level),
            paused: self.paused,
            channel: level(&self.channel_level),
            channel_muted: self.channel_muted,
            space: level(&self.space_level),
            space_muted: self.space_muted,
        }
    }
}

struct Recipient {
    id: i64,
    external_id: String,
    kind: Kind,
    hold: bool,
}

#[derive(Default)]
struct Mentions {
    users: HashSet<String>,
    everyone: bool,
    here: bool,
}

impl Mentions {
    fn from(value: Option<&Value>) -> Self {
        let mut mentions = Self::default();
        for mention in value.and_then(Value::as_array).into_iter().flatten() {
            match mention["type"].as_str() {
                Some("user") => {
                    if let Some(id) = mention["id"].as_str() {
                        mentions.users.insert(id.to_owned());
                    }
                }
                Some("everyone") => mentions.everyone = true,
                Some("here") => mentions.here = true,
                _ => {}
            }
        }
        mentions
    }
}

/// Claims due jobs under a lease and expands each. A failed job is released
/// with a backoff; the lease covers a worker that stopped mid-way.
pub(super) async fn expand_pending(workers: &Workers) -> Result<bool, ()> {
    let jobs: Vec<(i64, i64, i16)> = sqlx::query_as(
        "WITH due AS (
             SELECT id FROM public.notification_jobs
             WHERE expanded_at IS NULL AND available_at <= now()
               AND (locked_at IS NULL OR locked_at < now() - interval '2 minutes')
             ORDER BY available_at, id LIMIT $1 FOR UPDATE SKIP LOCKED)
         UPDATE public.notification_jobs j SET locked_at=now(), attempts=LEAST(j.attempts+1, 32767)
         FROM due WHERE j.id=due.id
         RETURNING j.id, j.message_id, j.attempts",
    )
    .bind(BATCH)
    .fetch_all(&workers.pool)
    .await
    .map_err(|_| ())?;
    if jobs.is_empty() {
        return Ok(false);
    }
    for (job, message, attempts) in jobs {
        if expand(workers, job, message, attempts).await.is_err() {
            let delay = super::delivery::backoff(attempts);
            let _ = sqlx::query(
                "UPDATE public.notification_jobs SET locked_at=NULL, available_at=now()+make_interval(secs => $2)
                 WHERE id=$1",
            )
            .bind(job)
            .bind(delay.as_secs_f64())
            .execute(&workers.pool)
            .await;
            tracing::warn!(
                event_name = "notification_job_retry",
                attempts,
                "notification job will retry"
            );
        }
    }
    workers.deliveries.notify_one();
    Ok(true)
}

pub(super) async fn expand(
    workers: &Workers,
    job: i64,
    message: i64,
    attempts: i16,
) -> Result<(), ()> {
    let pool = &workers.pool;
    // Edits never enqueue. Guests, demo spaces, deleted conversations and
    // messages older than a day (a stopped worker) notify nobody.
    let source: Option<Source> = sqlx::query_as(
        "SELECT m.channel_id, c.space_id, cs.user_id AS author, m.thread_root_id IS NOT NULL AS thread_reply,
                -- A forward's note shows its mentions but never notifies them.
                CASE WHEN m.forward_source_id IS NULL THEN m.payload->'content'->'mentions' END AS mentions,
                (c.deleted_at IS NULL AND (c.space_id IS NULL OR (s.deleted_at IS NULL AND NOT s.demo))
                 AND m.created_at > now() - interval '24 hours') AS sendable
         FROM public.messages m
         JOIN public.chat_sessions cs ON cs.id=m.session_id
         JOIN public.channels c ON c.id=m.channel_id
         LEFT JOIN public.spaces s ON s.id=c.space_id
         WHERE m.id=$1",
    )
    .bind(message)
    .fetch_optional(pool)
    .await
    .map_err(|_| ())?;
    let presence_required = attempts <= PRESENCE_ATTEMPTS;
    let (source, recipients) = match source {
        Some(source) if source.sendable && source.author.is_some() => {
            let recipients = resolve(workers, &source, presence_required).await?;
            (Some(source), recipients)
        }
        _ => (None, Vec::new()),
    };
    let devices = devices(workers, &recipients, presence_required).await?;
    let mut tx = pool.begin().await.map_err(|_| ())?;
    if let Some(source) = &source
        && !recipients.is_empty()
    {
        let ids: Vec<i64> = recipients.iter().map(|r| r.id).collect();
        let kinds: Vec<&str> = recipients.iter().map(|r| r.kind.name()).collect();
        sqlx::query(
            "INSERT INTO public.notifications (user_id,kind,message_id,channel_id,space_id,actor_id)
             SELECT r.user_id, r.kind, $3, $4, $5, $6 FROM UNNEST($1::bigint[], $2::text[]) AS r(user_id, kind)
             ON CONFLICT (message_id,user_id) DO NOTHING",
        )
        .bind(&ids)
        .bind(&kinds)
        .bind(message)
        .bind(source.channel_id)
        .bind(source.space_id)
        .bind(source.author)
        .execute(&mut *tx)
        .await
        .map_err(|_| ())?;
        let users: Vec<i64> = devices.iter().map(|d| d.0).collect();
        let device_ids: Vec<i64> = devices.iter().map(|d| d.1).collect();
        let held: Vec<bool> = devices.iter().map(|d| d.2).collect();
        sqlx::query(
            "INSERT INTO public.notification_deliveries (notification_id,device_id,held,available_at)
             SELECT n.id, d.device_id, d.held, now() + CASE WHEN d.held THEN interval '60 seconds' ELSE interval '0' END
             FROM UNNEST($1::bigint[], $2::bigint[], $3::boolean[]) AS d(user_id, device_id, held)
             JOIN public.notifications n ON n.message_id=$4 AND n.user_id=d.user_id
             ON CONFLICT (notification_id,device_id) DO NOTHING",
        )
        .bind(&users)
        .bind(&device_ids)
        .bind(&held)
        .bind(message)
        .execute(&mut *tx)
        .await
        .map_err(|_| ())?;
    }
    sqlx::query(
        "UPDATE public.notification_jobs SET expanded_at=now(), locked_at=NULL WHERE id=$1",
    )
    .bind(job)
    .execute(&mut *tx)
    .await
    .map_err(|_| ())?;
    tx.commit().await.map_err(|_| ())?;
    if !recipients.is_empty() {
        tracing::info!(
            event_name = "notifications_created",
            recipients = recipients.len(),
            deliveries = devices.len(),
            "notifications created"
        );
    }
    Ok(())
}

async fn resolve(
    workers: &Workers,
    source: &Source,
    presence_required: bool,
) -> Result<Vec<Recipient>, ()> {
    // DMs: the other participant of an accepted conversation (not personal
    // notes) unless either blocked the other. Channels: every reader except
    // the author and anyone who blocked them; readers who haven't joined
    // hear only about messages that @mention them by name.
    let candidates: Vec<Candidate> = sqlx::query_as(
        "WITH candidates AS (
             SELECT u.id, true AS joined FROM public.direct_conversations d
             JOIN public.users u ON u.id = CASE WHEN d.low_user_id=$3 THEN d.high_user_id ELSE d.low_user_id END
             WHERE $2::bigint IS NULL AND d.channel_id=$1 AND d.low_user_id<>d.high_user_id
               AND $3 IN (d.low_user_id,d.high_user_id) AND d.accepted_at IS NOT NULL AND u.deleted_at IS NULL
               AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.deleted_at IS NULL
                   AND ((b.blocker_id=u.id AND b.blocked_id=$3) OR (b.blocker_id=$3 AND b.blocked_id=u.id)))
             UNION ALL
             SELECT u.id, EXISTS(SELECT 1 FROM public.channel_joins cj
                 WHERE cj.channel_id=c.id AND cj.user_id=u.id AND cj.deleted_at IS NULL) AS joined
             FROM public.channels c
             JOIN public.spaces s ON s.id=c.space_id
             JOIN public.space_members sm ON sm.space_id=s.id AND sm.deleted_at IS NULL
             JOIN public.users u ON u.id=sm.user_id
             WHERE $2::bigint IS NOT NULL AND c.id=$1 AND u.deleted_at IS NULL AND u.id<>$3
               AND (NOT c.private OR s.owner_id=u.id OR EXISTS(SELECT 1 FROM public.channel_members cm
                   WHERE cm.channel_id=c.id AND cm.user_id=u.id AND cm.deleted_at IS NULL))
               AND NOT EXISTS(SELECT 1 FROM public.user_blocks b
                   WHERE b.blocker_id=u.id AND b.blocked_id=$3 AND b.deleted_at IS NULL))
         SELECT u.id, u.external_id, candidates.joined, ns.default_level AS account_level,
                COALESCE(ns.paused_until > now(), false) AS paused,
                COALESCE(ns.mobile, 'whenInactive') AS mobile,
                co.level AS channel_level, COALESCE(co.muted_until > now(), false) AS channel_muted,
                so.level AS space_level, COALESCE(so.muted_until > now(), false) AS space_muted
         FROM candidates JOIN public.users u ON u.id=candidates.id
         LEFT JOIN public.notification_settings ns ON ns.user_id=u.id
         LEFT JOIN public.notification_overrides co ON co.user_id=u.id AND co.channel_id=$1
         LEFT JOIN public.notification_overrides so ON so.user_id=u.id AND so.space_id=$2
         ORDER BY u.id",
    )
    .bind(source.channel_id)
    .bind(source.space_id)
    .bind(source.author)
    .fetch_all(&workers.pool)
    .await
    .map_err(|_| ())?;
    let mentions = Mentions::from(source.mentions.as_ref());
    let mut recipients = Vec::new();
    let mut here = Vec::new();
    for candidate in candidates {
        let preferences = candidate.preferences();
        let kind = if source.space_id.is_none() {
            Kind::DirectMessage
        } else if mentions.users.contains(&candidate.external_id) {
            Kind::MentionUser
        } else if !candidate.joined || source.thread_reply {
            // Readers who haven't joined, and thread replies, notify only
            // the people they @mention.
            continue;
        } else if mentions.everyone {
            Kind::MentionEveryone
        } else if mentions.here && notifies(Kind::MentionEveryone, &preferences) {
            here.push((candidate, preferences));
            continue;
        } else {
            Kind::ChannelMessage
        };
        if notifies(kind, &preferences) {
            recipients.push(recipient(candidate, kind));
        }
    }
    if !here.is_empty() {
        // `@here` reaches readers whose presence is `online`; for everyone
        // else the message is an ordinary channel message.
        let users: Vec<String> = here.iter().map(|(c, _)| c.external_id.clone()).collect();
        let online = match online(workers, &users).await {
            Ok(online) => online,
            Err(()) if !presence_required => HashSet::new(),
            Err(()) => return Err(()),
        };
        for (candidate, preferences) in here {
            let kind = if online.contains(&candidate.external_id) {
                Kind::MentionEveryone
            } else {
                Kind::ChannelMessage
            };
            if notifies(kind, &preferences) {
                recipients.push(recipient(candidate, kind));
            }
        }
    }
    Ok(recipients)
}

fn recipient(candidate: Candidate, kind: Kind) -> Recipient {
    Recipient {
        id: candidate.id,
        external_id: candidate.external_id,
        kind,
        hold: candidate.mobile != "always",
    }
}

async fn presence(workers: &Workers) -> Result<Presence, ()> {
    workers.presence().await.ok_or(())
}

async fn online(workers: &Workers, users: &[String]) -> Result<HashSet<String>, ()> {
    let presence = presence(workers).await?;
    let mut online = HashSet::new();
    for chunk in users.chunks(PRESENCE_CHUNK) {
        for status in presence.statuses(chunk).await.map_err(|_| ())? {
            if status["status"] == "online"
                && let Some(user) = status["userId"].as_str()
            {
                online.insert(user.to_owned());
            }
        }
    }
    Ok(online)
}

/// Session tags of each user's active gateway connections.
pub(super) async fn active_sessions(
    workers: &Workers,
    users: &[String],
) -> Result<HashMap<String, Vec<String>>, ()> {
    let presence = presence(workers).await?;
    let mut active = HashMap::new();
    for chunk in users.chunks(PRESENCE_CHUNK) {
        let tags = presence.active_sessions(chunk).await.map_err(|_| ())?;
        active.extend(chunk.iter().cloned().zip(tags));
    }
    Ok(active)
}

/// `(user, device, held)` for every active device on a platform that is
/// available now, whose session is still valid. A device is held while its
/// owner (with `mobile = whenInactive`) is active on a different session.
async fn devices(
    workers: &Workers,
    recipients: &[Recipient],
    presence_required: bool,
) -> Result<Vec<(i64, i64, bool)>, ()> {
    if recipients.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<i64> = recipients.iter().map(|r| r.id).collect();
    let platforms: Vec<&str> = workers
        .push
        .platforms()
        .iter()
        .map(|p: &Platform| p.name())
        .collect();
    let devices: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT d.user_id, d.id, encode(substring(d.account_session_hash FROM 1 FOR 8), 'hex')
         FROM public.notification_devices d
         JOIN public.account_sessions a ON a.token_hash=d.account_session_hash
         WHERE d.user_id=ANY($1) AND d.transport=ANY($2) AND d.revoked_at IS NULL
           AND a.user_id=d.user_id AND a.revoked_at IS NULL AND a.expires_at > now()
         ORDER BY d.id",
    )
    .bind(&ids)
    .bind(&platforms)
    .fetch_all(&workers.pool)
    .await
    .map_err(|_| ())?;
    let device_users: HashSet<i64> = devices.iter().map(|device| device.0).collect();
    let holding: Vec<String> = recipients
        .iter()
        .filter(|r| r.hold && device_users.contains(&r.id))
        .map(|r| r.external_id.clone())
        .collect();
    let active = if holding.is_empty() {
        HashMap::new()
    } else {
        match active_sessions(workers, &holding).await {
            Ok(active) => active,
            Err(()) if !presence_required => HashMap::new(),
            Err(()) => return Err(()),
        }
    };
    let recipients: HashMap<i64, &Recipient> = recipients
        .iter()
        .map(|recipient| (recipient.id, recipient))
        .collect();
    Ok(devices
        .into_iter()
        .map(|(user, device, tag)| {
            let recipient = recipients.get(&user);
            let held = recipient.is_some_and(|r| {
                r.hold
                    && active
                        .get(&r.external_id)
                        .is_some_and(|tags| tags.iter().any(|other| *other != tag))
            });
            (user, device, held)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn levels_mutes_and_mentions_follow_the_resolution_order() {
        let default = Preferences::default();
        assert!(notifies(Kind::ChannelMessage, &default));
        assert!(notifies(Kind::DirectMessage, &default));
        let mentions = Preferences {
            account: Some(Level::Mentions),
            ..Preferences::default()
        };
        assert!(!notifies(Kind::ChannelMessage, &mentions));
        assert!(notifies(Kind::MentionEveryone, &mentions));
        assert!(notifies(Kind::MentionUser, &mentions));
        assert!(
            notifies(Kind::DirectMessage, &mentions),
            "DMs notify under mentions"
        );
        let off = Preferences {
            account: Some(Level::Nothing),
            channel: Some(Level::All),
            ..Preferences::default()
        };
        for kind in [Kind::DirectMessage, Kind::ChannelMessage, Kind::MentionUser] {
            assert!(!notifies(kind, &off), "account off silences {kind:?}");
        }
        let paused = Preferences {
            paused: true,
            ..Preferences::default()
        };
        assert!(!notifies(Kind::MentionUser, &paused));
        // Channel beats space beats account.
        let channel_all = Preferences {
            channel: Some(Level::All),
            space: Some(Level::Nothing),
            ..Preferences::default()
        };
        assert!(notifies(Kind::ChannelMessage, &channel_all));
        let space_mentions = Preferences {
            space: Some(Level::Mentions),
            account: Some(Level::All),
            ..Preferences::default()
        };
        assert!(!notifies(Kind::ChannelMessage, &space_mentions));
        let channel_nothing = Preferences {
            channel: Some(Level::Nothing),
            ..Preferences::default()
        };
        assert!(!notifies(Kind::MentionUser, &channel_nothing));
        assert!(!notifies(Kind::MentionEveryone, &channel_nothing));
        // Mutes silence everything but a direct @mention.
        for muted in [
            Preferences {
                channel_muted: true,
                ..Preferences::default()
            },
            Preferences {
                space_muted: true,
                ..Preferences::default()
            },
        ] {
            assert!(!notifies(Kind::ChannelMessage, &muted));
            assert!(!notifies(Kind::MentionEveryone, &muted));
            assert!(!notifies(Kind::DirectMessage, &muted));
            assert!(notifies(Kind::MentionUser, &muted));
        }
        let dm_off = Preferences {
            channel: Some(Level::Nothing),
            ..Preferences::default()
        };
        assert!(!notifies(Kind::DirectMessage, &dm_off));
    }

    #[test]
    fn reads_user_and_special_mentions() {
        let mentions = Mentions::from(Some(&json!([
            {"type": "user", "id": "abc", "username": "alice"},
            {"type": "here"},
            {"type": "unknown"},
        ])));
        assert!(mentions.users.contains("abc"));
        assert!(mentions.here && !mentions.everyone);
        assert!(Mentions::from(None).users.is_empty());
    }
}
