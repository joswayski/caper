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
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reaction {
    pub emoji: String,
    pub author_ids: Vec<String>,
}

/// One person in a "who reacted" list. `id` is the public user ID used in
/// snapshot `authorIds`; either name may be missing.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reactor {
    pub id: String,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub avatar_id: Option<i32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ReactorGroup {
    pub emoji: String,
    /// In reaction order: the first person to react comes first.
    pub authors: Vec<Reactor>,
}

/// `GET /api/chat/channels/{channel}/messages/{message}/reactions`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Reactors {
    pub message_id: String,
    pub reaction_seq: String,
    pub reactions: Vec<ReactorGroup>,
}

/// Who reacted, worded identically on every Caper client: names in reaction
/// order with you first as "You", at most three names before "and N others".
pub fn reactor_summary(
    authors: &[Reactor],
    self_id: Option<&str>,
    emoji_name: Option<&str>,
    emoji: &str,
) -> String {
    let named = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let mut you = false;
    let mut names = Vec::new();
    for author in authors {
        if self_id == Some(author.id.as_str()) {
            you = true;
        } else {
            names.push(
                named(&author.display_name)
                    .or_else(|| named(&author.username))
                    .unwrap_or_else(|| "Someone".into()),
            );
        }
    }
    if you {
        names.insert(0, "You".into());
    }
    let people = match names.as_slice() {
        [] => "No one".to_owned(),
        [only] => only.clone(),
        [first, second] => format!("{first} and {second}"),
        [first, second, third] => format!("{first}, {second} and {third}"),
        [first, second, third, rest @ ..] => format!(
            "{first}, {second}, {third} and {} {}",
            rest.len(),
            if rest.len() == 1 { "other" } else { "others" }
        ),
    };
    format!("{people}{}", reacted_with(emoji_name, emoji))
}

/// The summary from the message snapshot alone, before names load or when
/// loading them fails.
pub fn reactor_fallback(
    author_ids: &[String],
    self_id: Option<&str>,
    emoji_name: Option<&str>,
    emoji: &str,
) -> String {
    let people = match author_ids {
        [only] if self_id == Some(only.as_str()) => "You".to_owned(),
        [_] => "1 person".to_owned(),
        _ => format!("{} people", author_ids.len()),
    };
    format!("{people}{}", reacted_with(emoji_name, emoji))
}

fn reacted_with(emoji_name: Option<&str>, emoji: &str) -> String {
    match emoji_name {
        Some(name) => format!(" reacted with :{name}:"),
        None => format!(" reacted with {emoji}"),
    }
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

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Pin {
    pub author: Author,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PinUpdate {
    #[serde(rename = "type")]
    pub kind: String,
    pub schema_version: u8,
    pub channel_id: String,
    pub seq: String,
    pub message: Message,
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
    #[serde(default)]
    pub pin: Option<Pin>,
    #[serde(default)]
    pub pin_seq: Option<String>,
}

impl Message {
    pub fn validate(&self) -> Result<(), String> {
        sequence(&self.seq)?;
        if self.content.version != 1 || self.content.kind != "text" {
            return Err("unsupported message content".into());
        }
        if let Some(revision) = &self.reaction_seq {
            sequence(revision)?;
        }
        if let Some(revision) = &self.pin_seq {
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
    #[serde(default)]
    pub pinned_messages: Vec<Message>,
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
    pin_snapshot_cursor: u64,
    messages: BTreeMap<u64, Message>,
    ids: BTreeSet<String>,
    buffered: BTreeMap<u64, Message>,
    unseen_reactions: BTreeMap<String, ReactionUpdate>,
    unseen_pins: BTreeMap<String, Message>,
    pinned: BTreeMap<u64, Message>,
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
        self.pin_snapshot_cursor = 0;
        self.messages.clear();
        self.ids.clear();
        self.buffered.clear();
        self.unseen_reactions.clear();
        self.unseen_pins.clear();
        self.pinned.clear();
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

    /// Merge an HTTP acknowledgement without moving the gateway replay cursor.
    pub fn merge_reaction_ack(&mut self, update: ReactionUpdate) -> Result<(), String> {
        if self.merge_reactions(&update)? {
            Ok(())
        } else {
            self.unseen_reactions.clear();
            Err("too many reactions for unloaded messages".into())
        }
    }

    pub fn reset_pins(&mut self, messages: Vec<Message>) -> Result<(), String> {
        self.pin_snapshot_cursor = self.cursor;
        self.pinned.clear();
        for message in messages {
            self.merge_pin_message(message)?;
        }
        // HTTP confirmations newer than the captured history survive refresh.
        let retained: Vec<_> = self.messages.values().cloned().collect();
        for message in &retained {
            if message
                .pin_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .is_some_and(|seq| seq > self.pin_snapshot_cursor)
            {
                self.merge_pin_message(message.clone())?;
            }
        }
        for message in retained {
            self.merge(message)?;
        }
        Ok(())
    }

    pub fn apply_pin(&mut self, update: PinUpdate) -> Result<Apply, String> {
        self.merge_pin_update(update.clone())?;
        self.apply_sequence(&update.seq)
    }

    /// Merge an HTTP acknowledgement without moving the gateway replay cursor.
    pub fn merge_pin_ack(&mut self, update: PinUpdate) -> Result<(), String> {
        self.merge_pin_update(update)
    }

    fn merge_pin_update(&mut self, update: PinUpdate) -> Result<(), String> {
        if update.kind != "message.pin"
            || update.schema_version != 1
            || update.channel_id != update.message.channel_id
            || update.message.pin_seq.as_deref() != Some(update.seq.as_str())
        {
            return Err("invalid pin update".into());
        }
        update.message.validate()?;
        let revision = sequence(&update.seq)?;
        if revision <= self.pin_snapshot_cursor {
            return Ok(());
        }
        let mut accepted = false;
        if let Some(existing) = self
            .messages
            .values_mut()
            .find(|item| item.id == update.message.id)
        {
            let current = existing
                .pin_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .unwrap_or(0);
            if revision > current {
                existing.pin.clone_from(&update.message.pin);
                existing.pin_seq.clone_from(&update.message.pin_seq);
                accepted = true;
            }
        } else {
            let replace = self
                .unseen_pins
                .get(&update.message.id)
                .and_then(|message| message.pin_seq.as_deref())
                .map(sequence)
                .transpose()?
                .is_none_or(|current| revision > current);
            if replace {
                self.unseen_pins
                    .insert(update.message.id.clone(), update.message.clone());
                accepted = true;
            }
        }
        if accepted {
            self.merge_pin_message(update.message)?;
        }
        Ok(())
    }

    fn merge_pin_message(&mut self, message: Message) -> Result<(), String> {
        message.validate()?;
        self.pinned.retain(|_, item| item.id != message.id);
        if message.pin.is_some() {
            let revision = message
                .pin_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .unwrap_or(0);
            self.pinned.insert(revision, message);
        }
        Ok(())
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

    pub fn pinned_messages(&self) -> impl DoubleEndedIterator<Item = &Message> {
        self.pinned.values().rev()
    }

    fn merge(&mut self, mut message: Message) -> Result<(), String> {
        message.validate()?;
        if self.pin_snapshot_cursor > 0
            && message
                .pin_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .unwrap_or(0)
                <= self.pin_snapshot_cursor
        {
            if let Some(snapshot) = self.pinned.values().find(|pin| pin.id == message.id) {
                message.pin.clone_from(&snapshot.pin);
                message.pin_seq.clone_from(&snapshot.pin_seq);
            } else {
                message.pin = None;
                message.pin_seq = Some(self.pin_snapshot_cursor.to_string());
            }
        }
        if let Some(update) = self.unseen_pins.remove(&message.id) {
            let incoming = update
                .pin_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .unwrap_or(0);
            let current = message
                .pin_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .unwrap_or(0);
            if incoming > current {
                message.pin = update.pin;
                message.pin_seq = update.pin_seq;
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
            let old_pin = existing
                .pin_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .unwrap_or(0);
            let new_pin = message
                .pin_seq
                .as_deref()
                .map(sequence)
                .transpose()?
                .unwrap_or(0);
            if old_pin > new_pin {
                message.pin = existing.pin.take();
                message.pin_seq = existing.pin_seq.take();
            }
            *existing = message;
        } else if !self.messages.contains_key(&seq) && self.ids.insert(message.id.clone()) {
            self.messages.insert(seq, message);
        }
        Ok(())
    }
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
            },
            reactions: Vec::new(),
            reaction_seq: None,
            pin: None,
            pin_seq: None,
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

    fn pin_update(mut message: Message, seq: u64, active: bool) -> PinUpdate {
        message.pin = active.then(|| Pin {
            author: message.author.clone(),
            created_at: "2026-01-02T00:00:00Z".into(),
        });
        message.pin_seq = Some(seq.to_string());
        PinUpdate {
            kind: "message.pin".into(),
            schema_version: 1,
            channel_id: "channel".into(),
            seq: seq.to_string(),
            message,
        }
    }

    #[test]
    fn old_pins_stay_separate_and_gateway_sequence_advances() {
        let mut timeline = Timeline::default();
        timeline.reset(vec![message("new", 10)], "10").unwrap();
        let update = pin_update(message("old", 1), 11, true);
        assert_eq!(timeline.apply_pin(update), Ok(Apply::Applied));
        assert_eq!(timeline.cursor(), "11");
        assert_eq!(
            timeline.messages().count(),
            1,
            "an old pin does not enter paginated history"
        );
        assert_eq!(timeline.pinned_messages().next().unwrap().id, "old");
    }

    #[test]
    fn stale_history_cannot_resurrect_an_unpin() {
        let mut timeline = Timeline::default();
        timeline.reset(vec![message("new", 10)], "10").unwrap();
        timeline
            .merge_pin_ack(pin_update(message("old", 1), 15, false))
            .unwrap();
        let mut stale = message("old", 1);
        stale.pin = Some(Pin {
            author: stale.author.clone(),
            created_at: stale.created_at.clone(),
        });
        stale.pin_seq = Some("12".into());
        timeline.prepend(vec![stale]).unwrap();
        let old = timeline
            .messages()
            .find(|message| message.id == "old")
            .unwrap();
        assert!(old.pin.is_none());
        assert_eq!(old.pin_seq.as_deref(), Some("15"));
        assert_eq!(timeline.pinned_messages().count(), 0);
    }

    #[test]
    fn pin_http_ack_does_not_advance_gateway_cursor() {
        let mut timeline = Timeline::default();
        timeline.reset(vec![message("one", 1)], "1").unwrap();
        timeline
            .merge_pin_ack(pin_update(message("one", 1), 3, true))
            .unwrap();
        assert_eq!(timeline.cursor(), "1");
        assert_eq!(timeline.apply_sequence("2"), Ok(Apply::Applied));
        assert_eq!(
            timeline.apply_pin(pin_update(message("one", 1), 3, true)),
            Ok(Apply::Applied)
        );
        assert_eq!(timeline.cursor(), "3");
    }

    #[test]
    fn complete_pin_snapshot_rejects_old_ack_and_stale_page() {
        let mut timeline = Timeline::default();
        let stale = pin_update(message("old", 1), 4, true);
        timeline.reset(vec![message("new", 50)], "60").unwrap();
        timeline.reset_pins(vec![]).unwrap();
        timeline.merge_pin_ack(stale.clone()).unwrap();
        assert_eq!(timeline.pinned_messages().count(), 0);
        timeline.prepend(vec![stale.message]).unwrap();
        assert!(
            timeline
                .messages()
                .find(|message| message.id == "old")
                .unwrap()
                .pin
                .is_none()
        );
        assert_eq!(
            timeline.apply_pin(pin_update(message("old", 1), 61, true)),
            Ok(Apply::Applied)
        );
        assert!(
            timeline
                .messages()
                .find(|message| message.id == "old")
                .unwrap()
                .pin
                .is_some()
        );
        assert_eq!(timeline.pinned_messages().count(), 1);
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

    fn reactor(id: &str, display_name: Option<&str>, username: Option<&str>) -> Reactor {
        Reactor {
            id: id.into(),
            username: username.map(str::to_owned),
            display_name: display_name.map(str::to_owned),
            avatar_id: None,
        }
    }

    #[test]
    fn reactor_summary_matches_shared_wording() {
        let me = reactor("me", Some("Me Myself"), Some("me"));
        let alice = reactor("alice", Some("Alice A"), Some("alice"));
        let bob = reactor("bob", Some("Bob B"), Some("bob"));
        let carol = reactor("carol", Some("Carol C"), Some("carol"));
        let dave = reactor("dave", Some("Dave D"), Some("dave"));
        let erin = reactor("erin", Some("Erin E"), Some("erin"));
        let summary = |authors: &[Reactor], name: Option<&str>, emoji: &str| {
            reactor_summary(authors, Some("me"), name, emoji)
        };
        assert_eq!(
            summary(std::slice::from_ref(&me), Some("thumbs-up"), "👍"),
            "You reacted with :thumbs-up:"
        );
        assert_eq!(
            summary(&[bob.clone(), me.clone()], Some("thumbs-up"), "👍"),
            "You and Bob B reacted with :thumbs-up:",
            "you move to the front"
        );
        assert_eq!(
            summary(
                &[alice.clone(), bob.clone(), carol.clone()],
                Some("party-popper"),
                "🎉"
            ),
            "Alice A, Bob B and Carol C reacted with :party-popper:"
        );
        assert_eq!(
            summary(
                &[
                    alice.clone(),
                    bob.clone(),
                    me.clone(),
                    carol.clone(),
                    dave.clone()
                ],
                Some("thumbs-up"),
                "👍"
            ),
            "You, Alice A, Bob B and 2 others reacted with :thumbs-up:"
        );
        assert_eq!(
            summary(
                &[alice.clone(), bob.clone(), carol.clone(), dave],
                Some("thumbs-up"),
                "👍"
            ),
            "Alice A, Bob B, Carol C and 1 other reacted with :thumbs-up:"
        );
        assert_eq!(
            summary(&[alice.clone(), erin], None, "🫨"),
            "Alice A and Erin E reacted with 🫨",
            "unknown emoji names fall back to the glyph"
        );
        assert_eq!(
            reactor_summary(&[me, alice], None, Some("red-heart"), "❤️"),
            "Me Myself and Alice A reacted with :red-heart:",
            "signed-out readers see every name"
        );
    }

    #[test]
    fn reactor_names_fall_back_to_username_then_someone() {
        let authors = [
            reactor("one", None, Some("only_username")),
            reactor("two", Some("  "), None),
            reactor("three", None, None),
        ];
        assert_eq!(
            reactor_summary(&authors, None, Some("looking"), "👀"),
            "only_username, Someone and Someone reacted with :looking:"
        );
    }

    #[test]
    fn reactor_fallback_counts_people_from_the_snapshot() {
        let ids = |ids: &[&str]| ids.iter().map(|id| (*id).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            reactor_fallback(&ids(&["me"]), Some("me"), Some("thumbs-up"), "👍"),
            "You reacted with :thumbs-up:"
        );
        assert_eq!(
            reactor_fallback(&ids(&["bob"]), Some("me"), Some("thumbs-up"), "👍"),
            "1 person reacted with :thumbs-up:"
        );
        assert_eq!(
            reactor_fallback(&ids(&["me", "a", "b"]), Some("me"), None, "🫨"),
            "3 people reacted with 🫨"
        );
        assert_eq!(
            reactor_fallback(&ids(&["me"]), None, Some("thumbs-up"), "👍"),
            "1 person reacted with :thumbs-up:"
        );
    }

    #[test]
    fn reactor_list_contract_decodes_server_json() {
        let payload = r#"{
            "messageId": "abc123",
            "reactionSeq": "12",
            "reactions": [
                {"emoji": "👍", "authors": [
                    {"id": "bob", "username": "bob", "displayName": "Bob B", "avatarId": 101},
                    {"id": "alice", "username": null, "displayName": null, "avatarId": 100}
                ]},
                {"emoji": "🎉", "authors": [{"id": "alice"}]}
            ]
        }"#;
        let list: Reactors = serde_json::from_str(payload).unwrap();
        assert_eq!(list.message_id, "abc123");
        assert_eq!(sequence(&list.reaction_seq), Ok(12));
        assert_eq!(list.reactions[0].emoji, "👍");
        assert_eq!(
            list.reactions[0].authors[0],
            reactor("bob", Some("Bob B"), Some("bob")).with_avatar(101)
        );
        assert_eq!(
            list.reactions[0].authors[1],
            reactor("alice", None, None).with_avatar(100),
            "null names stay missing"
        );
        assert_eq!(list.reactions[1].authors, [reactor("alice", None, None)]);
        assert!(
            serde_json::from_str::<Reactors>(r#"{"messageId":"m","reactions":[]}"#).is_err(),
            "a list without its revision cannot be cached"
        );
    }

    impl Reactor {
        fn with_avatar(mut self, avatar_id: i32) -> Self {
            self.avatar_id = Some(avatar_id);
            self
        }
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
}
