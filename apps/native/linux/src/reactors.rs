//! Who reacted, as on the egui desktop: a reaction chip's hover card names
//! the people (loaded on hover), and **View reactions** opens a window with
//! one tab per emoji and the people who reacted with it. The window follows
//! the live snapshot: a removed emoji moves the selection, and no reactions
//! close it. A child of `app`.

use super::{App, State, clear};
use crate::images::{Avatar, AvatarView};
use crate::ui::label;
use caper_client::model::{self, Message, Reaction, Reactor};
use caper_client::reactions::ReactorState;
use caper_client::worker::Command;
use gtk::{glib, prelude::*};
use std::rc::Rc;

/// The open View reactions window and what it last showed.
pub(super) struct ReactorsWindow {
    window: gtk::Window,
    tabs: gtk::Box,
    heading: gtk::Label,
    list: gtk::Box,
    shown: Option<Shown>,
}

/// What the window draws, so it redraws only when this changes.
#[derive(PartialEq)]
struct Shown {
    tabs: Vec<(String, usize)>,
    selected: String,
    people: Result<Vec<Reactor>, bool>,
}

impl State {
    /// The id reactions list you by: your chat author, else your account.
    pub(super) fn reactor_id(&self) -> Option<&str> {
        self.session
            .as_ref()
            .map(|session| session.author.id.as_str())
            .or_else(|| self.account.as_ref().map(|account| account.id.as_str()))
    }

    /// `message`'s reactions with yours still on their way drawn in.
    pub(super) fn shown_reactions(&self, message: &Message) -> Vec<Reaction> {
        self.reactions
            .projected(&message.reactions, &message.id, self.reactor_id())
            .into_iter()
            .filter(|reaction| !reaction.author_ids.is_empty())
            .collect()
    }

    fn reacted_message(&self, id: &str) -> Option<&Message> {
        self.message(id).or_else(|| {
            self.timeline
                .pinned_messages()
                .find(|message| message.id == id)
        })
    }
}

impl App {
    /// Starts reading who reacted to `id` unless its current reaction
    /// revision is known or already loading.
    pub(super) fn load_reactors(&self, id: &str) {
        let Ok(mut state) = self.state.try_borrow_mut() else {
            return;
        };
        let Some(message) = state.reacted_message(id) else {
            return;
        };
        let (channel, revision) = (
            message.channel_id.clone(),
            message.reaction_seq.clone().unwrap_or_else(|| "0".into()),
        );
        if !caper_client::reactions::start_reactors(&mut state.reactors, id, &revision) {
            return;
        }
        self.worker.send(Command::LoadReactors {
            generation: state.generation,
            token: state.token.clone(),
            channel,
            message: id.to_owned(),
            revision,
        });
    }

    pub(super) fn open_reactors(self: &Rc<Self>, id: String) {
        let first = {
            let state = self.state.borrow();
            let Some(message) = state.reacted_message(&id) else {
                return;
            };
            match state.shown_reactions(message).first() {
                Some(reaction) => reaction.emoji.clone(),
                None => return,
            }
        };
        self.close_reactors();
        self.state.borrow_mut().reactors_view = Some((id, first));
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title("Reactions")
            .default_width(320)
            .resizable(false)
            .build();
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("dialog-body");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = label("Reactions", &["dialog-title"]);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        top.append(&title);
        let close: gtk::Button = super::named("GtkButton", "Close reactions");
        close.set_child(Some(&self.images.icon_image(
            crate::images::Icon::Close,
            "#b9bcbe",
            14,
        )));
        close.add_css_class("flat");
        top.append(&close);
        root.append(&top);
        let tabs = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tabs.add_css_class("reactor-tabs");
        root.append(&tabs);
        let heading = label("", &["muted", "reactor-heading"]);
        heading.set_xalign(0.0);
        root.append(&heading);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .max_content_height(280)
                .child(&list)
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
                app.reactors_window.borrow_mut().take();
                if let Ok(mut state) = app.state.try_borrow_mut() {
                    state.reactors_view = None;
                }
            }
            glib::Propagation::Proceed
        });
        *self.reactors_window.borrow_mut() = Some(ReactorsWindow {
            window: window.clone(),
            tabs,
            heading,
            list,
            shown: None,
        });
        self.render_reactors();
        window.present();
    }

    fn close_reactors(&self) {
        let open = self.reactors_window.borrow_mut().take();
        if let Some(open) = open {
            open.window.close();
        }
    }

    /// Redraws the open window from the latest snapshot and names.
    pub(super) fn render_reactors(self: &Rc<Self>) {
        let view = self.state.borrow().reactors_view.clone();
        let Some((id, selected)) = view else {
            self.close_reactors();
            return;
        };
        let snapshot = {
            let state = self.state.borrow();
            state
                .reacted_message(&id)
                .map(|message| (state.shown_reactions(message), message.reaction_seq.clone()))
        };
        let Some((reactions, revision)) = snapshot else {
            self.state.borrow_mut().reactors_view = None;
            self.close_reactors();
            return;
        };
        let Some(current) = reactions
            .iter()
            .find(|reaction| reaction.emoji == selected)
            .or_else(|| reactions.first())
            .cloned()
        else {
            self.state.borrow_mut().reactors_view = None;
            self.close_reactors();
            return;
        };
        // Load on open and for each new snapshot; a failure waits for Retry.
        let revision = revision.unwrap_or_else(|| "0".into());
        let failed_now = self.state.borrow().reactors.get(&id).is_some_and(|cached| {
            cached.revision == revision && matches!(cached.state, ReactorState::Failed(_))
        });
        if !failed_now {
            self.load_reactors(&id);
        }
        let people = {
            let mut state = self.state.borrow_mut();
            state.reactors_view = Some((id.clone(), current.emoji.clone()));
            let me = state.reactor_id().map(str::to_owned);
            let cached = state.reactors.get(&id);
            let failed =
                cached.is_some_and(|cached| matches!(cached.state, ReactorState::Failed(_)));
            match model::reactor_list(
                cached.map_or(&[][..], |cached| cached.groups.as_slice()),
                &current,
                me.as_deref(),
            ) {
                // You may not be in the loaded list yet (a pending reaction).
                Some(authors) => Ok(authors
                    .into_iter()
                    .map(|author| match &state.account {
                        Some(account)
                            if Some(&author.id) == me.as_ref()
                                && author.username.is_none()
                                && author.display_name.is_none() =>
                        {
                            Reactor {
                                id: author.id,
                                username: account.username.clone(),
                                display_name: account.display_name.clone(),
                                avatar_id: account.avatar_id,
                                avatar_asset: account.avatar_asset.clone(),
                                avatar_url: account.avatar_url.clone(),
                            }
                        }
                        _ => author,
                    })
                    .collect()),
                None => Err(failed),
            }
        };
        let next = Shown {
            tabs: reactions
                .iter()
                .map(|reaction| (reaction.emoji.clone(), reaction.author_ids.len()))
                .collect(),
            selected: current.emoji.clone(),
            people,
        };
        let mut open = self.reactors_window.borrow_mut();
        let Some(view) = open.as_mut() else {
            return;
        };
        if view.shown.as_ref() == Some(&next) {
            return;
        }
        clear(&view.tabs);
        for (emoji, count) in &next.tabs {
            let tab: gtk::ToggleButton =
                super::named("GtkToggleButton", &format!("{emoji}, {count}"));
            tab.add_css_class("reactor-tab");
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            match caper_client::emoji::find(emoji) {
                Some(entry) => line.append(&self.images.emoji_image(entry, 20)),
                None => line.append(&gtk::Label::new(Some(emoji))),
            }
            line.append(&label(&count.to_string(), &[]));
            tab.set_child(Some(&line));
            tab.set_active(*emoji == next.selected);
            let (app, emoji) = (Rc::downgrade(self), emoji.clone());
            tab.connect_clicked(move |_| {
                let Some(app) = app.upgrade() else { return };
                if let Some(view) = app.state.borrow_mut().reactors_view.as_mut() {
                    view.1.clone_from(&emoji);
                }
                let app = app.clone();
                glib::idle_add_local_once(move || app.render_reactors());
            });
            view.tabs.append(&tab);
        }
        view.heading.set_label(
            &caper_client::emoji::name(&next.selected)
                .map_or_else(|| next.selected.clone(), |name| format!(":{name}:")),
        );
        clear(&view.list);
        match &next.people {
            Ok(people) => {
                for person in people {
                    let name = model::reactor_name(person);
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
                    row.add_css_class("reactor-row");
                    let avatar = AvatarView::new(28, false);
                    avatar.set(
                        &self.images,
                        &Avatar::new(
                            &name,
                            person.avatar_id,
                            &person.avatar_asset,
                            &person.avatar_url,
                        ),
                    );
                    row.append(&avatar.root);
                    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    let title = label(&name, &["author"]);
                    title.set_xalign(0.0);
                    text.append(&title);
                    if let Some(username) = &person.username {
                        let user = label(&format!("@{username}"), &["muted", "small"]);
                        user.set_xalign(0.0);
                        text.append(&user);
                    }
                    row.append(&text);
                    view.list.append(&row);
                }
            }
            Err(true) => {
                let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                line.append(&label("Couldn’t load reactions.", &["error"]));
                let retry = gtk::Button::with_label("Retry");
                retry.add_css_class("inline");
                let (app, id) = (Rc::downgrade(self), id.clone());
                retry.connect_clicked(move |_| {
                    let Some(app) = app.upgrade() else { return };
                    // No snapshot has an empty revision, so this always asks.
                    if let Some(cached) = app.state.borrow_mut().reactors.get_mut(&id) {
                        cached.revision.clear();
                    }
                    app.load_reactors(&id);
                    let app = app.clone();
                    glib::idle_add_local_once(move || app.render_reactors());
                });
                line.append(&retry);
                view.list.append(&line);
            }
            Err(false) => view.list.append(&label("Loading reactions…", &["muted"])),
        }
        view.shown = Some(next);
    }
}
