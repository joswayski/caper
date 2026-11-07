//! Message edits: what an edited text must be, and the retained
//! versions the history pages through and compares as web does.

use crate::model::MessageVersion;

#[path = "../../../../shared/messages/history_diff.rs"]
mod history_diff;

pub fn valid_text(text: &str) -> bool {
    !text.trim().is_empty()
        && text.chars().count() <= 4000
        && !text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
}

pub type DiffTokens = Vec<(String, bool)>;

/// The before and after texts as runs, each marked when web's history view
/// highlights it (`shared/messages/history_diff.rs`).
pub fn diff(before: &str, after: &str) -> (DiffTokens, DiffTokens) {
    let (old, new) = history_diff::highlights(before, after);
    let tokens = |text: &str, runs: history_diff::Highlights| {
        runs.into_iter()
            .map(|(range, changed)| (text[range].to_owned(), changed))
            .collect()
    };
    (tokens(before, old), tokens(after, new))
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
    fn highlights_follow_the_shared_cases() {
        let cases: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../shared/messages/history-diff-cases.json"
        ))
        .unwrap();
        for case in cases["cases"].as_array().unwrap() {
            let text = |key: &str| case[key].as_str().unwrap();
            let expected =
                |key: &str| -> DiffTokens { serde_json::from_value(case[key].clone()).unwrap() };
            let (old, new) = diff(text("before"), text("after"));
            assert_eq!(
                (old, new),
                (expected("old"), expected("new")),
                "{}",
                text("note")
            );
        }
    }

    #[test]
    fn edited_text_rules() {
        assert!(valid_text(&"🙂".repeat(4000)));
        assert!(!valid_text(&"🙂".repeat(4001)));
        assert!(!valid_text(" \n\t"));
    }
}
