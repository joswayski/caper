//! Live cards for message links in message text, as web's `MessageLinkCards`
//! draws them: up to five distinct links per message, each resolved through
//! `GET /api/chat/message-links/{channel}/{message}` (`Api::resolve_message_link`),
//! which answers only for readers who can see the original. A card shows the
//! original (author, text, attachments, embeds) with Jump to message, or
//! "Message you can't view" when the API says no. Until the first answer, or
//! when it fails, no card shows and the link stays plain text. Shown cards ask
//! again every [`REFRESH`], so edits, reactions and deletions follow the
//! original.

use crate::model::ForwardSource;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Cards per message, like link previews.
pub const MAX: usize = 5;
/// How long an answer stands before a shown card asks again.
pub const REFRESH: Duration = Duration::from_secs(30);

/// The distinct message links in `text` as (channel, message), in order, at
/// most [`MAX`].
pub fn links_in(text: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    for segment in crate::links::segments(text) {
        let Some(link) = segment
            .href
            .as_deref()
            .and_then(crate::message_links::parse)
        else {
            continue;
        };
        if !found.iter().any(|(_, message)| message == link.message) {
            found.push((link.channel.to_owned(), link.message.to_owned()));
        }
        if found.len() == MAX {
            break;
        }
    }
    found
}

#[derive(Default)]
struct Entry {
    asked: Option<Instant>,
    pending: bool,
    /// The latest answer: `Some(None)` when this account can't see it.
    value: Option<Option<ForwardSource>>,
}

/// What each linked message resolved to, by message ID.
#[derive(Default)]
pub struct LinkCards {
    entries: HashMap<String, Entry>,
}

impl LinkCards {
    /// The answer for `message`: `None` until one arrived (no card yet),
    /// `Some(None)` for "Message you can't view".
    pub fn card(&self, message: &str) -> Option<Option<&ForwardSource>> {
        self.entries
            .get(message)
            .and_then(|entry| entry.value.as_ref())
            .map(Option::as_ref)
    }

    /// The links in `text` to ask about now: never asked, or answered more
    /// than [`REFRESH`] ago, and not already on their way. They count as on
    /// their way until [`resolved`](Self::resolved).
    pub fn due(&mut self, text: &str, now: Instant) -> Vec<(String, String)> {
        let mut due = Vec::new();
        for (channel, message) in links_in(text) {
            let entry = self.entries.entry(message.clone()).or_default();
            if entry.pending
                || entry
                    .asked
                    .is_some_and(|asked| now.duration_since(asked) < REFRESH)
            {
                continue;
            }
            entry.pending = true;
            entry.asked = Some(now);
            due.push((channel, message));
        }
        due
    }

    /// An answer arrived. A failure keeps the last answer (or no card) and
    /// is asked again at the next refresh. True when the card changed.
    pub fn resolved(
        &mut self,
        message: &str,
        result: Result<Option<ForwardSource>, String>,
    ) -> bool {
        let entry = self.entries.entry(message.to_owned()).or_default();
        entry.pending = false;
        let Ok(value) = result else {
            return false;
        };
        if entry.value.as_ref() == Some(&value) {
            return false;
        }
        entry.value = Some(value);
        true
    }

    /// Signed out or another account: nothing carries over.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_distinct_caper_message_links_only() {
        let text = "see https://caper.chat/m/Chan12345678/Msg123456789012 and \
            caper.chat/m/x https://example.com/m/Chan12345678/Msg999999999999 \
            https://www.caper.chat/m/Chan12345678/Msg123456789012/ \
            https://caper.chat/m/Chan12345678/Msg000000000002";
        assert_eq!(
            links_in(text),
            vec![
                ("Chan12345678".into(), "Msg123456789012".into()),
                ("Chan12345678".into(), "Msg000000000002".into()),
            ]
        );
        let many: String = (0..7)
            .map(|i| format!("https://caper.chat/m/Chan12345678/Msg00000000000{i} "))
            .collect();
        assert_eq!(links_in(&many).len(), MAX);
    }

    #[test]
    fn asks_once_until_answered_then_after_the_refresh() {
        let mut cards = LinkCards::default();
        let text = "https://caper.chat/m/Chan12345678/Msg123456789012";
        let start = Instant::now();
        assert_eq!(cards.due(text, start).len(), 1);
        // On its way: not asked twice, and no card yet.
        assert!(cards.due(text, start + REFRESH * 2).is_empty());
        assert_eq!(cards.card("Msg123456789012"), None);
        // A failure leaves no card and waits for the refresh.
        assert!(!cards.resolved("Msg123456789012", Err("offline".into())));
        assert!(cards.due(text, start + REFRESH / 2).is_empty());
        assert_eq!(cards.due(text, start + REFRESH).len(), 1);
        assert!(cards.resolved("Msg123456789012", Ok(None)));
        assert_eq!(cards.card("Msg123456789012"), Some(None));
        // The same answer again changes nothing; a failure keeps it.
        assert_eq!(cards.due(text, start + REFRESH * 2).len(), 1);
        assert!(!cards.resolved("Msg123456789012", Ok(None)));
        assert!(!cards.resolved("Msg123456789012", Err("offline".into())));
        assert_eq!(cards.card("Msg123456789012"), Some(None));
        cards.clear();
        assert_eq!(cards.card("Msg123456789012"), None);
    }
}
