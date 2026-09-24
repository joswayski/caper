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
}

impl Message {
    pub fn validate(&self) -> Result<(), String> {
        sequence(&self.seq)?;
        if self.content.version != 1 || self.content.kind != "text" {
            return Err("unsupported message content".into());
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

    fn merge(&mut self, message: Message) -> Result<(), String> {
        message.validate()?;
        let seq = sequence(&message.seq)?;
        if !self.messages.contains_key(&seq) && self.ids.insert(message.id.clone()) {
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
}
