use crate::model::{ForwardConversation, Message};
use crate::worker::{Command, SendFailure, Worker};
pub use caper_client::forwarding::{Operation, Picker, Response};
use eframe::egui::{self, RichText};

enum View {
    Picker {
        picker: Box<Picker>,
        search: String,
        text: String,
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
    resolving: bool,
    error: Option<String>,
    /// The frame the current view first showed in: the click that opened it
    /// is not a click outside it.
    shown_since: Option<u64>,
}

impl Forwarding {
    /// Whether the forward picker or a forwarded conversation is showing.
    pub fn is_open(&self) -> bool {
        self.view.is_some()
    }

    pub fn close(&mut self) {
        self.view = None;
        self.shown_since = None;
        self.request += 1;
        self.busy = false;
        self.resolving = false;
        self.error = None;
    }
    pub fn resolving(&self, request: u64) -> bool {
        self.resolving && request == self.request
    }
    pub fn cancel_conversation(&mut self) {
        if matches!(self.view, Some(View::Conversation { .. })) {
            self.close();
        }
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
        self.resolving = false;
        self.shown_since = None;
        self.view = Some(View::Picker {
            picker: Box::new(Picker::new(source)),
            search: String::new(),
            text: String::new(),
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
        if self.resolving {
            return;
        }
        self.resolving = true;
        self.shown_since = None;
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
        self.resolving = false;
        match result {
            Ok(Response::Sent(message)) => {
                if let Some(View::Picker { picker, .. }) = &mut self.view
                    && picker.sent(&message)
                {
                    self.close();
                }
                return Some(*message);
            }
            Ok(Response::Destinations(values, capability)) => {
                if let Some(View::Picker { picker, .. }) = &mut self.view {
                    picker.loaded(values, capability);
                }
            }
            Ok(Response::Conversation(value)) => {
                if let Some(View::Conversation { page, oldest, .. }) = &mut self.view {
                    *oldest = value.messages.first().map(|message| message.seq.clone());
                    *page = Some(value);
                }
            }
            Err(error) => {
                let detail = match &mut self.view {
                    Some(View::Picker { picker, .. }) => picker.failed(error),
                    Some(View::Conversation { page, .. }) => {
                        *page = None;
                        error.message
                    }
                    None => error.message,
                };
                self.error = Some(detail);
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
        // Resolve access before displaying anything; authorized readers navigate.
        if self.resolving {
            return;
        }
        if let Some(View::Conversation { wrapper, .. }) = &self.view
            && messages
                .iter()
                .find(|message| message.id == wrapper.id)
                .is_none_or(|message| {
                    message.deleted_at.is_some()
                        || message
                            .forward
                            .as_ref()
                            .is_none_or(|forward| forward.message.is_none())
                })
        {
            self.close();
            return;
        }
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
        let frame = context.cumulative_frame_nr();
        let fresh = *self.shown_since.get_or_insert(frame) == frame;
        let mut operation = None;
        let mut open = true;
        let title = match view {
            View::Picker { .. } => "Forward message",
            View::Conversation { .. } => "Forwarded conversation",
        };
        let shown = egui::Window::new(title).id(egui::Id::unique("live-forward")).open(&mut open).collapsible(false).resizable(true).default_width(460.0).max_height(context.content_rect().height() - 48.0).show(context, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                match view {
                    View::Picker { picker, search, text } => {
                        let pending = &picker.pending;
                        ui.label(RichText::new("Shares this conversation, including future edits, reactions and replies. People in the destination can read and forward it.").size(12.0).color(crate::MUTED));
                        original(ui, picker.source.forward.as_ref().and_then(|forward| forward.message.as_ref()).unwrap_or(&picker.source), textures);
                        ui.add_enabled(pending.is_none(), egui::TextEdit::singleline(search).hint_text("Find a space, channel or DM"));
                        let mut changes = vec![];
                        for destination in picker.visible(search) {
                            let mut checked = picker.selected.contains(&destination.id);
                            ui.add_enabled_ui(pending.is_none(), |ui| {
                                ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 44.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    let checkbox = ui.checkbox(&mut checked, "");
                                    let row = ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 44.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                                        ui.spacing_mut().item_spacing.y = 2.0;
                                        ui.label(RichText::new(format!("{}{}", if destination.direct { "" } else { "# " }, destination.name)).strong());
                                        ui.label(RichText::new(&destination.space_name).small().color(crate::MUTED));
                                    }).response.interact(egui::Sense::click());
                                    if row.clicked() { checked = !checked; }
                                    if checkbox.changed() || row.clicked() {
                                        changes.push((destination.id.clone(), checked));
                                    }
                                    checkbox.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), checked, format!("{} · {}", destination.name, destination.space_name)));
                                    row.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, ui.is_enabled(), checked, format!("{} · {}", destination.name, destination.space_name)));
                                });
                            });
                        }
                        for (id, checked) in changes { picker.set(&id, checked); }
                        let pending = picker.pending.is_some();
                        let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, width| {
                            crate::draft_layout(ui, text.as_str(), width, &[], crate::mentions::Specials::default(), egui::FontId::proportional(14.0))
                        };
                        let note_id = ui.make_persistent_id("forward-note");
                        crate::formatting::prepare(ui, text, note_id, !pending);
                        let note = ui.add_enabled(!pending, egui::TextEdit::multiline(&mut *text).id(note_id).layouter(&mut layouter).hint_text("Add a note (optional)").desired_rows(2));
                        crate::formatting::controls(ui, text, &note, !pending);
                        if !self.busy && picker.destinations.is_empty() { ui.label("Join a channel or start a DM to forward here."); }
                        // Web's eggshell primary, neutral while disabled.
                        let send = crate::toned(ui, crate::PRIMARY_TONE, picker.ready(self.busy, text), egui::Button::new(picker.label())).clicked();
                        if !self.busy && (send || self.error.is_none()) {
                            operation = picker.next(token, text, send);
                        }
                        if self.error.is_some() && picker.destinations.is_empty() && ui.button("Retry loading").clicked() { operation = Some(Operation::Destinations { token: token.into() }); }
                    }
                    View::Conversation { wrapper, page, pages, oldest, .. } => {
                        if let Some(page) = page {
                            if let Some(root) = &page.root { original(ui, root, textures); }
                            if page.has_more && ui.add_enabled(!self.busy, egui::Button::new("Load older replies")).clicked() { *pages += 1; operation = Some(Operation::Conversation { token: token.into(), wrapper: wrapper.clone(), pages: *pages, oldest: oldest.clone() }); }
                            for message in &page.messages { original(ui, message, textures); }
                            if page.root.is_none() { ui.label("Original conversation unavailable."); }
                            else if page.messages.is_empty() { ui.label("No replies yet."); }
                        } else if self.error.is_none() && let Some(root) = wrapper.forward.as_ref().and_then(|forward| forward.message.as_ref()) {
                            original(ui, root, textures);
                        }
                        if self.error.is_some() && ui.add_enabled(!self.busy, egui::Button::new("Retry")).clicked() { operation = Some(Operation::Conversation { token: token.into(), wrapper: wrapper.clone(), pages: *pages, oldest: oldest.clone() }); }
                    }
                }
                if self.busy && matches!(view, View::Picker { .. }) { ui.spinner(); }
                if let Some(error) = &self.error { ui.colored_label(crate::ERROR, error); }
            });
        });
        if let Some(shown) = &shown {
            crate::motion::overlay(
                context,
                shown.response.layer_id,
                crate::motion::BASE,
                8.0,
                false,
            );
        }
        // As on web, Escape or a click outside the dialog closes it.
        let outside = !fresh && shown.is_some_and(|shown| shown.response.clicked_elsewhere());
        if !open
            || outside
            || context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
        {
            self.close();
        } else if let Some(operation) = operation {
            self.dispatch(worker, generation, operation);
        }
    }
}

pub fn original(ui: &mut egui::Ui, message: &Message, textures: &mut crate::emoji::Textures) {
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        // Web's 24 px author avatar: their picture, else their mascot.
        let (avatar, _) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::hover());
        crate::paint_avatar(
            ui,
            avatar,
            &message.author.name,
            message.author.avatar_id,
            message.author.picture(),
        );
        ui.label(RichText::new(&message.author.name).strong().size(13.0));
        if message.author.admin {
            crate::admin_badge(ui);
        }
    });
    if message.is_deleted() {
        ui.label(
            RichText::new("This message was deleted.")
                .italics()
                .color(crate::MUTED),
        );
        return;
    }
    crate::linked_text(ui, &message.id, &message.content.text);
    if message.content.text.is_empty()
        && message
            .content
            .attachments
            .iter()
            .any(|attachment| attachment.voice)
    {
        ui.label(RichText::new("Voice message").color(crate::MUTED));
    }
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
    use crate::TestPass as _;

    #[test]
    fn picker_keeps_same_space_rows_compact_and_search_preserves_hidden_selections() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let worker = Worker::new(
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            crate::host::Repaint(context.clone()),
        );
        let source = serde_json::from_value(serde_json::json!({
            "id":"source", "channelId":"private", "seq":"1", "clientMessageId":"source-key",
            "author":{"id":"author","name":"Author","isGuest":false},
            "content":{"version":1,"type":"text","text":"original"}, "createdAt":"2026-10-06T12:00:00Z"
        })).unwrap();
        let mut state = Forwarding {
            view: Some(View::Picker {
                picker: Box::new(Picker {
                    source: Box::new(source),
                    destinations: vec![
                        crate::model::ForwardDestination {
                            id: "first".into(),
                            name: "general".into(),
                            space_name: "Gamers".into(),
                            direct: false,
                        },
                        crate::model::ForwardDestination {
                            id: "second".into(),
                            name: "tomato-soup".into(),
                            space_name: "Gamers".into(),
                            direct: false,
                        },
                        crate::model::ForwardDestination {
                            id: "third".into(),
                            name: "general".into(),
                            space_name: "Workshop".into(),
                            direct: false,
                        },
                    ],
                    selected: vec!["first".into(), "third".into()],
                    pending: None,
                    confirmed: 0,
                    session: Some("capability".into()),
                }),
                search: String::new(),
                text: String::new(),
            }),
            ..Default::default()
        };
        let mut textures = crate::emoji::Textures::default();
        for query in ["", "  #TOMATO-SOUP   GAMERS ", "gamers #tomato-soup"] {
            if let Some(View::Picker { search, .. }) = &mut state.view {
                *search = query.into();
            }
            let mut output = egui::FullOutput::default();
            for _ in 0..3 {
                output = context.test_pass(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1440.0, 900.0),
                        )),
                        ..Default::default()
                    },
                    |context| state.show(context, &worker, 0, Some("token"), &[], &mut textures),
                );
            }
            let labels: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text),
                    _ => None,
                })
                .collect();
            let names: Vec<_> = labels
                .iter()
                .filter(|text| text.galley.job.text.starts_with("# "))
                .collect();
            assert!(
                labels
                    .iter()
                    .any(|text| text.galley.job.text == "Forward (2)")
            );
            if query.is_empty() {
                assert_eq!(
                    names.len(),
                    3,
                    "All rows must fit above the note and send controls"
                );
                let nodes = &output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes;
                for label in [
                    "general · Gamers",
                    "tomato-soup · Gamers",
                    "general · Workshop",
                ] {
                    assert!(
                        nodes.iter().any(|(_, node)| node.label() == Some(label)),
                        "Destination checkboxes need distinct accessible names: {label}"
                    );
                }
                for pair in names.windows(2) {
                    assert_eq!(pair[0].pos.x, pair[1].pos.x);
                    assert!(
                        (30.0..=52.0).contains(&(pair[1].pos.y - pair[0].pos.y)),
                        "Rows must not consume the window's remaining height"
                    );
                }
                assert_eq!(
                    labels
                        .iter()
                        .filter(|text| text.galley.job.text == "Gamers")
                        .count(),
                    2
                );
            } else {
                assert_eq!(names.len(), 1);
                assert_eq!(names[0].galley.job.text, "# tomato-soup");
                assert!(labels.iter().all(|text| text.galley.job.text != "Workshop"));
            }
        }
    }
}
