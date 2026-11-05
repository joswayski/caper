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
