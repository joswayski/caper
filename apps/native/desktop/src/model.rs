#![allow(dead_code)] // Complete API contracts retain fields not yet rendered by this client.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
    #[serde(default)]
    pub debug_enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: String,
    pub name: String,
    pub owner_id: String,
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
}

/// Spectators receive identity and status, never media track capabilities.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceOccupant {
    pub id: String,
    pub name: String,
    pub country_code: Option<String>,
    pub muted: bool,
    pub deafened: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub owner: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Spaces {
    pub spaces: Vec<Space>,
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
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Author {
    pub id: String,
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
        if self.reactions.iter().any(|reaction| {
            reaction.emoji.is_empty() || reaction.author_ids.iter().any(String::is_empty)
        }) {
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
}

#[derive(Default)]
pub struct Timeline {
    cursor: u64,
    messages: BTreeMap<u64, Message>,
    ids: BTreeSet<String>,
    buffered: BTreeMap<u64, Message>,
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
        self.merge_reactions(&update)?;
        self.apply_sequence(&update.seq)
    }

    /// Merge an HTTP acknowledgement without moving the gateway replay cursor.
    pub fn merge_reaction_ack(&mut self, update: ReactionUpdate) -> Result<(), String> {
        self.merge_reactions(&update)
    }

    fn merge_reactions(&mut self, update: &ReactionUpdate) -> Result<(), String> {
        let seq = sequence(&update.seq)?;
        if update.kind != "message.reactions" || update.schema_version != 1 {
            return Err("unsupported reaction update".into());
        }
        if let Some(message) = self
            .messages
            .values_mut()
            .find(|item| item.id == update.message_id)
        {
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
        }
        Ok(())
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
            *existing = message;
        } else if !self.messages.contains_key(&seq) && self.ids.insert(message.id.clone()) {
            self.messages.insert(seq, message);
        }
        Ok(())
    }
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

    fn message(id: &str, seq: u64) -> Message {
        Message {
            id: id.into(),
            channel_id: "channel".into(),
            seq: seq.to_string(),
            created_at: "2026-01-01T00:00:00Z".into(),
            client_message_id: format!("client-{id}"),
            author: Author {
                id: "author".into(),
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
}
