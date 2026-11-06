//! Message edits: what an edited text must be, and the retained
//! versions the history pages through and compares word by word.

use crate::model::MessageVersion;
use similar::{ChangeTag, TextDiff};

pub fn valid_text(text: &str) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= 4000
        && !text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}

pub type DiffTokens = Vec<(String, bool)>;

/// Word-level before and after tokens, each marked when it changed.
pub fn diff(before: &str, after: &str) -> (DiffTokens, DiffTokens) {
    let mut old = Vec::new();
    let mut new = Vec::new();
    for change in TextDiff::from_words(before, after).iter_all_changes() {
        let token = (change.value().to_owned(), change.tag() != ChangeTag::Equal);
        match change.tag() {
            ChangeTag::Equal => {
                old.push(token.clone());
                new.push(token);
            }
            ChangeTag::Delete => old.push(token),
            ChangeTag::Insert => new.push(token),
        }
    }
    (old, new)
}

/// Adds a page of versions, newest first: an older page appends the
/// revisions not yet listed, a fresh page replaces the list.
pub fn merge_versions(versions: &mut Vec<MessageVersion>, page: Vec<MessageVersion>, older: bool) {
    if !older {
        *versions = page;
        return;
    }
    for version in page {
        if !versions.iter().any(|old| old.revision == version.revision) {
            versions.push(version);
        }
    }
}

/// A version's name and local time, as the history lists it.
pub fn version_label(version: &MessageVersion) -> String {
    let date = chrono::DateTime::parse_from_rfc3339(&version.created_at)
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%b %-d · %-I:%M %p")
                .to_string()
        })
        .unwrap_or_else(|_| version.created_at.clone());
    format!(
        "{} · {date}",
        if version.revision == 1 {
            "Original version".into()
        } else {
            format!("Version {}", version.revision)
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separated_changes_preserve_unicode_and_whitespace() {
        let before = "Meet Friday 🙂\nKeep this unchanged\nAt 9";
        let after = "Meet Saturday 🚀\nKeep this unchanged\nAt 11";
        let (old, new) = diff(before, after);
        assert_eq!(
            old.iter()
                .map(|(text, _)| text.as_str())
                .collect::<String>(),
            before
        );
        assert_eq!(
            new.iter()
                .map(|(text, _)| text.as_str())
                .collect::<String>(),
            after
        );
        assert!(
            old.iter()
                .filter(|(text, _)| text.contains("Keep"))
                .all(|(_, changed)| !changed)
        );
        assert!(
            old.iter()
                .filter(|(_, changed)| *changed)
                .map(|(text, _)| text.as_str())
                .collect::<String>()
                .contains("Friday")
        );
        assert!(
            new.iter()
                .filter(|(_, changed)| *changed)
                .map(|(text, _)| text.as_str())
                .collect::<String>()
                .contains("Saturday")
        );
        assert!(valid_text(&"🙂".repeat(4000)));
        assert!(!valid_text(&"🙂".repeat(4001)));
        assert!(!valid_text(" \n\t"));
    }
}
