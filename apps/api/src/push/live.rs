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
    message: Value,
    sender: String,
    sender_id: String,
    sender_avatar_id: Option<i16>,
    sender_avatar_asset: Option<String>,
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
    fn event(&self, cdn: Option<&crate::assets::CdnSigner>) -> Option<Value> {
        let kind = match self.kind.as_str() {
            "direct.message" => Kind::DirectMessage,
            "channel.message" => Kind::ChannelMessage,
            "mention.user" => Kind::MentionUser,
            "mention.everyone" => Kind::MentionEveryone,
            "space.invitation" | "channel.invitation" => Kind::Invitation,
            "direct.request" => Kind::DirectRequest,
            "feedback" => Kind::Feedback,
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
        let alert = if kind == Kind::DirectRequest {
            Alert::request(
                self.id,
                &self.channel_id,
                &self.sender,
                &self.sender_id,
                self.sender_avatar_id,
            )
        } else if kind == Kind::Invitation {
            Alert::invitation(
                self.id,
                self.space_id.as_deref()?,
                self.space_name.as_deref()?,
                (self.kind == "channel.invitation")
                    .then_some((self.channel_id.as_str(), self.channel_name.as_str())),
                &self.sender,
                &self.sender_id,
                self.sender_avatar_id,
            )
        } else {
            Alert::new(
                &self.kind,
                &self.message_id,
                conversation,
                &self.sender,
                &self.sender_id,
                self.sender_avatar_id,
                self.message
                    .pointer("/content/text")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
            .with_content(&self.message, cdn)
        };
        let mut event = json!({
            "type": "notification.created", "seq": self.id.to_string(),
            "kind": alert.kind, "title": alert.title,
            "body": alert.body, "sender": alert.sender, "senderId": alert.sender_id,
            "senderAvatarId": alert.sender_avatar_id, "createdAt": self.created_at,
        });
        if let (Some(asset), Some(cdn)) = (&self.sender_avatar_asset, cdn) {
            event["senderAvatarUrl"] = json!(
                cdn.notification_url(&crate::assets::original_key(asset), Utc::now().timestamp(),)
            );
        }
        if let Some(root) = &alert.thread_root_id {
            event["threadRootId"] = json!(root);
        }
        if let Some(id) = &alert.notification_id {
            event["notificationId"] = json!(id);
        } else {
            event["messageId"] = json!(alert.message_id);
        }
        if let Some(attachment) = &alert.attachment {
            event["attachmentUrl"] = json!(attachment.url);
            event["attachmentContentType"] = json!(attachment.content_type);
            event["attachmentSize"] = json!(attachment.size);
        }
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
            Conversation::Invitation {
                space_id,
                channel_id,
            } => {
                event["spaceId"] = json!(space_id);
                if let Some(id) = channel_id {
                    event["channelId"] = json!(id);
                }
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
    cdn: Option<&crate::assets::CdnSigner>,
) -> Result<Vec<(i64, Option<Value>)>, sqlx::Error> {
    let mut rows: Vec<Row> = sqlx::query_as(
        "SELECT n.id,n.kind,COALESCE(m.external_id,'') AS message_id,COALESCE(m.created_at,n.created_at) AS created_at,
                COALESCE(m.payload,'{}'::jsonb) AS message,
                COALESCE(author.display_name,'') AS sender,COALESCE(author.external_id,'') AS sender_id,
                author.avatar_id AS sender_avatar_id,author.avatar_asset AS sender_avatar_asset,
                COALESCE(ch.external_id,'') AS channel_id,COALESCE(ch.name,'') AS channel_name,
                s.external_id AS space_id,s.name AS space_name,
                CASE WHEN n.kind='direct.request' THEN COALESCE(req.eligible,false) AND n.created_at > now()-interval '24 hours'
                WHEN n.message_id IS NULL THEN COALESCE(inv.eligible,false) AND n.created_at > now()-interval '24 hours'
                ELSE (n.dismissed_at IS NULL AND m.created_at > now()-interval '24 hours'
                 AND recipient.deleted_at IS NULL AND author.id IS NOT NULL
                 AND ch.deleted_at IS NULL AND (ch.space_id IS NULL OR s.deleted_at IS NULL)
                 AND NOT EXISTS(SELECT 1 FROM public.user_blocks b WHERE b.deleted_at IS NULL
                   AND ((b.blocker_id=n.user_id AND b.blocked_id=n.actor_id)
                     OR (ch.space_id IS NULL AND b.blocker_id=n.actor_id AND b.blocked_id=n.user_id)))
                 AND (CASE WHEN ch.space_id IS NULL THEN
                   EXISTS(SELECT 1 FROM public.direct_conversations d WHERE d.channel_id=ch.id
                     AND d.accepted_at IS NOT NULL AND n.user_id IN(d.low_user_id,d.high_user_id)
                     AND ((d.low_user_id<>d.high_user_id AND n.actor_id IN(d.low_user_id,d.high_user_id))
                       -- The team answering in a DM it visits, personal notes included.
                       OR (NOT n.actor_id IN(d.low_user_id,d.high_user_id) AND public.feedback_access(ch.id,n.actor_id))))
                   AND COALESCE(r.seq,0)<m.channel_seq
                 ELSE NOT s.demo
                   AND EXISTS(SELECT 1 FROM public.space_members sm WHERE sm.space_id=s.id AND sm.user_id=n.user_id AND sm.deleted_at IS NULL)
                   AND (NOT ch.private OR s.owner_id=n.user_id OR EXISTS(SELECT 1 FROM public.channel_members cm
                     WHERE cm.channel_id=ch.id AND cm.user_id=n.user_id AND cm.deleted_at IS NULL))
                   AND (n.kind IN ('mention.user','feedback') OR EXISTS(SELECT 1 FROM public.channel_joins cj
                     WHERE cj.channel_id=ch.id AND cj.user_id=n.user_id AND cj.deleted_at IS NULL))
                 END
                 -- The feedback team, visiting where it was asked for.
                 OR (n.kind='feedback' AND (s.id IS NULL OR NOT s.demo)
                   AND public.feedback_access(ch.id,n.user_id)))) END AS eligible,
                ns.default_level AS account_level,COALESCE(ns.paused_until>now(),false) AS paused,
                co.level AS channel_level,COALESCE(co.muted_until>now(),false) AS channel_muted,
                so.level AS space_level,COALESCE(so.muted_until>now(),false) AS space_muted
         FROM public.notifications n LEFT JOIN public.messages m ON m.id=n.message_id
         LEFT JOIN public.channels ch ON ch.id=n.channel_id LEFT JOIN public.spaces s ON s.id=n.space_id
         LEFT JOIN public.notification_invitation_sources inv ON inv.id=n.id
         LEFT JOIN public.notification_request_sources req ON req.id=n.id
         JOIN public.users recipient ON recipient.id=n.user_id
         LEFT JOIN public.users author ON author.id=n.actor_id AND author.deleted_at IS NULL
         LEFT JOIN public.notification_settings ns ON ns.user_id=n.user_id
         LEFT JOIN public.notification_overrides co ON co.user_id=n.user_id AND co.channel_id=ch.id
         LEFT JOIN public.notification_overrides so ON so.user_id=n.user_id AND so.space_id=ch.space_id
         LEFT JOIN public.direct_reads r ON r.channel_id=n.channel_id AND r.user_id=n.user_id
         WHERE n.user_id=$1 AND n.id>$2 AND n.id<=$3 ORDER BY n.id LIMIT 128"
    ).bind(user).bind(after).bind(head).fetch_all(pool).await?;
    let mut messages: Vec<_> = rows.iter().map(|row| row.message.clone()).collect();
    crate::assets::notification_content(pool, &mut messages)
        .await
        .map_err(|_| sqlx::Error::Protocol("attachment lookup unavailable".into()))?;
    for (row, message) in rows.iter_mut().zip(messages) {
        row.message = message;
    }
    Ok(rows
        .into_iter()
        .map(|row| (row.id, row.event(cdn)))
        .collect())
}
