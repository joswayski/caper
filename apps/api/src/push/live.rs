//! Account-wide notifications for running clients. Postgres is authoritative;
//! the gateway batches recovery-head reads once per watched account per pod.
use super::{
    Alert, Conversation, Level,
    expansion::{Kind, Preferences, notifies},
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

#[derive(sqlx::FromRow)]
struct Row {
    id: i64,
    kind: String,
    message_id: String,
    created_at: DateTime<Utc>,
    text: Option<String>,
    sender: String,
    sender_id: String,
    sender_avatar_id: Option<i16>,
    channel_id: String,
    channel_name: String,
    space_id: Option<String>,
    space_name: Option<String>,
    eligible: bool,
    account_level: Option<String>,
    paused: bool,
    channel_level: Option<String>,
    channel_muted: bool,
    space_level: Option<String>,
    space_muted: bool,
}

impl Row {
    fn event(&self) -> Option<Value> {
        let kind = match self.kind.as_str() {
            "direct.message" => Kind::DirectMessage,
            "channel.message" => Kind::ChannelMessage,
            "mention.user" => Kind::MentionUser,
            "mention.everyone" => Kind::MentionEveryone,
            _ => return None,
        };
        let level = |value: &Option<String>| value.as_deref().and_then(Level::parse);
        if !self.eligible
            || !notifies(
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
        {
            return None;
        }
        let conversation = match (&self.space_id, &self.space_name) {
            (Some(space), Some(name)) => Conversation::Channel {
                space_id: space.clone(),
                channel_id: self.channel_id.clone(),
                title: format!("#{} ({name})", self.channel_name),
                recipient_count: 0,
            },
            _ => Conversation::Direct {
                id: self.channel_id.clone(),
            },
        };
        let alert = Alert::new(
            &self.kind,
            &self.message_id,
            conversation,
            &self.sender,
            &self.sender_id,
            self.sender_avatar_id,
            self.text.as_deref().unwrap_or_default(),
        );
        let mut event = json!({
            "type": "notification.created", "seq": self.id.to_string(),
            "kind": alert.kind, "messageId": alert.message_id, "title": alert.title,
            "body": alert.body, "sender": alert.sender, "senderId": alert.sender_id,
            "senderAvatarId": alert.sender_avatar_id, "createdAt": self.created_at,
        });
        match alert.conversation {
            Conversation::Direct { id } => event["conversationId"] = json!(id),
            Conversation::Channel {
                space_id,
                channel_id,
                title,
                ..
            } => {
                event["spaceId"] = json!(space_id);
                event["channelId"] = json!(channel_id);
                event["conversationTitle"] = json!(title);
            }
        }
        Some(event)
    }
}

/// Includes suppressed IDs so a revoked grant/mute cannot pin the cursor or
/// cause a previously suppressed message to alert later after consent changes.
pub(crate) async fn read(
    pool: &PgPool,
    user: i64,
    after: i64,
    head: i64,
) -> Result<Vec<(i64, Option<Value>)>, sqlx::Error> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT n.id,n.kind,m.external_id AS message_id,m.created_at,
                m.payload->'content'->>'text' AS text,
                COALESCE(author.display_name,'') AS sender,COALESCE(author.external_id,'') AS sender_id,
                author.avatar_id AS sender_avatar_id,ch.external_id AS channel_id,ch.name AS channel_name,
                s.external_id AS space_id,s.name AS space_name,
                (n.dismissed_at IS NULL AND m.created_at > now()-interval '24 hours'
                 AND recipient.deleted_at IS NULL AND author.id IS NOT NULL
                 AND ch.deleted_at IS NULL AND (ch.space_id IS NULL OR s.deleted_at IS NULL)
                 AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.deleted_at IS NULL
                   AND ((b.blocker_id=n.user_id AND b.blocked_id=n.actor_id)
                     OR (ch.space_id IS NULL AND b.blocker_id=n.actor_id AND b.blocked_id=n.user_id)))
                 AND CASE WHEN ch.space_id IS NULL THEN
                   EXISTS(SELECT 1 FROM public.direct_conversations d WHERE d.channel_id=ch.id
                     AND d.accepted_at IS NOT NULL AND d.low_user_id<>d.high_user_id
                     AND n.user_id IN(d.low_user_id,d.high_user_id) AND n.actor_id IN(d.low_user_id,d.high_user_id))
                   AND COALESCE(r.seq,0)<m.channel_seq
                 ELSE NOT s.demo
                   AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=n.user_id AND sm.deleted_at IS NULL)
                   AND (NOT ch.private OR s.owner_id=n.user_id OR EXISTS(SELECT 1 FROM public.channel_members cm
                     WHERE cm.channel_id=ch.id AND cm.user_id=n.user_id AND cm.deleted_at IS NULL))
                   AND (n.kind='mention.user' OR EXISTS(SELECT 1 FROM public.channel_joins cj
                     WHERE cj.channel_id=ch.id AND cj.user_id=n.user_id AND cj.deleted_at IS NULL))
                 END) AS eligible,
                ns.default_level AS account_level,COALESCE(ns.paused_until>now(),false) AS paused,
                co.level AS channel_level,COALESCE(co.muted_until>now(),false) AS channel_muted,
                so.level AS space_level,COALESCE(so.muted_until>now(),false) AS space_muted
         FROM public.notifications n JOIN public.messages m ON m.id=n.message_id
         JOIN public.channels ch ON ch.id=n.channel_id LEFT JOIN public.spaces s ON s.id=ch.space_id
         JOIN public.users recipient ON recipient.id=n.user_id
         LEFT JOIN public.users author ON author.id=n.actor_id AND author.deleted_at IS NULL
         LEFT JOIN public.notification_settings ns ON ns.user_id=n.user_id
         LEFT JOIN public.notification_overrides co ON co.user_id=n.user_id AND co.channel_id=ch.id
         LEFT JOIN public.notification_overrides so ON so.user_id=n.user_id AND so.space_id=ch.space_id
         LEFT JOIN public.direct_reads r ON r.channel_id=n.channel_id AND r.user_id=n.user_id
         WHERE n.user_id=$1 AND n.id>$2 AND n.id<=$3 ORDER BY n.id LIMIT 128"
    ).bind(user).bind(after).bind(head).fetch_all(pool).await?;
    Ok(rows.into_iter().map(|row| (row.id, row.event())).collect())
}
