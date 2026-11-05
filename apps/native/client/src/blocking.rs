//! Blocked authors' messages collapse into one row per consecutive run, like
//! "⊘ 2 blocked messages — Show". Pure so the grouping is testable.

use crate::model::Message;
use std::collections::BTreeSet;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// Draw `messages[index]` normally.
    Message(usize),
    /// Consecutive messages by blocked accounts, keyed by the first one's id.
    Blocked { range: Range<usize>, key: String },
}

/// Whether `message` is hidden behind a blocked row. Guests and your own
/// messages never collapse.
pub fn hidden(message: &Message, blocked: &BTreeSet<String>, me: Option<&str>) -> bool {
    !message.author.is_guest
        && Some(message.author.id.as_str()) != me
        && blocked.contains(&message.author.id)
}

/// Groups messages in display order into normal rows and blocked runs.
pub fn rows(messages: &[&Message], blocked: &BTreeSet<String>, me: Option<&str>) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        if !hidden(messages[index], blocked, me) {
            rows.push(Row::Message(index));
            index += 1;
            continue;
        }
        let start = index;
        while index < messages.len() && hidden(messages[index], blocked, me) {
            index += 1;
        }
        rows.push(Row::Blocked {
            range: start..index,
            key: messages[start].id.clone(),
        });
    }
    rows
}

/// The row's text; the ⊘ before it is painted.
pub fn label(count: usize) -> String {
    if count == 1 {
        "1 blocked message".into()
    } else {
        format!("{count} blocked messages")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: &str, author: &str, guest: bool) -> Message {
        serde_json::from_value(serde_json::json!({
            "id": id, "channelId": "channel", "seq": "1",
            "createdAt": "2026-10-07T09:00:00Z", "clientMessageId": format!("client-{id}"),
            "author": {"id": author, "name": author, "isGuest": guest},
            "content": {"version": 1, "type": "text", "text": id}
        }))
        .unwrap()
    }

    #[test]
    fn consecutive_blocked_messages_collapse_into_runs() {
        let blocked = BTreeSet::from(["maya".to_owned()]);
        let list = [
            message("1", "owner", false),
            message("2", "maya", false),
            message("3", "maya", false),
            message("4", "alex", false),
            message("5", "maya", false),
            message("6", "maya", true),
            message("7", "me", false),
        ];
        let refs: Vec<_> = list.iter().collect();
        assert_eq!(
            rows(&refs, &blocked, Some("me")),
            [
                Row::Message(0),
                Row::Blocked {
                    range: 1..3,
                    key: "2".into()
                },
                Row::Message(3),
                Row::Blocked {
                    range: 4..5,
                    key: "5".into()
                },
                Row::Message(5),
                Row::Message(6),
            ]
        );
        let me_blocked = BTreeSet::from(["me".to_owned()]);
        assert_eq!(
            rows(&refs[6..], &me_blocked, Some("me")),
            [Row::Message(0)],
            "your own messages never collapse"
        );
        assert!(rows(&[], &blocked, None).is_empty());
        assert_eq!(label(1), "1 blocked message");
        assert_eq!(label(2), "2 blocked messages");
    }
}
