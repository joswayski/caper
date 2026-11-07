//! Forwarding, as on the egui desktop: the picker (search, a checkbox per
//! destination, an optional note, "Forward (N)" and "Retry forwards") and
//! the forwarded conversation behind a forward's "View conversation". A
//! reader who can open the original goes straight to it instead. A child of
//! `app`, so it reads and changes the app's state.

use super::App;
use crate::rows::{self, Action, Original};
use crate::ui::label;
use caper_client::forwarding::{Operation, Picker, Response};
use caper_client::model::{ForwardConversation, Message};
use caper_client::worker::{Command, SendFailure};
use gtk::{glib, prelude::*};
use std::rc::Rc;

/// Forward requests, and the open dialog.
#[derive(Default)]
pub(super) struct Forwarding {
    request: u64,
    /// The forward whose conversation is being looked up: nothing shows
    /// until the server says whether you can open the original.
    resolving: Option<Message>,
    dialog: Option<Dialog>,
}

pub(super) struct Dialog {
    window: gtk::Window,
    generation: u64,
    busy: bool,
    error: Option<String>,
    error_label: gtk::Label,
    spinner: gtk::Spinner,
    view: View,
}

enum View {
    Picker {
        picker: Box<Picker>,
        search: gtk::SearchEntry,
        list: gtk::Box,
        /// Each shown destination's checkbox.
        checks: Vec<(String, gtk::CheckButton)>,
        empty: gtk::Label,
        note: gtk::TextView,
        send: gtk::Button,
        retry_load: gtk::Button,
    },
    Conversation {
        wrapper: Box<Message>,
        page: Option<Box<ForwardConversation>>,
        pages: usize,
        oldest: Option<String>,
        /// The wrapper's forward revision this page shows.
        revision: String,
        body: gtk::Box,
    },
}

const NOTE: &str = "Shares this conversation, including future edits, reactions and replies. People in the destination can read and forward it.";

fn text(view: &gtk::TextView) -> String {
    let buffer = view.buffer();
    buffer
        .text(&buffer.start_iter(), &buffer.end_iter(), true)
        .to_string()
}

impl App {
    fn forward_dispatch(&self, generation: u64, operation: Operation) {
        let request = {
            let mut forwarding = self.forwarding.borrow_mut();
            forwarding.request += 1;
            if let Some(dialog) = &mut forwarding.dialog {
                dialog.busy = true;
                dialog.error = None;
            }
            forwarding.request
        };
        self.worker.send(Command::Forward {
            generation,
            request,
            operation,
        });
        self.render_forward();
    }

    /// Rows inside a dialog: files open as usual; people's cards would
    /// open behind the dialog, so they don't.
    fn forward_handler(self: &Rc<Self>) -> rows::Handler {
        let weak = Rc::downgrade(self);
        Rc::new(move |action| {
            if let Some(app) = weak.upgrade()
                && !matches!(action, Action::Profile { .. })
            {
                app.row_action(action, false);
            }
        })
    }

    fn forward_window(self: &Rc<Self>, title: &str) -> (gtk::Window, gtk::Box) {
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title(title)
            .default_width(460)
            .build();
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("dialog-body");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let heading = label(title, &["dialog-title"]);
        heading.set_hexpand(true);
        header.append(&heading);
        let close = crate::app::named::<gtk::Button>("GtkButton", "Close");
        close.set_child(Some(&self.images.icon_image(
            crate::images::Icon::Close,
            "#b9bcbe",
            16,
        )));
        close.add_css_class("flat");
        close.add_css_class("tool");
        let weak = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.close_forward();
            }
        });
        header.append(&close);
        root.append(&header);
        window.set_child(Some(&root));
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(app) = weak.upgrade() {
                let mut forwarding = app.forwarding.borrow_mut();
                forwarding.dialog = None;
                forwarding.request += 1;
            }
            glib::Propagation::Proceed
        });
        // As on web, Escape closes it.
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape
                && let Some(app) = weak.upgrade()
            {
                app.close_forward();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
        (window, root)
    }

    pub(super) fn close_forward(&self) {
        let dialog = {
            let mut forwarding = self.forwarding.borrow_mut();
            forwarding.request += 1;
            forwarding.resolving = None;
            forwarding.dialog.take()
        };
        if let Some(dialog) = dialog {
            dialog.window.destroy();
        }
    }

    /// Opens the picker for `id`, a message in the list or the thread.
    pub(super) fn open_forward(self: &Rc<Self>, id: &str) {
        let (source, token, generation, revealed) = {
            let state = self.state.borrow();
            let Some(source) = state.message(id).cloned() else {
                return;
            };
            let Some(token) = state.token.clone() else {
                return;
            };
            let revealed = self.images.media.revealed.borrow().clone();
            (source, token, state.generation, revealed)
        };
        if !forwardable(&source) {
            return;
        }
        self.close_forward();
        let (window, root) = self.forward_window("Forward message");
        let intro = label(NOTE, &["muted", "small"]);
        intro.set_wrap(true);
        root.append(&intro);
        let shown = source
            .forward
            .as_ref()
            .and_then(|forward| forward.message.as_ref())
            .unwrap_or(&source);
        let preview = Original::new(shown, &self.images, &revealed, false).view(
            &self.images,
            &self.forward_handler(),
            None,
        );
        preview.add_css_class("forward-card");
        root.append(&preview);
        let search: gtk::SearchEntry =
            crate::app::named("GtkSearchEntry", "Find a space, channel or DM");
        search.set_placeholder_text(Some("Find a space, channel or DM"));
        root.append(&search);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        // The list scrolls; the note and Forward stay in view.
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(132)
            .max_content_height(300)
            .propagate_natural_height(true)
            .child(&list)
            .build();
        scroll.add_css_class("forward-list");
        root.append(&scroll);
        let empty = label("Join a channel or start a DM to forward here.", &["muted"]);
        empty.set_visible(false);
        root.append(&empty);
        let note = gtk::TextView::builder()
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(false)
            .build();
        note.add_css_class("forward-note");
        crate::composing::attach(&note, &root);
        let placeholder = label("Add a note (optional)", &["muted"]);
        placeholder.set_can_target(false);
        placeholder.set_valign(gtk::Align::Start);
        placeholder.set_margin_start(10);
        placeholder.set_margin_top(8);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&note));
        overlay.add_overlay(&placeholder);
        let weak = Rc::downgrade(self);
        note.buffer().connect_changed(move |buffer| {
            placeholder.set_visible(buffer.char_count() == 0);
            if let Some(app) = weak.upgrade() {
                app.render_forward();
            }
        });
        root.append(&overlay);
        let spinner = gtk::Spinner::new();
        let error_label = label("", &["error"]);
        error_label.set_wrap(true);
        root.append(&spinner);
        root.append(&error_label);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        let retry_load = gtk::Button::with_label("Retry loading");
        let send = gtk::Button::with_label("Forward (0)");
        send.add_css_class("primary");
        buttons.append(&retry_load);
        buttons.append(&send);
        root.append(&buttons);

        let weak = Rc::downgrade(self);
        search.connect_search_changed(move |_| {
            if let Some(app) = weak.upgrade() {
                app.fill_destinations();
            }
        });
        let (weak, load_token) = (Rc::downgrade(self), token.clone());
        retry_load.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.forward_dispatch(
                    generation,
                    Operation::Destinations {
                        token: load_token.clone(),
                    },
                );
            }
        });
        let (weak, send_token) = (Rc::downgrade(self), token.clone());
        send.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.send_forward(&send_token, true);
            }
        });
        self.forwarding.borrow_mut().dialog = Some(Dialog {
            window: window.clone(),
            generation,
            busy: false,
            error: None,
            error_label,
            spinner,
            view: View::Picker {
                picker: Box::new(Picker::new(source)),
                search: search.clone(),
                list,
                checks: Vec::new(),
                empty,
                note,
                send,
                retry_load,
            },
        });
        window.present();
        search.grab_focus();
        self.forward_dispatch(generation, Operation::Destinations { token });
    }

    /// Sends the next queued forward; `start` queues the selection.
    fn send_forward(&self, token: &str, start: bool) {
        let operation = {
            let mut forwarding = self.forwarding.borrow_mut();
            let Some(Dialog {
                busy: false,
                generation,
                view: View::Picker { picker, note, .. },
                ..
            }) = &mut forwarding.dialog
            else {
                return;
            };
            if start && !picker.ready(false, &text(note)) {
                return;
            }
            picker
                .next(token, &text(note), start)
                .map(|operation| (*generation, operation))
        };
        if let Some((generation, operation)) = operation {
            self.forward_dispatch(generation, operation);
        }
    }

    /// Rebuilds the destination rows for the current search.
    fn fill_destinations(self: &Rc<Self>) {
        let mut forwarding = self.forwarding.borrow_mut();
        let Some(Dialog {
            view:
                View::Picker {
                    picker,
                    search,
                    list,
                    checks,
                    ..
                },
            ..
        }) = &mut forwarding.dialog
        else {
            return;
        };
        while let Some(child) = list.first_child() {
            list.remove(&child);
        }
        checks.clear();
        let locked = picker.pending.is_some();
        for destination in picker.visible(&search.text()) {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("forward-destination");
            let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
            names.set_hexpand(true);
            names.append(&label(
                &format!(
                    "{}{}",
                    if destination.direct { "" } else { "# " },
                    destination.name
                ),
                &["field"],
            ));
            names.append(&label(&destination.space_name, &["muted", "small"]));
            row.append(&names);
            let check = crate::app::named::<gtk::CheckButton>(
                "GtkCheckButton",
                &format!("{} · {}", destination.name, destination.space_name),
            );
            check.set_active(picker.selected.contains(&destination.id));
            check.set_sensitive(!locked);
            row.append(&check);
            let click = gtk::GestureClick::new();
            let toggle = check.clone();
            click.connect_released(move |gesture, _, _, _| {
                if toggle.is_sensitive() {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    toggle.set_active(!toggle.is_active());
                }
            });
            names.add_controller(click);
            let (weak, id) = (Rc::downgrade(self), destination.id.clone());
            check.connect_toggled(move |check| {
                if let Some(app) = weak.upgrade() {
                    if let Some(Dialog {
                        view: View::Picker { picker, .. },
                        ..
                    }) = &mut app.forwarding.borrow_mut().dialog
                    {
                        picker.set(&id, check.is_active());
                    }
                    app.render_forward();
                }
            });
            checks.push((destination.id.clone(), check));
            list.append(&row);
        }
    }

    /// Opens the conversation behind the forward `id`, or its original
    /// when you can read it there.
    pub(super) fn view_forward(self: &Rc<Self>, id: &str) {
        let (wrapper, token, generation) = {
            let state = self.state.borrow();
            let Some(wrapper) = state.message(id).cloned() else {
                return;
            };
            let Some(token) = state.token.clone() else {
                return;
            };
            (wrapper, token, state.generation)
        };
        if self.forwarding.borrow().resolving.is_some() {
            return;
        }
        self.close_forward();
        self.forwarding.borrow_mut().resolving = Some(wrapper.clone());
        self.forward_dispatch(
            generation,
            Operation::Conversation {
                token,
                wrapper: Box::new(wrapper),
                pages: 1,
                oldest: None,
            },
        );
    }

    fn open_conversation_dialog(self: &Rc<Self>, wrapper: Message, page: Box<ForwardConversation>) {
        let generation = self.state.borrow().generation;
        let (window, root) = self.forward_window("Forwarded conversation");
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .min_content_height(360)
            .max_content_height(600)
            .propagate_natural_height(true)
            .child(&body)
            .build();
        root.append(&scroll);
        let spinner = gtk::Spinner::new();
        let error_label = label("", &["error"]);
        error_label.set_wrap(true);
        root.append(&spinner);
        root.append(&error_label);
        let revision = wrapper
            .forward
            .as_ref()
            .map(|forward| forward.seq.clone())
            .unwrap_or_default();
        self.forwarding.borrow_mut().dialog = Some(Dialog {
            window: window.clone(),
            generation,
            busy: false,
            error: None,
            error_label,
            spinner,
            view: View::Conversation {
                wrapper: Box::new(wrapper),
                oldest: page.messages.first().map(|message| message.seq.clone()),
                page: Some(page),
                pages: 1,
                revision,
                body,
            },
        });
        self.fill_conversation();
        window.present();
    }

    fn fill_conversation(self: &Rc<Self>) {
        let handler = self.forward_handler();
        let revealed = self.images.media.revealed.borrow().clone();
        let token = self.state.borrow().token.clone().unwrap_or_default();
        let forwarding = self.forwarding.borrow();
        let Some(Dialog {
            view:
                View::Conversation {
                    wrapper,
                    page,
                    pages,
                    oldest,
                    body,
                    ..
                },
            error,
            busy,
            generation,
            ..
        }) = &forwarding.dialog
        else {
            return;
        };
        while let Some(child) = body.first_child() {
            body.remove(&child);
        }
        let original = |message: &Message| {
            let view = Original::new(message, &self.images, &revealed, false).view(
                &self.images,
                &handler,
                None,
            );
            view.add_css_class("forward-message");
            view
        };
        match page {
            Some(page) => {
                if let Some(root) = &page.root {
                    body.append(&original(root));
                }
                if page.has_more {
                    let older = gtk::Button::with_label("Load older replies");
                    older.set_sensitive(!busy);
                    older.set_halign(gtk::Align::Start);
                    let (weak, operation) = (
                        Rc::downgrade(self),
                        (
                            token.clone(),
                            wrapper.clone(),
                            *pages + 1,
                            oldest.clone(),
                            *generation,
                        ),
                    );
                    older.connect_clicked(move |_| {
                        let Some(app) = weak.upgrade() else {
                            return;
                        };
                        let (token, wrapper, pages, oldest, generation) = operation.clone();
                        if let Some(Dialog {
                            view: View::Conversation { pages: shown, .. },
                            ..
                        }) = &mut app.forwarding.borrow_mut().dialog
                        {
                            *shown = pages;
                        }
                        app.forward_dispatch(
                            generation,
                            Operation::Conversation {
                                token,
                                wrapper,
                                pages,
                                oldest,
                            },
                        );
                    });
                    body.append(&older);
                }
                for message in &page.messages {
                    body.append(&original(message));
                }
                if page.root.is_none() {
                    body.append(&label("Original conversation unavailable.", &["muted"]));
                } else if page.messages.is_empty() {
                    body.append(&label("No replies yet.", &["muted"]));
                }
            }
            None => {
                if error.is_none()
                    && let Some(root) = wrapper
                        .forward
                        .as_ref()
                        .and_then(|forward| forward.message.as_ref())
                {
                    body.append(&original(root));
                }
            }
        }
        if error.is_some() {
            let retry = gtk::Button::with_label("Retry");
            retry.set_sensitive(!busy);
            retry.set_halign(gtk::Align::Start);
            let (weak, operation) = (
                Rc::downgrade(self),
                (token, wrapper.clone(), *pages, oldest.clone(), *generation),
            );
            retry.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    let (token, wrapper, pages, oldest, generation) = operation.clone();
                    app.forward_dispatch(
                        generation,
                        Operation::Conversation {
                            token,
                            wrapper,
                            pages,
                            oldest,
                        },
                    );
                }
            });
            body.append(&retry);
        }
    }

    /// The picker's controls follow its state.
    pub(super) fn render_forward(&self) {
        let forwarding = self.forwarding.borrow();
        let Some(dialog) = &forwarding.dialog else {
            return;
        };
        dialog.spinner.set_visible(dialog.busy);
        dialog.spinner.set_spinning(dialog.busy);
        dialog
            .error_label
            .set_label(dialog.error.as_deref().unwrap_or_default());
        dialog.error_label.set_visible(dialog.error.is_some());
        if let View::Picker {
            picker,
            search,
            checks,
            empty,
            note,
            send,
            retry_load,
            ..
        } = &dialog.view
        {
            let locked = picker.pending.is_some();
            search.set_sensitive(!locked);
            note.set_sensitive(!locked);
            for (_, check) in checks {
                check.set_sensitive(!locked);
            }
            empty.set_visible(!dialog.busy && picker.destinations.is_empty());
            retry_load.set_visible(dialog.error.is_some() && picker.destinations.is_empty());
            send.set_label(&picker.label());
            send.set_sensitive(picker.ready(dialog.busy, &text(note)));
        }
    }

    /// A forward request finished.
    pub(super) fn forward_result(
        self: &Rc<Self>,
        generation: u64,
        request: u64,
        result: Result<Response, SendFailure>,
    ) {
        if request != self.forwarding.borrow().request {
            return;
        }
        let resolving = self.forwarding.borrow_mut().resolving.take();
        if let Some(wrapper) = resolving {
            if generation != self.state.borrow().generation {
                return;
            }
            match result {
                Ok(Response::Conversation(page)) => match page.source.clone() {
                    Some(source) => self.open_forward_source(source),
                    None => self.open_conversation_dialog(wrapper, page),
                },
                Err(error) => {
                    self.state.borrow_mut().error = Some(error.message);
                    self.render();
                }
                Ok(_) => {}
            }
            return;
        }
        let mut sent = None;
        let mut next = None;
        let mut close = false;
        let mut destinations = false;
        {
            let mut forwarding = self.forwarding.borrow_mut();
            let Some(dialog) = &mut forwarding.dialog else {
                return;
            };
            if dialog.generation != generation {
                return;
            }
            dialog.busy = false;
            match (result, &mut dialog.view) {
                (Ok(Response::Sent(message)), View::Picker { picker, .. }) => {
                    close = picker.sent(&message);
                    next = Some(());
                    sent = Some(*message);
                }
                (Ok(Response::Destinations(values, capability)), View::Picker { picker, .. }) => {
                    picker.loaded(values, capability);
                    destinations = true;
                }
                (Ok(Response::Conversation(value)), View::Conversation { page, oldest, .. }) => {
                    *oldest = value.messages.first().map(|message| message.seq.clone());
                    *page = Some(value);
                }
                (Err(error), View::Picker { picker, .. }) => {
                    dialog.error = Some(picker.failed(error));
                    destinations = true;
                }
                (Err(error), View::Conversation { page, .. }) => {
                    *page = None;
                    dialog.error = Some(error.message);
                }
                _ => {}
            }
        }
        if let Some(message) = sent {
            let mut state = self.state.borrow_mut();
            if state.channel.as_deref() == Some(&message.channel_id) {
                let _ = state.timeline.merge_sent(message);
                state.layout += 1;
            }
        }
        if close {
            self.close_forward();
            self.render();
            return;
        }
        if destinations {
            self.fill_destinations();
        }
        self.fill_conversation();
        self.render_forward();
        if next.is_some() {
            let token = self.state.borrow().token.clone();
            if let Some(token) = token {
                self.send_forward(&token, false);
            }
        }
        self.render();
    }

    /// An open forwarded conversation follows its forward: it closes when
    /// the forward goes away and reloads when the original changes.
    pub(super) fn follow_forward(self: &Rc<Self>) {
        let refresh = {
            let state = self.state.borrow();
            let mut forwarding = self.forwarding.borrow_mut();
            let Some(Dialog {
                view:
                    View::Conversation {
                        wrapper,
                        revision,
                        pages,
                        oldest,
                        ..
                    },
                generation,
                ..
            }) = &mut forwarding.dialog
            else {
                return;
            };
            match state.message(&wrapper.id) {
                Some(current)
                    if current.deleted_at.is_none()
                        && current
                            .forward
                            .as_ref()
                            .is_some_and(|forward| forward.message.is_some()) =>
                {
                    let next = current
                        .forward
                        .as_ref()
                        .map(|forward| forward.seq.clone())
                        .unwrap_or_default();
                    (&next != revision).then(|| {
                        *revision = next;
                        **wrapper = current.clone();
                        Some((
                            *generation,
                            Operation::Conversation {
                                token: state.token.clone().unwrap_or_default(),
                                wrapper: wrapper.clone(),
                                pages: *pages,
                                oldest: oldest.clone(),
                            },
                        ))
                    })
                }
                // Messages still loading are not gone.
                None if state.status == "Loading…" => None,
                _ => Some(None),
            }
        };
        match refresh {
            Some(Some((generation, operation))) => self.forward_dispatch(generation, operation),
            Some(None) => self.close_forward(),
            None => {}
        }
    }

    /// Opens a forward's original where it lives: its thread, after
    /// moving to its channel when it is elsewhere.
    fn open_forward_source(self: &Rc<Self>, source: caper_client::model::ForwardSource) {
        // A reply opens its root's thread, focused on the reply.
        let (root, focus) = match source.message.thread_root_id.clone() {
            Some(root) => (root, Some(source.message.id.clone())),
            None => (source.message.id.clone(), None),
        };
        let here = self.state.borrow().channel.as_deref() == Some(&source.message.channel_id);
        if here {
            self.open_thread_at(root, focus);
            return;
        }
        let channel = source.message.channel_id.clone();
        match source.space_id {
            Some(space) => {
                {
                    let mut state = self.state.borrow_mut();
                    self.navigate(&mut state, Some(space), Some(channel.clone()));
                    state.forward_focus = Some((state.navigation, channel, root, focus));
                }
                self.render();
            }
            None => {
                self.open_direct(channel.clone());
                let mut state = self.state.borrow_mut();
                state.forward_focus = Some((state.navigation, channel, root, focus));
            }
        }
    }
}

/// As on web, a forward whose original is unavailable cannot be forwarded.
pub(super) fn forwardable(message: &Message) -> bool {
    !message.is_deleted()
        && message
            .forward
            .as_ref()
            .is_none_or(|forward| forward.message.is_some())
}
