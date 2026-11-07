//! The notifications inbox, as on the egui desktop: "Notifications" (with
//! the unread count) under Direct messages opens a window with filter tabs
//! and "Unread only (N)", the rows newest first, Load more, and loading,
//! empty and Retry states. A row names who and when, says what happened,
//! and opens where it came from (an invitation shows the invitation);
//! opening or its check marks it read. Rows you can answer have Reply (an
//! inline composer with a GIF option) and React. It refreshes every 15
//! seconds while open. Paging and reads are `caper_client::activity`.
//! A child of `app`.

use super::{App, State, clear};
use crate::images::Icon;
use crate::ui::label;
use caper_client::activity::{Filter, Item};
use caper_client::model::DirectStatus;
use caper_client::worker::Command;
use gtk::{gdk, glib, prelude::*};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// An inline reply's send, by the row's draft key.
#[derive(Default)]
pub(super) struct Reply {
    pub sending: bool,
    pub status: Option<String>,
    pub error: Option<String>,
    /// An uncertain failed send retries with the same message id.
    pub retry_id: Option<String>,
    /// A GIF on its way, and what to tell KLIPY once it is sent.
    pub share: Option<caper_client::klipy::Share>,
}

/// The open window's widgets.
pub(super) struct InboxWindow {
    window: gtk::Window,
    tabs: Vec<(Filter, gtk::ToggleButton)>,
    unread: gtk::Switch,
    unread_label: gtk::Label,
    error: gtk::Label,
    retry: gtk::Button,
    status: gtk::Label,
    list: gtk::Box,
    more: gtk::Button,
    /// Each reply's text, kept while the window is open so a redraw or
    /// closing the reply keeps it.
    composers: HashMap<String, (gtk::Overlay, gtk::TextView)>,
    shown: Option<Shown>,
}

/// What the rows show, so they redraw only when it changes.
#[derive(PartialEq)]
struct Shown {
    rows: Vec<(String, bool, bool, bool, bool, String)>,
    replies: Vec<(String, bool, Option<String>, Option<String>)>,
    stale: bool,
    gifs: bool,
}

impl State {
    /// You can write to `item`'s conversation: it takes replies and is not
    /// a blocked DM or an unanswered request.
    fn inbox_writable(&self, item: &Item) -> bool {
        item.can_reply()
            && !self.directs.iter().any(|direct| {
                Some(&direct.id) == item.conversation_id.as_ref()
                    && (direct.blocked || direct.status == DirectStatus::Incoming)
            })
    }

    pub(super) fn inbox_label(&self) -> String {
        match self.inbox.unread_count {
            0 => "Notifications".into(),
            count => format!("Notifications · {count}"),
        }
    }
}

fn description(item: &Item) -> Option<String> {
    match item.kind.as_str() {
        "reaction" => Some(format!(
            "Reacted {} to your message",
            item.emoji.as_deref().unwrap_or_default()
        )),
        "direct.request" => Some("Wants to send you messages".into()),
        "space.invitation" => Some("Invited you to a space".into()),
        "channel.invitation" => Some("Invited you to a private channel".into()),
        _ => None,
    }
}

impl App {
    pub(super) fn connect_inbox(self: &Rc<Self>) {
        for button in [&self.ui.inbox, &self.ui.first_inbox] {
            let app = self.clone();
            button.connect_clicked(move |_| app.open_inbox());
        }
    }

    /// Asks for the first page (`more`: the next), or a refresh that keeps
    /// the shown rows (`poll`).
    fn load_inbox(&self, state: &mut State, more: bool, poll: bool) {
        let Some(token) = state.token.clone() else {
            return;
        };
        let inbox = &mut state.inbox;
        if poll && (inbox.loading || !inbox.reads.is_empty())
            || more && (inbox.loading || inbox.cursor.is_none())
        {
            return;
        }
        inbox.request += 1;
        inbox.loading = true;
        inbox.error = None;
        inbox.refreshed = Some(Instant::now());
        if !more && !poll {
            inbox.stale = !inbox.items.is_empty();
        }
        self.worker.send(Command::LoadActivity {
            epoch: state.epoch,
            request: inbox.request,
            token,
            more,
            filter: inbox.filter,
            unread: inbox.unread_only,
            cursor: more.then(|| inbox.cursor.clone()).flatten(),
            through: poll.then(|| inbox.items.last().cloned()).flatten(),
        });
    }

    /// `filter`'s first page, loaded ahead into its window.
    fn prefetch_inbox(&self, filter: Filter) {
        let Ok(mut state) = self.state.try_borrow_mut() else {
            return;
        };
        let Some(token) = state.token.clone() else {
            return;
        };
        let Some(request) = state.inbox.prefetch(filter) else {
            return;
        };
        self.worker.send(Command::LoadActivity {
            epoch: state.epoch,
            request,
            token,
            more: false,
            filter,
            unread: state.inbox.unread_only,
            cursor: None,
            through: None,
        });
    }

    pub(super) fn open_inbox(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if state.token.is_none() {
                return;
            }
            state.inbox.shown = true;
            self.load_inbox(&mut state, false, false);
        }
        if self.inbox_window.borrow().is_some() {
            self.render_inbox();
            return;
        }
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title("Notifications")
            .default_width(600)
            .default_height(640)
            .build();
        crate::app::close_on_escape(&window);
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("dialog-body");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = label("Notifications", &["dialog-title"]);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        top.append(&title);
        let done = gtk::Button::with_label("Done");
        top.append(&done);
        root.append(&top);
        // Packed at the start, as desktop's row of filters.
        let filters = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        let mut tabs = Vec::new();
        let mut group: Option<gtk::ToggleButton> = None;
        for filter in Filter::ALL {
            let tab = gtk::ToggleButton::with_label(filter.label());
            tab.add_css_class("inbox-tab");
            if let Some(first) = &group {
                tab.set_group(Some(first));
            } else {
                group = Some(tab.clone());
            }
            filters.append(&tab);
            tabs.push((filter, tab));
        }
        root.append(&filters);
        // As desktop: "Unread only (N)" with its switch at the right, 210px.
        let unread_line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        unread_line.set_size_request(210, -1);
        unread_line.set_halign(gtk::Align::Start);
        let unread: gtk::Switch = super::named("GtkSwitch", "Unread only");
        unread.set_valign(gtk::Align::Center);
        let unread_label = label("", &["settings-label"]);
        unread_label.set_xalign(0.0);
        unread_label.set_hexpand(true);
        unread_line.append(&unread_label);
        unread_line.append(&unread);
        root.append(&unread_line);
        let error_line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let error = label("", &["error"]);
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_hexpand(true);
        let retry = gtk::Button::with_label("Retry");
        error_line.append(&error);
        error_line.append(&retry);
        // As web, loading, empty and error states sit in the results, which
        // keep their height, so nothing above or below them moves.
        let status = label("", &["muted"]);
        status.set_vexpand(true);
        status.set_valign(gtk::Align::Center);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("inbox-list");
        let more = gtk::Button::with_label("Load more");
        more.set_halign(gtk::Align::Start);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 8);
        content.append(&error_line);
        content.append(&status);
        content.append(&list);
        content.append(&more);
        root.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&content)
                .build(),
        );
        window.set_child(Some(&root));

        let weak = window.downgrade();
        done.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
        });
        for (filter, tab) in &tabs {
            let (app, filter) = (Rc::downgrade(self), *filter);
            tab.connect_toggled(move |tab| {
                let Some(app) = app.upgrade() else { return };
                if !tab.is_active() {
                    return;
                }
                let Ok(mut state) = app.state.try_borrow_mut() else {
                    return;
                };
                if state.inbox.filter != filter {
                    state.inbox.filter = filter;
                    if state.inbox.switched() {
                        app.load_inbox(&mut state, false, false);
                    }
                    drop(state);
                    let app = app.clone();
                    glib::idle_add_local_once(move || app.render_inbox());
                }
            });
        }
        // As web, a hovered or focused filter loads ahead.
        for (filter, tab) in &tabs {
            let motion = gtk::EventControllerMotion::new();
            let focus = gtk::EventControllerFocus::new();
            for controller in [
                motion.upcast_ref::<gtk::EventController>(),
                focus.upcast_ref(),
            ] {
                let (app, filter) = (Rc::downgrade(self), *filter);
                controller.connect_local("enter", false, move |_| {
                    if let Some(app) = app.upgrade() {
                        app.prefetch_inbox(filter);
                    }
                    None
                });
            }
            tab.add_controller(motion);
            tab.add_controller(focus);
        }
        let app = Rc::downgrade(self);
        unread.connect_active_notify(move |switch| {
            let Some(app) = app.upgrade() else { return };
            let Ok(mut state) = app.state.try_borrow_mut() else {
                return;
            };
            if state.inbox.unread_only != switch.is_active() {
                state.inbox.unread_only = switch.is_active();
                if state.inbox.switched() {
                    app.load_inbox(&mut state, false, false);
                }
                drop(state);
                let app = app.clone();
                glib::idle_add_local_once(move || app.render_inbox());
            }
        });
        let app = Rc::downgrade(self);
        retry.connect_clicked(move |_| {
            let Some(app) = app.upgrade() else { return };
            app.load_inbox(&mut app.state.borrow_mut(), false, true);
            app.render_inbox();
        });
        let app = Rc::downgrade(self);
        more.connect_clicked(move |_| {
            let Some(app) = app.upgrade() else { return };
            app.load_inbox(&mut app.state.borrow_mut(), true, false);
            app.render_inbox();
        });
        let app = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(app) = app.upgrade() {
                let app = app.clone();
                glib::idle_add_local_once(move || app.close_inbox());
            }
            glib::Propagation::Proceed
        });
        // The 15-second refresh, checked each second while open.
        let app = Rc::downgrade(self);
        glib::timeout_add_seconds_local(1, move || {
            let Some(app) = app.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if app.inbox_window.borrow().is_none() {
                return glib::ControlFlow::Break;
            }
            if let Ok(mut state) = app.state.try_borrow_mut()
                && state.inbox.due()
            {
                app.load_inbox(&mut state, false, true);
            }
            app.render_inbox();
            glib::ControlFlow::Continue
        });
        *self.inbox_window.borrow_mut() = Some(InboxWindow {
            window: window.clone(),
            tabs,
            unread,
            unread_label,
            error,
            retry,
            status,
            list,
            more,
            composers: HashMap::new(),
            shown: None,
        });
        self.render_inbox();
        window.present();
    }

    pub(super) fn close_inbox(&self) {
        if let Some(open) = self.inbox_window.borrow_mut().take() {
            open.window.destroy();
        }
        self.close_activity_gifs();
        if let Ok(mut state) = self.state.try_borrow_mut() {
            state.inbox.close();
        }
    }

    /// Redraws the open window from the state.
    pub(super) fn render_inbox(self: &Rc<Self>) {
        let label = self
            .state
            .try_borrow()
            .ok()
            .map(|state| state.inbox_label());
        if let Some(label) = label {
            for button in [&self.ui.inbox, &self.ui.first_inbox] {
                if button.label().as_deref() != Some(label.as_str()) {
                    button.set_label(&label);
                }
            }
        }
        let Ok(mut open) = self.inbox_window.try_borrow_mut() else {
            return;
        };
        let Some(open) = open.as_mut() else {
            return;
        };
        let Ok(state) = self.state.try_borrow() else {
            return;
        };
        let inbox = &state.inbox;
        for (filter, tab) in &open.tabs {
            if tab.is_active() != (*filter == inbox.filter) && *filter == inbox.filter {
                tab.set_active(true);
            }
        }
        if open.unread.is_active() != inbox.unread_only {
            open.unread.set_active(inbox.unread_only);
        }
        open.unread_label
            .set_label(&format!("Unread only ({})", inbox.unread_count));
        open.error
            .set_label(inbox.error.as_deref().unwrap_or_default());
        open.error
            .parent()
            .inspect(|line| line.set_visible(inbox.error.is_some()));
        open.retry
            .set_sensitive(!inbox.loading && inbox.reads.is_empty());
        let visible = inbox.visible_items();
        let status = if inbox.loading && visible.is_empty() {
            "Loading notifications…"
        } else if !inbox.loading && visible.is_empty() && inbox.error.is_none() {
            "No notifications match these filters."
        } else {
            ""
        };
        open.status.set_label(status);
        open.status.set_visible(!status.is_empty());
        open.more.set_visible(inbox.cursor.is_some());
        open.more.set_sensitive(!inbox.loading);
        let now = chrono::Utc::now();
        let shown = Shown {
            rows: visible
                .iter()
                .map(|item| {
                    (
                        item.id.clone(),
                        item.unread,
                        inbox.expanded.contains_key(&item.id),
                        state.inbox_writable(item),
                        inbox.opening.as_ref() == Some(&item.id),
                        caper_client::feedback::relative_time(&item.created_at, now),
                    )
                })
                .collect(),
            replies: visible
                .iter()
                .filter(|item| inbox.expanded.contains_key(&item.id))
                .map(|item| {
                    let key = item.draft_key();
                    let reply = state.replies.get(&key);
                    (
                        key,
                        reply.is_some_and(|reply| reply.sending),
                        reply.and_then(|reply| reply.status.clone()),
                        reply.and_then(|reply| reply.error.clone()),
                    )
                })
                .collect(),
            stale: inbox.stale,
            gifs: state.klipy.is_some(),
        };
        if open.shown.as_ref() == Some(&shown) {
            return;
        }
        let focused = open
            .composers
            .iter()
            .find(|(_, (_, view))| view.has_focus())
            .map(|(key, _)| key.clone());
        clear(&open.list);
        // Inert at once, dimmed only if the new page takes over 150 ms, as
        // web: fast answers swap rows without a visible dim.
        open.list.set_sensitive(!inbox.stale);
        open.list.set_opacity(1.0);
        if inbox.stale {
            let list = open.list.downgrade();
            glib::timeout_add_local_once(Duration::from_millis(150), move || {
                if let Some(list) = list.upgrade().filter(|list| !list.is_sensitive()) {
                    list.set_opacity(0.5);
                }
            });
        }
        for item in &visible {
            let row = self.inbox_row(&state, item, &mut open.composers);
            open.list.append(&row);
        }
        open.shown = Some(shown);
        if let Some((_, view)) = focused.and_then(|key| open.composers.get(&key)) {
            view.grab_focus();
        }
    }

    fn inbox_row(
        self: &Rc<Self>,
        state: &State,
        item: &Item,
        composers: &mut HashMap<String, (gtk::Overlay, gtk::TextView)>,
    ) -> gtk::Box {
        let inbox = &state.inbox;
        let opening = inbox.opening.as_ref() == Some(&item.id);
        let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
        row.add_css_class("inbox-row");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = format!("{}{}", if item.unread { "● " } else { "" }, item.actor_name);
        let open: gtk::Button = super::named("GtkButton", &title);
        let name = label(&title, &["author"]);
        name.set_xalign(0.0);
        name.set_wrap(true);
        if item.unread {
            name.add_css_class("unread");
        }
        open.set_child(Some(&name));
        open.add_css_class("flat");
        open.add_css_class("inbox-open");
        open.set_hexpand(true);
        open.set_sensitive(!opening);
        top.append(&open);
        let when = label(
            &caper_client::feedback::relative_time(&item.created_at, chrono::Utc::now()),
            &["muted", "small"],
        );
        top.append(&when);
        if item.unread {
            let check: gtk::Button = super::named(
                "GtkButton",
                &format!("Mark notification from {} read", item.actor_name),
            );
            check.set_child(Some(&self.images.icon_image(Icon::Check, "#b9bcbe", 16)));
            check.set_tooltip_text(Some("Mark read"));
            check.add_css_class("flat");
            let (app, target) = (Rc::downgrade(self), item.clone());
            check.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    let app = app.clone();
                    let target = target.clone();
                    glib::idle_add_local_once(move || app.read_inbox_item(target, false));
                }
            });
            top.append(&check);
        }
        row.append(&top);
        if let Some(text) = description(item) {
            let line = label(&text, &["muted", "small"]);
            line.set_xalign(0.0);
            line.set_wrap(true);
            row.append(&line);
        }
        let body: gtk::Button = super::named("GtkButton", &item.body);
        let text = label(&item.body, &[]);
        text.set_xalign(0.0);
        text.set_wrap(true);
        text.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        body.set_child(Some(&text));
        body.add_css_class("flat");
        body.add_css_class("inbox-open");
        body.set_sensitive(!opening);
        row.append(&body);
        for button in [&open, &body] {
            let (app, target) = (Rc::downgrade(self), item.clone());
            button.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    let app = app.clone();
                    let target = target.clone();
                    glib::idle_add_local_once(move || app.read_inbox_item(target, true));
                }
            });
        }
        let place = [item.space_name.as_deref(), item.channel_name.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ");
        if !place.is_empty() {
            let line = label(&place, &["muted", "small"]);
            line.set_xalign(0.0);
            row.append(&line);
        }
        let writable = state.inbox_writable(item);
        let expanded = inbox.expanded.contains_key(&item.id);
        if writable {
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let reply = gtk::Button::with_label(if expanded { "Close reply" } else { "Reply" });
            reply.add_css_class("small-action");
            let (app, target) = (Rc::downgrade(self), item.clone());
            reply.connect_clicked(move |_| {
                let Some(app) = app.upgrade() else { return };
                {
                    let mut state = app.state.borrow_mut();
                    if state.inbox.expanded.remove(&target.id).is_none() {
                        state
                            .inbox
                            .expanded
                            .insert(target.id.clone(), target.clone());
                    }
                }
                let app = app.clone();
                let key = target.draft_key();
                glib::idle_add_local_once(move || {
                    app.render_inbox();
                    if let Some((_, view)) = app
                        .inbox_window
                        .borrow()
                        .as_ref()
                        .and_then(|open| open.composers.get(&key))
                    {
                        view.grab_focus();
                    }
                });
            });
            actions.append(&reply);
            let react = gtk::Button::with_label("React");
            react.add_css_class("small-action");
            let (app, target) = (Rc::downgrade(self), item.clone());
            react.connect_clicked(move |button| {
                let Some(app) = app.upgrade() else { return };
                let (weak, target) = (Rc::downgrade(&app), target.clone());
                crate::picker::show(
                    &app.images,
                    button.upcast_ref(),
                    "Add a reaction",
                    Default::default(),
                    move |emoji, _| {
                        if let Some(app) = weak.upgrade() {
                            app.react_to_inbox(target.clone(), emoji);
                        }
                    },
                );
            });
            actions.append(&react);
            row.append(&actions);
        }
        if expanded {
            let key = item.draft_key();
            let reply = state.replies.get(&key);
            let sending = reply.is_some_and(|reply| reply.sending);
            let hint = label(&item.reply_label(), &["muted", "small"]);
            hint.set_xalign(0.0);
            row.append(&hint);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.add_css_class("composer");
            line.add_css_class("profile-composer");
            let (overlay, view) = composers
                .entry(key.clone())
                .or_insert_with(|| self.inbox_composer(item))
                .clone();
            // The field moves from the row it was drawn in before.
            if let Some(old) = overlay.parent().and_downcast::<gtk::Box>() {
                old.remove(&overlay);
            }
            line.append(&overlay);
            view.set_editable(writable && !sending);
            let send: gtk::Button = super::named("GtkButton", "Send message");
            send.set_child(Some(&self.images.icon_image(Icon::Send, "#b9bcbe", 18)));
            send.set_valign(gtk::Align::End);
            send.add_css_class("flat");
            send.add_css_class("send");
            send.set_sensitive(writable && !sending);
            let (app, target) = (Rc::downgrade(self), item.clone());
            send.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    app.send_inbox_reply(&target, None);
                }
            });
            line.append(&send);
            row.append(&line);
            if state.klipy.is_some() {
                let gif = gtk::Button::with_label("GIF");
                gif.add_css_class("small-action");
                gif.set_halign(gtk::Align::Start);
                gif.set_sensitive(writable && !sending);
                let (app, target) = (Rc::downgrade(self), item.clone());
                gif.connect_clicked(move |button| {
                    if let Some(app) = app.upgrade() {
                        app.open_activity_gifs(target.clone(), button.upcast_ref());
                    }
                });
                row.append(&gif);
            }
            if let Some(text) =
                reply.and_then(|reply| reply.error.as_ref().or(reply.status.as_ref()))
            {
                let failed = reply.is_some_and(|reply| reply.error.is_some());
                let note = label(text, &[if failed { "error" } else { "muted" }, "small"]);
                note.set_xalign(0.0);
                note.set_wrap(true);
                row.append(&note);
            }
        }
        row.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        row
    }

    /// A reply's text field in its overlay, with "Message @actor" and
    /// Enter to send.
    fn inbox_composer(self: &Rc<Self>, item: &Item) -> (gtk::Overlay, gtk::TextView) {
        let view: gtk::TextView =
            super::named("GtkTextView", &format!("Reply to {}", item.actor_name));
        view.set_wrap_mode(gtk::WrapMode::WordChar);
        view.set_accepts_tab(false);
        view.set_hexpand(true);
        let placeholder = label(&format!("Message {}", item.actor_name), &["placeholder"]);
        placeholder.set_can_target(false);
        placeholder.set_valign(gtk::Align::Start);
        placeholder.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .propagate_natural_height(true)
                .max_content_height(120)
                .child(&view)
                .hexpand(true)
                .build(),
        ));
        overlay.add_overlay(&placeholder);
        overlay.set_hexpand(true);
        let empty = placeholder.clone();
        view.buffer()
            .connect_changed(move |buffer| empty.set_visible(buffer.char_count() == 0));
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (app, target) = (Rc::downgrade(self), item.clone());
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                && !modifiers.contains(gdk::ModifierType::SHIFT_MASK)
            {
                if let Some(app) = app.upgrade() {
                    app.send_inbox_reply(&target, None);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        view.add_controller(keys);
        (overlay, view)
    }

    /// Sends the row's reply text, or `gif` alone, to its conversation.
    pub(super) fn send_inbox_reply(
        self: &Rc<Self>,
        item: &Item,
        gif: Option<(caper_client::klipy::Gif, caper_client::klipy::Share)>,
    ) {
        let key = item.draft_key();
        let view = self
            .inbox_window
            .borrow()
            .as_ref()
            .and_then(|open| open.composers.get(&key).map(|(_, view)| view.clone()));
        let text = view
            .as_ref()
            .map(|view| {
                let buffer = view.buffer();
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string()
            })
            .unwrap_or_default();
        {
            let mut state = self.state.borrow_mut();
            let Some(token) = state.token.clone() else {
                return;
            };
            if !state.inbox_writable(item)
                || state.replies.values().any(|reply| reply.sending)
                || gif.is_none() && text.trim().is_empty()
            {
                return;
            }
            if gif.is_none() && text.chars().count() > 4_000 {
                state.replies.entry(key).or_default().error =
                    Some("Messages can be at most 4,000 characters.".into());
                drop(state);
                self.render_inbox();
                return;
            }
            let (name, uploads, epoch) = (state.name(), state.uploads.clone(), state.epoch);
            let reply = state.replies.entry(key.clone()).or_default();
            reply.sending = true;
            reply.error = None;
            reply.status = Some("Sending…".into());
            let client_id = reply
                .retry_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            let retrying = reply.retry_id.is_some();
            let (gif, share) = gif.unzip();
            reply.share = share;
            self.worker.send(Command::SendActivity {
                epoch,
                item: item.clone(),
                token,
                send: Box::new(caper_client::profile_dm::Send {
                    person: key,
                    username: item.actor_name.clone(),
                    name,
                    client_id,
                    text: if gif.is_some() { String::new() } else { text },
                    files: Vec::new(),
                    settings: uploads,
                    cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    retrying,
                    open_only: false,
                    voice_only: false,
                    direct: None,
                }),
                gif,
            });
        }
        self.render_inbox();
    }

    /// `ActivitySent`: a sent text clears its field; a failure keeps it,
    /// and an uncertain one retries with the same id.
    pub(super) fn inbox_reply_sent(
        &self,
        state: &mut State,
        send: &caper_client::profile_dm::Send,
        result: Result<(), caper_client::profile_dm::Failure>,
    ) {
        let reply = state.replies.entry(send.person.clone()).or_default();
        reply.sending = false;
        reply.status = None;
        let share = reply.share.take();
        let gif = share.is_some();
        match result {
            Ok(()) => {
                reply.retry_id = None;
                reply.error = None;
                if let Some(share) = share {
                    share.send();
                }
                if !gif
                    && let Some((_, view)) = self
                        .inbox_window
                        .borrow()
                        .as_ref()
                        .and_then(|open| open.composers.get(&send.person))
                {
                    view.buffer().set_text("");
                }
            }
            Err(error) => {
                reply.error = Some(error.message);
                reply.retry_id = (!gif && error.uncertain).then(|| send.client_id.clone());
            }
        }
    }

    fn react_to_inbox(&self, item: Item, emoji: String) {
        let mut state = self.state.borrow_mut();
        let Some(token) = state.token.clone() else {
            return;
        };
        state.inbox.reaction_request += 1;
        state.inbox.error = None;
        self.worker.send(Command::ReactActivity {
            epoch: state.epoch,
            request: state.inbox.reaction_request,
            item,
            token,
            emoji,
            name: state.name(),
        });
    }

    /// Marks `item` read; `open` also goes where it came from once the
    /// read succeeds. Only one open is in flight at a time.
    fn read_inbox_item(self: &Rc<Self>, item: Item, open: bool) {
        {
            let mut state = self.state.borrow_mut();
            let Some(token) = state.token.clone() else {
                return;
            };
            if open && state.inbox.opening.is_some() {
                return;
            }
            // A refresh already in flight could bring this row back unread;
            // a filter reload replacing stale rows keeps going.
            if !state.inbox.stale {
                state.inbox.request += 1;
                state.inbox.loading = false;
            }
            if open {
                state.inbox.opening = Some(item.id.clone());
            }
            state.inbox.error = None;
            state.inbox.mark_read(&item);
            self.worker.send(Command::ReadActivity {
                epoch: state.epoch,
                navigation: state.navigation,
                token,
                item,
                open,
            });
        }
        self.render_inbox();
    }

    /// `ActivityRead`: a successful open closes the inbox and goes there.
    pub(super) fn inbox_read(
        self: &Rc<Self>,
        navigation: u64,
        item: Item,
        open: bool,
        result: Result<caper_client::activity::Read, String>,
    ) {
        let go = {
            let mut state = self.state.borrow_mut();
            if open {
                state.inbox.opening = None;
            }
            let read = state.inbox.read_finished(&item, result);
            if read {
                self.refresh_unread(&mut state);
            }
            read && open && navigation == state.navigation && state.inbox.shown
        };
        if go {
            self.open_inbox_item(item);
        }
        self.render_inbox();
    }

    fn open_inbox_item(self: &Rc<Self>, item: Item) {
        self.close_inbox();
        {
            let mut state = self.state.borrow_mut();
            if matches!(
                item.kind.as_str(),
                "space.invitation" | "channel.invitation"
            ) {
                match item.space_id {
                    Some(space) => self.open_invitation(&mut state, space, item.channel_id),
                    None => state.error = Some("This invitation is no longer available.".into()),
                }
                drop(state);
                self.render();
                return;
            }
            let Some(channel) = item.channel().map(str::to_owned) else {
                state.error = Some("This notification is no longer available.".into());
                drop(state);
                self.render();
                return;
            };
            let Some(message) = item.message_id.clone() else {
                if state.channel.as_deref() != Some(&channel) {
                    self.go_to(&mut state, &item, channel);
                }
                drop(state);
                self.render();
                return;
            };
            if state.channel.as_deref() != Some(&channel) {
                self.go_to(&mut state, &item, channel.clone());
                state.arrival = Some((state.navigation, channel, message, item.thread_root_id));
                drop(state);
                self.render();
                return;
            }
        }
        if let Some(message) = item.message_id {
            self.jump_to_message(message, item.thread_root_id);
        }
    }

    /// As a clicked invitation notification: reload the spaces, then show it.
    fn open_invitation(&self, state: &mut State, space: String, channel: Option<String>) {
        let Some(token) = state.token.clone() else {
            return;
        };
        state.navigation += 1;
        self.worker.send(Command::OpenInvitation {
            epoch: state.epoch,
            navigation: state.navigation,
            token,
            space,
            channel,
        });
    }

    fn go_to(&self, state: &mut State, item: &Item, channel: String) {
        if item.conversation_id.is_some() {
            self.enter_direct(state, channel);
        } else {
            self.navigate(state, item.space_id.clone(), Some(channel));
        }
    }

    /// Inbox events from the worker.
    pub(super) fn inbox_reacted(
        &self,
        state: &mut State,
        request: u64,
        result: Result<(), String>,
    ) {
        if let Err(error) = result
            && request == state.inbox.reaction_request
        {
            state.inbox.error = Some(error);
        }
    }
}
