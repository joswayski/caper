//! Optimistic reactions: the viewer's intended reaction per (message, emoji),
//! drawn over the server's counts until the server acknowledges it. At most
//! one `React` request per message is in flight; a toggle made while one is
//! in flight waits and sends the latest intent when the reply arrives.

use crate::model::{Reaction, ReactionUpdate, ReactorGroup, Reactors};
use crate::worker::{SendFailure, dm_refusal};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingReaction {
    /// Whether the viewer wants to have reacted.
    pub desired: bool,
    /// A request for this emoji is in flight.
    pub sent: bool,
    /// Drawn over the server's counts; false once it failed (and awaits Retry).
    pub visible: bool,
    /// The viewer toggled again while the request was in flight.
    pub superseded: bool,
}

/// One `React` request to send now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReactionSend {
    pub message: String,
    pub emoji: String,
    pub active: bool,
}

/// What the client does with a `React` reply.
#[derive(Debug)]
pub enum Reacted {
    /// Merge this update into the timeline (reload the channel if it does
    /// not merge), then send the message's next request.
    Acknowledged(ReactionUpdate),
    /// The reply belongs to another message: reload the channel.
    Reload,
    /// The viewer lost access (401, 403 or 404 that is not a DM refusal):
    /// clear the channel with this message.
    ClearChannel(String),
    /// Failed. Unless the viewer had toggled again, the reaction rolled back
    /// and the message shows an error with Retry and Dismiss. Send the
    /// message's next request.
    Failed,
}

/// Every pending reaction and the reaction error shown under each message.
#[derive(Debug, Default)]
pub struct Reactions {
    /// By (message ID, emoji).
    pub pending: BTreeMap<(String, String), PendingReaction>,
    /// By message ID.
    pub errors: BTreeMap<String, String>,
}

impl Reactions {
    /// Forget everything, as when the channel or account changes.
    pub fn clear(&mut self) {
        self.pending.clear();
        self.errors.clear();
    }

    /// The viewer reacts (`active`) or un-reacts. Clears the message's error;
    /// follow with [`Reactions::next`].
    pub fn set(&mut self, message: &str, emoji: &str, active: bool) {
        self.errors.remove(message);
        let pending = self
            .pending
            .entry((message.to_owned(), emoji.to_owned()))
            .or_insert(PendingReaction {
                desired: active,
                sent: false,
                visible: true,
                superseded: false,
            });
        pending.desired = active;
        pending.visible = true;
        pending.superseded = pending.sent;
    }

    /// The message's next request, marked in flight, unless one already is.
    pub fn next(&mut self, message: &str) -> Option<ReactionSend> {
        if self
            .pending
            .iter()
            .any(|((id, _), pending)| id == message && pending.sent)
        {
            return None;
        }
        let ((message, emoji), pending) = self
            .pending
            .iter_mut()
            .find(|((id, _), pending)| id == message && pending.visible && !pending.sent)?;
        pending.sent = true;
        pending.superseded = false;
        Some(ReactionSend {
            message: message.clone(),
            emoji: emoji.clone(),
            active: pending.desired,
        })
    }

    /// Apply the reply to the request sent for `channel`/`message`/`emoji`.
    pub fn reacted(
        &mut self,
        channel: &str,
        message: &str,
        emoji: &str,
        active: bool,
        result: Result<ReactionUpdate, SendFailure>,
    ) -> Reacted {
        let key = (message.to_owned(), emoji.to_owned());
        match result {
            Ok(update) if update.channel_id == channel && update.message_id == message => {
                if self
                    .pending
                    .get(&key)
                    .is_some_and(|pending| pending.desired == active)
                {
                    self.pending.remove(&key);
                } else if let Some(pending) = self.pending.get_mut(&key) {
                    pending.sent = false;
                }
                Reacted::Acknowledged(update)
            }
            Ok(_) => Reacted::Reload,
            Err(error)
                if matches!(error.status, Some(401 | 403 | 404))
                    && !dm_refusal(error.code.as_deref()) =>
            {
                self.pending.remove(&key);
                Reacted::ClearChannel(error.message)
            }
            Err(error) => {
                if let Some(pending) = self.pending.get_mut(&key) {
                    pending.sent = false;
                    if !pending.superseded {
                        pending.visible = false;
                        self.errors.insert(key.0, error.message);
                    }
                }
                Reacted::Failed
            }
        }
    }

    /// The message's reaction error, if one shows.
    pub fn error(&self, message: &str) -> Option<&str> {
        self.errors.get(message).map(String::as_str)
    }

    /// What Retry sets again: the failed emoji and the viewer's intent.
    pub fn retry(&self, message: &str) -> Option<(String, bool)> {
        self.pending
            .iter()
            .find(|((id, _), pending)| id == message && !pending.visible)
            .map(|((_, emoji), pending)| (emoji.clone(), pending.desired))
    }

    /// Dismiss the message's error and drop its failed reactions.
    pub fn dismiss(&mut self, message: &str) {
        self.errors.remove(message);
        self.pending
            .retain(|(id, _), pending| id != message || pending.visible);
    }

    /// The message's reactions as the viewer (`author`) should see them: the
    /// server's counts with the viewer's visible pending intents applied.
    pub fn projected(
        &self,
        reactions: &[Reaction],
        message: &str,
        author: Option<&str>,
    ) -> Vec<Reaction> {
        let mut projected = reactions.to_vec();
        let Some(author) = author else {
            return projected;
        };
        for ((pending_message, emoji), intent) in &self.pending {
            if pending_message != message || !intent.visible {
                continue;
            }
            if let Some(reaction) = projected.iter_mut().find(|item| item.emoji == *emoji) {
                reaction.author_ids.retain(|id| id != author);
                if intent.desired {
                    reaction.author_ids.push(author.to_owned());
                }
            } else if intent.desired {
                projected.push(Reaction {
                    emoji: emoji.clone(),
                    author_ids: vec![author.to_owned()],
                });
            }
        }
        projected.retain(|reaction| !reaction.author_ids.is_empty());
        projected
    }
}

/// Who reacted to one message, read at the snapshot's `reactionSeq` for the
/// hover card and the reactions window.
pub struct ReactorCache {
    pub revision: String,
    pub state: ReactorState,
    /// Names are kept while refreshing; the shown reaction IDs decide who
    /// is listed.
    pub groups: Vec<ReactorGroup>,
}

pub enum ReactorState {
    Loading(Instant),
    Loaded,
    /// Hovering again retries after `REACTOR_RETRY`.
    Failed(Instant),
}

/// How long a failed read waits before hovering asks again.
pub const REACTOR_RETRY: Duration = Duration::from_secs(5);
/// Messages whose readers are kept; past it the cache starts over.
pub const REACTOR_CACHE_LIMIT: usize = 256;

impl ReactorCache {
    /// Whether hovering should (re)start the request for `revision`.
    pub fn wants_request(&self, revision: &str) -> bool {
        self.revision != revision
            || match self.state {
                ReactorState::Loaded => false,
                // The API client times out after 15 s, so this request is lost.
                ReactorState::Loading(at) => at.elapsed() >= Duration::from_secs(20),
                ReactorState::Failed(at) => at.elapsed() >= REACTOR_RETRY,
            }
    }

    /// The reply to the request for `message` at `revision`; a newer
    /// revision may have started its own request since, which this leaves.
    pub fn loaded(&mut self, message: &str, revision: &str, result: Result<Reactors, String>) {
        if self.revision != revision || !matches!(self.state, ReactorState::Loading(_)) {
            return;
        }
        self.state = match result {
            Ok(list) if list.message_id == message => {
                self.groups = list.reactions;
                ReactorState::Loaded
            }
            _ => ReactorState::Failed(Instant::now()),
        };
    }
}

/// Starts reading who reacted to `message` at `revision` unless that is
/// cached or already loading. True when the caller should send
/// `Command::LoadReactors` for it.
pub fn start_reactors(
    cache: &mut BTreeMap<String, ReactorCache>,
    message: &str,
    revision: &str,
) -> bool {
    if cache
        .get(message)
        .is_some_and(|cached| !cached.wants_request(revision))
    {
        return false;
    }
    if cache.len() >= REACTOR_CACHE_LIMIT && !cache.contains_key(message) {
        cache.clear();
    }
    let now = Instant::now();
    cache
        .entry(message.to_owned())
        .and_modify(|cached| {
            revision.clone_into(&mut cached.revision);
            cached.state = ReactorState::Loading(now);
        })
        .or_insert_with(|| ReactorCache {
            revision: revision.to_owned(),
            state: ReactorState::Loading(now),
            groups: Vec::new(),
        });
    true
}

/// Who reacted with `reaction`, as every client words it: the known names
/// (yours as "You"), else a count until they load.
pub fn reactor_summary(
    cache: Option<&ReactorCache>,
    reaction: &Reaction,
    self_id: Option<&str>,
) -> String {
    let name = crate::emoji::name(&reaction.emoji);
    let groups = cache.map_or(&[][..], |cached| cached.groups.as_slice());
    match crate::model::reactor_list(groups, reaction, self_id) {
        Some(authors) => crate::model::reactor_summary(&authors, self_id, name, &reaction.emoji),
        None => {
            crate::model::reactor_fallback(&reaction.author_ids, self_id, name, &reaction.emoji)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(desired: bool, sent: bool, visible: bool) -> PendingReaction {
        PendingReaction {
            desired,
            sent,
            visible,
            superseded: false,
        }
    }

    fn update(message: &str) -> ReactionUpdate {
        ReactionUpdate {
            kind: "message.reactions".into(),
            schema_version: 1,
            channel_id: "channel".into(),
            message_id: message.into(),
            seq: "5".into(),
            reactions: Vec::new(),
        }
    }

    fn failure(status: u16, code: Option<&str>) -> SendFailure {
        SendFailure {
            status: Some(status),
            message: "failed".into(),
            code: code.map(Into::into),
        }
    }

    #[test]
    fn pending_reactions_project_own_membership_over_latest_authoritative_counts() {
        let mut state = Reactions::default();
        state
            .pending
            .insert(("message".into(), "👍".into()), pending(false, true, true));
        state
            .pending
            .insert(("message".into(), "🎉".into()), pending(true, false, true));
        let authoritative = vec![
            Reaction {
                emoji: "👍".into(),
                author_ids: vec!["me".into(), "other".into(), "gateway".into()],
            },
            Reaction {
                emoji: "🎉".into(),
                author_ids: vec!["other".into()],
            },
        ];

        let projected = state.projected(&authoritative, "message", Some("me"));

        assert_eq!(
            authoritative[0].author_ids.len(),
            3,
            "source stays authoritative"
        );
        assert_eq!(projected[0].author_ids, ["other", "gateway"]);
        assert_eq!(projected[1].author_ids, ["other", "me"]);

        let only_me = vec![Reaction {
            emoji: "👍".into(),
            author_ids: vec!["me".into()],
        }];
        assert!(
            state
                .projected(&only_me, "message", Some("me"))
                .iter()
                .all(|reaction| reaction.emoji != "👍")
        );
        assert_eq!(
            state.projected(&only_me, "other message", Some("me")),
            only_me
        );
    }

    #[test]
    fn failed_reaction_overlay_rolls_back_without_hiding_other_authors() {
        let mut state = Reactions::default();
        state.pending.insert(
            ("message".into(), "👍".into()),
            pending(false, false, false),
        );
        let authoritative = vec![Reaction {
            emoji: "👍".into(),
            author_ids: vec!["me".into(), "other".into()],
        }];
        assert_eq!(
            state.projected(&authoritative, "message", Some("me")),
            authoritative
        );
    }

    #[test]
    fn one_request_per_message_in_flight_and_the_latest_intent_follows() {
        let mut state = Reactions::default();
        state.set("m", "👀", true);
        let first = state.next("m").unwrap();
        assert!(first.active);
        state.set("m", "👀", false);
        state.set("m", "🎉", true);
        assert_eq!(state.next("m"), None, "one request per message");
        state.set("n", "👍", true);
        assert_eq!(state.next("n").unwrap().message, "n", "other messages move");

        // The ack for the stale intent keeps the latest one pending.
        assert!(matches!(
            state.reacted("channel", "m", "👀", true, Ok(update("m"))),
            Reacted::Acknowledged(_)
        ));
        assert!(state.pending[&("m".into(), "👀".into())].visible);
        let second = state.next("m").unwrap();
        assert_eq!((second.emoji.as_str(), second.active), ("🎉", true));
        state.reacted("channel", "m", "🎉", true, Ok(update("m")));
        let third = state.next("m").unwrap();
        assert_eq!((third.emoji.as_str(), third.active), ("👀", false));
        assert!(matches!(
            state.reacted("channel", "m", "👀", false, Ok(update("m"))),
            Reacted::Acknowledged(_)
        ));
        assert!(!state.pending.keys().any(|(id, _)| id == "m"));
        state.set("m", "🎉", false);
        state.next("m").unwrap();
        assert!(matches!(
            state.reacted("channel", "m", "🎉", true, Ok(update("other"))),
            Reacted::Reload
        ));
    }

    #[test]
    fn failures_roll_back_unless_superseded_and_lost_access_clears_the_channel() {
        let mut state = Reactions::default();
        state.set("m", "👀", true);
        state.next("m").unwrap();
        state.set("m", "👀", false);
        assert!(matches!(
            state.reacted("channel", "m", "👀", true, Err(failure(503, None))),
            Reacted::Failed
        ));
        assert_eq!(state.error("m"), None, "a superseded failure stays quiet");
        let retried = state.next("m").unwrap();
        assert!(!retried.active, "the latest intent follows");

        state.reacted("channel", "m", "👀", false, Err(failure(503, None)));
        assert_eq!(state.error("m"), Some("failed"));
        assert!(!state.pending[&("m".into(), "👀".into())].visible);
        assert_eq!(state.next("m"), None, "failed intents wait for Retry");
        assert_eq!(state.retry("m"), Some(("👀".into(), false)));
        state.set("m", "👀", false);
        assert_eq!(state.error("m"), None, "Retry clears the error");
        state.next("m").unwrap();
        state.reacted("channel", "m", "👀", false, Err(failure(503, None)));
        state.dismiss("m");
        assert_eq!(state.error("m"), None);
        assert!(state.pending.is_empty(), "Dismiss drops failed intents");

        state.set("m", "👍", true);
        state.next("m").unwrap();
        assert!(matches!(
            state.reacted("channel", "m", "👍", true, Err(failure(403, None))),
            Reacted::ClearChannel(message) if message == "failed"
        ));
        assert!(state.pending.is_empty());
        state.set("m", "👍", true);
        state.next("m").unwrap();
        assert!(
            matches!(
                state.reacted(
                    "channel",
                    "m",
                    "👍",
                    true,
                    Err(failure(403, Some("dm_blocked")))
                ),
                Reacted::Failed
            ),
            "a DM refusal is an ordinary failure"
        );
        assert_eq!(state.error("m"), Some("failed"));
    }
}
