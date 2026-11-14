//! Who is typing in the open conversation, and your own typing state, as
//! web keeps them (`chat/client.ts`): a typing update replaces an author's
//! entry only with a newer revision, and a stop stays as a tombstone so a
//! late start can't revive it; entries expire after 6 s; a message from an
//! author ends their typing. Your typing is sent when the draft changes (at
//! most every 0.5 s) and stops 0.5 s after the last change or when the draft
//! empties. The row above the composer names who types (`label`).
use std::time::{Duration, Instant};

/// How long a typing (or stop) entry lasts without a newer update.
const SHOWN: Duration = Duration::from_secs(6);
/// Your typing is resent at most this often, and stops after this pause.
const PULSE: Duration = Duration::from_millis(500);
/// Entries kept at most, as web and egui cap them.
const MAX: usize = 64;

struct Typer {
    id: String,
    name: String,
    typing: bool,
    revision: u64,
    expires: Instant,
}

#[derive(Default)]
pub struct Typing {
    /// In arrival order, which the row names them in.
    typers: Vec<Typer>,
    /// When you last sent "typing", while it stands.
    sent: Option<Instant>,
    /// When your draft last changed, and its text then. None until the
    /// conversation's draft is known: restoring a saved draft isn't typing.
    edited: Option<Instant>,
    draft: Option<String>,
}

impl Typing {
    /// A gateway typing update from someone else (not you, not blocked).
    /// True when who is shown changed.
    pub fn update(
        &mut self,
        id: &str,
        name: &str,
        typing: bool,
        revision: u64,
        now: Instant,
    ) -> bool {
        let before = self.shown();
        let full = self.typers.len() >= MAX;
        match self.typers.iter_mut().find(|t| t.id == id) {
            Some(t) if revision > t.revision => {
                t.name = name.to_owned();
                t.typing = typing;
                t.revision = revision;
                t.expires = now + SHOWN;
            }
            Some(_) => return false,
            None if !full => self.typers.push(Typer {
                id: id.to_owned(),
                name: name.to_owned(),
                typing,
                revision,
                expires: now + SHOWN,
            }),
            None => return false,
        }
        self.shown() != before
    }

    fn shown(&self) -> Vec<String> {
        self.names().into_iter().map(str::to_owned).collect()
    }

    /// A message from `id` arrived: they stopped typing. True when shown
    /// names changed.
    pub fn sent_message(&mut self, id: &str) -> bool {
        match self.typers.iter_mut().find(|t| t.id == id && t.typing) {
            Some(t) => {
                t.typing = false;
                true
            }
            None => false,
        }
    }

    /// A new conversation or a lost connection: nobody is typing, and
    /// nothing of yours stands.
    pub fn clear(&mut self) {
        self.typers.clear();
        self.sent = None;
        self.edited = None;
        self.draft = None;
    }

    /// Who is typing, in arrival order.
    pub fn names(&self) -> Vec<&str> {
        self.typers
            .iter()
            .filter(|t| t.typing)
            .map(|t| t.name.as_str())
            .collect()
    }

    /// The composer changed (its text, or only its caret). Some(typing) to
    /// send when its text was edited.
    pub fn edited(&mut self, draft: &str, now: Instant) -> Option<bool> {
        let known = self.draft.replace(draft.to_owned());
        if known.as_deref().is_none_or(|known| known == draft) {
            return None;
        }
        if draft.trim().is_empty() {
            return self.sent.take().map(|_| false);
        }
        self.edited = Some(now);
        if self.sent.is_some_and(|at| now.duration_since(at) < PULSE) {
            return None;
        }
        self.sent = Some(now);
        Some(true)
    }

    /// The composer now holds a conversation's saved draft: later edits
    /// compare with it, and restoring it is not typing.
    pub fn adopt(&mut self, draft: &str) {
        self.draft = Some(draft.to_owned());
    }

    /// You sent the draft, or left the conversation: Some(false) to send
    /// when typing stood.
    pub fn stop(&mut self) -> Option<bool> {
        self.edited = None;
        self.sent.take().map(|_| false)
    }

    /// Expires entries and ends your typing after a pause. Returns what to
    /// send and whether shown names changed.
    pub fn tick(&mut self, now: Instant) -> (Option<bool>, bool) {
        let before = self.typers.iter().filter(|t| t.typing).count();
        self.typers.retain(|t| t.expires > now);
        let changed = self.typers.iter().filter(|t| t.typing).count() != before;
        let send = if self.sent.is_some()
            && self.edited.is_none_or(|at| now.duration_since(at) >= PULSE)
        {
            self.stop()
        } else {
            None
        };
        (send, changed)
    }

    /// Anything left to tick: entries to expire or your typing to end.
    pub fn busy(&self) -> bool {
        self.sent.is_some() || !self.typers.is_empty()
    }
}

/// The row's words, as web and egui word them.
pub fn label(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => format!("{one} is typing\u{2026}"),
        [one, two] => format!("{one} and {two} are typing\u{2026}"),
        _ => "Several people are typing\u{2026}".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_starts_cannot_revive_a_stop_and_entries_expire() {
        let t0 = Instant::now();
        let mut t = Typing::default();
        assert!(t.update("a", "Ana", true, 4, t0));
        assert!(t.update("a", "Ana", false, 6, t0));
        // A start older than the stop arrives late, or one repeating its revision.
        assert!(!t.update("a", "Ana", true, 5, t0));
        assert!(!t.update("a", "Ana", true, 6, t0));
        assert!(t.names().is_empty());
        assert!(t.busy(), "the stop stays as a tombstone");
        let (_, changed) = t.tick(t0 + SHOWN);
        assert!(!changed);
        assert!(!t.busy(), "tombstones expire");
        t.update("b", "Ben", true, 1, t0);
        let (_, changed) = t.tick(t0 + SHOWN);
        assert!(changed, "a shown typer expiring changes the row");
    }

    #[test]
    fn names_keep_arrival_order_and_a_message_ends_typing() {
        let t0 = Instant::now();
        let mut t = Typing::default();
        t.update("z", "Zoe", true, 1, t0);
        t.update("a", "Ana", true, 1, t0);
        assert_eq!(t.names(), ["Zoe", "Ana"]);
        assert_eq!(label(&t.names()), "Zoe and Ana are typing\u{2026}");
        assert!(t.sent_message("z"));
        assert!(!t.sent_message("z"));
        assert_eq!(label(&t.names()), "Ana is typing\u{2026}");
        t.update("b", "Ben", true, 1, t0);
        t.update("c", "Cy", true, 1, t0);
        assert_eq!(label(&t.names()), "Several people are typing\u{2026}");
    }

    #[test]
    fn own_typing_pulses_and_stops_after_a_pause_or_an_empty_draft() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut t = Typing::default();
        assert_eq!(
            t.edited("saved draft", t0),
            None,
            "a restored draft isn't typing"
        );
        assert_eq!(
            t.edited("saved draft", t0),
            None,
            "a caret move isn't typing"
        );
        assert_eq!(t.edited("h", t0), Some(true));
        assert_eq!(t.edited("hi", ms(300)), None, "at most every 0.5 s");
        assert_eq!(t.tick(ms(700)).0, None, "edited 0.4 s ago");
        assert_eq!(t.edited("hi!", ms(600)), Some(true));
        assert_eq!(t.tick(ms(1100)).0, Some(false));
        assert_eq!(t.tick(ms(1200)).0, None, "stopped once");
        assert_eq!(t.edited("  ", ms(1300)), None, "nothing stands to stop");
        assert_eq!(t.edited("a", ms(1400)), Some(true));
        assert_eq!(
            t.edited("", ms(1500)),
            Some(false),
            "sending empties the draft"
        );
        t.edited("b", ms(1600));
        assert_eq!(t.stop(), Some(false));
        assert_eq!(t.stop(), None);
        t.clear();
        assert_eq!(
            t.edited("unknown", ms(1700)),
            None,
            "after a clear, adopted"
        );
        t.adopt("other draft");
        assert_eq!(t.edited("other draft", ms(1800)), None, "restoring a draft");
        assert_eq!(t.edited("other drafts", ms(1900)), Some(true));
    }
}
