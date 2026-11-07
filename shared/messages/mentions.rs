//! The `@mention` token grammar of contract v1, shared by the native clients
//! (`apps/native/client`, and Android and Apple through `apps/native/mobile`):
//! an `@` at the start of the text, after whitespace or after `(`, `[` or `{`,
//! followed by 1–32 ASCII letters, digits or underscores. Names compare
//! case-insensitively. The API resolves the same grammar
//! (`apps/api/src/mentions.rs`). Every client runs
//! `shared/messages/mention-cases.json`; change them together.

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

#[cfg(test)]
mod shared_cases {
    #[test]
    fn tokens_follow_the_shared_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("mention-cases.json")).unwrap();
        let cases = cases["cases"].as_array().unwrap();
        assert!(cases.len() > 10);
        for case in cases {
            let text = case["text"].as_str().unwrap();
            let expected: Vec<(String, String)> = case["tokens"]
                .as_array()
                .unwrap()
                .iter()
                .map(|token| {
                    (
                        token["text"].as_str().unwrap().to_owned(),
                        token["name"].as_str().unwrap().to_owned(),
                    )
                })
                .collect();
            let actual: Vec<(String, String)> = super::tokens(text)
                .into_iter()
                .map(|(range, name)| (text[range].to_owned(), name))
                .collect();
            assert_eq!(actual, expected, "{}: {text:?}", case["note"]);
        }
    }
}
