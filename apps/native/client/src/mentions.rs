//! `@mention` contract v1 for sent messages, shared with the API and every
//! client: the token grammar, which tokens the server resolved into pills,
//! the `@feedback` team names, and whether a message mentions the viewer.
//! Composer suggestions and insertion stay with each client's editor.

use crate::model::Mention;
use std::ops::Range;

/// Longer runs after `@` are plain text.
pub const MAX_NAME: usize = 32;

pub fn name_character(value: char) -> bool {
    value.is_ascii_alphanumeric() || value == '_'
}

/// Same start rule as `:` emoji: text start, whitespace, `(`, `[` or `{`.
pub fn starts_token(previous: Option<char>) -> bool {
    previous.is_none_or(|value| value.is_whitespace() || matches!(value, '(' | '[' | '{'))
}

/// The team suggestion's secondary label.
pub const FEEDBACK_LABEL: &str = "Notify the Caper team";
/// Under a message that asked the team.
pub const FEEDBACK_NOTICE: &str = "Caper team has been notified 🫡";
/// The server's reserved team names, in suggestion order. Every one
/// resolves to the message's `feedback` entry.
pub const FEEDBACK_ALIASES: [&str; 17] = [
    "feedback",
    "caper",
    "capers",
    "caperchat",
    "caper_feedback",
    "caper_support",
    "support",
    "team",
    "staff",
    "admin",
    "admins",
    "administrator",
    "administrators",
    "mod",
    "mods",
    "moderator",
    "moderators",
];

/// Every mention token in sent text, as UTF-8 byte ranges with lowercase names.
pub fn tokens(text: &str) -> Vec<(Range<usize>, String)> {
    let mut found = Vec::new();
    let mut previous = None;
    let mut chars = text.char_indices().peekable();
    while let Some((index, character)) = chars.next() {
        let starts = character == '@' && starts_token(previous);
        previous = Some(character);
        if !starts {
            continue;
        }
        let mut end = index + 1;
        while let Some(&(next_index, next)) = chars.peek() {
            if !name_character(next) {
                break;
            }
            end = next_index + next.len_utf8();
            previous = Some(next);
            chars.next();
        }
        // Name characters are ASCII, so bytes equal characters.
        if (1..=MAX_NAME).contains(&(end - index - 1)) {
            found.push((index..end, text[index + 1..end].to_ascii_lowercase()));
        }
    }
    found
}

/// The server entry backing a token's lowercase name. Unresolved names, and
/// `everyone`/`here` without their entry (as in DMs), stay plain. One
/// `feedback` entry backs every team-name token (`@feedback`, `@caper`, …).
fn resolved<'a>(name: &str, mentions: &'a [Mention]) -> Option<&'a Mention> {
    mentions.iter().find(|entry| match entry.kind.as_str() {
        "everyone" | "here" => entry.kind == name,
        "feedback" => FEEDBACK_ALIASES.contains(&name),
        "user" => entry
            .username
            .as_deref()
            .is_some_and(|username| username.eq_ignore_ascii_case(name)),
        _ => false,
    })
}

/// The tokens in `text` to draw as mention pills: byte ranges and the entry
/// each one resolved to (only `user` entries open a card).
pub fn highlights<'a>(text: &str, mentions: &'a [Mention]) -> Vec<(Range<usize>, &'a Mention)> {
    if mentions.is_empty() {
        return Vec::new();
    }
    tokens(text)
        .into_iter()
        .filter_map(|(range, name)| resolved(&name, mentions).map(|entry| (range, entry)))
        .collect()
}

/// The message asked the Caper team: show "Caper team notified" under it.
pub fn notifies_team(mentions: &[Mention]) -> bool {
    mentions.iter().any(|entry| entry.kind == "feedback")
}

/// A message mentions `me` when a `user` entry has my account id, or when it
/// has `everyone`/`here` and someone else wrote it.
pub fn mentions_me(mentions: &[Mention], author_id: &str, me: Option<&str>) -> bool {
    let Some(me) = me else {
        return false;
    };
    mentions.iter().any(|entry| match entry.kind.as_str() {
        "user" => entry.id.as_deref() == Some(me),
        "everyone" | "here" => author_id != me,
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, id: Option<&str>, username: Option<&str>) -> Mention {
        Mention {
            kind: kind.into(),
            id: id.map(Into::into),
            username: username.map(Into::into),
        }
    }

    fn user(id: &str, username: &str) -> Mention {
        entry("user", Some(id), Some(username))
    }

    fn pills<'a>(text: &'a str, mentions: &[Mention]) -> Vec<&'a str> {
        highlights(text, mentions)
            .into_iter()
            .map(|(range, _)| &text[range])
            .collect()
    }

    #[test]
    fn highlights_only_tokens_the_server_resolved() {
        let mentions = [
            user("user00000001", "alice"),
            entry("everyone", None, None),
            entry("role", Some("r"), Some("bob")),
        ];
        let text = "hey @Alice's (@alice) @everyone @here @bob bob@alice.com x/@alice @@alice @al";
        assert_eq!(pills(text, &mentions), ["@Alice", "@alice", "@everyone"]);
        assert!(pills("@alice", &[]).is_empty());
        assert!(
            pills("@everyone @here", &[user("user00000001", "alice")]).is_empty(),
            "specials need their own entries (DMs never have them)"
        );
        assert_eq!(pills("@here now", &[entry("here", None, None)]), ["@here"]);
        let long = format!("@{} @alice", "a".repeat(33));
        assert_eq!(pills(&long, &mentions), ["@alice"]);
        assert_eq!(pills("é @ALICE ✨", &mentions), ["@ALICE"]);
    }

    #[test]
    fn mentions_me_by_account_id_or_by_someone_elses_everyone_or_here() {
        let me = Some("user00000001");
        assert!(mentions_me(&[user("user00000001", "alice")], "author", me));
        assert!(mentions_me(
            &[user("user00000001", "alice")],
            "user00000001",
            me
        ));
        assert!(!mentions_me(&[user("user00000002", "alice")], "author", me));
        for special in ["everyone", "here"] {
            let mentions = [entry(special, None, None)];
            assert!(mentions_me(&mentions, "author", me), "{special}");
            assert!(!mentions_me(&mentions, "user00000001", me), "{special}");
        }
        assert!(!mentions_me(
            &[entry("role", Some("user00000001"), None)],
            "author",
            me
        ));
        assert!(!mentions_me(
            &[entry("everyone", None, None)],
            "author",
            None
        ));
        assert!(!mentions_me(&[], "author", me));
    }

    #[test]
    fn team_name_tokens_become_pills_with_the_team_notice() {
        let mentions = [entry("feedback", None, None)];
        let text = "@Feedback uploads fail (@caper) @ADMIN @mods @Caper_Support feedback@caper.chat @feedbacks @moderatorx x@caper";
        assert_eq!(
            pills(text, &mentions),
            ["@Feedback", "@caper", "@ADMIN", "@mods", "@Caper_Support"],
            "case-insensitive, same boundaries as other mentions"
        );
        for alias in FEEDBACK_ALIASES {
            assert_eq!(pills(&format!("@{alias}"), &mentions).len(), 1, "{alias}");
        }
        assert!(
            pills("@feedback @admin", &[]).is_empty(),
            "plain text without the server's entry (a guest's message)"
        );
        assert!(notifies_team(&mentions));
        assert!(!notifies_team(&[user("u1", "feedback")]));
        assert!(!notifies_team(&[]));
        let highlighted = highlights("@team hi", &mentions);
        assert_eq!(highlighted[0].1.kind, "feedback", "no profile card");
        let decoded: crate::model::Content = serde_json::from_value(serde_json::json!({
            "version": 1, "type": "text", "text": "@feedback help",
            "mentions": [{"type": "feedback"}]
        }))
        .unwrap();
        assert_eq!(decoded.mentions, mentions);
        assert!(
            !mentions_me(&mentions, "someone", Some("me")),
            "asking the team is not mentioning you"
        );
        assert_eq!(FEEDBACK_NOTICE, "Caper team has been notified 🫡");
        assert_eq!(FEEDBACK_LABEL, "Notify the Caper team");
    }

    #[test]
    fn highlights_carry_the_entry_each_pill_resolved_to() {
        let mentions = [user("user00000001", "alice"), entry("everyone", None, None)];
        let kinds: Vec<_> = highlights("@ALICE @everyone", &mentions)
            .into_iter()
            .map(|(_, entry)| (entry.kind.as_str(), entry.id.as_deref()))
            .collect();
        assert_eq!(kinds, [("user", Some("user00000001")), ("everyone", None)]);
    }
}
