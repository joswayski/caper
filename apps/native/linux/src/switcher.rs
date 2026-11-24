//! Keyboard navigation, as on web: the quick switcher (Ctrl+K), stepping
//! through the sidebar's conversations (Alt+Up/Down, with Shift only the
//! unread ones), Escape to mark the open conversation read, and the
//! shortcuts sheet (Ctrl+/). Matching and stepping are `caper_client::switcher`.
//! A child of `app`, so it reads and changes the app's state.

use super::{App, State};
use crate::images::{Avatar, AvatarView, Icon};
use crate::ui::label;
use caper_client::model::DirectStatus;
use caper_client::switcher;
use caper_client::worker::Command;
use gtk::{gdk, glib, prelude::*};
use std::rc::Rc;

#[derive(Clone)]
enum Target {
    Channel(String, String),
    Direct(String),
    Space(String),
    /// Someone you share a space with but have no DM with: opening starts one.
    Person(String),
}

#[derive(Clone)]
struct Item {
    name: String,
    alternates: Vec<String>,
    detail: String,
    unread: bool,
    icon: Option<Icon>,
    avatar: Option<Avatar>,
    target: Target,
}

pub(super) struct Switcher {
    window: gtk::Window,
}

/// The shortcuts sheet's rows, in GTK's key names.
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigation",
        &[
            ("Quick switcher", "Ctrl+K"),
            ("Previous or next conversation", "Alt+↑ / Alt+↓"),
            (
                "Previous or next unread conversation",
                "Alt+Shift+↑ / Alt+Shift+↓",
            ),
            ("Settings", "Ctrl+,"),
            ("Keyboard shortcuts", "Ctrl+/"),
        ],
    ),
    (
        "Conversation",
        &[
            ("Mark as read", "Esc"),
            ("Edit your last message (in an empty message box)", "↑"),
            ("Jump to latest (from the messages)", "End"),
            ("New line", "Shift+Enter"),
            ("Bold, italic, underline, code", "Ctrl+B / I / U / E"),
            ("Zoom in, out, reset", "Ctrl+= / Ctrl+− / Ctrl+0"),
        ],
    ),
    (
        "Voice",
        &[
            ("Mute or unmute", "Ctrl+Shift+M"),
            ("Deafen or undeafen", "Ctrl+Shift+D"),
        ],
    ),
];

fn channel_unread(state: &State, space: &str, channel: &str) -> bool {
    state
        .unread
        .get(space)
        .and_then(|channels| channels.get(channel))
        .is_some_and(|(count, _)| *count > 0)
}

fn direct_unread(state: &State, direct: &caper_client::model::DirectConversation) -> bool {
    direct.status != DirectStatus::Incoming
        && !direct.blocked
        && !state.blocked(&direct.peer.id)
        && direct.has_unread()
}

/// The sidebar's conversations in its order: the space's joined channels,
/// then your notes and DMs (requests stay behind their row).
fn sidebar(state: &State) -> Vec<(Target, bool)> {
    let me = state.account.as_ref().map(|account| account.id.as_str());
    let mut items = Vec::new();
    if let Some(detail) = &state.detail {
        for channel in detail.channels.iter().filter(|channel| channel.joined) {
            let unread = channel_unread(state, &detail.space.id, &channel.id);
            items.push((
                Target::Channel(detail.space.id.clone(), channel.id.clone()),
                unread,
            ));
        }
    }
    let own = state
        .directs
        .iter()
        .filter(|direct| Some(direct.peer.id.as_str()) == me);
    let others = state
        .directs
        .iter()
        .filter(|direct| Some(direct.peer.id.as_str()) != me);
    for direct in own
        .chain(others)
        .filter(|direct| direct.status != DirectStatus::Incoming)
    {
        items.push((
            Target::Direct(direct.id.clone()),
            direct_unread(state, direct),
        ));
    }
    items
}

fn items(state: &State) -> Vec<Item> {
    let me = state.account.as_ref().map(|account| account.id.as_str());
    let mut items = Vec::new();
    if let Some(detail) = &state.detail {
        for channel in detail.channels.iter().filter(|channel| channel.joined) {
            items.push(Item {
                name: channel.name.clone(),
                alternates: Vec::new(),
                detail: detail.space.name.clone(),
                unread: channel_unread(state, &detail.space.id, &channel.id),
                icon: Some(if channel.private {
                    Icon::Lock
                } else {
                    Icon::Hash
                }),
                avatar: None,
                target: Target::Channel(detail.space.id.clone(), channel.id.clone()),
            });
        }
    }
    for direct in &state.directs {
        let own = Some(direct.peer.id.as_str()) == me;
        let peer = &direct.peer;
        items.push(Item {
            name: if own {
                format!("{} (you)", peer.display_name)
            } else {
                peer.display_name.clone()
            },
            alternates: vec![peer.username.clone()],
            detail: format!("@{}", peer.username),
            unread: direct.status == DirectStatus::Incoming || direct_unread(state, direct),
            icon: None,
            avatar: Some(Avatar::new(
                &peer.display_name,
                peer.avatar_id,
                &peer.avatar_asset,
                &peer.avatar_url,
            )),
            target: Target::Direct(direct.id.clone()),
        });
    }
    for space in &state.spaces {
        if space.demo {
            continue;
        }
        items.push(Item {
            name: space.name.clone(),
            alternates: Vec::new(),
            detail: "Space".into(),
            unread: state
                .unread
                .get(&space.id)
                .is_some_and(|channels| channels.values().any(|(count, _)| *count > 0)),
            icon: Some(Icon::Users),
            avatar: None,
            target: Target::Space(space.id.clone()),
        });
    }
    for person in state.people.iter().flatten() {
        if Some(person.id.as_str()) == me
            || state.blocked(&person.id)
            || state
                .directs
                .iter()
                .any(|direct| direct.peer.id == person.id)
        {
            continue;
        }
        items.push(Item {
            name: person.display_name.clone(),
            alternates: vec![person.username.clone()],
            detail: format!("@{}", person.username),
            unread: false,
            icon: None,
            avatar: Some(Avatar::new(
                &person.display_name,
                person.avatar_id,
                &person.avatar_asset,
                &person.avatar_url,
            )),
            target: Target::Person(person.username.clone()),
        });
    }
    items
}

fn ranked(items: &[Item], query: &str) -> Vec<Item> {
    switcher::rank(
        items.iter().map(|item| {
            (
                std::iter::once(item.name.as_str())
                    .chain(item.alternates.iter().map(String::as_str)),
                item.unread,
            )
        }),
        query,
    )
    .into_iter()
    .take(50)
    .map(|index| items[index].clone())
    .collect()
}

impl App {
    /// Ctrl+K, Ctrl+/, Alt+Up/Down and Ctrl+Shift+M/D, from anywhere in the
    /// main window. Returns whether the key was used.
    pub(super) fn navigation_key(
        self: &Rc<Self>,
        key: gdk::Key,
        modifiers: gdk::ModifierType,
    ) -> bool {
        let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        let alt = modifiers.contains(gdk::ModifierType::ALT_MASK);
        let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
        if modifiers.contains(gdk::ModifierType::SUPER_MASK)
            || self.state.borrow().page() != super::Page::Chat
        {
            return false;
        }
        let key = key.to_lower();
        match key {
            gdk::Key::k if control && !alt && !shift => {
                let app = self.clone();
                glib::idle_add_local_once(move || app.open_switcher());
                true
            }
            gdk::Key::slash | gdk::Key::question if control && !alt => {
                let app = self.clone();
                glib::idle_add_local_once(move || app.open_shortcut_help());
                true
            }
            gdk::Key::Up | gdk::Key::Down | gdk::Key::KP_Up | gdk::Key::KP_Down
                if alt && !control =>
            {
                self.step_conversation(matches!(key, gdk::Key::Down | gdk::Key::KP_Down), shift);
                true
            }
            gdk::Key::m | gdk::Key::d if control && shift && !alt => {
                if !self.in_call() {
                    return false;
                }
                if key == gdk::Key::m {
                    self.toggle_mute();
                } else {
                    self.toggle_deafen();
                }
                true
            }
            _ => false,
        }
    }

    fn step_conversation(self: &Rc<Self>, forward: bool, unread_only: bool) {
        let target = {
            let state = self.state.borrow();
            let order = sidebar(&state);
            let current = order.iter().position(|(target, _)| match target {
                Target::Channel(_, id) => {
                    state.direct.is_none() && state.channel.as_deref() == Some(id)
                }
                Target::Direct(id) => state.direct.as_deref() == Some(id),
                _ => false,
            });
            let unread: Vec<bool> = order.iter().map(|(_, unread)| *unread).collect();
            switcher::adjacent(&unread, current, forward, unread_only)
                .map(|index| order[index].0.clone())
        };
        if let Some(target) = target {
            self.open_target(target);
        }
    }

    fn open_target(self: &Rc<Self>, target: Target) {
        match target {
            Target::Channel(space, channel) => {
                let mut state = self.state.borrow_mut();
                if state.direct.is_some() || state.channel.as_deref() != Some(&channel) {
                    self.navigate(&mut state, Some(space), Some(channel));
                }
                self.browsing.set(false);
                state.feedback_open = false;
                drop(state);
                self.render();
            }
            Target::Direct(id) => self.open_direct(id),
            Target::Space(id) => self.select_space(id),
            Target::Person(username) => {
                let mut state = self.state.borrow_mut();
                let Some(token) = state.token.clone() else {
                    return;
                };
                if state.direct_opening {
                    return;
                }
                state.direct_opening = true;
                self.worker.send(Command::CreateDirect {
                    generation: state.epoch,
                    navigation: state.navigation,
                    token,
                    username,
                });
            }
        }
    }

    /// Escape in a conversation: marks it read through its newest message,
    /// even while reading further back.
    pub(super) fn mark_open_read(self: &Rc<Self>) -> bool {
        let mut state = self.state.borrow_mut();
        if state.page() != super::Page::Chat
            || state.visiting().is_some()
            || state.open_request().is_some()
        {
            return false;
        }
        let (Some(token), Some(channel)) = (state.token.clone(), state.channel.clone()) else {
            return false;
        };
        let last = match &state.direct {
            Some(direct) => state
                .directs
                .iter()
                .find(|entry| &entry.id == direct)
                .filter(|entry| entry.has_unread())
                .and_then(|entry| entry.last_seq.parse::<u64>().ok()),
            None => state
                .reads
                .get(&channel)
                .filter(|(read, last)| last > read)
                .map(|(_, last)| *last)
                .filter(|_| {
                    state
                        .space
                        .as_ref()
                        .is_some_and(|space| channel_unread(&state, space, &channel))
                }),
        };
        let Some(last) = last.filter(|last| {
            state
                .read_through
                .get(&channel)
                .is_none_or(|read| read < last)
        }) else {
            return false;
        };
        state.read_through.insert(channel.clone(), last);
        let seq = last.to_string();
        if let Some(direct) = state.direct.clone() {
            if let Some(entry) = state.directs.iter_mut().find(|entry| entry.id == direct) {
                entry.unread_count = Some(0);
                entry.read_seq.clone_from(&seq);
            }
            self.worker.send(Command::ReadDirect {
                token,
                id: direct,
                seq,
            });
        } else if let Some(space) = state.space.clone() {
            self.worker.send(Command::ReadChannel {
                epoch: state.epoch,
                token,
                space,
                channel,
                seq,
            });
        }
        self.clear_open_unread(&mut state);
        state.layout += 1;
        drop(state);
        self.render();
        true
    }

    fn close_switcher(&self) {
        if let Some(switcher) = self.switcher.borrow_mut().take() {
            switcher.window.destroy();
        }
    }

    fn open_switcher(self: &Rc<Self>) {
        if self.switcher.borrow().is_some() {
            return self.close_switcher();
        }
        let all = items(&self.state.borrow());
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title("Quick switcher")
            .default_width(520)
            .default_height(440)
            .build();
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("dialog-body");
        let search: gtk::SearchEntry = crate::app::named("GtkSearchEntry", "Find a conversation");
        search.set_placeholder_text(Some("Find a channel, DM, space or person"));
        root.append(&search);
        let list = gtk::ListBox::new();
        list.add_css_class("forward-list");
        list.set_selection_mode(gtk::SelectionMode::Browse);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        root.append(&scroll);
        let empty = label("No matches", &["muted"]);
        empty.set_visible(false);
        root.append(&empty);
        root.append(&label(
            "↑ ↓ to choose, Enter to open, Esc to close",
            &["muted", "small"],
        ));
        window.set_child(Some(&root));

        let shown: Rc<std::cell::RefCell<Vec<Item>>> = Rc::default();
        let fill = {
            let (list, empty, shown, images) = (
                list.clone(),
                empty.clone(),
                shown.clone(),
                self.images.clone(),
            );
            move |query: &str| {
                while let Some(row) = list.first_child() {
                    list.remove(&row);
                }
                let results = ranked(&all, query);
                empty.set_visible(results.is_empty());
                empty.set_label(if query.trim().is_empty() {
                    "Nothing to switch to yet"
                } else {
                    "No matches"
                });
                for item in &results {
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    line.add_css_class("forward-destination");
                    if let Some(avatar) = &item.avatar {
                        let view = AvatarView::new(22, false);
                        view.set(&images, avatar);
                        line.append(&view.root);
                    } else if let Some(icon) = item.icon {
                        let image = images.icon_image(icon, "#b9bcbe", 16);
                        image.set_size_request(22, -1);
                        line.append(&image);
                    }
                    let name = label(&item.name, if item.unread { &["unread"] } else { &[] });
                    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    line.append(&name);
                    let detail = label(&item.detail, &["muted", "small"]);
                    detail.set_hexpand(true);
                    detail.set_xalign(0.0);
                    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    line.append(&detail);
                    if item.unread {
                        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                        dot.add_css_class("self-unread");
                        dot.set_valign(gtk::Align::Center);
                        line.append(&dot);
                    }
                    let spoken = format!(
                        "{}, {}{}",
                        item.name,
                        item.detail,
                        if item.unread { ", unread" } else { "" }
                    );
                    let row: gtk::ListBoxRow = crate::app::named("GtkListBoxRow", &spoken);
                    row.set_child(Some(&line));
                    list.append(&row);
                }
                if let Some(first) = list.row_at_index(0) {
                    list.select_row(Some(&first));
                }
                *shown.borrow_mut() = results;
            }
        };
        fill("");
        search.connect_search_changed(move |search| fill(&search.text()));

        let choose = {
            let (weak, shown) = (Rc::downgrade(self), shown.clone());
            move |index: i32| {
                let target = usize::try_from(index)
                    .ok()
                    .and_then(|index| shown.borrow().get(index).map(|item| item.target.clone()));
                if let (Some(app), Some(target)) = (weak.upgrade(), target) {
                    app.close_switcher();
                    app.open_target(target);
                }
            }
        };
        let activate = choose.clone();
        list.connect_row_activated(move |_, row| activate(row.index()));
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (weak, list_keys) = (Rc::downgrade(self), list.clone());
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let selected = list_keys.selected_row().map_or(0, |row| row.index());
            let count = shown.borrow().len() as i32;
            match key {
                gdk::Key::Escape => {
                    if let Some(app) = weak.upgrade() {
                        app.close_switcher();
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    choose(selected);
                    glib::Propagation::Stop
                }
                gdk::Key::Up | gdk::Key::Down if count > 0 && modifiers.is_empty() => {
                    let step = if key == gdk::Key::Down { 1 } else { count - 1 };
                    // The field keeps focus while the highlight moves.
                    if let Some(row) = list_keys.row_at_index((selected + step) % count) {
                        list_keys.select_row(Some(&row));
                        if let (Some(bounds), Some(adjustment)) =
                            (row.compute_bounds(&list_keys), list_keys.adjustment())
                        {
                            let (top, bottom) = (
                                f64::from(bounds.y()),
                                f64::from(bounds.y() + bounds.height()),
                            );
                            if top < adjustment.value() {
                                adjustment.set_value(top);
                            } else if bottom > adjustment.value() + adjustment.page_size() {
                                adjustment.set_value(bottom - adjustment.page_size());
                            }
                        }
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        window.add_controller(keys);
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(app) = weak.upgrade() {
                app.switcher.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });
        window.present();
        search.grab_focus();
        *self.switcher.borrow_mut() = Some(Switcher { window });
    }

    fn open_shortcut_help(self: &Rc<Self>) {
        if self.switcher.borrow().is_some() {
            self.close_switcher();
        }
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title("Keyboard shortcuts")
            .default_width(520)
            .build();
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
        root.add_css_class("dialog-body");
        root.append(&label("Keyboard shortcuts", &["dialog-title"]));
        for (group, rows) in SHORTCUTS {
            let heading = label(&group.to_uppercase(), &["muted", "small"]);
            heading.set_xalign(0.0);
            root.append(&heading);
            let grid = gtk::Grid::builder()
                .row_spacing(6)
                .column_spacing(24)
                .build();
            for (index, (action, keys)) in rows.iter().enumerate() {
                let action = label(action, &[]);
                action.set_xalign(0.0);
                action.set_hexpand(true);
                action.set_wrap(true);
                grid.attach(&action, 0, index as i32, 1, 1);
                let keys = label(keys, &["shortcut-keys"]);
                keys.set_xalign(1.0);
                grid.attach(&keys, 1, index as i32, 1, 1);
            }
            root.append(&grid);
        }
        let keys = gtk::EventControllerKey::new();
        let close = window.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                close.destroy();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
        window.set_child(Some(&root));
        window.present();
    }
}
