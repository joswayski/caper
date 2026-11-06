//! Message history, as on the egui desktop: "(edited)" and the message
//! menu's "Message history" open the retained versions. The newest change
//! shows as Previous and Current versions, compared word by word; "View
//! previous versions" picks an earlier one, and older pages load on
//! request. A newer edit reloads the list. A child of `app`.

use super::{App, State, clear};
use crate::ui::label;
use caper_client::edits::{diff, merge_versions, version_label};
use caper_client::model::{Message, MessageVersion, MessageVersions};
use caper_client::worker::Command;
use gtk::{glib, prelude::*};
use std::rc::Rc;

pub(super) struct History {
    message: Message,
    versions: Vec<MessageVersion>,
    /// The earlier version picked under "View previous versions".
    selected: Option<u32>,
    loading: bool,
    older: bool,
    more: bool,
    error: Option<String>,
    request: u64,
    /// The message's revision when the list was asked for.
    requested_revision: u32,
}

/// The open window's widgets and what they last showed.
pub(super) struct HistoryWindow {
    window: gtk::Window,
    body: gtk::Box,
    shown: Option<Shown>,
}

#[derive(PartialEq)]
struct Shown {
    revisions: Vec<u32>,
    selected: Option<u32>,
    loading: bool,
    more: bool,
    error: Option<String>,
}

impl State {
    fn history_message(&self, id: &str) -> Option<&Message> {
        self.message(id).or_else(|| {
            self.timeline
                .pinned_messages()
                .find(|message| message.id == id)
        })
    }
}

/// One side of a comparison: its heading, the version's name and time, and
/// the text with changed words highlighted.
fn side(heading: &str, version: &MessageVersion, tokens: &[(String, bool)], old: bool) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.set_hexpand(true);
    column.set_homogeneous(false);
    let title = label(heading, &["author"]);
    title.set_xalign(0.0);
    column.append(&title);
    let when = label(&version_label(version), &["muted", "small"]);
    when.set_xalign(0.0);
    column.append(&when);
    let markup: String = tokens
        .iter()
        .map(|(text, changed)| {
            let text = glib::markup_escape_text(text);
            if *changed {
                // Removed words in terracotta, added ones in caper green.
                format!(
                    "<span background=\"{}\" bgalpha=\"30%\">{text}</span>",
                    if old { "#b64d32" } else { "#637a43" }
                )
            } else {
                text.to_string()
            }
        })
        .collect();
    let text = gtk::Label::new(None);
    text.set_markup(&markup);
    text.set_wrap(true);
    text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    text.set_xalign(0.0);
    text.set_yalign(0.0);
    text.add_css_class("version-text");
    column.append(&text);
    column
}

fn comparison(before: &MessageVersion, after: &MessageVersion, current: bool) -> gtk::Box {
    let (old, new) = diff(&before.content.text, &after.content.text);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.set_homogeneous(true);
    let (left, right) = if current {
        ("Previous version", "Current version")
    } else {
        ("Before", "After")
    };
    row.append(&side(left, before, &old, true));
    row.append(&side(right, after, &new, false));
    row
}

impl App {
    pub(super) fn open_history(self: &Rc<Self>, id: &str) {
        {
            let mut state = self.state.borrow_mut();
            let Some(message) = state.history_message(id).cloned() else {
                return;
            };
            if message.forward.is_some() || message.is_deleted() {
                return;
            }
            state.history = Some(History {
                requested_revision: message.revision,
                message,
                versions: Vec::new(),
                selected: None,
                loading: false,
                older: false,
                more: false,
                error: None,
                request: 0,
            });
            self.load_versions(&mut state, false);
        }
        self.close_history();
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title("Message history")
            .default_width(840)
            .default_height(420)
            .build();
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("dialog-body");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = label("Message history", &["dialog-title"]);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        top.append(&title);
        let close = gtk::Button::with_label("Close");
        top.append(&close);
        root.append(&top);
        let author = self
            .state
            .borrow()
            .history
            .as_ref()
            .map(|history| history.message.author.name.clone())
            .unwrap_or_default();
        let note = label(
            &format!("{author} · Previous versions are retained."),
            &["muted", "small"],
        );
        note.set_xalign(0.0);
        root.append(&note);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&body)
                .build(),
        );
        window.set_child(Some(&root));
        let weak = window.downgrade();
        close.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let weak = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape
                && let Some(window) = weak.upgrade()
            {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
        let app = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(app) = app.upgrade() {
                app.history_window.borrow_mut().take();
                if let Ok(mut state) = app.state.try_borrow_mut() {
                    state.history = None;
                }
            }
            glib::Propagation::Proceed
        });
        *self.history_window.borrow_mut() = Some(HistoryWindow {
            window: window.clone(),
            body,
            shown: None,
        });
        self.render_history();
        window.present();
    }

    fn close_history(&self) {
        let open = self.history_window.borrow_mut().take();
        if let Some(open) = open {
            open.window.close();
        }
    }

    fn load_versions(&self, state: &mut State, older: bool) {
        state.history_request += 1;
        let (request, generation, token) =
            (state.history_request, state.generation, state.token.clone());
        let Some(history) = state.history.as_mut() else {
            return;
        };
        history.request = request;
        history.loading = true;
        history.older = older;
        history.error = None;
        self.worker.send(Command::MessageVersions {
            generation,
            request,
            token,
            channel: history.message.channel_id.clone(),
            message: history.message.id.clone(),
            before: if older {
                history.versions.last().map(|version| version.revision)
            } else {
                None
            },
        });
    }

    pub(super) fn accept_versions(
        &self,
        state: &mut State,
        request: u64,
        message: &str,
        result: Result<MessageVersions, String>,
    ) {
        let Some(history) = state
            .history
            .as_mut()
            .filter(|history| history.request == request && history.message.id == message)
        else {
            return;
        };
        history.loading = false;
        match result {
            Ok(page) => {
                merge_versions(&mut history.versions, page.versions, history.older);
                history.more = page.has_more;
                if history.selected.is_some_and(|selected| {
                    !history
                        .versions
                        .iter()
                        .skip(1)
                        .any(|version| version.revision == selected)
                }) {
                    history.selected = None;
                }
            }
            Err(error) => history.error = Some(error),
        }
    }

    pub(super) fn render_history(self: &Rc<Self>) {
        // A newer edit while it's open lists again from the newest.
        {
            let Ok(mut state) = self.state.try_borrow_mut() else {
                return;
            };
            let Some(history) = state.history.as_ref() else {
                drop(state);
                self.close_history();
                return;
            };
            let current = state
                .history_message(&history.message.id)
                .map_or(history.requested_revision, |message| message.revision);
            if current > history.requested_revision {
                if let Some(history) = state.history.as_mut() {
                    history.requested_revision = current;
                }
                self.load_versions(&mut state, false);
            }
        }
        let state = self.state.borrow();
        let Some(history) = state.history.as_ref() else {
            return;
        };
        let next = Shown {
            revisions: history
                .versions
                .iter()
                .map(|version| version.revision)
                .collect(),
            selected: history.selected,
            loading: history.loading,
            more: history.more,
            error: history.error.clone(),
        };
        let mut open = self.history_window.borrow_mut();
        let Some(view) = open.as_mut() else {
            return;
        };
        if view.shown.as_ref() == Some(&next) {
            return;
        }
        let body = &view.body;
        clear(body);
        let versions = &history.versions;
        if versions.len() > 1 {
            body.append(&comparison(&versions[1], &versions[0], true));
        } else if let Some(original) = versions.first() {
            let title = label("Original version", &["author"]);
            title.set_xalign(0.0);
            body.append(&title);
            let text = label(&original.content.text, &["version-text"]);
            text.set_wrap(true);
            text.set_xalign(0.0);
            body.append(&text);
        }
        if versions.len() > 1 {
            body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
            let pick_label = label("View previous versions", &[]);
            pick_label.set_xalign(0.0);
            body.append(&pick_label);
            let mut names = vec!["Choose an earlier version…".to_owned()];
            names.extend(versions.iter().skip(1).map(version_label));
            let names: Vec<&str> = names.iter().map(String::as_str).collect();
            let pick: gtk::DropDown = crate::app::named("GtkDropDown", "View previous versions");
            pick.set_model(Some(&gtk::StringList::new(&names)));
            pick.set_halign(gtk::Align::Start);
            let index = history
                .selected
                .and_then(|selected| {
                    versions
                        .iter()
                        .skip(1)
                        .position(|version| version.revision == selected)
                })
                .map_or(0, |index| index + 1);
            pick.set_selected(index as u32);
            let revisions: Vec<u32> = versions.iter().skip(1).map(|v| v.revision).collect();
            let app = Rc::downgrade(self);
            pick.connect_selected_notify(move |pick| {
                let Some(app) = app.upgrade() else { return };
                let selected = (pick.selected() as usize)
                    .checked_sub(1)
                    .and_then(|index| revisions.get(index).copied());
                if let Ok(mut state) = app.state.try_borrow_mut()
                    && let Some(history) = state.history.as_mut()
                {
                    history.selected = selected;
                }
                // Not from inside the dropdown's own handler.
                let app = app.clone();
                glib::idle_add_local_once(move || app.render_history());
            });
            body.append(&pick);
            if let Some(after) = versions
                .iter()
                .find(|version| Some(version.revision) == history.selected)
            {
                match versions
                    .iter()
                    .find(|version| version.revision + 1 == after.revision)
                {
                    Some(before) => body.append(&comparison(before, after, false)),
                    None => {
                        let title = label(
                            &if after.revision == 1 {
                                "Original version".into()
                            } else {
                                format!("Version {}", after.revision)
                            },
                            &["author"],
                        );
                        title.set_xalign(0.0);
                        body.append(&title);
                        let text = label(&after.content.text, &["version-text"]);
                        text.set_wrap(true);
                        text.set_xalign(0.0);
                        body.append(&text);
                        if after.revision > 1 && history.more {
                            body.append(&label(
                                "Load older versions to compare this change.",
                                &["muted", "small"],
                            ));
                        }
                    }
                }
            }
        }
        if history.loading {
            let loading = label("Loading versions…", &["muted"]);
            loading.set_xalign(0.0);
            body.append(&loading);
        }
        let load = |text: &str| {
            let button = gtk::Button::with_label(text);
            button.set_halign(gtk::Align::Start);
            button.set_sensitive(!history.loading);
            let older = !versions.is_empty();
            let app = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                let Some(app) = app.upgrade() else { return };
                {
                    let mut state = app.state.borrow_mut();
                    app.load_versions(&mut state, older);
                }
                let app = app.clone();
                glib::idle_add_local_once(move || app.render_history());
            });
            button
        };
        if let Some(error) = &history.error {
            let error = label(error, &["error"]);
            error.set_xalign(0.0);
            body.append(&error);
            body.append(&load("Retry"));
        } else if history.more {
            body.append(&load("Load older versions"));
        }
        view.shown = Some(next);
    }
}
