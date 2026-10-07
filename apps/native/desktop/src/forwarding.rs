use crate::api::{Api, ApiError};
use crate::model::{ForwardConversation, ForwardDestination, Message, sequence};
use crate::worker::{Command, SendFailure, Worker};
use eframe::egui::{self, RichText};

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
        })
    }
}

fn invalid() -> ApiError {
    ApiError {
        status: Some(reqwest::StatusCode::BAD_GATEWAY),
        message: "Caper returned an invalid forward.".into(),
        attempts_remaining: None,
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
    {
        return Err(invalid());
    }
    Ok(())
}

enum View {
    Picker {
        source: Box<Message>,
        destinations: Vec<ForwardDestination>,
        selected: Option<String>,
        search: String,
        text: String,
        key: Option<String>,
        session: Option<String>,
    },
    Conversation {
        wrapper: Box<Message>,
        page: Option<Box<ForwardConversation>>,
        pages: usize,
        oldest: Option<String>,
        revision: String,
    },
}

#[derive(Default)]
pub struct Forwarding {
    view: Option<View>,
    request: u64,
    busy: bool,
    error: Option<String>,
}

impl Forwarding {
    pub fn close(&mut self) {
        self.view = None;
        self.request += 1;
        self.busy = false;
        self.error = None;
    }
    fn dispatch(&mut self, worker: &Worker, generation: u64, operation: Operation) {
        self.request += 1;
        self.busy = true;
        self.error = None;
        worker.send(Command::Forward {
            generation,
            request: self.request,
            operation,
        });
    }
    pub fn picker(&mut self, worker: &Worker, generation: u64, token: String, source: Message) {
        self.view = Some(View::Picker {
            source: Box::new(source),
            destinations: vec![],
            selected: None,
            search: String::new(),
            text: String::new(),
            key: None,
            session: None,
        });
        self.dispatch(worker, generation, Operation::Destinations { token });
    }
    pub fn conversation(
        &mut self,
        worker: &Worker,
        generation: u64,
        token: String,
        wrapper: Message,
    ) {
        let revision = wrapper
            .forward
            .as_ref()
            .map(|forward| forward.seq.clone())
            .unwrap_or_default();
        self.view = Some(View::Conversation {
            wrapper: Box::new(wrapper.clone()),
            page: None,
            pages: 1,
            oldest: None,
            revision,
        });
        self.dispatch(
            worker,
            generation,
            Operation::Conversation {
                token,
                wrapper: Box::new(wrapper),
                pages: 1,
                oldest: None,
            },
        );
    }
    pub fn receive(
        &mut self,
        request: u64,
        result: Result<Response, SendFailure>,
    ) -> Option<Message> {
        if request != self.request || self.view.is_none() {
            return None;
        }
        self.busy = false;
        match result {
            Ok(Response::Sent(message)) => {
                self.close();
                return Some(*message);
            }
            Ok(Response::Destinations(mut values, capability)) => {
                values.sort_by_key(|item| format!("{} {}", item.space_name, item.name));
                if let Some(View::Picker {
                    destinations,
                    session,
                    ..
                }) = &mut self.view
                {
                    *destinations = values;
                    *session = Some(capability);
                }
            }
            Ok(Response::Conversation(value)) => {
                if let Some(View::Conversation { page, oldest, .. }) = &mut self.view {
                    *oldest = value.messages.first().map(|message| message.seq.clone());
                    *page = Some(value);
                }
            }
            Err(error) => {
                if let Some(View::Picker { key, .. }) = &mut self.view
                    && matches!(error.status, Some(400 | 401 | 403 | 404 | 409 | 422))
                {
                    *key = None;
                }
                if let Some(View::Conversation { page, .. }) = &mut self.view {
                    *page = None;
                }
                self.error = Some(error.message);
            }
        }
        None
    }
    pub fn show(
        &mut self,
        context: &egui::Context,
        worker: &Worker,
        generation: u64,
        token: Option<&str>,
        messages: &[Message],
        textures: &mut crate::emoji::Textures,
    ) {
        let Some(token) = token else {
            self.close();
            return;
        };
        if let Some(View::Conversation {
            wrapper,
            revision,
            pages,
            oldest,
            ..
        }) = &mut self.view
            && let Some(current) = messages.iter().find(|message| message.id == wrapper.id)
        {
            let next = current
                .forward
                .as_ref()
                .map(|forward| forward.seq.clone())
                .unwrap_or_default();
            if &next != revision {
                *revision = next;
                **wrapper = current.clone();
                let operation = Operation::Conversation {
                    token: token.into(),
                    wrapper: wrapper.clone(),
                    pages: *pages,
                    oldest: oldest.clone(),
                };
                self.dispatch(worker, generation, operation);
            }
        }
        let Some(view) = &mut self.view else {
            return;
        };
        let mut operation = None;
        let mut open = true;
        let title = match view {
            View::Picker { .. } => "Forward message",
            View::Conversation { .. } => "Forwarded conversation",
        };
        egui::Window::new(title).id(egui::Id::new("live-forward")).open(&mut open).collapsible(false).resizable(true).default_width(460.0).max_height(context.content_rect().height() - 48.0).show(context, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                match view {
                    View::Picker { source, destinations, selected, search, text, key, session } => {
                        ui.label(RichText::new("Shares this conversation live, including future edits, reactions and replies. People in the destination can read and forward it.").size(12.0).color(crate::MUTED));
                        original(ui, source.forward.as_ref().and_then(|forward| forward.message.as_ref()).unwrap_or(source), textures);
                        ui.add_enabled(key.is_none(), egui::TextEdit::singleline(search).hint_text("Find a channel or DM"));
                        for destination in destinations.iter().filter(|item| format!("{} {}", item.space_name, item.name).to_lowercase().contains(&search.to_lowercase())) {
                            if ui.add_enabled(key.is_none(), egui::Button::new(format!("{}{} · {}", if destination.direct { "" } else { "# " }, destination.name, destination.space_name)).selected(selected.as_ref() == Some(&destination.id))).clicked() { *selected = Some(destination.id.clone()); }
                        }
                        ui.add_enabled(key.is_none(), egui::TextEdit::multiline(text).hint_text("Add a note (optional)").desired_rows(2));
                        if !self.busy && destinations.is_empty() { ui.label("Join a channel or start a DM to forward here."); }
                        if ui.add_enabled(!self.busy && selected.is_some() && session.is_some() && text.chars().count() <= 4000, egui::Button::new(if key.is_some() { "Retry forward" } else { "Forward" })).clicked() {
                            let key = key.get_or_insert_with(|| uuid::Uuid::new_v4().to_string()).clone();
                            operation = Some(Operation::Send { token: token.into(), session: session.clone().unwrap(), source: source.clone(), destination: selected.clone().unwrap(), key, text: text.trim().into() });
                        }
                        if self.error.is_some() && destinations.is_empty() && ui.button("Retry loading").clicked() { operation = Some(Operation::Destinations { token: token.into() }); }
                    }
                    View::Conversation { wrapper, page, pages, oldest, .. } => {
                        ui.label(RichText::new("Live · Read-only original. Replies to the forward stay in the destination.").size(12.0).color(crate::MUTED));
                        if let Some(page) = page {
                            if let Some(root) = &page.root { original(ui, root, textures); }
                            if page.has_more && ui.add_enabled(!self.busy, egui::Button::new("Load older replies")).clicked() { *pages += 1; operation = Some(Operation::Conversation { token: token.into(), wrapper: wrapper.clone(), pages: *pages, oldest: oldest.clone() }); }
                            for message in &page.messages { original(ui, message, textures); }
                            if page.root.is_none() { ui.label("Original conversation unavailable."); }
                            else if page.messages.is_empty() { ui.label("No replies yet."); }
                        }
                        if self.error.is_some() && ui.add_enabled(!self.busy, egui::Button::new("Retry")).clicked() { operation = Some(Operation::Conversation { token: token.into(), wrapper: wrapper.clone(), pages: *pages, oldest: oldest.clone() }); }
                    }
                }
                if self.busy { ui.spinner(); }
                if let Some(error) = &self.error { ui.colored_label(crate::ERROR, error); }
            });
        });
        if !open || context.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.close();
        } else if let Some(operation) = operation {
            self.dispatch(worker, generation, operation);
        }
    }
}

pub fn original(ui: &mut egui::Ui, message: &Message, textures: &mut crate::emoji::Textures) {
    ui.add_space(8.0);
    ui.label(RichText::new(&message.author.name).strong().size(13.0));
    ui.label(&message.content.text);
    if message.edited_at.is_some() {
        ui.label(RichText::new("edited").size(10.0).color(crate::MUTED));
    }
    ui.horizontal_wrapped(|ui| {
        for reaction in &message.reactions {
            if let Some(entry) = crate::emoji::find(&reaction.emoji) {
                let image = textures.image(ui, entry, 18.0).alt_text(&reaction.emoji);
                ui.add(image);
            } else {
                ui.label(&reaction.emoji);
            }
            ui.label(reaction.author_ids.len().to_string());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_result_preserves_retry_capability_and_definitive_rejection_unlocks_intent() {
        let source: Message = serde_json::from_value(serde_json::json!({
            "id":"source", "channelId":"private", "seq":"89", "clientMessageId":"source-key",
            "author":{"id":"author","name":"Author","isGuest":false},
            "content":{"version":1,"type":"text","text":"original"}, "createdAt":"2026-10-06T12:00:00Z"
        })).unwrap();
        let mut state = Forwarding {
            view: Some(View::Picker {
                source: Box::new(source),
                destinations: vec![],
                selected: Some("destination".into()),
                search: String::new(),
                text: "note".into(),
                key: Some("retry-key".into()),
                session: Some("same-capability".into()),
            }),
            request: 7,
            busy: true,
            error: None,
        };
        state.receive(
            7,
            Err(SendFailure {
                status: None,
                message: "Not confirmed".into(),
            }),
        );
        let Some(View::Picker {
            key,
            session,
            text,
            selected,
            ..
        }) = &state.view
        else {
            panic!("picker closed")
        };
        assert_eq!(key.as_deref(), Some("retry-key"));
        assert_eq!(session.as_deref(), Some("same-capability"));
        assert_eq!(text, "note");
        assert_eq!(selected.as_deref(), Some("destination"));
        state.receive(
            6,
            Err(SendFailure {
                status: Some(400),
                message: "stale failure".into(),
            }),
        );
        assert!(matches!(
            &state.view,
            Some(View::Picker { key: Some(_), .. })
        ));
        state.receive(
            7,
            Err(SendFailure {
                status: Some(400),
                message: "Not sent".into(),
            }),
        );
        assert!(matches!(
            &state.view,
            Some(View::Picker {
                key: None,
                session: Some(_),
                ..
            })
        ));
    }
}
