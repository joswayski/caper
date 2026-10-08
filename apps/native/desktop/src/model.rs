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
    #[serde(default)]
    pub avatar_id: Option<i32>,
}

/// Whether a DM is a message request. Old servers omit it: `Accepted`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DirectStatus {
    #[default]
    Accepted,
    /// You started it; they have not accepted yet.
    Outgoing,
    /// A request to you from someone you share no space with.
    Incoming,
}

fn direct_status<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<DirectStatus, D::Error> {
    Ok(
        match Option::<String>::deserialize(deserializer)?.as_deref() {
            Some("outgoing") => DirectStatus::Outgoing,
            Some("incoming") => DirectStatus::Incoming,
            _ => DirectStatus::Accepted,
        },
    )
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DirectConversation {
    pub id: String,
    pub peer: DirectPeer,
    pub last_seq: String,
    pub read_seq: String,
    #[serde(default, deserialize_with = "direct_status")]
    pub status: DirectStatus,
    /// You blocked the peer (they are never told). Old servers omit it.
    #[serde(default)]
    pub blocked: bool,
}

/// `GET /api/blocks` entry: an account you blocked.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BlockedAccount {
    pub id: String,
    pub username: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar_id: Option<i32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Blocks {
    pub blocks: Vec<BlockedAccount>,
}

/// `GET/PUT /api/account/privacy`: who can start a DM with you, as
/// `anyone`, `spaces` or `nobody`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Privacy {
    pub direct_messages: String,
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

/// A `GET /api/people` entry: someone who shares an active space or a DM with
/// you (never yourself), for DM `@` suggestions.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub id: String,
    pub username: String,
    pub display_name: String,
    #[serde(default)]
    pub avatar_id: Option<i32>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct People {
    pub people: Vec<Person>,
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
    /// Server-resolved `@` mentions in first-appearance order. Older messages
    /// and older servers omit it.
    #[serde(
        default,
        deserialize_with = "mention_entries",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub mentions: Vec<Mention>,
}

/// A `content.mentions` entry: `user` (with `id`/`username`), `everyone` or
/// `here`. Other types are future additions; readers ignore them.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Mention {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

/// Mentions only decorate text, so a missing, `null` or unexpected list, or an
/// entry shaped differently by a newer server, never rejects the message.
fn mention_entries<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Mention>, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Array(entries) => entries
            .into_iter()
            .filter_map(|entry| serde_json::from_value(entry).ok())
            .collect(),
        _ => Vec::new(),
    })
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
pub struct EditUpdate {
    #[serde(rename = "type")]
    pub kind: String,
    pub schema_version: u8,
    pub channel_id: String,
    pub seq: String,
    pub message: Message,
}

fn original_revision() -> u32 {
    1
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
    #[serde(default)]
    pub thread_root_id: Option<String>,
    #[serde(default)]
    pub broadcast: bool,
    #[serde(default)]
    pub thread: Option<ThreadSummary>,
    #[serde(default)]
    pub forward: Option<Box<MessageForward>>,
    #[serde(default)]
    pub forward_seq: Option<String>,
    #[serde(default = "original_revision")]
    pub revision: u32,
    #[serde(default)]
    pub edited_at: Option<String>,
    #[serde(default)]
    pub edit_seq: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct MessageForward {
    pub message: Option<Message>,
    pub seq: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardDestination {
    pub id: String,
    pub name: String,
    pub space_name: String,
    pub direct: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ForwardDestinations {
    pub destinations: Vec<ForwardDestination>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardConversation {
    pub root: Option<Message>,
    pub messages: Vec<Message>,
    pub cursor: String,
    pub has_more: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardUpdate {
    #[serde(rename = "type")]
    pub kind: String,
    pub schema_version: u8,
    pub channel_id: String,
    pub seq: String,
    pub message: Message,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub reply_count: u64,
    pub participants: Vec<Author>,
    pub seq: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadHistory {
    pub root: Message,
    pub messages: Vec<Message>,
    pub cursor: String,
    pub has_more: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageVersion {
    pub revision: u32,
    pub content: Content,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageVersions {
    pub message_id: String,
    pub versions: Vec<MessageVersion>,
    pub has_more: bool,
}

impl MessageVersions {
    pub fn valid(&self, message: &str, before: Option<u32>) -> bool {
        self.message_id == message
            && self.versions.len() <= 50
            && self.versions.iter().enumerate().all(|(index, version)| {
                version.revision > 0
                    && before.is_none_or(|before| version.revision < before)
                    && (index == 0 || version.revision < self.versions[index - 1].revision)
                    && version.content.version == 1
                    && version.content.kind == "text"
                    && crate::edits::valid_text(&version.content.text)
                    && chrono::DateTime::parse_from_rfc3339(&version.created_at).is_ok()
            })
    }
}

impl Message {
    pub fn is_channel_message(&self) -> bool {
        self.thread_root_id.is_none() || self.broadcast
    }

    pub fn validate(&self) -> Result<(), String> {
        sequence(&self.seq)?;
        if let Some(forward) = &self.forward {
            sequence(&forward.seq)?;
            if let Some(original) = &forward.message {
                if original.forward.is_some() {
                    return Err("nested forward reference".into());
                }
                original.validate()?;
            }
        }
        if let Some(revision) = &self.forward_seq {
            sequence(revision)?;
        }
        if self.revision == 0 {
            return Err("invalid content revision".into());
        }
        if self.revision == 1 {
            if self.edited_at.is_some() || self.edit_seq.is_some() {
                return Err("unexpected edit metadata".into());
            }
        } else {
            let edited_at = self.edited_at.as_deref().ok_or("missing edit timestamp")?;
            chrono::DateTime::parse_from_rfc3339(edited_at)
                .map_err(|_| "invalid edit timestamp")?;
            let edit_seq = self.edit_seq.as_deref().ok_or("missing edit sequence")?;
            if sequence(edit_seq)? <= sequence(&self.seq)? {
                return Err("invalid edit sequence".into());
            }
        }
        if self.broadcast && self.thread_root_id.is_none() {
            return Err("invalid broadcast reply".into());
        }
        if let Some(summary) = &self.thread {
            sequence(&summary.seq)?;
            if summary.reply_count == 0
                || summary.participants.len() > 5
                || summary
                    .participants
                    .iter()
                    .map(|person| &person.id)
                    .collect::<BTreeSet<_>>()
                    .len()
                    != summary.participants.len()
            {
                return Err("invalid thread summary".into());
            }
        }
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

    fn merge_edit(&mut self, incoming: &Self) {
        if self.id == incoming.id
            && self.channel_id == incoming.channel_id
            && incoming.revision > self.revision
        {
            self.content.clone_from(&incoming.content);
            self.revision = incoming.revision;
            self.edited_at.clone_from(&incoming.edited_at);
            self.edit_seq.clone_from(&incoming.edit_seq);
        }
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
    forward_updates: BTreeMap<String, Message>,
    pinned: BTreeMap<u64, Message>,
    thread_summaries: BTreeMap<String, ThreadSummary>,
    edits: BTreeMap<String, Message>,
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
        self.forward_updates.clear();
        self.pinned.clear();
        self.thread_summaries.clear();
        self.edits.clear();
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

    pub fn apply_edit(&mut self, update: EditUpdate) -> Result<Apply, String> {
        if update.kind != "message.edited"
            || update.schema_version != 1
            || update.channel_id != update.message.channel_id
            || update.message.edit_seq.as_deref() != Some(update.seq.as_str())
            || update.message.revision <= 1
        {
            return Err("invalid edit update".into());
        }
        if !self.merge_edit_snapshot(update.message)? {
            return Ok(Apply::Resync);
        }
        self.apply_sequence(&update.seq)
    }

    /// HTTP confirmations merge content only, never gateway/read position or unloaded rows.
    pub fn merge_edit_ack(&mut self, message: Message) -> Result<(), String> {
        if self.merge_edit_snapshot(message)? {
            Ok(())
        } else {
            Err("too many edits for unloaded messages".into())
        }
    }

    fn merge_edit_snapshot(&mut self, message: Message) -> Result<bool, String> {
        message.validate()?;
        if !self.ids.contains(&message.id)
            && !self.edits.contains_key(&message.id)
            && self
                .edits
                .keys()
                .filter(|id| !self.ids.contains(*id))
                .count()
                >= 256
        {
            return Ok(false);
        }
        let message = self.remember_edit(message);
        for loaded in self
            .messages
            .values_mut()
            .chain(self.pinned.values_mut())
            .chain(self.unseen_pins.values_mut())
        {
            loaded.merge_edit(&message);
        }
        Ok(true)
    }

    fn remember_edit(&mut self, mut message: Message) -> Message {
        if let Some(snapshot) = self.edits.get(&message.id) {
            message.merge_edit(snapshot);
        }
        if message.revision > 1 {
            self.edits.insert(message.id.clone(), message.clone());
        }
        message
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
        let message = self.remember_edit(message);
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

    pub fn apply_forward(&mut self, update: ForwardUpdate) -> Result<Apply, String> {
        update.message.validate()?;
        if update.kind != "message.forward"
            || update.schema_version != 1
            || update.message.channel_id != update.channel_id
            || update.message.forward.is_none()
            || update.message.forward_seq.as_deref() != Some(&update.seq)
        {
            return Err("invalid forward event".into());
        }
        self.merge_forward(&update.message);
        self.apply_sequence(&update.seq)
    }

    fn merge_forward(&mut self, message: &Message) {
        if message.forward.is_none() {
            return;
        }
        let mut snapshot = message.clone();
        if let Some(previous) = self.forward_updates.get(&message.id) {
            overlay_forward(&mut snapshot, previous);
        }
        if self.forward_updates.len() >= 256 && !self.forward_updates.contains_key(&message.id) {
            self.forward_updates.pop_first();
        }
        self.forward_updates
            .insert(message.id.clone(), snapshot.clone());
        for visible in self
            .messages
            .values_mut()
            .chain(self.pinned.values_mut())
            .filter(|m| m.id == message.id)
        {
            overlay_forward(visible, &snapshot);
        }
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
        self.merge_forward(&message);
        if let Some(snapshot) = self.forward_updates.get(&message.id) {
            overlay_forward(&mut message, snapshot);
        }
        message = self.remember_edit(message);
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
        let root = message
            .thread_root_id
            .as_ref()
            .unwrap_or(&message.id)
            .clone();
        if let Some(summary) = &message.thread {
            let previous = self
                .thread_summaries
                .get(&root)
                .map(|old| sequence(&old.seq))
                .transpose()?
                .unwrap_or(0);
            if sequence(&summary.seq)? > previous {
                self.thread_summaries.insert(root.clone(), summary.clone());
                for loaded in self
                    .messages
                    .values_mut()
                    .filter(|loaded| loaded.id == root)
                {
                    loaded.thread = Some(summary.clone());
                }
            }
        }
        if let Some(summary) = self.thread_summaries.get(&root) {
            message.thread = Some(summary.clone());
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
            overlay_forward(&mut message, existing);
            *existing = message;
        } else if !self.messages.contains_key(&seq) && self.ids.insert(message.id.clone()) {
            self.messages.insert(seq, message);
        }
        Ok(())
    }
}

fn overlay_forward(current: &mut Message, incoming: &Message) {
    let Some(next) = &incoming.forward else {
        return;
    };
    if current.forward.as_ref().is_none_or(|old| {
        next.message.is_none()
            || (old.message.is_some()
                && sequence(&next.seq).unwrap_or(0) >= sequence(&old.seq).unwrap_or(0))
    }) {
        current.forward.clone_from(&incoming.forward);
    }
    if sequence(incoming.forward_seq.as_deref().unwrap_or("0")).unwrap_or(0)
        > sequence(current.forward_seq.as_deref().unwrap_or("0")).unwrap_or(0)
    {
        current.forward_seq.clone_from(&incoming.forward_seq);
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
                mentions: Vec::new(),
            },
            reactions: Vec::new(),
            reaction_seq: None,
            pin: None,
            pin_seq: None,
            thread_root_id: None,
            broadcast: false,
            thread: None,
            forward: None,
            forward_seq: None,
            revision: 1,
            edited_at: None,
            edit_seq: None,
        }
    }

    fn edit_update(mut message: Message, seq: u64, revision: u32) -> EditUpdate {
        message.content.text = "corrected".into();
        message.revision = revision;
        message.edited_at = Some("2026-10-06T00:00:00Z".into());
        message.edit_seq = Some(seq.to_string());
        EditUpdate {
            kind: "message.edited".into(),
            schema_version: 1,
            channel_id: message.channel_id.clone(),
            seq: seq.to_string(),
            message,
        }
    }

    #[test]
    fn http_edit_snapshots_do_not_skip_replay_or_insert_unloaded_rows() {
        let original = message("one", 3);
        let mut timeline = Timeline::default();
        timeline.reset(vec![original.clone()], "10").unwrap();
        let corrected = edit_update(original.clone(), 12, 2).message;
        timeline.merge_edit_ack(corrected).unwrap();
        assert_eq!(timeline.cursor(), "10");
        assert_eq!(
            timeline.messages().next().unwrap().content.text,
            "corrected"
        );
        timeline.merge_edit_ack(original).unwrap();
        assert_eq!(timeline.messages().next().unwrap().revision, 2);
        let unloaded = message("unloaded", 2);
        timeline
            .merge_edit_ack(edit_update(unloaded.clone(), 13, 3).message)
            .unwrap();
        assert_eq!(timeline.messages().count(), 1);
        timeline.prepend(vec![unloaded]).unwrap();
        assert_eq!(timeline.messages().next().unwrap().revision, 3);
        assert_eq!(timeline.cursor(), "10");
        assert_eq!(
            timeline.apply_edit(edit_update(message("earlier", 5), 11, 2)),
            Ok(Apply::Applied)
        );
        assert_eq!(
            timeline.cursor(),
            "11",
            "The earlier event must remain replayable after the HTTP snapshot"
        );
    }

    #[test]
    fn edits_preserve_identity_and_independent_revisions_on_loaded_and_pinned_rows() {
        let mut original = message("one", 3);
        original.reaction_seq = Some("12".into());
        original.reactions = vec![Reaction {
            emoji: "👍".into(),
            author_ids: vec!["other".into()],
        }];
        original.pin_seq = Some("13".into());
        original.pin = Some(Pin {
            author: original.author.clone(),
            created_at: "2026-10-05T00:00:00Z".into(),
        });
        original.thread_root_id = Some("root".into());
        original.broadcast = true;
        original.thread = Some(ThreadSummary {
            reply_count: 4,
            participants: vec![original.author.clone()],
            seq: "11".into(),
        });
        let mut timeline = Timeline::default();
        timeline.reset(vec![original.clone()], "13").unwrap();
        timeline.reset_pins(vec![original.clone()]).unwrap();
        let mut update = edit_update(original.clone(), 14, 2);
        update.message.reactions.clear();
        update.message.reaction_seq = None;
        update.message.pin = None;
        update.message.pin_seq = None;
        update.message.thread = None;
        assert_eq!(timeline.apply_edit(update), Ok(Apply::Applied));
        assert_eq!(timeline.cursor(), "14");
        let current = timeline.messages().next().unwrap();
        assert_eq!(current.content.text, "corrected");
        assert_eq!(current.seq, "3");
        assert_eq!(current.created_at, "2026-01-01T00:00:00Z");
        assert_eq!(current.reaction_seq.as_deref(), Some("12"));
        assert_eq!(current.reactions[0].author_ids, ["other"]);
        assert_eq!(current.pin_seq.as_deref(), Some("13"));
        assert_eq!(current.thread.as_ref().unwrap().seq, "11");
        assert!(current.broadcast);
        assert_eq!(
            timeline.pinned_messages().next().unwrap().content.text,
            "corrected"
        );
        timeline.prepend(vec![original.clone()]).unwrap();
        timeline.reset_pins(vec![original]).unwrap();
        assert_eq!(timeline.messages().next().unwrap().revision, 2);
        assert_eq!(
            timeline.pinned_messages().next().unwrap().content.text,
            "corrected"
        );
        assert_eq!(timeline.cursor(), "14");
    }

    #[test]
    fn unloaded_edits_do_not_insert_ghost_messages_and_overlay_later_thread_pages() {
        let mut timeline = Timeline::default();
        timeline.reset(vec![message("recent", 10)], "10").unwrap();
        let mut older = message("older", 3);
        older.thread_root_id = Some("root".into());
        assert_eq!(
            timeline.apply_edit(edit_update(older.clone(), 11, 2)),
            Ok(Apply::Applied)
        );
        assert_eq!(timeline.messages().count(), 1);
        timeline.prepend(vec![older.clone()]).unwrap();
        let loaded = timeline.messages().next().unwrap();
        assert_eq!(loaded.content.text, "corrected");
        assert_eq!(loaded.seq, "3");
        assert_eq!(loaded.thread_root_id.as_deref(), Some("root"));
        assert_eq!(timeline.cursor(), "11");
        assert_eq!(
            timeline.apply_edit(edit_update(older.clone(), 11, 2)),
            Ok(Apply::Duplicate)
        );
        assert_eq!(
            timeline.apply_edit(edit_update(older, 13, 3)),
            Ok(Apply::Resync)
        );
    }

    #[test]
    fn edit_sequence_is_not_creation_sequence_and_old_messages_default_to_revision_one() {
        let mut value = serde_json::to_value(message("one", 3)).unwrap();
        value.as_object_mut().unwrap().remove("revision");
        assert_eq!(
            serde_json::from_value::<Message>(value).unwrap().revision,
            1
        );
        let mut timeline = Timeline::default();
        let mut invalid = edit_update(message("one", 3), 4, 2);
        invalid.seq = "3".into();
        assert!(timeline.apply_edit(invalid).is_err());
        assert!(
            edit_update(message("one", 3), 3, 2)
                .message
                .validate()
                .is_err()
        );
    }

    #[test]
    fn forward_source_and_destination_revisions_merge_independently() {
        let mut wrapper = message("wrapper", 1);
        let mut source = message("original", 89);
        source.channel_id = "private-source".into();
        wrapper.forward = Some(Box::new(MessageForward {
            message: Some(source.clone()),
            seq: "9007199254740995".into(),
        }));
        wrapper.forward_seq = Some("1".into());
        let mut timeline = Timeline::default();
        timeline.reset(vec![wrapper.clone()], "1").unwrap();
        let mut stale = wrapper.clone();
        stale.forward.as_mut().unwrap().seq = "9007199254740993".into();
        stale
            .forward
            .as_mut()
            .unwrap()
            .message
            .as_mut()
            .unwrap()
            .content
            .text = "stale".into();
        stale.forward_seq = Some("2".into());
        assert_eq!(
            timeline
                .apply_forward(ForwardUpdate {
                    kind: "message.forward".into(),
                    schema_version: 1,
                    channel_id: "channel".into(),
                    seq: "2".into(),
                    message: stale
                })
                .unwrap(),
            Apply::Applied
        );
        let visible = timeline.messages().next().unwrap();
        assert_eq!(
            visible
                .forward
                .as_ref()
                .unwrap()
                .message
                .as_ref()
                .unwrap()
                .content
                .text,
            "original"
        );
        assert_eq!(visible.forward_seq.as_deref(), Some("2"));
        assert_eq!(visible.seq, "1");
        assert_eq!(timeline.cursor(), "2");
        wrapper.id = "unloaded".into();
        wrapper.forward_seq = Some("3".into());
        timeline
            .apply_forward(ForwardUpdate {
                kind: "message.forward".into(),
                schema_version: 1,
                channel_id: "channel".into(),
                seq: "3".into(),
                message: wrapper.clone(),
            })
            .unwrap();
        assert_eq!(timeline.messages().count(), 1);
        let mut old_page = wrapper;
        old_page.seq = "0".into();
        old_page.forward.as_mut().unwrap().seq = "89".into();
        old_page
            .forward
            .as_mut()
            .unwrap()
            .message
            .as_mut()
            .unwrap()
            .content
            .text = "old page".into();
        timeline.prepend(vec![old_page]).unwrap();
        assert_eq!(
            timeline
                .messages()
                .next()
                .unwrap()
                .forward
                .as_ref()
                .unwrap()
                .message
                .as_ref()
                .unwrap()
                .content
                .text,
            "original"
        );
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
    fn people_decode_with_optional_avatars() {
        let people: People = serde_json::from_value(serde_json::json!({"people": [
            {"id": "user00000001", "username": "alex", "displayName": "Alex", "avatarId": 16},
            {"id": "user00000002", "username": "maya", "displayName": "Maya", "avatarId": null},
            {"id": "user00000003", "username": "sam", "displayName": "Sam"}
        ]}))
        .unwrap();
        assert_eq!(
            people.people[0],
            Person {
                id: "user00000001".into(),
                username: "alex".into(),
                display_name: "Alex".into(),
                avatar_id: Some(16),
            }
        );
        assert_eq!(people.people[1].avatar_id, None);
        assert_eq!(people.people[2].avatar_id, None);
    }

    #[test]
    fn content_mentions_are_optional_and_tolerate_unknown_entries() {
        let legacy: Content =
            serde_json::from_str(r#"{"version":1,"type":"text","text":"hi"}"#).unwrap();
        assert!(legacy.mentions.is_empty());
        let null: Content =
            serde_json::from_str(r#"{"version":1,"type":"text","text":"hi","mentions":null}"#)
                .unwrap();
        assert!(null.mentions.is_empty());
        let content: Content = serde_json::from_value(serde_json::json!({
            "version": 1, "type": "text", "text": "hey @alice and @everyone",
            "mentions": [
                {"type": "user", "id": "user00000001", "username": "alice"},
                {"type": "everyone"},
                {"type": "role", "id": 7, "color": "red"},
                {"type": "channel", "id": "chan00000001"},
                "malformed"
            ]
        }))
        .unwrap();
        assert_eq!(
            content.mentions,
            [
                Mention {
                    kind: "user".into(),
                    id: Some("user00000001".into()),
                    username: Some("alice".into()),
                },
                Mention {
                    kind: "everyone".into(),
                    id: None,
                    username: None,
                },
                Mention {
                    kind: "channel".into(),
                    id: Some("chan00000001".into()),
                    username: None,
                },
            ]
        );
        let message: Message = serde_json::from_value(serde_json::json!({
            "id": "m", "channelId": "c", "seq": "1", "createdAt": "2026-01-01T00:00:00Z",
            "clientMessageId": "client",
            "author": {"id": "a", "name": "A", "isGuest": false},
            "content": {"version": 1, "type": "text", "text": "@here", "mentions": {"type": "here"}}
        }))
        .unwrap();
        assert!(message.validate().is_ok());
        assert!(message.content.mentions.is_empty());
        assert_eq!(
            serde_json::to_value(&legacy).unwrap(),
            serde_json::json!({"version": 1, "type": "text", "text": "hi"})
        );
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
        assert_eq!(
            direct.status,
            DirectStatus::Accepted,
            "old servers omit status"
        );
        assert!(!direct.blocked, "old servers omit blocked");
        assert_eq!(direct.peer.avatar_id, None);
    }

    #[test]
    fn request_status_blocks_and_privacy_decode() {
        let parsed: DirectConversations = serde_json::from_value(serde_json::json!({
            "conversations": [
                {"id": "a", "peer": {"id": "p1", "username": "jordan", "displayName": "Jordan", "avatarId": 412},
                 "lastSeq": "1", "readSeq": "0", "status": "incoming", "blocked": false},
                {"id": "b", "peer": {"id": "p2", "username": "sam", "displayName": "Sam", "avatarId": null},
                 "lastSeq": "0", "readSeq": "0", "status": "outgoing", "blocked": true},
                {"id": "c", "peer": {"id": "p3", "username": "alex", "displayName": "Alex"},
                 "lastSeq": "0", "readSeq": "0", "status": null},
                {"id": "d", "peer": {"id": "p4", "username": "kim", "displayName": "Kim"},
                 "lastSeq": "0", "readSeq": "0", "status": "archived"}
            ]
        }))
        .unwrap();
        let statuses: Vec<_> = parsed
            .conversations
            .iter()
            .map(|direct| (direct.status, direct.blocked))
            .collect();
        assert_eq!(
            statuses,
            [
                (DirectStatus::Incoming, false),
                (DirectStatus::Outgoing, true),
                (DirectStatus::Accepted, false),
                (DirectStatus::Accepted, false),
            ]
        );
        assert_eq!(parsed.conversations[0].peer.avatar_id, Some(412));
        let blocks: Blocks = serde_json::from_value(serde_json::json!({"blocks": [
            {"id": "member000001", "username": "maya", "displayName": "Maya", "avatarId": null}
        ]}))
        .unwrap();
        assert_eq!(blocks.blocks[0].username, "maya");
        let privacy: Privacy =
            serde_json::from_value(serde_json::json!({"directMessages": "spaces"})).unwrap();
        assert_eq!(privacy.direct_messages, "spaces");
        assert_eq!(
            serde_json::to_value(&privacy).unwrap(),
            serde_json::json!({"directMessages": "spaces"})
        );
    }
}
