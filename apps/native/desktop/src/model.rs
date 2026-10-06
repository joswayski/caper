#![allow(dead_code)] // Complete API contracts retain fields not yet rendered by this client.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    #[serde(default)]
    pub avatar_id: Option<i32>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    #[serde(default)]
    pub debug_enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inviter {
    pub username: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: String,
    pub name: String,
    pub owner_id: String,
    pub inviter: Option<Inviter>,
    #[serde(default)]
    pub demo: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    pub id: String,
    pub space_id: String,
    pub name: String,
    pub private: bool,
    /// Older servers predate explicit channel membership; their channels were joined.
    #[serde(default = "default_joined")]
    pub joined: bool,
}

fn default_joined() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelInvitation {
    pub channel: Channel,
    pub inviter: Inviter,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DirectPeer {
    pub id: String,
    pub username: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DirectConversation {
    pub id: String,
    pub peer: DirectPeer,
    pub last_seq: String,
    pub read_seq: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DirectConversations {
    pub conversations: Vec<DirectConversation>,
}

/// Spectators receive identity and status, never media track capabilities.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceOccupant {
    pub id: String,
    #[serde(default)]
    pub avatar_id: Option<i32>,
    pub name: String,
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: String,
    #[serde(default)]
    pub avatar_id: Option<i32>,
    pub username: String,
    pub display_name: String,
    pub owner: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Spaces {
    pub spaces: Vec<Space>,
    #[serde(default)]
    pub invitations: Vec<Space>,
    #[serde(default)]
    pub limits: Option<SpaceLimits>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceLimits {
    pub owned_spaces: usize,
    pub total_spaces: usize,
    pub channels_per_space: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SpaceDetail {
    pub space: Space,
    pub channels: Vec<Channel>,
    pub members: Vec<Member>,
    #[serde(default)]
    pub channel_invitations: Vec<ChannelInvitation>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub id: String,
    #[serde(default)]
    pub avatar_id: Option<i32>,
    pub name: String,
    pub is_guest: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Content {
    pub version: u8,
    #[serde(rename = "type")]
    pub kind: String,
    pub text: String,
    /// Additive on version 1 `text` content. Malformed entries are skipped so
    /// one bad file never rejects a message or a history page.
    #[serde(
        default,
        deserialize_with = "tolerant_attachments",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentKind {
    Image,
    Video,
    Audio,
    File,
}

/// Server-side processing state. Old payloads have none and are ready.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentStatus {
    Processing,
    #[default]
    Ready,
    Failed,
}

impl AttachmentStatus {
    fn is_ready(&self) -> bool {
        *self == Self::Ready
    }
}

impl AttachmentKind {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "image" => Self::Image,
            "video" => Self::Video,
            "audio" => Self::Audio,
            "file" => Self::File,
            _ => return None,
        })
    }
}

/// A file on a message, as the API describes it. URLs are signed per response
/// and expire; `preview` only says a preview object exists.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub kind: AttachmentKind,
    pub content_type: String,
    pub name: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(
        skip_serializing_if = "std::ops::Not::not",
        serialize_with = "preview_marker"
    )]
    pub preview: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_url: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unavailable: bool,
    /// Absent (ready) is not written back, so old payloads round-trip.
    #[serde(skip_serializing_if = "AttachmentStatus::is_ready")]
    pub status: AttachmentStatus,
    /// A GIF or animated image stored as a silent looping MP4.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub animated: bool,
}

fn preview_marker<S: serde::Serializer>(_: &bool, serializer: S) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    serializer.serialize_map(Some(0))?.end()
}

impl Attachment {
    /// Mirrors web `isChatAttachment`: required strings and kind, optional
    /// non-negative numbers, and only http(s) URLs.
    pub fn parse(value: &serde_json::Value) -> Option<Self> {
        let object = value.as_object()?;
        let text = |name: &str| object.get(name)?.as_str().map(str::to_owned);
        let optional_number = |name: &str| -> Result<Option<u64>, ()> {
            match object.get(name) {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(value) => value
                    .as_u64()
                    .or_else(|| {
                        value
                            .as_f64()
                            .filter(|number| number.is_finite() && *number >= 0.0)
                            .map(|number| number.round() as u64)
                    })
                    .map(Some)
                    .ok_or(()),
            }
        };
        let optional_url = |name: &str| -> Result<Option<String>, ()> {
            match object.get(name) {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(serde_json::Value::String(url))
                    if url.starts_with("https://") || url.starts_with("http://") =>
                {
                    Ok(Some(url.clone()))
                }
                Some(_) => Err(()),
            }
        };
        let id = text("id").filter(|id| !id.is_empty())?;
        let size = object.get("size")?;
        let size = size.as_u64().or_else(|| {
            size.as_f64()
                .filter(|number| number.is_finite() && *number >= 0.0)
                .map(|number| number as u64)
        })?;
        let dimension = |name: &str| -> Result<Option<u32>, ()> {
            optional_number(name)?
                .map(|value| u32::try_from(value).map_err(|_| ()))
                .transpose()
        };
        Some(Self {
            id,
            kind: AttachmentKind::parse(object.get("kind")?.as_str()?)?,
            content_type: text("contentType")?,
            name: text("name")?,
            size,
            width: dimension("width").ok()?,
            height: dimension("height").ok()?,
            duration_ms: optional_number("durationMs").ok()?,
            preview: object
                .get("preview")
                .is_some_and(serde_json::Value::is_object),
            url: optional_url("url").ok()?,
            preview_url: optional_url("previewUrl").ok()?,
            unavailable: object.get("unavailable") == Some(&serde_json::Value::Bool(true)),
            // Unknown future states read as ready: the URLs decide what shows.
            status: match object.get("status").and_then(serde_json::Value::as_str) {
                Some("processing") => AttachmentStatus::Processing,
                Some("failed") => AttachmentStatus::Failed,
                _ => AttachmentStatus::Ready,
            },
            animated: object.get("animated") == Some(&serde_json::Value::Bool(true)),
        })
    }
}

/// Never fails: a missing, null or non-array field is no attachments, and
/// malformed entries are dropped.
fn tolerant_attachments<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Attachment>, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_array()
        .map(|items| items.iter().filter_map(Attachment::parse).collect())
        .unwrap_or_default())
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reaction {
    pub emoji: String,
    pub author_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReactionUpdate {
    #[serde(rename = "type")]
    pub kind: String,
    pub schema_version: u8,
    pub channel_id: String,
    pub seq: String,
    pub message_id: String,
    pub reactions: Vec<Reaction>,
}

/// `message.attachments`: the message's files after the media worker
/// produced a preview, finished or failed. Sequenced like reactions.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUpdate {
    #[serde(rename = "type")]
    pub kind: String,
    pub schema_version: u8,
    pub channel_id: String,
    pub seq: String,
    pub message_id: String,
    #[serde(deserialize_with = "tolerant_attachments")]
    pub attachments: Vec<Attachment>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub channel_id: String,
    pub seq: String,
    pub created_at: String,
    pub client_message_id: String,
    pub author: Author,
    pub content: Content,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
    #[serde(default)]
    pub reaction_seq: Option<String>,
    /// Sequence of the last `message.attachments` update folded into this
    /// payload, like `reaction_seq`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attachments_seq: Option<String>,
}

impl Message {
    pub fn validate(&self) -> Result<(), String> {
        sequence(&self.seq)?;
        if self.content.version != 1 || self.content.kind != "text" {
            return Err("unsupported message content".into());
        }
        for revision in [&self.reaction_seq, &self.attachments_seq]
            .into_iter()
            .flatten()
        {
            sequence(revision)?;
        }
        if !valid_reactions(&self.reactions) {
            return Err("invalid message reactions".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct HistoryPlace {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub messages: Vec<Message>,
    pub cursor: String,
    pub has_more: bool,
    pub space: HistoryPlace,
    pub channel: HistoryPlace,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChatSession {
    pub token: String,
    pub author: Author,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Presence {
    pub user_id: String,
    pub status: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Members {
    pub members: Vec<Member>,
    #[serde(default)]
    pub invitations: Vec<Member>,
}

#[derive(Default)]
pub struct Timeline {
    cursor: u64,
    messages: BTreeMap<u64, Message>,
    ids: BTreeSet<String>,
    buffered: BTreeMap<u64, Message>,
    unseen_reactions: BTreeMap<String, ReactionUpdate>,
    unseen_attachments: BTreeMap<String, AttachmentUpdate>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Apply {
    Applied,
    Duplicate,
    Buffered,
    Resync,
}

impl Timeline {
    pub fn reset(&mut self, messages: Vec<Message>, cursor: &str) -> Result<(), String> {
        self.cursor = sequence(cursor)?;
        self.messages.clear();
        self.ids.clear();
        self.buffered.clear();
        self.unseen_reactions.clear();
        self.unseen_attachments.clear();
        for message in messages {
            self.merge(message)?;
        }
        Ok(())
    }

    pub fn apply(&mut self, message: Message) -> Result<Apply, String> {
        message.validate()?;
        let seq = sequence(&message.seq)?;
        if seq <= self.cursor {
            self.merge(message)?;
            return Ok(Apply::Duplicate);
        }
        if seq > self.cursor + 1 {
            if self.buffered.len() >= 256 {
                return Ok(Apply::Resync);
            }
            self.buffered.entry(seq).or_insert(message);
            return Ok(Apply::Buffered);
        }
        self.merge(message)?;
        self.cursor = seq;
        while let Some(next) = self.buffered.remove(&(self.cursor + 1)) {
            self.cursor += 1;
            self.merge(next)?;
        }
        Ok(Apply::Applied)
    }

    pub fn apply_sequence(&mut self, value: &str) -> Result<Apply, String> {
        let seq = sequence(value)?;
        if seq <= self.cursor {
            return Ok(Apply::Duplicate);
        }
        if seq != self.cursor + 1 {
            return Ok(Apply::Resync);
        }
        self.cursor = seq;
        while let Some(next) = self.buffered.remove(&(self.cursor + 1)) {
            self.cursor += 1;
            self.merge(next)?;
        }
        Ok(Apply::Applied)
    }

    pub fn apply_reactions(&mut self, update: ReactionUpdate) -> Result<Apply, String> {
        if !self.merge_reactions(&update)? {
            self.unseen_reactions.clear();
            return Ok(Apply::Resync);
        }
        let applied = self.apply_sequence(&update.seq)?;
        if applied == Apply::Resync {
            self.unseen_reactions.clear();
        }
        Ok(applied)
    }

    /// Same sequencing as [`Timeline::apply_reactions`].
    pub fn apply_attachments(&mut self, update: AttachmentUpdate) -> Result<Apply, String> {
        if !self.merge_attachments(&update)? {
            self.unseen_attachments.clear();
            return Ok(Apply::Resync);
        }
        let applied = self.apply_sequence(&update.seq)?;
        if applied == Apply::Resync {
            self.unseen_attachments.clear();
        }
        Ok(applied)
    }

    /// Replace a message's files when the update is newer than its
    /// `attachments_seq`; updates for unloaded messages wait for them.
    fn merge_attachments(&mut self, update: &AttachmentUpdate) -> Result<bool, String> {
        let seq = sequence(&update.seq)?;
        if update.kind != "message.attachments" || update.schema_version != 1 {
            return Err("unsupported attachment update".into());
        }
        if update.channel_id.is_empty() || update.message_id.is_empty() {
            return Err("invalid attachment update".into());
        }
        if let Some(message) = self
            .messages
            .values_mut()
            .find(|item| item.id == update.message_id)
        {
            if message.channel_id != update.channel_id {
                return Err("attachment update is for another channel".into());
            }
            if seq > revision(message.attachments_seq.as_deref())? {
                message.content.attachments.clone_from(&update.attachments);
                message.attachments_seq = Some(update.seq.clone());
            }
            return Ok(true);
        }
        if self
            .messages
            .values()
            .next()
            .is_some_and(|message| message.channel_id != update.channel_id)
        {
            return Err("attachment update is for another channel".into());
        }
        match self.unseen_attachments.get(&update.message_id) {
            Some(current) if seq <= sequence(&current.seq)? => {}
            None if self.unseen_attachments.len() >= 256 => return Ok(false),
            _ => {
                self.unseen_attachments
                    .insert(update.message_id.clone(), update.clone());
            }
        }
        Ok(true)
    }

    /// Merge an HTTP acknowledgement without moving the gateway replay cursor.
    pub fn merge_reaction_ack(&mut self, update: ReactionUpdate) -> Result<(), String> {
        if self.merge_reactions(&update)? {
            Ok(())
        } else {
            self.unseen_reactions.clear();
            Err("too many reactions for unloaded messages".into())
        }
    }

    fn merge_reactions(&mut self, update: &ReactionUpdate) -> Result<bool, String> {
        let seq = sequence(&update.seq)?;
        if update.kind != "message.reactions" || update.schema_version != 1 {
            return Err("unsupported reaction update".into());
        }
        if update.channel_id.is_empty()
            || update.message_id.is_empty()
            || !valid_reactions(&update.reactions)
        {
            return Err("invalid reaction update".into());
        }
        if let Some(message) = self
            .messages
            .values_mut()
            .find(|item| item.id == update.message_id)
        {
            if message.channel_id != update.channel_id {
                return Err("reaction update is for another channel".into());
            }
            let current = message
                .reaction_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .unwrap_or(0);
            if seq > current {
                message.reactions.clone_from(&update.reactions);
                message.reaction_seq = Some(update.seq.clone());
            }
        } else {
            if self
                .messages
                .values()
                .next()
                .is_some_and(|message| message.channel_id != update.channel_id)
            {
                return Err("reaction update is for another channel".into());
            }
            if let Some(current) = self.unseen_reactions.get(&update.message_id) {
                if seq > sequence(&current.seq)? {
                    self.unseen_reactions
                        .insert(update.message_id.clone(), update.clone());
                }
            } else {
                if self.unseen_reactions.len() >= 256 {
                    return Ok(false);
                }
                self.unseen_reactions
                    .insert(update.message_id.clone(), update.clone());
            }
        }
        Ok(true)
    }

    pub fn merge_sent(&mut self, message: Message) -> Result<(), String> {
        self.merge(message)
    }

    pub fn prepend(&mut self, messages: Vec<Message>) -> Result<(), String> {
        for message in messages {
            self.merge(message)?;
        }
        Ok(())
    }

    pub fn cursor(&self) -> String {
        self.cursor.to_string()
    }

    pub fn messages(&self) -> impl Iterator<Item = &Message> {
        self.messages.values()
    }

    fn merge(&mut self, mut message: Message) -> Result<(), String> {
        message.validate()?;
        if let Some(update) = self.unseen_attachments.remove(&message.id) {
            if update.channel_id != message.channel_id {
                return Err("attachment update is for another channel".into());
            }
            if sequence(&update.seq)? > revision(message.attachments_seq.as_deref())? {
                message.content.attachments = update.attachments;
                message.attachments_seq = Some(update.seq);
            }
        }
        if let Some(update) = self.unseen_reactions.remove(&message.id) {
            if update.channel_id != message.channel_id {
                return Err("reaction update is for another channel".into());
            }
            let update_revision = sequence(&update.seq)?;
            let message_revision = message
                .reaction_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .unwrap_or(0);
            if update_revision > message_revision {
                message.reactions = update.reactions;
                message.reaction_seq = Some(update.seq);
            }
        }
        let seq = sequence(&message.seq)?;
        if let Some(existing) = self
            .messages
            .get_mut(&seq)
            .filter(|item| item.id == message.id)
        {
            let old_revision = existing
                .reaction_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .unwrap_or(0);
            let new_revision = message
                .reaction_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .unwrap_or(0);
            if old_revision > new_revision {
                message.reactions = std::mem::take(&mut existing.reactions);
                message.reaction_seq = existing.reaction_seq.take();
            }
            // A stale page or replay never turns ready files back into
            // processing ones.
            if revision(existing.attachments_seq.as_deref())?
                > revision(message.attachments_seq.as_deref())?
            {
                message.content.attachments = std::mem::take(&mut existing.content.attachments);
                message.attachments_seq = existing.attachments_seq.take();
            }
            *existing = message;
        } else if !self.messages.contains_key(&seq) && self.ids.insert(message.id.clone()) {
            self.messages.insert(seq, message);
        }
        Ok(())
    }
}

/// An optional revision; absent is "0".
fn revision(value: Option<&str>) -> Result<u64, String> {
    value
        .map(sequence)
        .transpose()
        .map(Option::unwrap_or_default)
}

fn valid_reactions(reactions: &[Reaction]) -> bool {
    let mut emojis = BTreeSet::new();
    reactions.iter().all(|reaction| {
        !reaction.emoji.is_empty()
            && !reaction.author_ids.is_empty()
            && emojis.insert(reaction.emoji.as_str())
            && reaction.author_ids.iter().all(|author| !author.is_empty())
            && reaction.author_ids.iter().collect::<BTreeSet<_>>().len()
                == reaction.author_ids.len()
    })
}

pub fn sequence(value: &str) -> Result<u64, String> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err("invalid message sequence".into());
    }
    value
        .parse::<i64>()
        .map(|number| number as u64)
        .map_err(|_| "invalid message sequence".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_default_missing_invitations_to_empty() {
        let spaces: Spaces = serde_json::from_value(serde_json::json!({
            "spaces": [],
            "limits": null
        }))
        .unwrap();
        assert!(spaces.invitations.is_empty());
    }

    #[test]
    fn invitation_decodes_inviter_and_legacy_metadata() {
        let legacy: Space = serde_json::from_value(serde_json::json!({
            "id": "space", "name": "Studio", "ownerId": "owner"
        }))
        .unwrap();
        assert!(legacy.inviter.is_none());
        let invitation: Space = serde_json::from_value(serde_json::json!({
            "id": "space", "name": "Studio", "ownerId": "owner",
            "inviter": { "username": "host_user", "displayName": "Space Host" }
        }))
        .unwrap();
        let inviter = invitation.inviter.unwrap();
        assert_eq!(inviter.username, "host_user");
        assert_eq!(inviter.display_name, "Space Host");
    }

    #[test]
    fn channel_membership_and_invitation_defaults_decode_safely() {
        let detail: SpaceDetail = serde_json::from_value(serde_json::json!({
            "space": {"id":"s", "name":"Studio", "ownerId":"owner"},
            "channels": [{"id":"c", "spaceId":"s", "name":"general", "private":false}],
            "members": []
        }))
        .unwrap();
        assert!(detail.channels[0].joined);
        assert!(detail.channel_invitations.is_empty());
        let members: Members = serde_json::from_value(serde_json::json!({"members":[]})).unwrap();
        assert!(members.invitations.is_empty());
    }

    #[test]
    fn avatar_id_is_optional_json() {
        let old: Account = serde_json::from_str(r#"{"id":"old"}"#).unwrap();
        let saved: Account = serde_json::from_str(r#"{"id":"saved","avatarId":16}"#).unwrap();
        assert_eq!(old.avatar_id, None);
        assert_eq!(saved.avatar_id, Some(16));
    }

    fn message(id: &str, seq: u64) -> Message {
        Message {
            id: id.into(),
            channel_id: "channel".into(),
            seq: seq.to_string(),
            created_at: "2026-01-01T00:00:00Z".into(),
            client_message_id: format!("client-{id}"),
            author: Author {
                id: "author".into(),
                avatar_id: None,
                name: "A".into(),
                is_guest: false,
            },
            content: Content {
                version: 1,
                kind: "text".into(),
                text: id.into(),
                attachments: Vec::new(),
            },
            reactions: Vec::new(),
            reaction_seq: None,
            attachments_seq: None,
        }
    }

    #[test]
    fn deduplicates_and_closes_asymmetric_gap() {
        let mut timeline = Timeline::default();
        timeline.reset(vec![message("one", 1)], "1").unwrap();
        assert_eq!(
            timeline.apply(message("three", 3)).unwrap(),
            Apply::Buffered
        );
        assert_eq!(timeline.apply(message("two", 2)).unwrap(), Apply::Applied);
        assert_eq!(
            timeline.apply(message("three-copy", 3)).unwrap(),
            Apply::Duplicate
        );
        assert_eq!(timeline.cursor(), "3");
        assert_eq!(
            timeline
                .messages()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["one", "two", "three"]
        );
    }

    #[test]
    fn rejects_noncanonical_cursor() {
        assert!(sequence("01").is_err());
        assert!(sequence("-1").is_err());
        assert!(sequence("+1").is_err());
        assert!(sequence("١").is_err());
        assert!(sequence("9223372036854775808").is_err());
        assert_eq!(sequence("9223372036854775807"), Ok(i64::MAX as u64));
        assert_eq!(sequence("0"), Ok(0));
    }

    #[test]
    fn unsupported_content_does_not_advance_cursor_or_render() {
        let mut timeline = Timeline::default();
        let mut invalid = message("bad", 1);
        invalid.content.version = 2;
        assert!(timeline.apply(invalid.clone()).is_err());
        invalid.content.version = 1;
        invalid.content.kind = "attachment".into();
        assert!(timeline.merge_sent(invalid).is_err());
        assert_eq!(timeline.cursor(), "0");
        assert_eq!(timeline.messages().count(), 0);
    }

    fn wire_message(attachments: serde_json::Value) -> serde_json::Value {
        serde_json::json!({
            "id": "m1", "channelId": "channel", "seq": "1", "createdAt": "2026-10-03T00:00:00Z",
            "clientMessageId": "client", "author": {"id": "u", "name": "U", "isGuest": false},
            "content": {"version": 1, "type": "text", "text": "", "attachments": attachments}
        })
    }

    #[test]
    fn attachments_parse_tolerantly_without_rejecting_messages() {
        let message: Message = serde_json::from_value(wire_message(serde_json::json!([
            {"id": "img", "kind": "image", "contentType": "image/png", "name": "a.png", "size": 1200,
             "width": 640, "height": 480, "preview": {}, "url": "https://cdn.caper.chat/original/img?exp=9&sig=s",
             "previewUrl": "https://cdn.caper.chat/preview/img?exp=9&sig=p", "futureField": 1},
            {"id": "gone", "kind": "file", "contentType": "application/pdf", "name": "a.pdf", "size": 9,
             "unavailable": true},
            {"id": "video", "kind": "video", "contentType": "video/mp4", "name": "v.mp4", "size": 2.0,
             "durationMs": 1500.4},
            {"id": "bad-kind", "kind": "hologram", "contentType": "x/y", "name": "x", "size": 1},
            {"id": "bad-url", "kind": "file", "contentType": "x/y", "name": "x", "size": 1, "url": "javascript:alert(1)"},
            {"id": "bad-width", "kind": "image", "contentType": "image/png", "name": "x", "size": 1, "width": -4},
            {"kind": "file", "contentType": "x/y", "name": "missing id", "size": 1},
            {"id": "no-size", "kind": "file", "contentType": "x/y", "name": "x"},
            "not an object",
            null
        ])))
        .unwrap();
        message.validate().unwrap();
        let ids: Vec<_> = message
            .content
            .attachments
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        assert_eq!(ids, ["img", "gone", "video"]);
        let image = &message.content.attachments[0];
        assert_eq!(image.kind, AttachmentKind::Image);
        assert_eq!((image.width, image.height), (Some(640), Some(480)));
        assert!(image.preview);
        assert!(image.preview_url.is_some());
        assert!(message.content.attachments[1].unavailable);
        assert_eq!(message.content.attachments[2].duration_ms, Some(1500));

        // A non-array field is no attachments, never a failed message.
        for value in [
            serde_json::json!({"id": "x"}),
            serde_json::json!("text"),
            serde_json::Value::Null,
        ] {
            let message: Message = serde_json::from_value(wire_message(value)).unwrap();
            assert!(message.content.attachments.is_empty());
        }
        let mut legacy = wire_message(serde_json::Value::Null);
        legacy["content"]
            .as_object_mut()
            .unwrap()
            .remove("attachments");
        let message: Message = serde_json::from_value(legacy).unwrap();
        assert!(message.content.attachments.is_empty());
    }

    #[test]
    fn attachments_round_trip_through_serialization() {
        let message: Message = serde_json::from_value(wire_message(serde_json::json!([
            {"id": "img", "kind": "image", "contentType": "image/png", "name": "a.png", "size": 3,
             "preview": {}, "url": "https://cdn.caper.chat/original/img?exp=9&sig=s"}
        ])))
        .unwrap();
        let encoded = serde_json::to_value(&message).unwrap();
        assert_eq!(
            encoded["content"]["attachments"][0],
            serde_json::json!({"id": "img", "kind": "image", "contentType": "image/png", "name": "a.png",
                "size": 3, "preview": {}, "url": "https://cdn.caper.chat/original/img?exp=9&sig=s"})
        );
        let decoded: Message = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, message);
        let plain = serde_json::to_value(message_without_attachments()).unwrap();
        assert!(plain["content"].get("attachments").is_none());
    }

    fn message_without_attachments() -> Message {
        message("plain", 1)
    }

    #[test]
    fn http_confirmation_does_not_advance_replay_cursor() {
        let mut timeline = Timeline::default();
        timeline.reset(vec![message("ten", 10)], "10").unwrap();
        timeline.merge_sent(message("twelve", 12)).unwrap();
        assert_eq!(
            timeline.cursor(),
            "10",
            "only ordered gateway replay advances the cursor"
        );
        assert_eq!(
            timeline.apply(message("eleven", 11)).unwrap(),
            Apply::Applied
        );
        assert_eq!(
            timeline.apply(message("twelve", 12)).unwrap(),
            Apply::Applied
        );
        assert_eq!(timeline.cursor(), "12");
    }

    #[test]
    fn reaction_sequences_bridge_messages_and_reject_gaps() {
        let mut timeline = Timeline::default();
        assert_eq!(timeline.apply(message("one", 1)).unwrap(), Apply::Applied);
        assert_eq!(timeline.apply_sequence("2").unwrap(), Apply::Applied);
        assert_eq!(timeline.apply_sequence("2").unwrap(), Apply::Duplicate);
        assert_eq!(timeline.apply(message("three", 3)).unwrap(), Apply::Applied);
        assert_eq!(timeline.apply_sequence("5").unwrap(), Apply::Resync);
        assert_eq!(timeline.cursor(), "3");
        assert_eq!(
            timeline.messages().count(),
            2,
            "reaction does not manufacture a message"
        );
    }

    #[test]
    fn reaction_updates_are_revisioned_and_http_ack_does_not_move_cursor() {
        let mut timeline = Timeline::default();
        timeline.apply(message("one", 1)).unwrap();
        let update = |seq: &str, authors: &[&str]| ReactionUpdate {
            kind: "message.reactions".into(),
            schema_version: 1,
            channel_id: "channel".into(),
            seq: seq.into(),
            message_id: "one".into(),
            reactions: vec![Reaction {
                emoji: "👍".into(),
                author_ids: authors.iter().map(|id| (*id).into()).collect(),
            }],
        };
        timeline.merge_reaction_ack(update("3", &["me"])).unwrap();
        assert_eq!(timeline.cursor(), "1");
        timeline
            .merge_reaction_ack(update("2", &["other"]))
            .unwrap();
        let reaction = &timeline.messages().next().unwrap().reactions[0];
        assert_eq!(reaction.author_ids, ["me"]);
        assert_eq!(timeline.apply_sequence("2"), Ok(Apply::Applied));
        assert_eq!(
            timeline.apply_reactions(update("3", &["me"])),
            Ok(Apply::Applied)
        );
        assert_eq!(timeline.cursor(), "3");
    }

    #[test]
    fn stale_history_cannot_erase_newer_reactions() {
        let mut timeline = Timeline::default();
        let mut current = message("one", 1);
        current.reaction_seq = Some("8".into());
        current.reactions = vec![Reaction {
            emoji: "❤️".into(),
            author_ids: vec!["me".into()],
        }];
        timeline.reset(vec![current], "8").unwrap();
        timeline.prepend(vec![message("one", 1)]).unwrap();
        assert_eq!(
            timeline.messages().next().unwrap().reaction_seq.as_deref(),
            Some("8")
        );
        assert_eq!(timeline.messages().next().unwrap().reactions.len(), 1);
    }

    fn reaction_update(message_id: &str, seq: u64, author: &str) -> ReactionUpdate {
        ReactionUpdate {
            kind: "message.reactions".into(),
            schema_version: 1,
            channel_id: "channel".into(),
            seq: seq.to_string(),
            message_id: message_id.into(),
            reactions: vec![Reaction {
                emoji: "👍".into(),
                author_ids: vec![author.into()],
            }],
        }
    }

    #[test]
    fn reaction_arriving_before_older_page_is_overlaid() {
        let mut timeline = Timeline::default();
        timeline.reset(vec![message("newer", 10)], "10").unwrap();
        assert_eq!(
            timeline
                .apply_reactions(reaction_update("older", 11, "live"))
                .unwrap(),
            Apply::Applied
        );
        timeline.prepend(vec![message("older", 1)]).unwrap();
        let older = timeline
            .messages()
            .find(|message| message.id == "older")
            .unwrap();
        assert_eq!(older.reaction_seq.as_deref(), Some("11"));
        assert_eq!(older.reactions[0].author_ids, ["live"]);
    }

    #[test]
    fn stale_older_page_does_not_erase_unseen_reaction() {
        let mut timeline = Timeline::default();
        timeline.reset(vec![message("newer", 10)], "10").unwrap();
        timeline
            .merge_reaction_ack(reaction_update("older", 15, "live"))
            .unwrap();
        let mut stale = message("older", 1);
        stale.reaction_seq = Some("12".into());
        stale.reactions = vec![Reaction {
            emoji: "👍".into(),
            author_ids: vec!["stale".into()],
        }];
        timeline.prepend(vec![stale]).unwrap();
        let older = timeline
            .messages()
            .find(|message| message.id == "older")
            .unwrap();
        assert_eq!(older.reaction_seq.as_deref(), Some("15"));
        assert_eq!(older.reactions[0].author_ids, ["live"]);
    }

    #[test]
    fn too_many_unseen_reactions_trigger_resync_and_clear_cache() {
        let mut timeline = Timeline::default();
        for seq in 1..=256 {
            assert_eq!(
                timeline
                    .apply_reactions(reaction_update(&format!("message-{seq}"), seq, "author"))
                    .unwrap(),
                Apply::Applied
            );
        }
        assert_eq!(
            timeline
                .apply_reactions(reaction_update("overflow", 257, "author"))
                .unwrap(),
            Apply::Resync
        );
        timeline.prepend(vec![message("message-1", 1)]).unwrap();
        assert!(timeline.messages().next().unwrap().reactions.is_empty());
    }

    #[test]
    fn direct_conversation_contract_uses_string_sequences_and_camel_case_peer() {
        let payload = r#"{"conversations":[{"id":"dm0000000001","peer":{"id":"peer","username":"fixture_alex","displayName":"TEST FIXTURE Alex"},"lastSeq":"12","readSeq":"9"}]}"#;
        let parsed: DirectConversations = serde_json::from_str(payload).unwrap();
        let direct = &parsed.conversations[0];
        assert_eq!(direct.id, "dm0000000001");
        assert_eq!(direct.peer.display_name, "TEST FIXTURE Alex");
        assert_eq!(sequence(&direct.last_seq), Ok(12));
        assert_eq!(sequence(&direct.read_seq), Ok(9));
    }

    fn file(id: &str, status: &str) -> serde_json::Value {
        serde_json::json!({"id": id, "kind": "image", "contentType": "image/avif", "name": "a.avif",
            "size": 5, "status": status})
    }

    fn attachment_update(message_id: &str, seq: u64, status: &str) -> AttachmentUpdate {
        serde_json::from_value(serde_json::json!({
            "type": "message.attachments", "schemaVersion": 1, "channelId": "channel",
            "seq": seq.to_string(), "messageId": message_id,
            "attachments": [file("img", status), {"bad": true}]
        }))
        .unwrap()
    }

    fn with_files(id: &str, seq: u64, status: &str, attachments_seq: Option<&str>) -> Message {
        let mut message = message(id, seq);
        message.content.attachments = vec![Attachment::parse(&file("img", status)).unwrap()];
        message.attachments_seq = attachments_seq.map(str::to_owned);
        message
    }

    fn status_of(timeline: &Timeline, id: &str) -> AttachmentStatus {
        timeline
            .messages()
            .find(|message| message.id == id)
            .unwrap()
            .content
            .attachments[0]
            .status
    }

    #[test]
    fn attachment_status_and_animation_parse_with_ready_default() {
        let parse = |value| Attachment::parse(&value).unwrap();
        assert_eq!(
            parse(file("a", "processing")).status,
            AttachmentStatus::Processing
        );
        assert_eq!(parse(file("a", "failed")).status, AttachmentStatus::Failed);
        assert_eq!(parse(file("a", "ready")).status, AttachmentStatus::Ready);
        assert_eq!(parse(file("a", "future")).status, AttachmentStatus::Ready);
        let mut legacy = file("a", "");
        legacy.as_object_mut().unwrap().remove("status");
        let legacy = parse(legacy);
        assert_eq!(legacy.status, AttachmentStatus::Ready);
        assert!(!legacy.animated);
        let mut gif = file("a", "ready");
        gif["animated"] = serde_json::json!(true);
        assert!(parse(gif).animated);
        let encoded = serde_json::to_value(parse(file("a", "processing"))).unwrap();
        assert_eq!(encoded["status"], "processing");
        assert!(
            serde_json::to_value(&legacy)
                .unwrap()
                .get("status")
                .is_none()
        );
    }

    #[test]
    fn attachment_updates_are_sequenced_like_reactions() {
        let mut timeline = Timeline::default();
        timeline
            .reset(vec![with_files("one", 1, "processing", None)], "1")
            .unwrap();
        let update = attachment_update("one", 2, "ready");
        assert_eq!(update.attachments.len(), 1, "malformed entries are skipped");
        assert_eq!(timeline.apply_attachments(update), Ok(Apply::Applied));
        assert_eq!(timeline.cursor(), "2");
        assert_eq!(status_of(&timeline, "one"), AttachmentStatus::Ready);
        // A replayed older update is a duplicate and changes nothing.
        assert_eq!(
            timeline.apply_attachments(attachment_update("one", 2, "processing")),
            Ok(Apply::Duplicate)
        );
        assert_eq!(status_of(&timeline, "one"), AttachmentStatus::Ready);
        assert_eq!(
            timeline.apply_attachments(attachment_update("one", 9, "failed")),
            Ok(Apply::Resync)
        );
        let mut wrong = attachment_update("one", 3, "ready");
        wrong.kind = "message.reactions".into();
        assert!(timeline.apply_attachments(wrong).is_err());
    }

    #[test]
    fn stale_snapshots_and_replays_never_regress_ready_files() {
        let mut timeline = Timeline::default();
        timeline
            .reset(vec![with_files("one", 1, "ready", Some("7"))], "7")
            .unwrap();
        // An older history page or replayed message.created stays ready.
        timeline
            .prepend(vec![with_files("one", 1, "processing", None)])
            .unwrap();
        assert_eq!(status_of(&timeline, "one"), AttachmentStatus::Ready);
        timeline
            .prepend(vec![with_files("one", 1, "processing", Some("5"))])
            .unwrap();
        assert_eq!(status_of(&timeline, "one"), AttachmentStatus::Ready);
        // A newer snapshot wins.
        timeline
            .prepend(vec![with_files("one", 1, "failed", Some("8"))])
            .unwrap();
        assert_eq!(status_of(&timeline, "one"), AttachmentStatus::Failed);
    }

    #[test]
    fn attachment_update_before_its_message_is_overlaid() {
        let mut timeline = Timeline::default();
        timeline.reset(vec![message("newer", 10)], "10").unwrap();
        assert_eq!(
            timeline.apply_attachments(attachment_update("older", 11, "ready")),
            Ok(Apply::Applied)
        );
        timeline
            .prepend(vec![with_files("older", 1, "processing", None)])
            .unwrap();
        assert_eq!(status_of(&timeline, "older"), AttachmentStatus::Ready);
        let older = timeline
            .messages()
            .find(|message| message.id == "older")
            .unwrap();
        assert_eq!(older.attachments_seq.as_deref(), Some("11"));
    }
}
