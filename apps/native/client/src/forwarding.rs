//! Forwarding a message: the requests behind the picker and the forwarded
//! conversation view.
use crate::api::{Api, ApiError};
use crate::model::{ForwardConversation, ForwardDestination, Message, sequence};
use crate::worker::SendFailure;

pub enum Operation {
    Destinations {
        token: String,
    },
    Send {
        token: String,
        session: String,
        source: Box<Message>,
        destination: String,
        key: String,
        text: String,
    },
    Conversation {
        token: String,
        wrapper: Box<Message>,
        pages: usize,
        oldest: Option<String>,
    },
}

pub enum Response {
    Destinations(Vec<ForwardDestination>, String),
    Sent(Box<Message>),
    Conversation(Box<ForwardConversation>),
}

impl Operation {
    pub fn run(self, api: &Api) -> Result<Response, SendFailure> {
        let run = || -> Result<Response, ApiError> {
            Ok(match self {
                Self::Destinations { token } => {
                    let destinations = api.forward_destinations(&token)?.destinations;
                    let session = api.chat_session(Some(&token), "Forward")?.token;
                    Response::Destinations(destinations, session)
                }
                Self::Send {
                    token,
                    session,
                    source,
                    destination,
                    key,
                    text,
                } => {
                    let message =
                        api.forward(&token, &session, &source, &destination, &key, &text)?;
                    if message.validate().is_err()
                        || message.channel_id != destination
                        || message.client_message_id != key
                        || message.forward.is_none()
                    {
                        return Err(invalid());
                    }
                    Response::Sent(Box::new(message))
                }
                Self::Conversation {
                    token,
                    wrapper,
                    pages,
                    oldest,
                } => {
                    let mut page = api.forward_conversation(&token, &wrapper, None)?;
                    validate(&page)?;
                    let mut loaded = 1;
                    while page.has_more
                        && page.messages.first().is_some_and(|first| {
                            loaded < pages
                                || oldest.as_ref().is_some_and(|old| {
                                    sequence(&first.seq).unwrap_or(0) > sequence(old).unwrap_or(0)
                                })
                        })
                    {
                        let first = &page.messages[0].seq;
                        let mut earlier =
                            api.forward_conversation(&token, &wrapper, Some(first))?;
                        validate(&earlier)?;
                        if earlier.messages.is_empty() {
                            page.has_more = false;
                            break;
                        }
                        if sequence(&earlier.messages[0].seq).unwrap_or(0)
                            >= sequence(first).unwrap_or(0)
                        {
                            return Err(invalid());
                        }
                        earlier.messages.append(&mut page.messages);
                        page.messages = earlier.messages;
                        page.has_more = earlier.has_more;
                        loaded += 1;
                    }
                    Response::Conversation(Box::new(page))
                }
            })
        };
        run().map_err(|error| SendFailure {
            status: error.status.map(|status| status.as_u16()),
            message: error.message,
            code: error.code,
        })
    }
}

/// The forward picker's state: where the message can go, what is chosen
/// and the forwards still waiting to be confirmed, each with its own
/// idempotency key so a retry sends the same forward.
pub struct Picker {
    pub source: Box<Message>,
    pub destinations: Vec<ForwardDestination>,
    pub selected: Vec<String>,
    /// Destinations and keys of the forwards still to confirm, in order.
    pub pending: Option<Vec<(String, String)>>,
    pub confirmed: usize,
    /// The chat session capability that sends the forwards.
    pub session: Option<String>,
}

impl Picker {
    pub fn new(source: Message) -> Self {
        Self {
            source: Box::new(source),
            destinations: vec![],
            selected: vec![],
            pending: None,
            confirmed: 0,
            session: None,
        }
    }

    /// Destinations matching every search term (a leading `#` is ignored),
    /// by space and name.
    pub fn visible<'a>(&'a self, query: &str) -> impl Iterator<Item = &'a ForwardDestination> {
        let query = query.to_lowercase();
        self.destinations.iter().filter(move |item| {
            let haystack = format!("{} {}", item.space_name, item.name).to_lowercase();
            query
                .split_whitespace()
                .all(|term| haystack.contains(term.trim_start_matches('#')))
        })
    }

    pub fn set(&mut self, id: &str, checked: bool) {
        self.selected.retain(|selected| selected != id);
        if checked {
            self.selected.push(id.to_owned());
        }
    }

    /// Whether "Forward (N)" can be pressed.
    pub fn ready(&self, busy: bool, note: &str) -> bool {
        !busy && !self.selected.is_empty() && self.session.is_some() && note.chars().count() <= 4000
    }

    /// "Forward (N)", or "Retry forwards (N)" after an unconfirmed send.
    pub fn label(&self) -> String {
        format!(
            "{} ({})",
            if self.pending.is_some() {
                "Retry forwards"
            } else {
                "Forward"
            },
            self.selected.len()
        )
    }

    /// The next forward to send. `start` queues the selection (keeping
    /// any keys already queued) when pressed.
    pub fn next(&mut self, token: &str, note: &str, start: bool) -> Option<Operation> {
        if start {
            self.pending.get_or_insert_with(|| {
                self.selected
                    .iter()
                    .map(|id| (id.clone(), uuid::Uuid::new_v4().to_string()))
                    .collect()
            });
        }
        let (destination, key) = self.pending.as_ref()?.first()?.clone();
        Some(Operation::Send {
            token: token.into(),
            session: self.session.clone()?,
            source: self.source.clone(),
            destination,
            key,
            text: note.trim().into(),
        })
    }

    pub fn loaded(&mut self, mut values: Vec<ForwardDestination>, capability: String) {
        values.sort_by_key(|item| format!("{} {}", item.space_name, item.name));
        self.destinations = values;
        self.session = Some(capability);
    }

    /// One forward is confirmed; true once none remain.
    pub fn sent(&mut self, message: &Message) -> bool {
        let Some(pending) = &mut self.pending else {
            return false;
        };
        if !pending.is_empty() {
            pending.remove(0);
        }
        self.selected.retain(|id| id != &message.channel_id);
        self.confirmed += 1;
        pending.is_empty()
    }

    /// The error to show for a failed request. A definitive rejection
    /// drops the queued forwards; an unknown outcome keeps them (and their
    /// keys) for Retry.
    pub fn failed(&mut self, error: SendFailure) -> String {
        let rejected = matches!(error.status, Some(400 | 401 | 403 | 404 | 409 | 422));
        let mut detail = error.message;
        if self.pending.is_some() {
            let confirmed = self.confirmed;
            detail = format!(
                "Forwarded to {confirmed} {}. {} {detail}",
                if confirmed == 1 {
                    "destination"
                } else {
                    "destinations"
                },
                if rejected {
                    "Remaining forwards not sent."
                } else {
                    "Remaining forwards not confirmed. Retry checks the same forwards."
                }
            );
        }
        if rejected {
            self.pending = None;
        }
        detail
    }
}

fn invalid() -> ApiError {
    ApiError {
        status: Some(reqwest::StatusCode::BAD_GATEWAY),
        message: "Caper returned an invalid forward.".into(),
        attempts_remaining: None,
        code: None,
    }
}
fn validate(page: &ForwardConversation) -> Result<(), ApiError> {
    if sequence(&page.cursor).is_err()
        || page
            .root
            .as_ref()
            .is_some_and(|root| root.validate().is_err())
        || page
            .messages
            .iter()
            .any(|message| message.validate().is_err())
        || page.source.as_ref().is_some_and(|source| {
            source.message.validate().is_err()
                || page.root.as_ref().is_none_or(|root| {
                    root.id != source.message.id || root.channel_id != source.message.channel_id
                })
        })
    {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(channel: &str, key: &str) -> Message {
        serde_json::from_value(serde_json::json!({
            "id":"source", "channelId":channel, "seq":"89", "clientMessageId":key,
            "author":{"id":"author","name":"Author","isGuest":false},
            "content":{"version":1,"type":"text","text":"original"}, "createdAt":"2026-10-06T12:00:00Z"
        }))
        .unwrap()
    }

    fn failure(status: Option<u16>, message: &str) -> SendFailure {
        SendFailure {
            status,
            message: message.into(),
            code: None,
        }
    }

    fn queued(picker: &Picker) -> Vec<(String, String)> {
        picker.pending.clone().unwrap_or_default()
    }

    #[test]
    fn unknown_result_preserves_retry_capability_and_definitive_rejection_unlocks_intent() {
        let mut picker = Picker::new(message("private", "source-key"));
        picker.loaded(vec![], "same-capability".into());
        picker.set("destination", true);
        picker.set("other", true);
        let Some(Operation::Send { key, .. }) = picker.next("token", " note ", true) else {
            panic!("pressing Forward sends the first destination")
        };
        let keys = queued(&picker);
        assert_eq!(keys[0].1, key);
        let error = picker.failed(failure(None, "Not confirmed"));
        assert!(error.contains("Retry checks the same forwards"));
        assert_eq!(picker.label(), "Retry forwards (2)");
        // Retrying reuses the same keys rather than queueing new ones.
        let Some(Operation::Send {
            key: retried,
            session,
            text,
            ..
        }) = picker.next("token", " note ", true)
        else {
            panic!("retry sends again")
        };
        assert_eq!(
            (retried, session.as_str(), text.as_str()),
            (key, "same-capability", "note")
        );
        assert_eq!(queued(&picker), keys);
        assert_eq!(picker.selected, vec!["destination", "other"]);
        picker.failed(failure(Some(400), "Not sent"));
        assert!(picker.pending.is_none());
        assert_eq!(picker.session.as_deref(), Some("same-capability"));
        assert_eq!(picker.label(), "Forward (2)");
    }

    #[test]
    fn partial_success_removes_only_confirmed_destination_and_keeps_remaining_retry_key() {
        let mut picker = Picker::new(message("private", "source-key"));
        picker.loaded(vec![], "capability".into());
        picker.set("first", true);
        picker.set("second", true);
        picker.next("token", "", true);
        let keys = queued(&picker);
        assert!(!picker.sent(&message("first", &keys[0].1)));
        let error = picker.failed(failure(None, "lost response"));
        assert!(error.contains("Forwarded to 1 destination."));
        assert_eq!(picker.selected, vec!["second"]);
        assert_eq!(queued(&picker), vec![keys[1].clone()]);
        assert!(picker.sent(&message("second", &keys[1].1)));
    }

    #[test]
    fn search_matches_every_term_by_space_and_name() {
        let mut picker = Picker::new(message("private", "source-key"));
        let destination = |id: &str, name: &str, space: &str| ForwardDestination {
            id: id.into(),
            name: name.into(),
            space_name: space.into(),
            direct: false,
        };
        picker.loaded(
            vec![
                destination("third", "general", "Workshop"),
                destination("second", "tomato-soup", "Gamers"),
                destination("first", "general", "Gamers"),
            ],
            "capability".into(),
        );
        let ids = |query| {
            picker
                .visible(query)
                .map(|item| item.id.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(""), vec!["first", "second", "third"]);
        assert_eq!(ids("  #TOMATO-SOUP   GAMERS "), vec!["second"]);
        assert_eq!(ids("general"), vec!["first", "third"]);
    }
}
