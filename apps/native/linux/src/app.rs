//! The window's state and the worker events that change it.
//!
//! Widgets are built once (`ui`) and stay alive; worker events update the
//! state, then only the widgets whose content changed. The message lists are
//! `GtkListView`s over `GListStore`s that receive row diffs, so unchanged rows
//! keep their items and widgets.

use crate::host::{Signal, Wakes};
use crate::images::{Avatar, AvatarView, Icon, Images};
use crate::rows::{self, Action, Row};
use crate::ui::{Ui, label};
use crate::uploads::{ChipAction, DraftFile};
use caper_client::api::Api;
use caper_client::gateway::GatewayEvent;
use caper_client::mentions;
use caper_client::model::{
    Account, Apply, ChatSession, DirectConversation, Message, Space, SpaceDetail, Timeline,
};
use caper_client::reactions::Reacted;
use caper_client::sign_in::{MAX_RESENDS, resend_countdown, resend_wait};
use caper_client::worker::{
    AccountOperation, AccountResult, AdminOperation, AdminResult, Command, Event, Worker, current,
};
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

#[path = "members.rs"]
mod members;
#[path = "profile.rs"]
mod profile;

const STYLE: &str = include_str!("style.css");
/// Others' typing shows this long after their last update (desktop's).
const TYPING_SHOWN: Duration = Duration::from_secs(6);
/// Your typing is re-announced this often while you type, and ends this
/// long after your last keystroke.
const TYPING_PULSE: Duration = Duration::from_secs(2);
const TYPING_IDLE: Duration = Duration::from_secs(3);

/// A widget of GType `class` with the accessible name `name`. gtk4-rs binds
/// `gtk_accessible_update_property` only from GTK 4.10 (`v4_10`), so this uses
/// GtkBuilder's `<accessibility>` element, which GTK 4.0 already reads.
pub fn named<T: IsA<glib::Object>>(class: &str, name: &str) -> T {
    let xml = format!(
        "<interface><object class=\"{class}\" id=\"widget\"><accessibility><property name=\"label\">{}</property></accessibility></object></interface>",
        glib::markup_escape_text(name)
    );
    gtk::Builder::from_string(&xml)
        .object("widget")
        .expect("a GTK widget class")
}

fn clear(container: &gtk::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
}

/// `gtk_list_box_remove_all` is GTK 4.12; rows must leave through `remove`
/// so the list's own bookkeeping forgets them.
fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Loading,
    SignIn,
    Chat,
}

struct Pending {
    client_id: String,
    text: String,
    created_at: String,
    thread: Option<String>,
    /// Its uploaded files, returned to the composer if the send fails.
    files: Vec<DraftFile>,
}

/// What the main composer is doing besides a new message.
enum Compose {
    Reply {
        id: String,
        author: String,
    },
    Edit {
        original: Box<Message>,
        request: u64,
    },
}

struct Thread {
    root: String,
    /// The root as the thread read returned it (it may be outside history).
    message: Option<Message>,
    request: u64,
    loading: bool,
}

struct Typer {
    name: String,
    revision: u64,
    expires: Instant,
}

#[derive(Default)]
struct State {
    page: Option<Page>,
    /// Fences conversation events: advanced by every conversation load.
    generation: u64,
    /// Fences account events: advanced by every sign-in and sign-out.
    epoch: u64,
    navigation: u64,
    request: u64,
    busy: bool,
    error: Option<String>,
    email: String,
    challenge: Option<String>,
    code_sent_at: Option<Instant>,
    resends: u8,
    resending: bool,
    signin_status: Option<&'static str>,
    token: Option<String>,
    account: Option<Account>,
    spaces: Vec<Space>,
    space: Option<String>,
    detail: Option<SpaceDetail>,
    channel: Option<String>,
    /// The open DM, when `channel` is one.
    direct: Option<String>,
    directs: Vec<DirectConversation>,
    channel_name: String,
    session: Option<ChatSession>,
    session_error: Option<String>,
    timeline: Timeline,
    has_more: bool,
    loading_older: bool,
    status: String,
    pending: Option<Pending>,
    send_error: Option<String>,
    /// A draft that failed to send, returned to an empty composer.
    returned_draft: Option<(String, bool)>,
    compose: Option<Compose>,
    edit_request: u64,
    delete_request: u64,
    reactions: caper_client::reactions::Reactions,
    joining: bool,
    thread: Option<Thread>,
    rows: Vec<Row>,
    thread_rows: Vec<Row>,
    markup: rows::Markup,
    /// Unread messages and mentions per space, then channel.
    unread: HashMap<String, HashMap<String, (u64, u64)>>,
    unread_revision: u64,
    /// The newest sequence each conversation was marked read through.
    read_through: HashMap<String, u64>,
    presence: HashMap<String, String>,
    typers: HashMap<String, Typer>,
    typing_sent: Option<Instant>,
    typing_edited: Option<Instant>,
    /// Advanced when the rail, sidebar or member list changes.
    layout: u64,
    /// Upload settings once `GET /api/assets/usage` answered (`None`:
    /// unavailable or not yet known).
    uploads: Option<caper_client::compress::Compression>,
    uploads_checked: bool,
    /// Files in every composer, uploading or uploaded.
    files: Vec<DraftFile>,
    file_keys: u64,
    /// Advanced when any file changes, so chips redraw.
    files_revision: u64,
    /// `GET /api/people`: `@` suggestions in DMs and the new DM field.
    people: Option<Vec<caper_client::model::Person>>,
    /// The new DM dialog: its lookup request, whether a lookup or the
    /// create is pending, the person found, and its error.
    direct_lookup: u64,
    direct_finding: bool,
    direct_opening: bool,
    direct_recipient: Option<caper_client::model::Person>,
    direct_error: Option<String>,
    /// A DM created from the dialog, opened once events are handled.
    open_created: Option<String>,
    /// "Go to message" from Pins: earlier pages load until it is in view.
    jump_to: Option<String>,
    /// The open profile card's state.
    card: Option<profile::CardState>,
    /// People you blocked (`GET /api/blocks`), once loaded.
    blocks: Option<Vec<caper_client::model::BlockedAccount>>,
    block_busy: bool,
    /// Blocked runs you chose to show, by their first message.
    shown_blocked: HashSet<String>,
    /// Fences space invitations and membership changes.
    membership_request: u64,
}

impl State {
    fn page(&self) -> Page {
        self.page.unwrap_or(Page::Loading)
    }

    fn joined(&self) -> bool {
        self.direct.is_some()
            || self
                .detail
                .as_ref()
                .and_then(|detail| {
                    detail
                        .channels
                        .iter()
                        .find(|channel| Some(&channel.id) == self.channel.as_ref())
                })
                .is_none_or(|channel| channel.joined)
    }

    fn message(&self, id: &str) -> Option<&Message> {
        self.timeline
            .messages()
            .find(|message| message.id == id)
            .or_else(|| {
                self.thread
                    .as_ref()
                    .and_then(|thread| thread.message.as_ref())
                    .filter(|message| message.id == id)
            })
    }

    /// Who `@` can suggest, never yourself, and which specials apply, as on
    /// the desktop: a space channel offers its members, `everyone` and
    /// `here`; a DM offers `GET /api/people`, plus its peer.
    fn mention_people(&self) -> (Vec<mentions::Person>, mentions::Specials) {
        let me = self.account.as_ref().map(|account| account.id.as_str());
        if let Some(id) = &self.direct {
            let known = self.people.as_deref().unwrap_or_default();
            let peer = self
                .directs
                .iter()
                .find(|direct| &direct.id == id)
                .map(|direct| &direct.peer)
                .filter(|peer| !known.iter().any(|person| person.id == peer.id))
                .map(|peer| caper_client::model::Person {
                    admin: peer.admin,
                    id: peer.id.clone(),
                    username: peer.username.clone(),
                    display_name: peer.display_name.clone(),
                    avatar_id: peer.avatar_id,
                    avatar_asset: peer.avatar_asset.clone(),
                    avatar_url: peer.avatar_url.clone(),
                });
            let people = known
                .iter()
                .chain(peer.as_ref())
                .filter(|person| Some(person.id.as_str()) != me)
                .map(mentions::Person::from)
                .collect();
            return (
                people,
                mentions::Specials {
                    channel: false,
                    feedback: true,
                },
            );
        }
        let people = self
            .detail
            .as_ref()
            .filter(|detail| self.space.as_ref() == Some(&detail.space.id))
            .map_or_else(Vec::new, |detail| {
                detail
                    .members
                    .iter()
                    .filter(|member| Some(member.id.as_str()) != me)
                    .map(mentions::Person::from)
                    .collect()
            });
        (
            people,
            mentions::Specials {
                channel: true,
                feedback: true,
            },
        )
    }

    fn name(&self) -> String {
        self.account
            .as_ref()
            .and_then(|account| account.display_name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Guest".into())
    }
}

pub struct App {
    ui: Ui,
    images: Rc<Images>,
    worker: Worker<Wakes>,
    state: RefCell<State>,
    /// Keep the newest message in view as rows arrive and are measured.
    follow: Cell<bool>,
    /// `pin_tail` is moving the list: its scrolling is not the reader's.
    settling: Cell<bool>,
    rendered_layout: Cell<u64>,
    rendered_files: Cell<u64>,
    /// The open file dialog: GTK drops a native dialog nothing holds.
    chooser: RefCell<Option<gtk::FileChooserNative>>,
    channel_ids: RefCell<Vec<String>>,
    direct_ids: RefCell<Vec<String>>,
    /// What the member list shows.
    members_shown: RefCell<members::Shown>,
    activity: Cell<Instant>,
    /// The typing timer runs only while someone types.
    ticking: Cell<bool>,
    /// The sign-in countdown's pending redraw.
    countdown: Cell<bool>,
    /// The channel and thread composers' `:`/`@` suggestions.
    suggestions: RefCell<Vec<Rc<crate::suggest::Popup>>>,
    direct_dialog: RefCell<Option<DirectDialog>>,
    /// The open Pins view: its list and the rows it shows.
    pins: RefCell<Option<PinsView>>,
    /// The open profile card's widgets.
    profile: RefCell<Option<profile::Card>>,
}

struct PinsView {
    popover: gtk::Popover,
    list: gtk::Box,
    rows: Vec<Row>,
    /// Each card's widgets, alive while it shows.
    cards: Vec<rows::Standalone>,
}

/// The new DM dialog's widgets.
struct DirectDialog {
    window: gtk::Window,
    field: Rc<crate::people::PeopleField>,
    recipient: gtk::Label,
    error: gtk::Label,
    open: gtk::Button,
}

pub fn start(application: &gtk::Application, base: &str) {
    let api = match Api::new(base) {
        Ok(api) => api,
        Err(error) => {
            eprintln!("{error}");
            application.quit();
            return;
        }
    };
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_application_prefer_dark_theme(true);
    }
    if let Some(display) = gdk::Display::default() {
        let provider = gtk::CssProvider::new();
        provider.load_from_data(STYLE);
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    let (sender, receiver) = async_channel::unbounded();
    let images = Rc::new(Images::new(api.clone(), sender.clone()));
    // Rows reach the app once it exists.
    let slot: Rc<RefCell<Weak<App>>> = Rc::default();
    let handler = |thread: bool| -> rows::Handler {
        let slot = slot.clone();
        Rc::new(move |action| {
            if let Some(app) = slot.borrow().upgrade() {
                app.row_action(action, thread);
            }
        })
    };
    let ui = Ui::build(application, &images, handler(false), handler(true));
    let app = Rc::new(App {
        ui,
        images,
        worker: Worker::new(api, Wakes(sender)),
        state: RefCell::new(State::default()),
        follow: Cell::new(true),
        settling: Cell::new(false),
        rendered_layout: Cell::new(u64::MAX),
        rendered_files: Cell::new(u64::MAX),
        chooser: RefCell::default(),
        channel_ids: RefCell::default(),
        direct_ids: RefCell::default(),
        members_shown: RefCell::default(),
        activity: Cell::new(Instant::now()),
        ticking: Cell::new(false),
        countdown: Cell::new(false),
        suggestions: RefCell::default(),
        direct_dialog: RefCell::default(),
        pins: RefCell::default(),
        profile: RefCell::default(),
    });
    *slot.borrow_mut() = Rc::downgrade(&app);
    for composer in [&app.ui.composer, &app.ui.thread_composer] {
        let weak = Rc::downgrade(&app);
        let popup = crate::suggest::Popup::new(composer, app.images.clone(), move || {
            let app = weak.upgrade()?;
            let state = app.state.try_borrow().ok()?;
            Some(state.mention_people())
        });
        app.suggestions.borrow_mut().push(popup);
    }
    app.connect();
    app.connect_drop();
    app.install_actions();
    app.render();
    // The window shows before any network request finishes.
    app.ui.window.present();
    app.worker.send(Command::Restore { generation: 0 });
    glib::spawn_future_local(async move {
        while let Ok(signal) = receiver.recv().await {
            match signal {
                Signal::Wake => app.receive(),
                Signal::Title(title) => app.ui.window.set_title(Some(&title)),
                Signal::Front => app.ui.window.present(),
                Signal::Media {
                    generation,
                    key,
                    url,
                    result,
                } => app.images.media.loaded(generation, key, url, result),
                Signal::RefreshMedia => app.refresh_media(),
                Signal::Picture { asset, url, image } => {
                    app.images.loaded(asset, url, image);
                    app.state.borrow_mut().layout += 1;
                    app.render();
                }
            }
        }
    });
}

impl App {
    fn connect(self: &Rc<Self>) {
        let ui = &self.ui;
        let app = self.clone();
        ui.email.connect_activate(move |_| app.request_code(false));
        let app = self.clone();
        ui.email_button
            .connect_clicked(move |_| app.request_code(false));
        let app = self.clone();
        ui.code.connect_activate(move |_| app.verify());
        let app = self.clone();
        ui.code_button.connect_clicked(move |_| app.verify());
        let app = self.clone();
        ui.resend.connect_clicked(move |_| app.request_code(true));
        let app = self.clone();
        ui.other_email.connect_clicked(move |_| {
            {
                let mut state = app.state.borrow_mut();
                state.challenge = None;
                state.error = None;
                state.signin_status = None;
                state.resends = 0;
            }
            app.render();
        });
        let app = self.clone();
        ui.sign_out.connect_clicked(move |_| app.sign_out());

        let app = self.clone();
        ui.members_toggle.connect_toggled(move |toggle| {
            if toggle.is_active() {
                app.state.borrow_mut().thread = None;
            }
            app.render();
        });
        let app = self.clone();
        ui.thread_close.connect_clicked(move |_| {
            app.state.borrow_mut().thread = None;
            app.render();
        });

        let app = self.clone();
        ui.channels.connect_row_activated(move |_, row| {
            let id = usize::try_from(row.index())
                .ok()
                .and_then(|index| app.channel_ids.borrow().get(index).cloned());
            if let Some(id) = id {
                let mut state = app.state.borrow_mut();
                if state.channel.as_deref() != Some(&id) {
                    let space = state.space.clone();
                    app.navigate(&mut state, space, Some(id));
                }
                drop(state);
                app.render();
            }
        });
        let app = self.clone();
        ui.directs.connect_row_activated(move |_, row| {
            let id = usize::try_from(row.index())
                .ok()
                .and_then(|index| app.direct_ids.borrow().get(index).cloned());
            if let Some(id) = id {
                app.open_direct(id);
            }
        });
        let app = self.clone();
        ui.join.connect_clicked(move |_| app.join());
        let app = self.clone();
        ui.member_search
            .connect_search_changed(move |_| app.render_members());
        let app = self.clone();
        ui.member_search.connect_stop_search(move |entry| {
            entry.set_text("");
            entry.set_visible(false);
            app.render_members();
        });
        let entry = ui.member_search.clone();
        ui.member_search_button.connect_clicked(move |_| {
            entry.set_visible(true);
            entry.grab_focus();
        });
        let toggle = ui.members_toggle.clone();
        ui.members_close
            .connect_clicked(move |_| toggle.set_active(false));
        let app = self.clone();
        ui.pins_button
            .connect_clicked(move |button| app.open_pins(button.upcast_ref()));
        let app = self.clone();
        ui.new_direct
            .connect_clicked(move |_| app.open_start_direct());
        let app = self.clone();
        ui.compose_cancel
            .connect_clicked(move |_| app.cancel_compose());

        for thread in [false, true] {
            let composer = if thread {
                &ui.thread_composer
            } else {
                &ui.composer
            };
            // Enter sends; Shift+Enter is a new line; Escape cancels a
            // reply or an edit.
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let app = self.clone();
            keys.connect_key_pressed(move |_, key, _, modifiers| {
                app.active();
                let popup = app.suggestions.borrow().get(usize::from(thread)).cloned();
                if popup.is_some_and(|popup| popup.key(key, modifiers)) {
                    return glib::Propagation::Stop;
                }
                if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                    && !modifiers.contains(gdk::ModifierType::SHIFT_MASK)
                {
                    app.send_message(thread);
                    return glib::Propagation::Stop;
                }
                if matches!(key, gdk::Key::v | gdk::Key::V)
                    && modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                    && app.paste_files(thread)
                {
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::Escape && !thread && app.state.borrow().compose.is_some() {
                    app.cancel_compose();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            composer.view.add_controller(keys);
            let app = self.clone();
            composer
                .send
                .connect_clicked(move |_| app.send_message(thread));
            let app = self.clone();
            composer
                .attach
                .connect_clicked(move |button| app.attach_menu(thread, button.upcast_ref()));
            let app = self.clone();
            composer.view.buffer().connect_changed(move |buffer| {
                if buffer.char_count() > 0 {
                    app.typed();
                }
            });
        }

        let adjustment = ui.list.scroller.vadjustment();
        let app = self.clone();
        adjustment.connect_value_changed(move |adjustment| {
            if app.settling.get() {
                return;
            }
            let bottom = adjustment.upper() - adjustment.page_size();
            app.follow.set(adjustment.value() >= bottom - 48.0);
            app.active();
            // Near the top: load earlier history, after this signal.
            if adjustment.upper() > adjustment.page_size()
                && adjustment.value() < adjustment.page_size()
            {
                let app = app.clone();
                glib::idle_add_local_once(move || app.load_older());
            }
        });
    }

    /// The row menu's actions, each taking the message id.
    fn install_actions(self: &Rc<Self>) {
        let group = gio::SimpleActionGroup::new();
        type Run = fn(&Rc<App>, String);
        let actions: [(&str, Run); 5] = [
            ("edit", |app, id| app.start_edit(&id)),
            ("delete", |app, id| app.confirm_delete(id)),
            ("pin", |app, id| app.pin(id, true)),
            ("unpin", |app, id| app.pin(id, false)),
            ("copy", |app, id| {
                if let Some(message) = app.state.borrow().message(&id) {
                    app.ui.window.clipboard().set_text(&message.content.text);
                }
            }),
        ];
        for (name, run) in actions {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let app = Rc::downgrade(self);
            action.connect_activate(move |_, parameter| {
                if let (Some(app), Some(id)) = (
                    app.upgrade(),
                    parameter.and_then(|value| value.get::<String>()),
                ) {
                    run(&app, id);
                }
            });
            group.add_action(&action);
        }
        self.ui.window.insert_action_group("row", Some(&group));
    }

    /// Asks for fresh signed URLs for the media that wants them.
    fn refresh_media(&self) {
        let Some(token) = self.state.borrow().token.clone() else {
            return;
        };
        let ids = self.images.media.take_refresh();
        if !ids.is_empty() {
            self.worker
                .send(Command::RefreshAttachmentUrls { token, ids });
        }
    }

    /// Tells the gateway someone is here, at most every few seconds.
    fn active(&self) {
        if self.activity.get().elapsed() > Duration::from_secs(5) {
            self.activity.set(Instant::now());
            self.worker.send(Command::Activity);
        }
    }

    // ---------------------------------------------------------------------
    // Sign-in and sign-out.

    fn request_code(self: &Rc<Self>, resend: bool) {
        {
            let mut state = self.state.borrow_mut();
            if state.busy {
                return;
            }
            let email = if resend {
                state.email.clone()
            } else {
                self.ui.email.text().trim().to_owned()
            };
            if email.contains('@') {
                state.busy = true;
                state.error = None;
                state.signin_status = None;
                state.resending = resend;
                if !resend {
                    state.resends = 0;
                }
                state.email.clone_from(&email);
                self.worker.send(Command::RequestCode {
                    generation: state.generation,
                    email,
                });
            } else {
                state.error = Some("Enter your email address.".into());
            }
        }
        self.render();
    }

    fn verify(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let mut code = self.ui.code.text().to_string();
            caper_client::sign_in::clean_sign_in_code(&mut code);
            if state.busy {
                return;
            }
            let Some(challenge) = state.challenge.clone() else {
                return;
            };
            if code.is_empty() {
                state.error = Some("Enter the code from your email.".into());
            } else {
                state.busy = true;
                state.error = None;
                self.worker.send(Command::VerifyCode {
                    generation: state.generation,
                    challenge,
                    code,
                });
            }
        }
        self.render();
    }

    fn establish(&self, state: &mut State, token: String, account: Account, spaces: Vec<Space>) {
        state.epoch += 1;
        state.token = Some(token);
        state.account = Some(account);
        state.spaces = spaces;
        state.page = Some(Page::Chat);
        state.layout += 1;
        self.load_directs(state);
        self.refresh_unread(state);
        if let Some(token) = state.token.clone() {
            self.worker.send(Command::AssetUsage { token });
        }
        self.account_op(state, AccountOperation::LoadBlocks);
        if let Some(space) = state.spaces.first().map(|space| space.id.clone()) {
            self.navigate(state, Some(space), None);
        }
    }

    /// Desktop's sign-out: stop the gateway, forget the saved session, revoke
    /// it on the server, and return to sign-in.
    fn sign_out(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let token = state.token.take();
            let uploaded: Vec<String> = state
                .files
                .iter()
                .inspect(|file| file.stop())
                .filter_map(|file| file.attachment.as_ref().map(|file| file.id.clone()))
                .collect();
            if let Some(token) = token.clone()
                && !uploaded.is_empty()
            {
                self.worker.send(Command::CancelAssets {
                    token,
                    ids: uploaded,
                });
            }
            state.generation += 1;
            self.worker.send(Command::StopGateway);
            self.worker.send(Command::ClearCredential {
                generation: state.generation,
            });
            if let Some(token) = token {
                self.worker.send(Command::Logout {
                    generation: state.generation,
                    token,
                });
            }
            self.worker.showing(None);
            let keep = (state.generation, state.epoch + 1, state.email.clone());
            *state = State {
                page: Some(Page::SignIn),
                generation: keep.0,
                epoch: keep.1,
                email: keep.2,
                layout: state.layout + 1,
                rows: std::mem::take(&mut state.rows),
                thread_rows: std::mem::take(&mut state.thread_rows),
                ..State::default()
            };
        }
        self.images.clear_pictures();
        self.ui.code.set_text("");
        self.ui.composer.view.buffer().set_text("");
        self.ui.thread_composer.view.buffer().set_text("");
        self.render();
    }

    // ---------------------------------------------------------------------
    // Navigation.

    fn navigate(&self, state: &mut State, space: Option<String>, channel: Option<String>) {
        self.leave_conversation(state);
        if state.space != space {
            state.detail = None;
            state.space.clone_from(&space);
            state.presence.clear();
        }
        if let Some(name) = state.detail.as_ref().and_then(|detail| {
            detail
                .channels
                .iter()
                .find(|entry| Some(&entry.id) == channel.as_ref())
                .map(|entry| entry.name.clone())
        }) {
            state.channel_name = name;
        }
        state.channel.clone_from(&channel);
        self.prepare(state, space, channel);
    }

    fn open_direct(self: &Rc<Self>, id: String) {
        {
            let mut state = self.state.borrow_mut();
            if state.channel.as_deref() == Some(&id) {
                return;
            }
            self.leave_conversation(&mut state);
            state.channel_name = state
                .directs
                .iter()
                .find(|direct| direct.id == id)
                .map(|direct| direct.peer.display_name.clone())
                .unwrap_or_default();
            state.channel = Some(id.clone());
            state.direct = Some(id.clone());
            self.load_people(&state);
            self.prepare(&mut state, None, Some(id));
        }
        self.render();
    }

    fn leave_conversation(&self, state: &mut State) {
        self.set_typing(state, false);
        state.generation += 1;
        state.navigation += 1;
        state.loading_older = false;
        state.session = None;
        state.session_error = None;
        state.pending = None;
        state.compose = None;
        state.thread = None;
        state.typers.clear();
        state.reactions.clear();
        state.files_revision += 1;
        state.direct = None;
        state.jump_to = None;
        if let Some(pins) = self.pins.borrow_mut().take() {
            pins.popover.popdown();
        }
        state.status = "Loading…".into();
        state.error = None;
        state.send_error = None;
        state.layout += 1;
        // DMs started or answered elsewhere appear as you move around.
        self.load_directs(state);
    }

    fn prepare(&self, state: &mut State, space: Option<String>, channel: Option<String>) {
        self.worker.send(Command::PrepareNavigation {
            generation: state.generation,
            navigation: state.navigation,
            token: state.token.clone(),
            space,
            channel,
            name: state.name(),
            cached: None,
            visit: false,
            preferred: None,
        });
    }

    fn select_space(self: &Rc<Self>, id: String) {
        {
            let mut state = self.state.borrow_mut();
            if state.space.as_deref() == Some(&id) && state.direct.is_none() {
                return;
            }
            self.navigate(&mut state, Some(id), None);
        }
        self.render();
    }

    fn reload(&self, state: &mut State) {
        match state.direct.clone() {
            Some(direct) => {
                self.leave_conversation(state);
                state.channel = Some(direct.clone());
                state.direct = Some(direct.clone());
                self.prepare(state, None, Some(direct));
            }
            None => {
                let (space, channel) = (state.space.clone(), state.channel.clone());
                self.navigate(state, space, channel);
            }
        }
    }

    fn connect_gateway(&self, state: &mut State, channel: String) {
        state.status = "Connecting…".into();
        self.worker.send(Command::Connect {
            generation: state.generation,
            token: state.token.clone(),
            channel,
            cursor: state.timeline.cursor(),
            presence: state
                .detail
                .as_ref()
                .filter(|_| state.token.is_some())
                .map(|detail| (detail.space.id.clone(), Vec::new())),
            media: Default::default(),
        });
    }

    fn join(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let (Some(token), Some(space), Some(channel)) = (
                state.token.clone(),
                state.space.clone(),
                state.channel.clone(),
            ) else {
                return;
            };
            if state.joining {
                return;
            }
            state.joining = true;
            state.error = None;
            self.worker.send(Command::Admin {
                generation: state.generation,
                token,
                operation: AdminOperation::JoinChannel { space, channel },
            });
        }
        self.render();
    }

    fn load_older(self: &Rc<Self>) {
        let Ok(mut state) = self.state.try_borrow_mut() else {
            return;
        };
        if state.loading_older || !state.has_more {
            return;
        }
        let Some(channel) = state.channel.clone() else {
            return;
        };
        let Some(before) = state
            .timeline
            .messages()
            .find(|message| message.is_channel_message())
            .map(|message| message.seq.clone())
        else {
            return;
        };
        state.loading_older = true;
        state.request += 1;
        self.worker.send(Command::LoadOlder {
            generation: state.generation,
            request: state.request,
            token: state.token.clone(),
            channel,
            before,
        });
    }

    // ---------------------------------------------------------------------
    // Unread state and DMs.

    /// The desktop's New direct message dialog: an exact username, with
    /// known people suggested below it. "Find person" looks the name up;
    /// once someone is known, "Open conversation" creates or reopens the DM.
    fn open_start_direct(self: &Rc<Self>) {
        self.close_start_direct();
        {
            let mut state = self.state.borrow_mut();
            if state.token.is_none() {
                return;
            }
            state.direct_lookup += 1;
            state.direct_finding = false;
            state.direct_opening = false;
            state.direct_recipient = None;
            state.direct_error = None;
            self.load_people(&state);
        }
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title("New direct message")
            .default_width(420)
            .resizable(false)
            .build();
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("dialog-body");
        root.append(&label("New direct message", &["dialog-title"]));
        root.append(&label("Enter an exact username.", &["muted"]));
        let field = crate::people::PeopleField::new(self.images.clone(), "Username");
        root.append(&field.root);
        let recipient = label("", &[]);
        recipient.set_visible(false);
        root.append(&recipient);
        let error = label("", &["error"]);
        error.set_wrap(true);
        error.set_visible(false);
        root.append(&error);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        let cancel = gtk::Button::with_label("Cancel");
        let open = gtk::Button::with_label("Find person");
        open.add_css_class("primary");
        buttons.append(&cancel);
        buttons.append(&open);
        root.append(&buttons);
        window.set_child(Some(&root));

        let app = self.clone();
        field.connect_changed(move |_| {
            {
                let mut state = app.state.borrow_mut();
                state.direct_lookup += 1;
                state.direct_finding = false;
                state.direct_recipient = None;
                state.direct_error = None;
            }
            app.render_start_direct();
        });
        let app = self.clone();
        field
            .entry
            .connect_activate(move |_| app.submit_start_direct());
        let app = self.clone();
        open.connect_clicked(move |_| app.submit_start_direct());
        let app = self.clone();
        cancel.connect_clicked(move |_| app.close_start_direct());
        let app = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(app) = app.upgrade() {
                app.direct_dialog.borrow_mut().take();
            }
            glib::Propagation::Proceed
        });
        *self.direct_dialog.borrow_mut() = Some(DirectDialog {
            window: window.clone(),
            field: field.clone(),
            recipient,
            error,
            open,
        });
        self.render_start_direct();
        window.present();
        field.entry.grab_focus();
    }

    fn close_start_direct(&self) {
        let dialog = self.direct_dialog.borrow_mut().take();
        if let Some(dialog) = dialog {
            dialog.window.destroy();
        }
    }

    fn submit_start_direct(self: &Rc<Self>) {
        let Some(username) = self
            .direct_dialog
            .borrow()
            .as_ref()
            .map(|dialog| dialog.field.text())
        else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            let Some(token) = state.token.clone() else {
                return;
            };
            if state.direct_finding || state.direct_opening || username.len() < 3 {
                return;
            }
            state.direct_error = None;
            let known = state.direct_recipient.clone().or_else(|| {
                state
                    .people
                    .iter()
                    .flatten()
                    .find(|person| person.username == username)
                    .cloned()
            });
            if let Some(person) = known {
                state.direct_opening = true;
                self.worker.send(Command::CreateDirect {
                    generation: state.epoch,
                    navigation: state.navigation,
                    token,
                    username: person.username,
                });
            } else {
                state.direct_lookup += 1;
                state.direct_finding = true;
                self.worker.send(Command::LookupDirect {
                    generation: state.epoch,
                    navigation: state.navigation,
                    request: state.direct_lookup,
                    token,
                    username,
                });
            }
        }
        self.render_start_direct();
    }

    fn render_start_direct(&self) {
        let dialog = self.direct_dialog.borrow();
        let Some(dialog) = dialog.as_ref() else {
            return;
        };
        let state = self.state.borrow();
        let text = dialog.field.text();
        let pending = state.direct_finding || state.direct_opening;
        let people: Vec<mentions::Person> = state
            .people
            .iter()
            .flatten()
            .filter(|_| !pending)
            .filter(|person| Some(&person.id) != state.account.as_ref().map(|account| &account.id))
            .map(mentions::Person::from)
            .collect();
        dialog.field.set_people(people);
        dialog.field.entry.set_editable(!pending);
        let recipient = state.direct_recipient.as_ref().or_else(|| {
            state
                .people
                .iter()
                .flatten()
                .find(|person| person.username == text)
        });
        match recipient {
            Some(person) => {
                dialog
                    .recipient
                    .set_label(&format!("{} · @{}", person.display_name, person.username));
                dialog.recipient.set_visible(true);
            }
            None => dialog.recipient.set_visible(false),
        }
        dialog.open.set_label(if state.direct_opening {
            "Opening…"
        } else if state.direct_finding {
            "Finding…"
        } else if recipient.is_some() {
            "Open conversation"
        } else {
            "Find person"
        });
        dialog.open.set_sensitive(!pending && text.len() >= 3);
        let error = state.direct_error.as_deref().unwrap_or_default();
        dialog.error.set_label(error);
        dialog.error.set_visible(!error.is_empty());
    }

    /// The desktop's Pins: the conversation's pinned messages, newest pin
    /// first, each with Go to message.
    fn open_pins(self: &Rc<Self>, anchor: &gtk::Widget) {
        if let Some(pins) = self.pins.borrow_mut().take() {
            pins.popover.popdown();
        }
        let popover = gtk::Popover::new();
        popover.add_css_class("pins");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.set_size_request(560, -1);
        root.append(&label("Pins", &["dialog-title"]));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(640)
            .child(&list)
            .build();
        root.append(&scroller);
        popover.set_child(Some(&root));
        let app = Rc::downgrade(self);
        popover.connect_closed(move |_| {
            if let Some(app) = app.upgrade()
                && let Ok(mut pins) = app.pins.try_borrow_mut()
            {
                pins.take();
            }
        });
        *self.pins.borrow_mut() = Some(PinsView {
            popover: popover.clone(),
            list,
            rows: Vec::new(),
            cards: Vec::new(),
        });
        popup(&popover, anchor);
        self.render_pins();
    }

    fn render_pins(self: &Rc<Self>) {
        let Ok(mut pins) = self.pins.try_borrow_mut() else {
            return;
        };
        let Some(view) = pins.as_mut() else {
            return;
        };
        let rows = {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            let me = state.account.as_ref().map(|account| account.id.clone());
            let author = state
                .session
                .as_ref()
                .map(|session| session.author.id.clone());
            let joined = state.session.is_some() && state.joined();
            let revealed = self.images.media.revealed.borrow();
            let blocked = state.blocked_ids();
            let viewer = rows::Viewer {
                revealed: &revealed,
                blocked: &blocked,
                shown_blocked: &state.shown_blocked,
                me: me.as_deref(),
                author: author.as_deref(),
                joined,
                in_thread: true,
                images: &self.images,
                reactions: &state.reactions,
            };
            let pinned: Vec<Message> = state.timeline.pinned_messages().cloned().collect();
            pinned
                .iter()
                .flat_map(|message| {
                    rows::build(std::iter::once(message), None, &mut state.markup, &viewer)
                })
                .map(|mut row| {
                    row.grouped = false;
                    row.day = None;
                    row
                })
                .collect::<Vec<Row>>()
        };
        if rows == view.rows {
            return;
        }
        while let Some(child) = view.list.first_child() {
            view.list.remove(&child);
        }
        view.cards.clear();
        if rows.is_empty() {
            let empty = gtk::Box::new(gtk::Orientation::Vertical, 4);
            empty.add_css_class("pins-empty");
            empty.append(&label("No pinned messages.", &["field"]));
            empty.append(&label(
                "Messages you pin will appear here.",
                &["muted", "small"],
            ));
            view.list.append(&empty);
        }
        let weak = Rc::downgrade(self);
        let handler: rows::Handler = Rc::new(move |action| {
            if let Some(app) = weak.upgrade() {
                app.row_action(action, true);
            }
        });
        for row in &rows {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
            card.add_css_class("pin-card");
            let standalone = rows::Standalone::new(&self.images, row, &handler);
            card.append(standalone.root());
            let go = gtk::Button::with_label("Go to message");
            go.add_css_class("flat");
            go.set_halign(gtk::Align::End);
            let (weak, id) = (Rc::downgrade(self), row.id.clone());
            go.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.go_to_message(id.clone());
                }
            });
            card.append(&go);
            view.list.append(&card);
            view.cards.push(standalone);
        }
        view.rows = rows;
    }

    /// Closes Pins and scrolls to `id`, loading earlier pages until it is
    /// in the list; a thread reply opens its thread.
    fn go_to_message(self: &Rc<Self>, id: String) {
        if let Some(pins) = self.pins.borrow_mut().take() {
            pins.popover.popdown();
        }
        let root = self
            .state
            .borrow()
            .timeline
            .pinned_messages()
            .find(|message| message.id == id)
            .and_then(|message| message.thread_root_id.clone());
        if let Some(root) = root {
            self.open_thread(root);
            return;
        }
        self.state.borrow_mut().jump_to = Some(id);
        self.continue_jump();
    }

    fn continue_jump(self: &Rc<Self>) {
        let (position, more) = {
            let state = self.state.borrow();
            let Some(id) = &state.jump_to else {
                return;
            };
            (
                state.rows.iter().position(|row| &row.id == id),
                state.has_more && !state.loading_older,
            )
        };
        match position {
            Some(position) => {
                self.state.borrow_mut().jump_to = None;
                self.follow.set(false);
                let _ = self.ui.list.view.activate_action(
                    "list.scroll-to-item",
                    Some(&u32::try_from(position).unwrap_or_default().to_variant()),
                );
            }
            None if more => self.load_older(),
            None => {
                let loading = self.state.borrow().loading_older;
                if !loading {
                    self.state.borrow_mut().jump_to = None;
                }
            }
        }
    }

    /// An account-scoped request (blocks, invitations, settings), fenced by
    /// the sign-in epoch.
    fn account_op(&self, state: &State, operation: AccountOperation) {
        if let Some(token) = state.token.clone() {
            self.worker.send(Command::Account {
                epoch: state.epoch,
                token,
                operation,
            });
        }
    }

    fn account_result(&self, state: &mut State, result: AccountResult) {
        match result {
            AccountResult::Blocks(Ok(blocks)) => state.blocks = Some(blocks),
            AccountResult::Block {
                account,
                blocked,
                result,
                ..
            } => {
                state.block_busy = false;
                let card = state.card.as_mut().filter(|card| card.person == account.id);
                if let Err(error) = result {
                    match card {
                        Some(card) => card.error = Some(error),
                        None => state.error = Some(error),
                    }
                    return;
                }
                if !blocked && let Some(card) = card {
                    card.status = Some("Unblocked.".into());
                }
                let blocks = state.blocks.get_or_insert_with(Vec::new);
                blocks.retain(|entry| entry.id != account.id);
                if blocked {
                    blocks.insert(0, account.clone());
                }
                for direct in &mut state.directs {
                    if direct.peer.id == account.id {
                        direct.blocked = blocked;
                    }
                }
                state.layout += 1;
            }
            AccountResult::Invitation {
                space,
                user,
                request,
                result,
            } => {
                if let Some(card) = state.card.as_mut().filter(|card| {
                    card.person == user
                        && card.inviting.as_ref() == Some(&(request, space.id.clone()))
                }) {
                    card.inviting = None;
                    match result {
                        Ok(_) => {
                            card.invited.insert(space.id, "Invited");
                            card.status = Some(format!("Invitation sent to {}.", space.name));
                        }
                        Err(error) => {
                            if let Some(already) =
                                caper_client::invites::redundant_invitation(&error)
                            {
                                card.invited.insert(space.id, already);
                            }
                            card.error = Some(error);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Refreshes `GET /api/people`; the previous list stays until it answers.
    fn load_people(&self, state: &State) {
        if let Some(token) = state.token.clone() {
            self.worker.send(Command::LoadPeople {
                generation: state.epoch,
                token,
            });
        }
    }

    fn load_directs(&self, state: &State) {
        if let Some(token) = state.token.clone() {
            // Fenced by the account, not a conversation: navigation follows.
            self.worker.send(Command::LoadDirects {
                generation: state.epoch,
                token,
            });
        }
    }

    fn refresh_unread(&self, state: &mut State) {
        if let Some(token) = state.token.clone() {
            state.unread_revision += 1;
            self.worker.send(Command::LoadUnread {
                epoch: state.epoch,
                revision: state.unread_revision,
                visit: None,
                token,
            });
        }
    }

    /// Marks the open conversation read through its newest message.
    fn mark_read(&self, state: &mut State) {
        let (Some(token), Some(channel)) = (state.token.clone(), state.channel.clone()) else {
            return;
        };
        let seq = state.timeline.cursor();
        let through = seq.parse::<u64>().unwrap_or(0);
        if through == 0
            || state
                .read_through
                .get(&channel)
                .is_some_and(|read| *read >= through)
        {
            return;
        }
        state.read_through.insert(channel.clone(), through);
        if let Some(direct) = state.direct.clone() {
            if let Some(entry) = state.directs.iter_mut().find(|entry| entry.id == direct) {
                entry.unread_count = Some(0);
                entry.read_seq.clone_from(&seq);
                state.layout += 1;
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
    }

    // ---------------------------------------------------------------------
    // Typing.

    /// The composer changed: announce typing, at most every pulse.
    fn typed(self: &Rc<Self>) {
        let Ok(mut state) = self.state.try_borrow_mut() else {
            return;
        };
        if matches!(state.compose, Some(Compose::Edit { .. })) {
            return;
        }
        let now = Instant::now();
        state.typing_edited = Some(now);
        if state
            .typing_sent
            .is_none_or(|sent| now.duration_since(sent) >= TYPING_PULSE)
        {
            self.set_typing(&mut state, true);
        }
        drop(state);
        self.tick();
    }

    fn set_typing(&self, state: &mut State, typing: bool) {
        if !typing && state.typing_sent.is_none() {
            return;
        }
        let (Some(session), Some(channel)) = (&state.session, &state.channel) else {
            state.typing_sent = None;
            return;
        };
        self.worker.send(Command::Typing {
            token: state.token.clone(),
            chat_token: session.token.clone(),
            channel: channel.clone(),
            typing,
        });
        state.typing_sent = typing.then(Instant::now);
    }

    /// While anyone types: expire typing indicators, and end your own after
    /// a pause. Stops itself once nobody is typing.
    fn tick(self: &Rc<Self>) {
        if self.ticking.replace(true) {
            return;
        }
        let app = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(500), move || {
            let Some(app) = app.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let busy = {
                let mut state = app.state.borrow_mut();
                let now = Instant::now();
                state.typers.retain(|_, typer| typer.expires > now);
                if state.typing_sent.is_some()
                    && state
                        .typing_edited
                        .is_none_or(|edited| now.duration_since(edited) >= TYPING_IDLE)
                {
                    app.set_typing(&mut state, false);
                }
                state.typing_sent.is_some() || !state.typers.is_empty()
            };
            app.render_typing();
            if busy {
                glib::ControlFlow::Continue
            } else {
                app.ticking.set(false);
                glib::ControlFlow::Break
            }
        });
    }

    // ---------------------------------------------------------------------
    // Files: attach, paste, drop, upload, retry and remove.

    /// Whether this conversation takes files now.
    fn can_attach(state: &State) -> bool {
        state.uploads.is_some()
            && state.token.is_some()
            && state.session.is_some()
            && state.joined()
            && !state
                .detail
                .as_ref()
                .is_some_and(|detail| detail.space.demo)
    }

    fn files_in(state: &State, root: Option<&str>) -> usize {
        state
            .files
            .iter()
            .filter(|file| {
                state.channel.as_ref() == Some(&file.channel) && file.root.as_deref() == root
            })
            .count()
    }

    /// The composer's root: the open thread's for its composer.
    fn composer_root(state: &State, thread: bool) -> Option<String> {
        thread
            .then(|| state.thread.as_ref().map(|thread| thread.root.clone()))
            .flatten()
    }

    /// Starts preparing and uploading `paths` (at most ten per composer).
    fn add_files(
        self: &Rc<Self>,
        paths: Vec<(
            std::path::PathBuf,
            Option<std::sync::Arc<caper_client::uploads::Scratch>>,
        )>,
        thread: bool,
    ) {
        {
            let mut state = self.state.borrow_mut();
            let (Some(settings), Some(token), Some(channel)) = (
                state.uploads.clone(),
                state.token.clone(),
                state.channel.clone(),
            ) else {
                return;
            };
            if paths.is_empty() || !Self::can_attach(&state) {
                return;
            }
            let root = Self::composer_root(&state, thread);
            let (room, error) =
                crate::uploads::room(Self::files_in(&state, root.as_deref()), paths.len());
            state.send_error = error;
            for (path, scratch) in paths.into_iter().take(room) {
                state.file_keys += 1;
                let file = DraftFile::new(
                    state.file_keys,
                    channel.clone(),
                    root.clone(),
                    path.clone(),
                    scratch.clone(),
                );
                self.worker.send(Command::Upload {
                    key: file.key,
                    token: token.clone(),
                    channel: channel.clone(),
                    name: file.name.clone(),
                    path,
                    edit: None,
                    source_scratch: scratch,
                    prepared: None,
                    settings: settings.clone(),
                    cancel: file.cancel.clone(),
                });
                state.files.push(file);
            }
            state.files_revision += 1;
        }
        self.render();
    }

    fn upload_update(
        &self,
        state: &mut State,
        key: u64,
        token: String,
        update: caper_client::worker::UploadUpdate,
    ) {
        use caper_client::worker::UploadUpdate;
        let file = state
            .files
            .iter_mut()
            .chain(
                state
                    .pending
                    .iter_mut()
                    .flat_map(|pending| pending.files.iter_mut()),
            )
            .find(|file| file.key == key);
        let Some(file) = file else {
            // Removed while it uploaded: give its reservation back.
            if let UploadUpdate::Finished(Ok(attachment)) = update {
                self.worker.send(Command::CancelAssets {
                    token,
                    ids: vec![attachment.id],
                });
            }
            return;
        };
        match update {
            UploadUpdate::Retained(prepared) => file.prepared = Some(prepared),
            UploadUpdate::Prepared {
                name,
                kind,
                source_size,
                stored_size,
            } => {
                file.name = name;
                file.kind = kind;
                file.source_size = source_size;
                file.stored_size = Some(stored_size);
                file.compressing = None;
            }
            UploadUpdate::Compressing(fraction) => {
                file.compressing = Some(fraction.max(file.compressing.unwrap_or(0.0)));
            }
            UploadUpdate::Thumbnail(image) => {
                if let Some(attachment) = &file.attachment {
                    self.images.media.insert_local(&attachment.id, &image);
                }
                file.thumbnail = Some(image);
            }
            UploadUpdate::Progress(progress) => file.progress = progress.max(file.progress),
            UploadUpdate::Finished(Ok(attachment)) => {
                file.progress = 1.0;
                file.uploading = false;
                file.error = None;
                // You see your own copy without a download.
                if let Some(image) = &file.thumbnail {
                    self.images.media.insert_local(&attachment.id, image);
                }
                file.attachment = Some(attachment);
            }
            UploadUpdate::Finished(Err(error)) => {
                file.uploading = false;
                file.compressing = None;
                file.error = Some(if error.storage_full {
                    error.message
                } else {
                    "Failed to upload.".into()
                });
            }
        }
        state.files_revision += 1;
    }

    fn chip_action(self: &Rc<Self>, action: ChipAction) {
        {
            let mut state = self.state.borrow_mut();
            match action {
                ChipAction::Remove(key) => {
                    if let Some(index) = state.files.iter().position(|file| file.key == key) {
                        let file = state.files.remove(index);
                        file.stop();
                        if let (Some(token), Some(attachment)) =
                            (state.token.clone(), &file.attachment)
                        {
                            self.worker.send(Command::CancelAssets {
                                token,
                                ids: vec![attachment.id.clone()],
                            });
                        }
                    }
                    state.send_error = None;
                }
                ChipAction::Retry(key) => {
                    let (Some(token), Some(settings)) =
                        (state.token.clone(), state.uploads.clone())
                    else {
                        return;
                    };
                    if let Some(file) = state.files.iter_mut().find(|file| file.key == key) {
                        file.error = None;
                        file.progress = 0.0;
                        file.uploading = true;
                        self.worker.send(Command::Upload {
                            key,
                            token,
                            channel: file.channel.clone(),
                            path: file.path.clone(),
                            name: file.name.clone(),
                            edit: None,
                            source_scratch: file.scratch.clone(),
                            prepared: file.prepared.clone(),
                            settings,
                            cancel: file.cancel.clone(),
                        });
                    }
                }
                ChipAction::Spoiler(key, spoiler) => {
                    if let Some(file) = state.files.iter_mut().find(|file| file.key == key) {
                        file.spoiler = spoiler;
                    }
                }
            }
            state.files_revision += 1;
        }
        self.render();
    }

    /// The attach menu: \"Upload a file\", or why it cannot add more.
    fn attach_menu(self: &Rc<Self>, thread: bool, anchor: &gtk::Widget) {
        let attached = {
            let state = self.state.borrow();
            Self::files_in(&state, Self::composer_root(&state, thread).as_deref())
        };
        let popover = gtk::Popover::new();
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 4);
        menu.add_css_class("attach-menu");
        if let Some(hint) = (attached >= caper_client::uploads::MAX_ATTACHMENTS)
            .then_some("Remove a file to add another.")
        {
            menu.append(&label(hint, &["muted", "small"]));
        }
        let upload: gtk::Button = named("GtkButton", "Upload a file");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        line.append(&self.images.icon_image(Icon::FileUp, "#f3f4f5", 16));
        line.append(&label("Upload a file", &[]));
        upload.set_child(Some(&line));
        upload.add_css_class("flat");
        upload.set_sensitive(attached < caper_client::uploads::MAX_ATTACHMENTS);
        let (app, popover_ref) = (self.clone(), popover.downgrade());
        upload.connect_clicked(move |_| {
            if let Some(popover) = popover_ref.upgrade() {
                popover.popdown();
            }
            app.pick_files(thread);
        });
        menu.append(&upload);
        popover.set_child(Some(&menu));
        popup(&popover, anchor);
    }

    /// The system's file dialog for any files.
    fn pick_files(self: &Rc<Self>, thread: bool) {
        if self.chooser.borrow().is_some() {
            return;
        }
        #[allow(deprecated)]
        let dialog = gtk::FileChooserNative::new(
            Some("Upload a file"),
            Some(&self.ui.window),
            gtk::FileChooserAction::Open,
            Some("Upload"),
            Some("Cancel"),
        );
        #[allow(deprecated)]
        dialog.set_select_multiple(true);
        let app = self.clone();
        #[allow(deprecated)]
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                let files = dialog.files();
                let paths = (0..files.n_items())
                    .filter_map(|index| files.item(index).and_downcast::<gio::File>())
                    .filter_map(|file| file.path())
                    .map(|path| (path, None))
                    .collect();
                app.add_files(paths, thread);
            }
            dialog.destroy();
            app.chooser.replace(None);
        });
        #[allow(deprecated)]
        dialog.show();
        self.chooser.replace(Some(dialog));
    }

    /// Ctrl+V in a composer: copied files, else a copied image. Text pastes
    /// stay with the text view. Returns whether it took the paste.
    fn paste_files(self: &Rc<Self>, thread: bool) -> bool {
        if !Self::can_attach(&self.state.borrow()) {
            return false;
        }
        let clipboard = self.ui.window.clipboard();
        let formats = clipboard.formats();
        let app = self.clone();
        if formats.contains_type(gdk::FileList::static_type()) {
            clipboard.read_value_async(
                gdk::FileList::static_type(),
                glib::Priority::DEFAULT,
                gio::Cancellable::NONE,
                move |value| {
                    let paths = value
                        .ok()
                        .and_then(|value| value.get::<gdk::FileList>().ok())
                        .map(|list| {
                            list.files()
                                .iter()
                                .filter_map(|file| file.path())
                                .map(|path| (path, None))
                                .collect()
                        })
                        .unwrap_or_default();
                    app.add_files(paths, thread);
                },
            );
            return true;
        }
        if formats.contains_type(gdk::Texture::static_type())
            && !formats.contain_mime_type("text/plain")
        {
            clipboard.read_texture_async(gio::Cancellable::NONE, move |texture| {
                let Ok(Some(texture)) = texture else {
                    return;
                };
                let scratch = std::sync::Arc::new(caper_client::uploads::Scratch::new("png"));
                match texture.save_to_png(scratch.path()) {
                    Ok(()) => {
                        app.add_files(vec![(scratch.path().to_owned(), Some(scratch))], thread)
                    }
                    Err(_) => {
                        app.state.borrow_mut().send_error =
                            Some("Could not save the clipboard image.".into());
                        app.render();
                    }
                }
            });
            return true;
        }
        false
    }

    /// Files dragged onto the conversation attach to its composer.
    fn connect_drop(self: &Rc<Self>) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let app = self.clone();
        target.connect_enter(move |_, _, _| {
            let state = app.state.borrow();
            if !Self::can_attach(&state) {
                return gdk::DragAction::empty();
            }
            let (title, detail) = caper_client::uploads::drop_text(
                &state.channel_name,
                state.direct.is_some(),
                Self::files_in(&state, None),
            );
            app.ui.drop_title.set_label(&title);
            app.ui.drop_detail.set_label(&detail);
            app.ui.drop.set_visible(true);
            gdk::DragAction::COPY
        });
        let app = self.clone();
        target.connect_leave(move |_| app.ui.drop.set_visible(false));
        let app = self.clone();
        target.connect_drop(move |_, value, _, _| {
            app.ui.drop.set_visible(false);
            let Ok(list) = value.get::<gdk::FileList>() else {
                return false;
            };
            let paths: Vec<_> = list
                .files()
                .iter()
                .filter_map(|file| file.path())
                .map(|path| (path, None))
                .collect();
            let any = !paths.is_empty();
            app.add_files(paths, false);
            any
        });
        self.ui.conversation.add_controller(target);
    }

    fn render_files(self: &Rc<Self>) {
        let state = self.state.borrow();
        if self.rendered_files.replace(state.files_revision) == state.files_revision {
            return;
        }
        let handler: Rc<dyn Fn(ChipAction)> = {
            let app = Rc::downgrade(self);
            Rc::new(move |action| {
                if let Some(app) = app.upgrade() {
                    // Rebuilding the chips drops the clicked button.
                    glib::idle_add_local_once(move || app.chip_action(action));
                }
            })
        };
        for thread in [false, true] {
            let root = Self::composer_root(&state, thread);
            let files: Vec<&DraftFile> = state
                .files
                .iter()
                .filter(|file| state.channel.as_ref() == Some(&file.channel) && file.root == root)
                .collect();
            let composer = if thread {
                &self.ui.thread_composer
            } else {
                &self.ui.composer
            };
            crate::uploads::chips(&composer.chips, &self.images, &files, &handler);
        }
    }

    // ---------------------------------------------------------------------
    // Sending, replies, edits, deletes, pins and reactions.

    fn send_message(self: &Rc<Self>, thread: bool) {
        let composer = if thread {
            &self.ui.thread_composer
        } else {
            &self.ui.composer
        };
        let draft = composer.text();
        let text = draft.trim();
        {
            let mut state = self.state.borrow_mut();
            let Some(chat_token) = state.session.as_ref().map(|session| session.token.clone())
            else {
                return;
            };
            let Some(channel) = state.channel.clone() else {
                return;
            };
            let editing = !thread && matches!(state.compose, Some(Compose::Edit { .. }));
            let root = thread
                .then(|| state.thread.as_ref().map(|thread| thread.root.clone()))
                .flatten();
            let files = !editing
                && state
                    .files
                    .iter()
                    .any(|file| file.channel == channel && file.root == root);
            if text.is_empty() && !files {
                return;
            }
            if files {
                let mine = state
                    .files
                    .iter()
                    .filter(|file| file.channel == channel && file.root == root);
                let waiting = mine.clone().any(|file| file.uploading);
                let failed = mine.clone().any(|file| file.error.is_some());
                if waiting || failed {
                    state.send_error = Some(
                        if waiting {
                            "Wait for your files to finish uploading."
                        } else {
                            "Retry or remove the files that failed to upload."
                        }
                        .into(),
                    );
                    drop(state);
                    self.render();
                    return;
                }
            }
            if !thread && let Some(Compose::Edit { original, .. }) = &state.compose {
                if text.is_empty() {
                    return;
                }
                let original = original.clone();
                state.edit_request += 1;
                let request = state.edit_request;
                self.worker.send(Command::EditMessage {
                    generation: state.generation,
                    request,
                    token: state.token.clone(),
                    chat_token,
                    original: original.clone(),
                    text: text.to_owned(),
                });
                state.compose = Some(Compose::Edit { original, request });
                state.send_error = None;
                drop(state);
                self.render();
                return;
            }
            if state.pending.is_some() {
                state.send_error = Some("Wait for your last message to send first.".into());
                drop(state);
                self.render();
                return;
            }
            let thread_root = thread
                .then(|| state.thread.as_ref().map(|thread| thread.root.clone()))
                .flatten();
            if thread && thread_root.is_none() {
                return;
            }
            let reply_to_id = match (&state.compose, thread) {
                (Some(Compose::Reply { id, .. }), false) => Some(id.clone()),
                _ => None,
            };
            let client_id = uuid::Uuid::new_v4().to_string();
            let (sent, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut state.files)
                .into_iter()
                .partition(|file| file.channel == channel && file.root == thread_root);
            state.files = kept;
            state.files_revision += 1;
            let attachment_ids: Vec<String> = sent
                .iter()
                .filter_map(|file| file.attachment.as_ref().map(|file| file.id.clone()))
                .collect();
            let spoiler_attachment_ids: Vec<String> = sent
                .iter()
                .filter(|file| file.spoiler)
                .filter_map(|file| file.attachment.as_ref().map(|file| file.id.clone()))
                .collect();
            state.pending = Some(Pending {
                client_id: client_id.clone(),
                text: text.to_owned(),
                created_at: chrono::Local::now().to_rfc3339(),
                thread: thread_root.clone(),
                files: sent,
            });
            if !thread {
                state.compose = None;
            }
            state.send_error = None;
            self.set_typing(&mut state, false);
            self.worker.send(Command::Send {
                generation: state.generation,
                token: state.token.clone(),
                chat_token,
                channel,
                client_id,
                text: text.to_owned(),
                thread_root_id: thread_root,
                broadcast: false,
                reply_to_id,
                attachment_ids,
                spoiler_attachment_ids,
                gif: None,
                suppress_embeds: false,
            });
        }
        if !thread {
            self.follow.set(true);
        }
        composer.view.buffer().set_text("");
        self.render();
    }

    fn cancel_compose(self: &Rc<Self>) {
        let editing = matches!(
            self.state.borrow_mut().compose.take(),
            Some(Compose::Edit { .. })
        );
        if editing {
            self.ui.composer.view.buffer().set_text("");
        }
        self.render();
    }

    fn start_edit(self: &Rc<Self>, id: &str) {
        let text = {
            let mut state = self.state.borrow_mut();
            let Some(message) = state.message(id).cloned() else {
                return;
            };
            let text = message.content.text.clone();
            state.compose = Some(Compose::Edit {
                original: Box::new(message),
                request: 0,
            });
            text
        };
        self.ui.composer.view.buffer().set_text(&text);
        self.ui.composer.view.grab_focus();
        self.render();
    }

    fn confirm_delete(self: &Rc<Self>, id: String) {
        #[allow(deprecated)]
        let dialog = gtk::MessageDialog::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .message_type(gtk::MessageType::Question)
            .text("Delete this message?")
            .secondary_text("It will be removed for everyone. This can’t be undone.")
            .build();
        dialog.add_css_class("caper");
        #[allow(deprecated)]
        {
            dialog.add_button("Cancel", gtk::ResponseType::Cancel);
            dialog.add_button("Delete", gtk::ResponseType::Accept);
            if let Some(button) = dialog.widget_for_response(gtk::ResponseType::Accept) {
                button.add_css_class("destructive-action");
            }
        }
        let app = self.clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                let mut state = app.state.borrow_mut();
                if let (Some(session), Some(channel)) =
                    (state.session.clone(), state.channel.clone())
                {
                    state.delete_request += 1;
                    self_send_delete(&app, &state, session.token, channel, id.clone());
                }
            }
            dialog.close();
            // Focus would otherwise return to a selectable body label, which
            // selects its whole text.
            app.ui.composer.view.grab_focus();
        });
        dialog.present();
    }

    fn pin(self: &Rc<Self>, id: String, active: bool) {
        let state = self.state.borrow();
        if let (Some(session), Some(channel)) = (&state.session, &state.channel) {
            self.worker.send(Command::Pin {
                generation: state.generation,
                token: state.token.clone(),
                chat_token: session.token.clone(),
                channel: channel.clone(),
                message: id,
                active,
            });
        }
    }

    fn react(self: &Rc<Self>, id: String, emoji: String, active: bool) {
        let mut state = self.state.borrow_mut();
        if state.session.is_none() || state.channel.is_none() || !state.joined() {
            return;
        }
        state.reactions.set(&id, &emoji, active);
        self.send_next_reaction(&mut state, &id);
        drop(state);
        self.render();
    }

    /// Sends the message's next queued reaction unless one is in flight.
    fn send_next_reaction(&self, state: &mut State, message: &str) {
        let (Some(session), Some(channel)) = (state.session.clone(), state.channel.clone()) else {
            return;
        };
        let Some(next) = state.reactions.next(message) else {
            return;
        };
        self.worker.send(Command::React {
            generation: state.generation,
            token: state.token.clone(),
            chat_token: session.token,
            channel,
            message: next.message,
            emoji: next.emoji,
            active: next.active,
        });
    }

    fn open_thread(self: &Rc<Self>, root: String) {
        {
            let mut state = self.state.borrow_mut();
            state.files_revision += 1;
            let Some(channel) = state.channel.clone() else {
                return;
            };
            state.request += 1;
            let message = state.message(&root).cloned();
            state.thread = Some(Thread {
                root: root.clone(),
                message,
                request: state.request,
                loading: true,
            });
            self.worker.send(Command::LoadThread {
                generation: state.generation,
                request: state.request,
                token: state.token.clone(),
                channel,
                root,
                before: None,
            });
        }
        self.ui.members_toggle.set_active(false);
        self.render();
        self.ui.thread_composer.view.grab_focus();
    }

    fn row_action(self: &Rc<Self>, action: Action, _thread: bool) {
        match action {
            Action::React { id, emoji, active } => self.react(id, emoji, active),
            Action::Pick { id, anchor } => self.reaction_picker(id, &anchor),
            Action::RetryReaction(id) => {
                let retry = self.state.borrow().reactions.retry(&id);
                if let Some((emoji, active)) = retry {
                    self.react(id, emoji, active);
                }
            }
            Action::DismissReaction(id) => {
                self.state.borrow_mut().reactions.dismiss(&id);
                self.render();
            }
            Action::Reply(id) => {
                {
                    let mut state = self.state.borrow_mut();
                    let Some(author) = state
                        .message(&id)
                        .map(|message| message.author.name.clone())
                    else {
                        return;
                    };
                    if matches!(state.compose, Some(Compose::Edit { .. })) {
                        self.ui.composer.view.buffer().set_text("");
                    }
                    state.compose = Some(Compose::Reply { id, author });
                }
                self.ui.composer.view.grab_focus();
                self.render();
            }
            Action::Thread(id) => {
                // A reply in the thread pane opens its root's thread.
                let root = self
                    .state
                    .borrow()
                    .message(&id)
                    .and_then(|message| message.thread_root_id.clone())
                    .unwrap_or(id);
                self.open_thread(root);
            }
            Action::Menu { id, anchor } => self.row_menu(&id, &anchor),
            Action::Reveal(id) => {
                self.images.media.revealed.borrow_mut().insert(id);
                self.render();
            }
            Action::View(files, index) => {
                crate::viewer::open(&self.ui.window, &self.images, files, index)
            }
            Action::Open(attachment) => self.images.media.open(&attachment),
            Action::OpenUrl(url) => crate::media::open_uri(&url),
            Action::ToggleBlocked(key) => {
                {
                    let mut state = self.state.borrow_mut();
                    if !state.shown_blocked.remove(&key) {
                        state.shown_blocked.insert(key);
                    }
                }
                self.render();
            }
            Action::Profile { id, author, anchor } => self.open_profile(id, author, &anchor),
        }
    }

    fn row_menu(self: &Rc<Self>, id: &str, anchor: &gtk::Widget) {
        let (mine, pinned) = {
            let state = self.state.borrow();
            let Some(message) = state.message(id) else {
                return;
            };
            (
                state.session.as_ref().is_some_and(|session| {
                    !session.author.is_guest && session.author.id == message.author.id
                }),
                message.pin.is_some(),
            )
        };
        let menu = gio::Menu::new();
        let item = |label: &str, action: &str| {
            let item = gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(Some(action), Some(&id.to_variant()));
            item
        };
        let actions = gio::Menu::new();
        if mine {
            actions.append_item(&item("Edit message", "row.edit"));
        }
        actions.append_item(&if pinned {
            item("Unpin message", "row.unpin")
        } else {
            item("Pin message", "row.pin")
        });
        actions.append_item(&item("Copy text", "row.copy"));
        menu.append_section(None, &actions);
        if mine {
            let danger = gio::Menu::new();
            danger.append_item(&item("Delete message", "row.delete"));
            menu.append_section(None, &danger);
        }
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popup(&popover, anchor);
    }

    fn reaction_picker(self: &Rc<Self>, id: String, anchor: &gtk::Widget) {
        let mine: HashSet<String> = {
            let state = self.state.borrow();
            let me = state
                .session
                .as_ref()
                .map(|session| session.author.id.clone())
                .or_else(|| state.account.as_ref().map(|account| account.id.clone()));
            state
                .message(&id)
                .map(|message| {
                    state
                        .reactions
                        .projected(&message.reactions, &message.id, me.as_deref())
                        .into_iter()
                        .filter(|reaction| {
                            me.as_ref()
                                .is_some_and(|me| reaction.author_ids.contains(me))
                        })
                        .map(|reaction| reaction.emoji)
                        .collect()
                })
                .unwrap_or_default()
        };
        let app = self.clone();
        crate::picker::show(&self.images, anchor, mine, move |emoji, active| {
            app.react(id.clone(), emoji, active);
        });
    }

    // ---------------------------------------------------------------------
    // Worker events.

    /// Drains the worker's events, then updates what they changed.
    fn receive(self: &Rc<Self>) {
        let mut typing = false;
        {
            let mut state = self.state.borrow_mut();
            while let Ok(event) = self.worker.events.try_recv() {
                typing |= matches!(event, Event::Gateway(GatewayEvent::Typing { .. }));
                self.handle(&mut state, event);
            }
        }
        if typing {
            self.tick();
        }
        let jumping = self.state.borrow().jump_to.is_some();
        if jumping {
            self.render_rows();
            self.continue_jump();
        }
        self.render_pins();
        let created = self.state.borrow_mut().open_created.take();
        if let Some(id) = created {
            self.close_start_direct();
            self.close_profile();
            self.open_direct(id);
        }
        self.render_profile();
        self.render_start_direct();
        self.render();
    }

    fn handle(&self, state: &mut State, event: Event) {
        match event {
            Event::Restored { generation, result } if generation == state.generation => {
                match result {
                    Ok(Some((token, account, spaces))) => {
                        self.establish(state, token, account, spaces.spaces)
                    }
                    Ok(None) => state.page = Some(Page::SignIn),
                    Err(error) => {
                        state.page = Some(Page::SignIn);
                        state.error = Some(error);
                    }
                }
            }
            Event::CodeRequested { generation, result } if generation == state.generation => {
                state.busy = false;
                let resend = std::mem::take(&mut state.resending);
                match result {
                    Ok(challenge) => {
                        state.challenge = Some(challenge);
                        state.code_sent_at = Some(Instant::now());
                        if resend {
                            state.resends = state.resends.saturating_add(1);
                            state.signin_status =
                                Some("We sent a new code. Earlier codes no longer work.");
                        }
                    }
                    Err(error) => state.error = Some(error),
                }
            }
            Event::Verified {
                generation,
                result,
                attempts_remaining,
            } if generation == state.generation => {
                state.busy = false;
                match result {
                    Ok((token, account, spaces)) => {
                        state.challenge = None;
                        state.signin_status = None;
                        self.worker.send(Command::PersistCredential {
                            generation: state.generation,
                            token: token.clone(),
                        });
                        self.establish(state, token, account, spaces.spaces);
                    }
                    Err(error) => {
                        state.error = Some(match attempts_remaining {
                            Some(left) => format!("{error} {left} attempts left."),
                            None => error,
                        })
                    }
                }
            }
            Event::Credential {
                result: Err(error), ..
            } => state.error = Some(error),
            Event::LoggedOut {
                generation,
                result: Err(error),
            } if generation == state.generation => {
                state.error = Some(format!(
                    "Signed out locally; server revocation failed: {error}"
                ));
            }
            Event::DirectsLoaded {
                generation,
                result: Ok(directs),
            } if generation == state.epoch => {
                state.directs = directs;
                if let Some(direct) = &state.direct
                    && let Some(entry) = state.directs.iter().find(|entry| &entry.id == direct)
                {
                    state.channel_name = entry.peer.display_name.clone();
                }
                state.layout += 1;
            }
            Event::DirectLookedUp {
                generation,
                request,
                username,
                result,
                ..
            } if generation == state.epoch
                && request == state.direct_lookup
                && state.direct_finding =>
            {
                state.direct_finding = false;
                let current = self
                    .direct_dialog
                    .borrow()
                    .as_ref()
                    .map(|dialog| dialog.field.text());
                if current.as_deref() == Some(username.as_str()) {
                    match result {
                        Ok(person) => state.direct_recipient = Some(person),
                        Err(error) => state.direct_error = Some(error),
                    }
                }
            }
            Event::DirectCreated {
                generation,
                navigation,
                result,
            } if generation == state.epoch => {
                state.direct_opening = false;
                match result {
                    Ok(direct) => {
                        state.directs.retain(|item| item.id != direct.id);
                        let id = direct.id.clone();
                        state.directs.push(direct);
                        state.layout += 1;
                        if navigation == state.navigation {
                            state.open_created = Some(id);
                        }
                    }
                    Err(error) if navigation == state.navigation => {
                        state.direct_error = Some(error);
                    }
                    Err(_) => {}
                }
            }
            Event::Account { epoch, result } if epoch == state.epoch => {
                self.account_result(state, result);
            }
            Event::ProfileStatus {
                epoch,
                person,
                status,
            } if epoch == state.epoch => {
                if let Some(card) = state.card.as_mut().filter(|card| card.person == person) {
                    card.status = Some(status);
                }
            }
            Event::ProfileSent {
                epoch,
                person,
                send,
                result,
            } if epoch == state.epoch => {
                if let Some(direct) = result.as_ref().ok().or(send.direct.as_ref()) {
                    state.directs.retain(|item| item.id != direct.id);
                    state.directs.push(direct.clone());
                    state.layout += 1;
                }
                if let Some(card) = state.card.as_mut().filter(|card| card.person == person) {
                    card.sending = false;
                    card.status = None;
                    match result {
                        Ok(direct) => {
                            state.card = None;
                            state.open_created = Some(direct.id);
                        }
                        Err(error) => {
                            if !send.open_only {
                                card.retry_id = error.uncertain.then_some(send.client_id);
                            }
                            card.error = Some(error.message);
                        }
                    }
                }
            }
            // A failed refresh keeps the previous list (or the peer fallback).
            Event::PeopleLoaded {
                generation,
                result: Ok(people),
            } if generation == state.epoch => state.people = Some(people),
            Event::UnreadLoaded {
                epoch,
                revision,
                result: Ok(unread),
                ..
            } if epoch == state.epoch && revision == state.unread_revision => {
                state.unread = unread
                    .spaces
                    .into_iter()
                    .map(|space| {
                        let channels = space
                            .channels
                            .into_iter()
                            .map(|channel| {
                                let read = channel
                                    .read_seq
                                    .as_deref()
                                    .and_then(|seq| seq.parse::<u64>().ok())
                                    .unwrap_or(0);
                                let last = channel.last_seq.parse::<u64>().unwrap_or(0);
                                let count = channel.unread_count.unwrap_or(u64::from(last > read));
                                (channel.id, (count, channel.mention_count.unwrap_or(0)))
                            })
                            .collect();
                        (space.id, channels)
                    })
                    .collect();
                state.layout += 1;
                self.clear_open_unread(state);
            }
            Event::ChannelRead {
                epoch,
                space,
                channel,
                result: Ok(()),
                ..
            } if epoch == state.epoch => {
                if let Some(entry) = state
                    .unread
                    .get_mut(&space)
                    .and_then(|channels| channels.get_mut(&channel))
                {
                    *entry = (0, 0);
                    state.layout += 1;
                }
            }
            Event::NavigationPrepared {
                generation,
                navigation,
                result,
            } if generation == state.generation && navigation == state.navigation => {
                self.navigation_prepared(state, result)
            }
            Event::OlderLoaded {
                generation,
                request,
                channel,
                result,
            } if generation == state.generation
                && request == state.request
                && state.channel.as_deref() == Some(&channel) =>
            {
                state.loading_older = false;
                match result {
                    Ok(history) => {
                        state.has_more = history.has_more;
                        if let Err(error) = state.timeline.prepend(history.messages) {
                            state.error = Some(error);
                        }
                    }
                    Err(error) => state.error = Some(error.message),
                }
            }
            Event::ThreadLoaded {
                generation,
                request,
                channel,
                root,
                result,
            } if generation == state.generation
                && state.channel.as_deref() == Some(&channel)
                && state
                    .thread
                    .as_ref()
                    .is_some_and(|thread| thread.root == root && thread.request == request) =>
            {
                match result {
                    Ok(history)
                        if history.root.channel_id == channel
                            && history
                                .messages
                                .iter()
                                .all(|message| message.channel_id == channel) =>
                    {
                        if let Some(thread) = &mut state.thread {
                            thread.loading = false;
                            thread.message = Some(history.root);
                        }
                        if let Err(error) = state.timeline.prepend(history.messages) {
                            state.error = Some(error);
                        }
                    }
                    Ok(_) => state.error = Some("Caper returned another conversation.".into()),
                    Err(error) => {
                        state.thread = None;
                        state.error = Some(error.message);
                    }
                }
            }
            Event::Sent {
                generation,
                channel,
                client_id,
                result,
            } if current(
                generation,
                state.generation,
                Some(&channel),
                state.channel.as_deref(),
            ) && state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.client_id == client_id) =>
            {
                let pending = state.pending.take();
                match result {
                    Ok(message) => {
                        if state.timeline.merge_sent(message).is_err() {
                            state.send_error =
                                Some("Caper returned an invalid send confirmation.".into());
                        }
                        self.mark_read(state);
                    }
                    Err(failure) => {
                        state.send_error = Some(failure.message);
                        if let Some(pending) = pending {
                            state.files.extend(pending.files);
                            state.files_revision += 1;
                            state.returned_draft = Some((pending.text, pending.thread.is_some()));
                        }
                    }
                }
            }
            Event::Reacted {
                generation,
                channel,
                message,
                emoji,
                active,
                result,
                ..
            } if current(
                generation,
                state.generation,
                Some(&channel),
                state.channel.as_deref(),
            ) =>
            {
                match state
                    .reactions
                    .reacted(&channel, &message, &emoji, active, result)
                {
                    Reacted::Acknowledged(update) => {
                        if state.timeline.merge_reaction_ack(update).is_err() {
                            self.reload(state);
                        }
                        self.send_next_reaction(state, &message);
                    }
                    Reacted::Reload => self.reload(state),
                    Reacted::ClearChannel(error) => {
                        self.reload(state);
                        state.error = Some(error);
                    }
                    Reacted::Failed => self.send_next_reaction(state, &message),
                }
            }
            Event::Pinned {
                generation,
                channel,
                message,
                result,
                ..
            } if current(
                generation,
                state.generation,
                Some(&channel),
                state.channel.as_deref(),
            ) =>
            {
                match result {
                    Ok(update) if update.channel_id == channel && update.message.id == message => {
                        if state.timeline.merge_pin_ack(update).is_err() {
                            self.reload(state);
                        }
                    }
                    Ok(_) => self.reload(state),
                    Err(error) => state.send_error = Some(error.message),
                }
            }
            Event::EditSnapshot {
                generation,
                request,
                channel,
                message,
                result,
                ..
            } if current(
                generation,
                state.generation,
                Some(&channel),
                state.channel.as_deref(),
            ) =>
            {
                let ours = matches!(&state.compose, Some(Compose::Edit { original, request: sent })
                    if original.id == message && *sent == request);
                match result {
                    Ok(snapshot) => {
                        if ours {
                            state.compose = None;
                            state.returned_draft = None;
                            self.ui.composer.view.buffer().set_text("");
                        }
                        if state.timeline.merge_edit_ack(*snapshot).is_err() {
                            self.reload(state);
                        }
                    }
                    Err(error) if ours => state.send_error = Some(error),
                    Err(_) => {}
                }
            }
            Event::Deleted {
                generation,
                channel,
                result,
                ..
            } if current(
                generation,
                state.generation,
                Some(&channel),
                state.channel.as_deref(),
            ) =>
            {
                match result {
                    Ok(snapshot) => {
                        if state.timeline.merge_edit_ack(*snapshot).is_err() {
                            self.reload(state);
                        }
                    }
                    Err(error) => {
                        state.send_error = Some(if error.trim().is_empty() {
                            "Couldn’t delete the message. Try again.".into()
                        } else {
                            error
                        })
                    }
                }
            }
            Event::Admin { generation, result } if generation == state.generation => {
                state.joining = false;
                match result {
                    Ok(AdminResult::ChannelMembership {
                        detail, channel, ..
                    }) => {
                        let space = Some(detail.space.id.clone());
                        state.detail = Some(detail);
                        self.navigate(state, space, Some(channel));
                    }
                    Ok(_) => {}
                    Err(error) => state.error = Some(error),
                }
            }
            Event::AssetUsage { token, settings } if state.token.as_ref() == Some(&token) => {
                state.uploads = settings;
                state.uploads_checked = true;
            }
            Event::Upload { key, token, update } => self.upload_update(state, key, token, update),
            Event::AttachmentUrls { token, ids, result }
                if state.token.as_ref() == Some(&token) =>
            {
                if let Some(url) = self
                    .images
                    .media
                    .refreshed(&ids, result.unwrap_or_default())
                {
                    crate::media::open_uri(&url);
                }
            }
            Event::Gateway(event) => self.gateway(state, event),
            _ => {}
        }
    }

    fn navigation_prepared(
        &self,
        state: &mut State,
        result: Result<caper_client::worker::PreparedNavigation, caper_client::worker::LoadError>,
    ) {
        match result {
            Ok(prepared) => {
                if let Some(detail) = prepared.detail {
                    state.detail = Some(detail);
                }
                match prepared.conversation {
                    Some((history, session)) => {
                        state.channel = Some(history.channel.id.clone());
                        if state.direct.is_none() {
                            state.channel_name = history.channel.name.clone();
                        }
                        state.markup.clear();
                        if let Err(error) = state
                            .timeline
                            .reset(history.messages, &history.cursor)
                            .and_then(|()| state.timeline.reset_pins(history.pinned_messages))
                        {
                            state.error = Some(error);
                        }
                        state.has_more = history.has_more;
                        match session {
                            Ok(session) => state.session = Some(session),
                            Err(error) => state.session_error = Some(error),
                        }
                        self.follow.set(true);
                        self.connect_gateway(state, history.channel.id);
                        self.worker.showing(state.channel.as_deref());
                        if state.joined() {
                            self.mark_read(state);
                        }
                        self.clear_open_unread(state);
                    }
                    None => {
                        state.channel = None;
                        state.channel_name.clear();
                        state.timeline = Timeline::default();
                        state.status.clear();
                    }
                }
                state.layout += 1;
            }
            Err(error) => {
                state.status.clear();
                state.error = Some(error.message);
            }
        }
    }

    /// The open conversation shows no unread badge while you read it.
    fn clear_open_unread(&self, state: &mut State) {
        if let (Some(space), Some(channel)) = (&state.space, &state.channel)
            && state.direct.is_none()
            && let Some(entry) = state
                .unread
                .get_mut(space)
                .and_then(|channels| channels.get_mut(channel))
            && *entry != (0, 0)
        {
            *entry = (0, 0);
            state.layout += 1;
        }
    }

    fn gateway(&self, state: &mut State, event: GatewayEvent) {
        let selected = state.channel.clone();
        let fresh = |generation: u64, channel: &str| {
            current(
                generation,
                state.generation,
                Some(channel),
                selected.as_deref(),
            )
        };
        let applied = match event {
            GatewayEvent::Status {
                generation,
                channel,
                online,
                detail,
            } if fresh(generation, &channel) => {
                state.status = if online { "Live".into() } else { detail };
                if !online {
                    state.typers.clear();
                }
                Ok(Apply::Applied)
            }
            GatewayEvent::Message {
                generation,
                channel,
                message,
            } if fresh(generation, &channel) => {
                let mine = state.session.as_ref().map(|session| &session.author.id);
                if state.pending.as_ref().is_some_and(|pending| {
                    pending.client_id == message.client_message_id
                        && Some(&message.author.id) == mine
                }) {
                    state.pending = None;
                }
                // Their message ends their typing.
                state.typers.remove(&message.author.id);
                let applied = state.timeline.apply(*message);
                if matches!(applied, Ok(Apply::Applied)) && self.follow.get() {
                    self.mark_read(state);
                }
                applied
            }
            GatewayEvent::Reactions {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => state.timeline.apply_reactions(update),
            GatewayEvent::Attachments {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => state.timeline.apply_attachments(update),
            GatewayEvent::Embeds {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => {
                state.timeline.apply_embeds(*update);
                Ok(Apply::Applied)
            }
            GatewayEvent::Pin {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => state.timeline.apply_pin(*update),
            GatewayEvent::Forward {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => state.timeline.apply_forward(*update),
            GatewayEvent::Edit {
                generation,
                channel,
                update,
            } if fresh(generation, &channel) => state.timeline.apply_edit(*update),
            GatewayEvent::Typing {
                generation,
                channel,
                author,
                typing,
                revision,
            } if fresh(generation, &channel) => {
                let revision = revision.parse::<u64>().unwrap_or(0);
                let me = state
                    .session
                    .as_ref()
                    .map(|session| session.author.id.as_str());
                if Some(author.id.as_str()) != me
                    && state
                        .typers
                        .get(&author.id)
                        .is_none_or(|typer| revision > typer.revision)
                    && (state.typers.contains_key(&author.id) || state.typers.len() < 64)
                {
                    if typing {
                        state.typers.insert(
                            author.id.clone(),
                            Typer {
                                name: author.name,
                                revision,
                                expires: Instant::now() + TYPING_SHOWN,
                            },
                        );
                    } else {
                        state.typers.remove(&author.id);
                    }
                }
                Ok(Apply::Duplicate)
            }
            GatewayEvent::Presence {
                generation,
                space,
                members,
            } if generation == state.generation && state.space.as_deref() == Some(&space) => {
                state.presence = members
                    .into_iter()
                    .map(|presence| (presence.user_id, presence.status))
                    .collect();
                Ok(Apply::Duplicate)
            }
            GatewayEvent::Resync {
                generation,
                channel,
            } if fresh(generation, &channel) => Ok(Apply::Resync),
            GatewayEvent::AccessDenied {
                generation,
                channel,
                detail,
            } if fresh(generation, &channel) => {
                state.session = None;
                state.session_error = Some(detail);
                Ok(Apply::Applied)
            }
            _ => Ok(Apply::Duplicate),
        };
        if matches!(applied, Ok(Apply::Resync) | Err(_)) {
            self.reload(state);
        }
    }

    // ---------------------------------------------------------------------
    // Rendering: each part updates only when its content changed.

    fn render(self: &Rc<Self>) {
        self.render_rows();
        self.render_layout();
        self.render_files();
        self.render_members();
        self.render_typing();
        let ui = &self.ui;
        let mut state = self.state.borrow_mut();
        ui.pages.set_visible_child_name(match state.page() {
            Page::Loading => "loading",
            Page::SignIn => "signin",
            Page::Chat => "chat",
        });
        ui.steps
            .set_visible_child_name(if state.challenge.is_some() {
                "code"
            } else {
                "email"
            });
        ui.code_hint.set_label(&format!(
            "Enter the six-character code sent to {}. It expires in 10 minutes.",
            state.email
        ));
        ui.email_button.set_sensitive(!state.busy);
        ui.code_button.set_sensitive(!state.busy);
        ui.email_button
            .set_label(if state.busy && !state.resending {
                "Sending…"
            } else {
                "Continue"
            });
        ui.code_button.set_label(if state.busy && !state.resending {
            "Checking…"
        } else {
            "Continue"
        });
        self.render_resend(&state);
        let status = state.signin_status.unwrap_or_default();
        ui.signin_status.set_label(status);
        ui.signin_status.set_visible(!status.is_empty());
        let error = state.error.as_deref().unwrap_or_default();
        ui.signin_error.set_label(error);
        ui.signin_error.set_visible(!error.is_empty());
        ui.error.set_label(error);
        ui.error.set_visible(!error.is_empty());

        let direct = state.direct.is_some();
        ui.channel_title
            .set_label(&if state.channel_name.is_empty() {
                String::new()
            } else if direct {
                format!("@ {}", state.channel_name)
            } else {
                format!("# {}", state.channel_name)
            });
        ui.status.set_label(&state.status);
        let joined = state.joined();
        let can_send = state.session.is_some();
        ui.composer.root.set_visible(joined);
        ui.composer.view.set_sensitive(can_send);
        ui.composer.send.set_sensitive(can_send);
        let attach = Self::can_attach(&state);
        let editing = matches!(state.compose, Some(Compose::Edit { .. }));
        ui.composer.attach.set_sensitive(attach && !editing);
        ui.thread_composer.attach.set_sensitive(attach);
        let why = if attach {
            "Attach files"
        } else if state.token.is_some() && state.uploads_checked && state.uploads.is_none()
            || !state.joined()
        {
            "File uploads are unavailable"
        } else {
            "Checking file uploads…"
        };
        ui.composer.attach.set_tooltip_text(Some(why));
        ui.thread_composer.attach.set_tooltip_text(Some(why));
        ui.join_bar
            .set_visible(!joined && state.channel.is_some() && state.page() == Page::Chat);
        ui.join_label.set_label(&format!(
            "You’re previewing #{}. Join to send messages and react.",
            state.channel_name
        ));
        ui.join.set_sensitive(!state.joining);
        ui.join.set_label(if state.joining {
            "Joining…"
        } else {
            "Join channel"
        });
        let prompt = match &state.session_error {
            Some(error) => error.clone(),
            None if direct => format!("Message @{}", state.channel_name),
            None => format!("Message #{}", state.channel_name),
        };
        ui.composer.placeholder.set_label(&prompt);
        match &state.compose {
            Some(Compose::Reply { author, .. }) => {
                ui.compose_label.set_markup(&format!(
                    "Replying to <b>{}</b>",
                    glib::markup_escape_text(author)
                ));
                ui.compose_bar.set_visible(true);
            }
            Some(Compose::Edit { .. }) => {
                ui.compose_label
                    .set_label("Editing message · Enter to save, Escape to cancel");
                ui.compose_bar.set_visible(true);
            }
            None => ui.compose_bar.set_visible(false),
        }
        let thread_open = state.thread.is_some();
        ui.thread_panel.set_visible(thread_open);
        ui.members_panel.set_visible(
            !thread_open && ui.members_toggle.is_active() && ui.members_toggle.is_visible(),
        );
        ui.thread_composer.view.set_sensitive(can_send && joined);
        ui.thread_composer.send.set_sensitive(can_send && joined);
        ui.thread_composer.placeholder.set_label("Reply in thread");
        let send_error = state.send_error.as_deref().unwrap_or_default();
        let in_thread = state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.thread.is_some());
        ui.composer_error.set_label(send_error);
        ui.composer_error.set_visible(!send_error.is_empty());
        ui.thread_error.set_label(send_error);
        ui.thread_error
            .set_visible(!send_error.is_empty() && thread_open && in_thread);
        if let Some((draft, thread)) = state.returned_draft.take() {
            let buffer = if thread {
                ui.thread_composer.view.buffer()
            } else {
                ui.composer.view.buffer()
            };
            if buffer.char_count() == 0 {
                drop(state);
                buffer.set_text(&draft);
            }
        }
    }

    fn render_resend(self: &Rc<Self>, state: &State) {
        let ui = &self.ui;
        ui.resend.set_visible(state.resends < MAX_RESENDS);
        match state
            .code_sent_at
            .and_then(|sent| resend_wait(sent.elapsed()))
        {
            Some(wait) => {
                let (label, next) = resend_countdown(wait);
                ui.resend.set_label(&label);
                ui.resend.set_sensitive(false);
                if state.challenge.is_some() && !self.countdown.replace(true) {
                    let app = Rc::downgrade(self);
                    glib::timeout_add_local_once(next, move || {
                        if let Some(app) = app.upgrade() {
                            app.countdown.set(false);
                            let state = app.state.borrow();
                            app.render_resend(&state);
                        }
                    });
                }
            }
            None => {
                ui.resend.set_label("Resend code");
                ui.resend.set_sensitive(!state.busy);
            }
        }
    }

    fn render_typing(&self) {
        let state = self.state.borrow();
        let mut names: Vec<&str> = state
            .typers
            .values()
            .map(|typer| typer.name.as_str())
            .collect();
        names.sort_unstable();
        let text = match names.as_slice() {
            [] => String::new(),
            [one] => format!("{one} is typing…"),
            [one, two] => format!("{one} and {two} are typing…"),
            [one, two, three] => format!("{one}, {two} and {three} are typing…"),
            _ => "Several people are typing…".into(),
        };
        if self.ui.typing.label() != text {
            self.ui.typing.set_label(&text);
        }
    }

    /// Scrolls to the newest row, again on each frame while rows near it
    /// are measured (heights start as estimates), until it stays at the end.
    fn pin_tail(self: &Rc<Self>) {
        if self.settling.replace(true) {
            return;
        }
        let app = self.clone();
        let frames = Cell::new(0);
        self.ui.list.view.add_tick_callback(move |list, _| {
            let frames = frames.replace(frames.get() + 1) + 1;
            let count = app.ui.list.store.n_items();
            let adjustment = app.ui.list.scroller.vadjustment();
            let settled = frames > 2
                && adjustment.value() >= adjustment.upper() - adjustment.page_size() - 1.0;
            if count == 0 || settled || frames > 60 {
                app.settling.set(false);
                return glib::ControlFlow::Break;
            }
            // Moving the adjustment alone leaves the list anchored where it was.
            let _ = list.activate_action("list.scroll-to-item", Some(&(count - 1).to_variant()));
            glib::ControlFlow::Continue
        });
    }

    /// Applies the row diffs to the lists' models: only new or changed rows
    /// get new items, so only they are bound again.
    fn render_rows(self: &Rc<Self>) {
        let (changed, thread_changed) = {
            let mut state = self.state.borrow_mut();
            let state = &mut *state;
            let me = state.account.as_ref().map(|account| account.id.clone());
            let author = state
                .session
                .as_ref()
                .map(|session| session.author.id.clone());
            let joined = state.session.is_some() && state.joined();
            let revealed = self.images.media.revealed.borrow();
            let blocked = state.blocked_ids();
            let viewer = |in_thread| rows::Viewer {
                revealed: &revealed,
                blocked: &blocked,
                shown_blocked: &state.shown_blocked,
                me: me.as_deref(),
                author: author.as_deref(),
                joined,
                in_thread,
                images: &self.images,
                reactions: &state.reactions,
            };
            let draft = |thread: Option<&str>| {
                state
                    .pending
                    .as_ref()
                    .filter(|pending| pending.thread.as_deref() == thread)
                    .zip(state.session.as_ref())
                    .map(|(pending, session)| rows::Draft {
                        id: &pending.client_id,
                        author: &session.author,
                        created_at: &pending.created_at,
                        text: &pending.text,
                        content: crate::content::Content {
                            attachments: pending
                                .files
                                .iter()
                                .filter_map(DraftFile::send_attachment)
                                .map(|attachment| (attachment, false))
                                .collect(),
                            ..Default::default()
                        },
                    })
            };
            let main = rows::build(
                state
                    .timeline
                    .messages()
                    .filter(|message| message.is_channel_message()),
                draft(None),
                &mut state.markup,
                &viewer(false),
            );
            let changed = rows::apply(&self.ui.list.store, &state.rows, &main);
            state.rows = main;
            let thread = match &state.thread {
                Some(thread) => {
                    let root = state
                        .timeline
                        .messages()
                        .find(|message| message.id == thread.root)
                        .or(thread.message.as_ref());
                    rows::build(
                        root.into_iter()
                            .chain(state.timeline.messages().filter(|message| {
                                message.thread_root_id.as_deref() == Some(thread.root.as_str())
                            })),
                        draft(Some(&thread.root)),
                        &mut state.markup,
                        &viewer(true),
                    )
                }
                None => Vec::new(),
            };
            let thread_changed =
                rows::apply(&self.ui.thread_list.store, &state.thread_rows, &thread);
            state.thread_rows = thread;
            (changed, thread_changed)
        };
        if changed && self.follow.get() {
            self.pin_tail();
        }
        let count = self.ui.thread_list.store.n_items();
        if thread_changed && count > 0 {
            let _ = self
                .ui
                .thread_list
                .view
                .activate_action("list.scroll-to-item", Some(&(count - 1).to_variant()));
        }
    }

    fn render_layout(self: &Rc<Self>) {
        let state = self.state.borrow();
        if self.rendered_layout.replace(state.layout) == state.layout {
            return;
        }
        let ui = &self.ui;
        let space_unread = |space: &str| {
            state
                .unread
                .get(space)
                .is_some_and(|channels| channels.values().any(|(count, _)| *count > 0))
        };
        clear(&ui.rail);
        for space in &state.spaces {
            let button: gtk::Button = named(
                "GtkButton",
                &if space_unread(&space.id) {
                    format!("{} (unread)", space.name)
                } else {
                    space.name.clone()
                },
            );
            button.set_label(&space_initials(&space.name));
            button.set_tooltip_text(Some(&space.name));
            if state.space.as_deref() == Some(&space.id) && state.direct.is_none() {
                button.add_css_class("selected");
            }
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&button));
            if space_unread(&space.id) {
                let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                dot.add_css_class("unread-dot");
                dot.set_halign(gtk::Align::Start);
                dot.set_valign(gtk::Align::Center);
                dot.set_can_target(false);
                overlay.add_overlay(&dot);
            }
            let app = self.clone();
            let id = space.id.clone();
            button.connect_clicked(move |_| app.select_space(id.clone()));
            ui.rail.append(&overlay);
        }

        let detail = state.detail.as_ref();
        ui.space_name
            .set_label(detail.map_or("", |detail| detail.space.name.as_str()));
        clear_list(&ui.channels);
        let mut ids = Vec::new();
        let unread = detail.and_then(|detail| state.unread.get(&detail.space.id));
        for channel in detail.map_or(&[][..], |detail| &detail.channels) {
            let (count, mentions) = unread
                .and_then(|channels| channels.get(&channel.id))
                .copied()
                .unwrap_or_default();
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.append(&self.images.icon_image(
                if channel.private {
                    Icon::Lock
                } else {
                    Icon::Hash
                },
                "#b9bcbe",
                14,
            ));
            let name = label(&channel.name, &[]);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_hexpand(true);
            if !channel.joined {
                name.add_css_class("muted");
            } else if count > 0 {
                name.add_css_class("unread");
            }
            line.append(&name);
            if count > 0 {
                line.append(&badge(count, mentions > 0));
            }
            ui.channels.append(&line);
            if state.direct.is_none()
                && state.channel.as_deref() == Some(&channel.id)
                && let Some(row) = ui.channels.row_at_index(ids.len() as i32)
            {
                ui.channels.select_row(Some(&row));
            }
            ids.push(channel.id.clone());
        }
        if state.direct.is_some() {
            ui.channels.unselect_all();
        }
        *self.channel_ids.borrow_mut() = ids;

        clear_list(&ui.directs);
        let mut ids = Vec::new();
        let me = state.account.as_ref().map(|account| account.id.as_str());
        for direct in &state.directs {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let avatar = AvatarView::new(22, false);
            avatar.set(&self.images, &peer_avatar(direct));
            line.append(&avatar.root);
            let you = Some(direct.peer.id.as_str()) == me;
            let name = label(
                &if you {
                    format!("{} (you)", direct.peer.display_name)
                } else {
                    direct.peer.display_name.clone()
                },
                &[],
            );
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_hexpand(true);
            let count = direct.unread_count.unwrap_or(0);
            if count > 0 {
                name.add_css_class("unread");
            }
            line.append(&name);
            if count > 0 {
                line.append(&badge(count, true));
            }
            ui.directs.append(&line);
            if state.direct.as_deref() == Some(&direct.id)
                && let Some(row) = ui.directs.row_at_index(ids.len() as i32)
            {
                ui.directs.select_row(Some(&row));
            }
            ids.push(direct.id.clone());
        }
        if state.direct.is_none() {
            ui.directs.unselect_all();
        }
        *self.direct_ids.borrow_mut() = ids;

        if let Some(account) = &state.account {
            ui.account_name.set_label(&state.name());
            ui.account_avatar.set(
                &self.images,
                &Avatar::new(
                    &state.name(),
                    account.avatar_id,
                    &account.avatar_asset,
                    &account.avatar_url,
                ),
            );
        }
    }
}

/// Sends the delete once the dialog is confirmed.
fn self_send_delete(app: &App, state: &State, chat_token: String, channel: String, id: String) {
    app.worker.send(Command::DeleteMessage {
        generation: state.generation,
        request: state.delete_request,
        token: state.token.clone(),
        chat_token,
        channel,
        message: id,
    });
}

fn peer_avatar(direct: &DirectConversation) -> Avatar {
    Avatar::new(
        &direct.peer.display_name,
        direct.peer.avatar_id,
        &direct.peer.avatar_asset,
        &direct.peer.avatar_url,
    )
}

/// An unread count; terracotta when it mentions you (or in a DM).
fn badge(count: u64, mention: bool) -> gtk::Label {
    let text = if count > 99 {
        "99+".to_owned()
    } else {
        count.to_string()
    };
    let badge = label(&text, &["count"]);
    if mention {
        badge.add_css_class("mention");
    }
    badge.set_tooltip_text(Some(&format!(
        "{count} unread message{}",
        if count == 1 { "" } else { "s" }
    )));
    badge
}

/// Shows `popover` from `anchor`, and detaches it once it closes.
pub fn popup(popover: &impl IsA<gtk::Popover>, anchor: &gtk::Widget) {
    let popover = popover.as_ref();
    popover.set_has_arrow(false);
    // Parented to the chat page, pointing at the anchor: list rows
    // are recycled and rebuilt, and a popover whose parent goes away keeps
    // its pointer grab on GTK 4.8, freezing every click.
    let stage = std::iter::successors(anchor.parent(), gtk::Widget::parent)
        .find(|widget| widget.has_css_class("popover-host"));
    match stage.as_ref().and_then(|stage| {
        anchor
            .compute_bounds(stage)
            .map(|bounds| (stage.clone(), bounds))
    }) {
        Some((stage, bounds)) => {
            popover.set_parent(&stage);
            popover.set_pointing_to(Some(&gdk::Rectangle::new(
                bounds.x() as i32,
                bounds.y() as i32,
                (bounds.width() as i32).max(1),
                (bounds.height() as i32).max(1),
            )));
            // X11 clips rather than flips a popover at the screen edge, so
            // open toward whichever half of the window has room.
            let low = bounds.y() + bounds.height() / 2.0 > stage.height() as f32 / 2.0;
            popover.set_position(if low {
                gtk::PositionType::Top
            } else {
                gtk::PositionType::Bottom
            });
        }
        None => popover.set_parent(anchor),
    }
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}

/// The rail's label: the first word's first two letters, as desktop shows.
fn space_initials(name: &str) -> String {
    let mut chars = name.split_whitespace().next().unwrap_or("?").chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars.take(1)).collect())
        .unwrap_or_default()
}
