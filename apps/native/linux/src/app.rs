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
use caper_client::Host;
use caper_client::api::Api;
use caper_client::effects::Effect;
use caper_client::gateway::GatewayEvent;
use caper_client::mentions;
use caper_client::model::{
    Account, Apply, ChatSession, DirectConversation, DirectStatus, Message, Space, SpaceDetail,
    Spaces, Timeline,
};
use caper_client::notifications::Scope;
use caper_client::reactions::Reacted;
use caper_client::sign_in::{MAX_RESENDS, resend_countdown, resend_wait};
use caper_client::worker::{
    AccountOperation, AccountResult, AdminOperation, Command, Event, Worker, current,
};
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

#[path = "account.rs"]
mod account;
#[path = "admin.rs"]
mod admin;
#[path = "audio.rs"]
mod audio;
#[path = "bench.rs"]
mod bench;
#[path = "call.rs"]
mod call;
#[path = "context.rs"]
mod context;
#[path = "editor.rs"]
mod editor;
#[path = "feedback.rs"]
mod feedback;
#[path = "forward.rs"]
mod forward;
#[path = "gifs.rs"]
mod gifs;
#[path = "history.rs"]
mod history;
#[path = "inbox.rs"]
mod inbox;
#[path = "members.rs"]
mod members;
#[path = "narrow.rs"]
mod narrow;
#[path = "notify.rs"]
mod notify;
#[path = "panes.rs"]
mod panes;
#[path = "picture.rs"]
mod picture;
#[path = "profile.rs"]
mod profile;
#[path = "reactors.rs"]
mod reactors;
#[path = "recording.rs"]
mod recording;
#[path = "requests.rs"]
mod requests;
#[path = "settings.rs"]
mod settings;
#[path = "updates.rs"]
mod updates;

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

/// Gives an existing widget the accessible name `name`, the same way: a
/// template's `<accessibility>` element applied to it.
pub fn rename(widget: &gtk::Widget, name: &str) {
    let class = widget.type_();
    let parent = class.parent().unwrap_or(gtk::Widget::static_type());
    let xml = format!(
        "<interface><template class=\"{}\" parent=\"{}\"><accessibility><property name=\"label\">{}</property></accessibility></template></interface>",
        class.name(),
        parent.name(),
        glib::markup_escape_text(name)
    );
    if let Err(error) = gtk::Builder::new().extend_with_template(widget, class, &xml) {
        eprintln!("caper: could not rename a widget: {error}");
    }
}

/// Direct messages rows that aren't DMs: your notes before they exist, and
/// New message. Neither is a valid conversation id.
const SELF_NOTES: &str = "\u{0}self-notes";
const NEW_MESSAGE: &str = "\u{0}new-message";

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

/// What the conversation's error banner can try again.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Retry {
    /// The conversation failed to open: "Try again".
    Reload,
    /// An older page failed: "Retry".
    Older,
    /// No messaging session: "Retry session".
    Session,
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
    /// A KLIPY GIF sent on its own, and what to tell KLIPY once it is.
    gif: Option<caper_client::klipy::Gif>,
    share: Option<caper_client::klipy::Share>,
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
    /// Older replies exist before the oldest loaded one.
    has_more: bool,
    /// A reply to bring into view: older pages load until it is there.
    focus: Option<String>,
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
    /// Code attempts the server allows before a new code is needed.
    attempts_remaining: Option<u64>,
    /// A new code arrived: the old one's field empties, as desktop.
    clear_code: bool,
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
    /// The open edit's last save failed (`send_error` says why).
    edit_failed: bool,
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
    /// Voice messages sending on their own (\"Send voice messages as soon
    /// as I stop recording\"): uploading, failed, or waiting their turn.
    voice_sends: Vec<recording::VoiceSend>,
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
    /// A forward's original in another conversation: the navigation that
    /// goes there, its channel and the thread to open once it loads.
    forward_focus: Option<(u64, String, String, Option<String>)>,
    /// That thread and the reply to focus in it, ready to open.
    forward_thread: Option<(String, Option<String>)>,
    /// The message opened from elsewhere, highlighted until the
    /// conversation or thread changes.
    focused: Option<String>,
    /// Each channel's read and last sequence from the latest unread load.
    reads: HashMap<String, (u64, u64)>,
    /// What was unread in the open channel when you arrived: "New messages"
    /// goes above the first such message from someone else.
    unread_boundary: Option<(u64, u64)>,
    /// The visit's read snapshot is on its way; marking read waits for it.
    unread_visit_pending: bool,
    /// The read snapshot a navigation requested beside its history, by
    /// navigation, with its answer once it came; the visit adopts it.
    unread_snapshot: Option<(u64, Option<caper_client::model::UnreadSpaces>)>,
    /// Thread roots in the open channel with unread replies, and the first
    /// unread reply's seq (`ThreadUnread`). One request is in flight at a
    /// time; `dirty` asks for a fresh snapshot after it.
    thread_unread: std::collections::BTreeMap<String, String>,
    thread_unread_busy: bool,
    thread_unread_dirty: bool,
    thread_unread_revision: u64,
    /// Spaces with someone in voice, for the rail.
    voice_activity: std::collections::BTreeSet<String>,
    voice_activity_loading: bool,
    /// "Toggle channels": only channels with live voice show.
    channels_collapsed: bool,
    /// What the error banner's button tries again.
    retry: Option<Retry>,
    /// Unsent composer text by conversation, restored on return.
    drafts: HashMap<String, String>,
    /// The opened conversation's draft, for the composer at the next render.
    restore_draft: Option<String>,
    /// The signed-in account still needs a username and display name.
    profile_setup: bool,
    /// The open profile card's state.
    card: Option<profile::CardState>,
    /// People you blocked (`GET /api/blocks`), once loaded.
    blocks: Option<Vec<caper_client::model::BlockedAccount>>,
    block_busy: bool,
    /// Blocked runs you chose to show, by their first message.
    shown_blocked: HashSet<String>,
    /// Fences space invitations and membership changes.
    membership_request: u64,
    /// Message requests: the section, and an answer in flight.
    requests: requests::Requests,
    /// Who reacted, by message, for chip hover cards and View reactions.
    reactors: std::collections::BTreeMap<String, caper_client::reactions::ReactorCache>,
    /// The open View reactions window: message ID and selected emoji.
    reactors_view: Option<(String, String)>,
    /// The open Message history and its latest request.
    history: Option<history::History>,
    history_request: u64,
    /// KLIPY's key once `GET /api/klipy` answered, and when it was asked.
    klipy: Option<caper_client::klipy::Config>,
    klipy_checked: Option<Instant>,
    /// A jump's history window, newer messages past it, and a context
    /// read in flight.
    context_window: Option<context::Window>,
    has_newer: bool,
    context_loading: bool,
    /// Reply quotes shown in full, by the replying message.
    expanded_quotes: HashSet<String>,
    /// The Caper team's feedback inbox, whether it shows, and how many
    /// reports wait for a reply (`app::feedback`).
    feedback: caper_client::feedback::Inbox,
    feedback_open: bool,
    feedback_waiting: u64,
    /// The open feedback visit, one on its way, and where the report is
    /// once its conversation loads: a reported reply's thread after the
    /// root's context, or the report in a channel the team member is in.
    visit: Option<caper_client::feedback::Visit>,
    visit_request: Option<caper_client::feedback::VisitRequest>,
    report_thread: Option<(String, caper_client::feedback::ThreadFocus)>,
    report_focus: Option<(u64, caper_client::feedback::VisitTarget)>,
    /// The notifications inbox, and each row's inline reply.
    inbox: caper_client::activity::Inbox,
    replies: HashMap<String, inbox::Reply>,
    /// An inbox row's message to bring into view once its conversation
    /// loads: the navigation, channel, message and thread root.
    arrival: Option<(u64, String, String, Option<String>)>,
    /// That message, ready to jump to.
    arrival_ready: Option<(String, Option<String>)>,
    /// A failed install reported from the update review.
    update_report: Option<updates::Report>,
    /// Privacy and notification settings.
    settings: settings::AccountSettings,
    /// Space and channel management, invites and membership.
    admin: admin::Admin,
    /// Voice rosters, availability and a join's membership check.
    calls: call::Calls,
}

impl State {
    /// A fresh DM list; an open DM keeps its peer's current name.
    fn set_directs(&mut self, directs: Vec<DirectConversation>) {
        self.directs = directs;
        if let Some(direct) = &self.direct
            && let Some(entry) = self.directs.iter().find(|entry| &entry.id == direct)
        {
            self.channel_name = entry.peer.display_name.clone();
        }
        self.layout += 1;
    }

    fn page(&self) -> Page {
        self.page.unwrap_or(Page::Loading)
    }

    /// You can write here. An incoming request stays read-only until it is
    /// accepted, and a block stops reactions, pins and edits too.
    fn joined(&self) -> bool {
        if self.direct.is_some() {
            return self.open_request().is_none() && !self.open_blocked();
        }
        self.detail
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
        // A visit has no member list: its reporter and the loaded authors
        // whose username is known.
        if let Some(visit) = self.visiting() {
            let authors: HashSet<&str> = self
                .timeline
                .messages()
                .map(|message| message.author.id.as_str())
                .collect();
            let known = self
                .people
                .iter()
                .flatten()
                .filter(|person| authors.contains(person.id.as_str()))
                .map(mentions::Person::from);
            let reporter = visit.reporter.as_ref().map(|person| mentions::Person {
                username: person.username.clone(),
                display_name: person.display_name.clone(),
                avatar_id: person.avatar_id,
                avatar_asset: person.avatar_asset.clone(),
                avatar_url: person.avatar_url.clone(),
                invited: false,
            });
            let me = self
                .account
                .as_ref()
                .and_then(|account| account.username.as_deref());
            let mut people: Vec<mentions::Person> = Vec::new();
            for person in reporter.into_iter().chain(known) {
                if Some(person.username.as_str()) != me
                    && !people.iter().any(|known| known.username == person.username)
                {
                    people.push(person);
                }
            }
            return (
                people,
                mentions::Specials {
                    channel: false,
                    feedback: true,
                },
            );
        }
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
    api: Api,
    /// The main composer's link preview (as desktop and web).
    link_preview: RefCell<caper_client::embeds::ComposerPreview>,
    /// Which link the shown preview card is for, so it redraws only on change.
    shown_preview: RefCell<Option<String>>,
    admin_window: RefCell<Option<admin::AdminWindow>>,
    confirm_window: RefCell<Option<admin::ConfirmWindow>>,
    images: Rc<Images>,
    worker: Worker<Wakes>,
    state: RefCell<State>,
    /// Keep the newest message in view as rows arrive and are measured.
    follow: Cell<bool>,
    /// The same for the thread pane.
    thread_follow: Cell<bool>,
    /// A conversation's draft is being put back: not typing.
    restoring: Cell<bool>,
    /// The app stylesheet, reloaded at the interface zoom.
    style: gtk::CssProvider,
    zoom: Cell<f64>,
    /// Hides "Zoom N%" when the latest change is two seconds old.
    zoom_shown: Cell<u64>,
    /// `pin_tail` is moving the list: its scrolling is not the reader's.
    settling: Cell<bool>,
    rendered_layout: Cell<u64>,
    rendered_files: Cell<u64>,
    /// The open file dialog: GTK drops a native dialog nothing holds.
    chooser: RefCell<Option<gtk::FileChooserNative>>,
    channel_ids: RefCell<Vec<String>>,
    direct_ids: RefCell<Vec<String>>,
    /// The message request rows, after "Message requests".
    request_ids: RefCell<Vec<String>>,
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
    forwarding: RefCell<forward::Forwarding>,
    profile_dialog: RefCell<Option<account::ProfileDialog>>,
    /// Wakes the main loop from other threads.
    wakes: Wakes,
    settings: RefCell<Option<settings::SettingsDialog>>,
    prefs: Rc<crate::prefs::Prefs>,
    effects: RefCell<caper_client::effects::Effects>,
    /// A redraw is waiting for the next timed mute to end.
    mute_timer: Cell<bool>,
    /// The voice call, and what the call card last showed.
    voice: RefCell<caper_client::voice::Voice>,
    heard: RefCell<call::Heard>,
    voice_ticking: Cell<bool>,
    rendered_toggles: Cell<Option<(bool, bool)>>,
    rendered_card: RefCell<String>,
    voice_timer: RefCell<Option<gtk::Label>>,
    audio: RefCell<Option<audio::AudioDialog>>,
    /// The sidebar's live room timers and their session starts.
    room_timers: RefCell<Vec<(gtk::Label, u64)>>,
    room_ticking: Cell<bool>,
    report: RefCell<Option<audio::ReportDialog>>,
    history_window: RefCell<Option<history::HistoryWindow>>,
    gif_picker: RefCell<Option<gifs::Picker>>,
    inbox_window: RefCell<Option<inbox::InboxWindow>>,
    feedback_page: RefCell<Option<feedback::Page>>,
    /// Background update checks, this revision's What's new, and their
    /// windows.
    updates: RefCell<caper_client::updates::Updates>,
    whats_new: RefCell<caper_client::whats_new::WhatsNew>,
    whats_new_window: RefCell<Option<updates::WhatsNewWindow>>,
    update_window: RefCell<Option<updates::UpdateWindow>>,
    /// The open View reactions window.
    reactors_window: RefCell<Option<reactors::ReactorsWindow>>,
    /// The open attachment editor.
    editor: RefCell<Option<editor::EditorWindow>>,
    /// Edited files opened in another app, kept until quit.
    opened: RefCell<Vec<std::sync::Arc<caper_client::uploads::Scratch>>>,
    /// The microphone recording from a composer, and a dictation being
    /// transcribed (`recording`).
    recording: RefCell<Option<recording::Recording>>,
    dictation: RefCell<Option<recording::Dictation>>,
    recording_ticking: Cell<bool>,
    /// What each composer's voice-send rows show, so they rebuild on change.
    voice_send_rows: RefCell<[String; 2]>,
    /// The member list's saved width is placed (`panes`).
    panes_placed: Cell<bool>,
    /// The open thread has its saved width (`panes`).
    thread_placed: Cell<bool>,
    /// The window is 760 px or less, Browse is open there, and whether a
    /// wide window showed the member list (`narrow`).
    narrow: Cell<bool>,
    browsing: Cell<bool>,
    wide_members: Cell<bool>,
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

pub fn start(application: &gtk::Application, base: &str, process_start: Instant) {
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
    let style = gtk::CssProvider::new();
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &style,
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
    let prefs = Rc::new(crate::prefs::Prefs::load());
    prefs.follow_window(&ui.window);
    let zoom = crate::zoom::valid(prefs.get::<f64>("zoom").unwrap_or(1.0));
    style.load_from_data(&crate::zoom::scale_css(STYLE, zoom));
    let repaint = sender.clone();
    let voice = caper_client::voice::Voice::new(
        api.base().clone(),
        caper_client::voice::Repaint::new(move |delay| {
            Wakes(repaint.clone()).wake_after(delay.unwrap_or_default());
        }),
    );
    let app = Rc::new(App {
        ui,
        voice: RefCell::new(voice),
        heard: RefCell::default(),
        voice_ticking: Cell::new(false),
        rendered_toggles: Cell::new(None),
        rendered_card: RefCell::default(),
        voice_timer: RefCell::default(),
        audio: RefCell::default(),
        room_timers: RefCell::default(),
        room_ticking: Cell::new(false),
        report: RefCell::default(),
        editor: RefCell::default(),
        opened: RefCell::default(),
        recording: RefCell::default(),
        dictation: RefCell::default(),
        recording_ticking: Cell::new(false),
        voice_send_rows: RefCell::default(),
        panes_placed: Cell::new(false),
        thread_placed: Cell::new(false),
        narrow: Cell::new(false),
        browsing: Cell::new(false),
        wide_members: Cell::new(true),
        images,
        worker: Worker::new(api.clone(), Wakes(sender.clone())),
        state: RefCell::new(State::default()),
        follow: Cell::new(true),
        thread_follow: Cell::new(true),
        restoring: Cell::new(false),
        style,
        zoom: Cell::new(zoom),
        zoom_shown: Cell::new(0),
        settling: Cell::new(false),
        rendered_layout: Cell::new(u64::MAX),
        rendered_files: Cell::new(u64::MAX),
        chooser: RefCell::default(),
        channel_ids: RefCell::default(),
        link_preview: RefCell::default(),
        shown_preview: RefCell::default(),
        direct_ids: RefCell::default(),
        request_ids: RefCell::default(),
        reactors_window: RefCell::default(),
        history_window: RefCell::default(),
        inbox_window: RefCell::default(),
        feedback_page: RefCell::default(),
        updates: RefCell::new(caper_client::updates::Updates::start(
            {
                let wakes = Wakes(sender.clone());
                move || wakes.wake()
            },
            true,
        )),
        whats_new: RefCell::new(caper_client::whats_new::WhatsNew::new(
            &api.base().origin().ascii_serialization(),
            caper_client::whats_new::PLATFORM,
        )),
        whats_new_window: RefCell::default(),
        update_window: RefCell::default(),
        gif_picker: RefCell::default(),
        members_shown: RefCell::default(),
        activity: Cell::new(Instant::now()),
        ticking: Cell::new(false),
        countdown: Cell::new(false),
        suggestions: RefCell::default(),
        direct_dialog: RefCell::default(),
        pins: RefCell::default(),
        profile: RefCell::default(),
        forwarding: RefCell::default(),
        profile_dialog: RefCell::default(),
        wakes: Wakes(sender.clone()),
        settings: RefCell::default(),
        mute_timer: Cell::new(false),
        api: api.clone(),
        admin_window: RefCell::default(),
        confirm_window: RefCell::default(),
        effects: RefCell::new(caper_client::effects::Effects::new(
            prefs.get::<bool>("sound-effects-v1").unwrap_or(true),
        )),
        prefs,
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
    // Insert emoji at the caret, as on desktop.
    for composer in [&app.ui.composer, &app.ui.thread_composer] {
        let weak = Rc::downgrade(&app);
        let view = composer.view.clone();
        composer.emoji.connect_clicked(move |button| {
            let Some(app) = weak.upgrade() else { return };
            let view = view.clone();
            crate::picker::show(
                &app.images,
                button.upcast_ref(),
                "Insert emoji",
                HashSet::new(),
                move |emoji, _| {
                    let buffer = view.buffer();
                    buffer.insert_at_cursor(&emoji);
                    view.grab_focus();
                },
            );
        });
    }
    app.restore_voice();
    app.connect();
    app.connect_voice();
    app.connect_drop();
    app.connect_requests();
    app.connect_gifs();
    app.connect_recording();
    app.connect_panes();
    app.connect_narrow();
    app.connect_feedback();
    app.connect_inbox();
    app.start_updates();
    app.install_actions();
    app.install_shortcuts();
    app.install_notification_actions();
    app.install_admin_actions();
    app.render();
    app.connect_bench(process_start);
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
            .button
            .connect_clicked(move |_| app.request_code(false));
        let app = self.clone();
        ui.code.connect_activate(move |_| app.verify());
        // As desktop: an edit clears the error and decides whether the
        // action can be used.
        for entry in [ui.email.clone(), ui.code.clone()] {
            let app = self.clone();
            entry.connect_changed(move |_| {
                if let Ok(mut state) = app.state.try_borrow_mut() {
                    if !state.busy {
                        state.error = None;
                    }
                    drop(state);
                    app.render();
                }
            });
        }
        let app = self.clone();
        ui.first_name
            .connect_activate(move |_| app.create_first_space());
        let app = self.clone();
        ui.first_create
            .connect_clicked(move |_| app.create_first_space());
        let app = self.clone();
        ui.first_name.connect_changed(move |_| {
            if let Ok(mut state) = app.state.try_borrow_mut() {
                state.error = None;
            }
            app.render();
        });
        let app = self.clone();
        ui.first_logout.connect_clicked(move |_| app.sign_out());
        // Your avatar and name open your own card (Edit profile).
        for target in [
            ui.account_avatar.root.clone().upcast::<gtk::Widget>(),
            ui.account_name.clone().upcast(),
        ] {
            let click = gtk::GestureClick::new();
            let (weak, anchor) = (Rc::downgrade(self), target.clone());
            click.connect_released(move |_, _, _, _| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                let me = app
                    .state
                    .borrow()
                    .account
                    .as_ref()
                    .map(|account| account.id.clone());
                if let Some(me) = me {
                    app.open_profile(me, None, &anchor);
                }
            });
            target.add_controller(click);
            target.set_cursor_from_name(Some("pointer"));
        }
        let app = self.clone();
        ui.code_button.button.connect_clicked(move |_| {
            // Out of attempts, it asks for a new code instead.
            if app.state.borrow().attempts_remaining == Some(0) {
                app.request_code(true);
            } else {
                app.verify();
            }
        });
        let app = self.clone();
        ui.resend.connect_clicked(move |_| app.request_code(true));
        let app = self.clone();
        ui.other_email.connect_clicked(move |_| {
            {
                let mut state = app.state.borrow_mut();
                state.challenge = None;
                state.error = None;
                state.signin_status = None;
                state.attempts_remaining = None;
                state.resends = 0;
            }
            app.render();
        });
        let app = self.clone();
        ui.settings
            .connect_clicked(move |button| app.settings_menu(button));

        let app = self.clone();
        // As desktop, the member list stays beside an open thread.
        ui.members_toggle.connect_toggled(move |_| app.render());
        let app = self.clone();
        ui.error_retry.connect_clicked(move |_| app.retry());
        let app = self.clone();
        ui.empty_action
            .connect_clicked(move |_| app.open_create_channel());
        for button in ui.thread_back.iter().chain([&ui.thread_close]) {
            let app = self.clone();
            button.connect_clicked(move |_| {
                {
                    let mut state = app.state.borrow_mut();
                    state.thread = None;
                    state.focused = None;
                }
                app.render();
            });
        }

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
                // The open channel too: narrow Browse or the inbox goes
                // back to it.
                app.browsing.set(false);
                state.feedback_open = false;
                drop(state);
                app.render();
            }
        });
        let app = self.clone();
        ui.channels_toggle.connect_clicked(move |_| {
            let expanded = {
                let mut state = app.state.borrow_mut();
                state.channels_collapsed = !state.channels_collapsed;
                state.layout += 1;
                !state.channels_collapsed
            };
            app.play(if expanded {
                Effect::ToggleOn
            } else {
                Effect::ToggleOff
            });
            app.render();
        });
        let app = self.clone();
        ui.directs.connect_row_activated(move |_, row| {
            let id = usize::try_from(row.index())
                .ok()
                .and_then(|index| app.direct_ids.borrow().get(index).cloned());
            match id {
                Some(id) if id == NEW_MESSAGE => app.open_start_direct(),
                Some(id) if id == SELF_NOTES => app.create_self_notes(),
                Some(id) => app.open_direct(id),
                None => {}
            }
        });
        let app = self.clone();
        ui.members
            .connect_row_activated(move |_, row| app.open_member(row));
        let app = self.clone();
        ui.join.connect_clicked(move |_| app.join());
        let app = self.clone();
        ui.load_newer.connect_clicked(move |_| app.load_newer());
        let app = self.clone();
        ui.latest.connect_clicked(move |_| app.back_to_latest());
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
        let app = self.clone();
        ui.compose_reload
            .connect_clicked(move |_| app.reload_edit());

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
            let (app, rich) = (self.clone(), composer.rich.clone());
            keys.connect_key_pressed(move |_, key, _, modifiers| {
                app.active();
                // An IME's Enter and Escape are its own.
                if rich.composing() {
                    return glib::Propagation::Proceed;
                }
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
            if let Some(send) = &composer.send {
                let app = self.clone();
                send.connect_clicked(move |_| app.send_message(thread));
            }
            let app = self.clone();
            composer
                .attach
                .connect_clicked(move |button| app.attach_menu(thread, button.upcast_ref()));
            let app = self.clone();
            composer.view.buffer().connect_changed(move |buffer| {
                if buffer.char_count() > 0 && !app.restoring.get() {
                    app.typed();
                }
                if !thread {
                    app.update_link_preview();
                }
            });
        }

        // The thread follows its newest reply while read at the bottom, and
        // loads older replies near the top.
        let app = self.clone();
        ui.thread_list
            .scroller
            .vadjustment()
            .connect_value_changed(move |adjustment| {
                let bottom = adjustment.upper() - adjustment.page_size();
                app.thread_follow.set(adjustment.value() >= bottom - 48.0);
                if adjustment.upper() > adjustment.page_size()
                    && adjustment.value() < adjustment.page_size()
                {
                    let app = app.clone();
                    glib::idle_add_local_once(move || app.load_older_replies());
                }
            });

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
    /// Window shortcuts the desktop has: Ctrl+, opens Settings, End jumps
    /// to the latest message (outside text fields), and Ctrl+= / Ctrl+− /
    /// Ctrl+0 zoom the interface.
    fn install_shortcuts(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(app) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
            let others =
                modifiers.intersects(gdk::ModifierType::ALT_MASK | gdk::ModifierType::SUPER_MASK);
            if others {
                return glib::Propagation::Proceed;
            }
            let zoom = app.zoom.get();
            let next = match key {
                gdk::Key::comma if control => {
                    let app = app.clone();
                    glib::idle_add_local_once(move || app.open_settings());
                    return glib::Propagation::Stop;
                }
                gdk::Key::equal | gdk::Key::plus | gdk::Key::KP_Add if control => {
                    crate::zoom::zoom_in(zoom)
                }
                gdk::Key::minus | gdk::Key::underscore | gdk::Key::KP_Subtract if control => {
                    crate::zoom::zoom_out(zoom)
                }
                gdk::Key::_0 | gdk::Key::KP_0 if control => 1.0,
                gdk::Key::End if !control => {
                    let typing =
                        gtk::prelude::GtkWindowExt::focus(&app.ui.window).is_some_and(|widget| {
                            widget.is::<gtk::TextView>()
                                || widget.is::<gtk::Text>()
                                || widget.is::<gtk::Entry>()
                                // A focused pane line takes End itself.
                                || widget.is::<gtk::Paned>()
                        });
                    if typing || app.state.borrow().page() != Page::Chat {
                        return glib::Propagation::Proceed;
                    }
                    app.follow.set(true);
                    app.pin_tail();
                    return glib::Propagation::Stop;
                }
                _ => return glib::Propagation::Proceed,
            };
            app.set_zoom(next);
            glib::Propagation::Stop
        });
        self.ui.window.add_controller(keys);
    }

    fn set_zoom(self: &Rc<Self>, factor: f64) {
        if factor != self.zoom.replace(factor) {
            self.prefs.set("zoom", factor);
            self.style
                .load_from_data(&crate::zoom::scale_css(STYLE, factor));
        }
        let notice = &self.ui.zoom_notice;
        notice.set_label(&format!("Zoom {:.0}%", factor * 100.0));
        notice.set_visible(true);
        let shown = self.zoom_shown.get() + 1;
        self.zoom_shown.set(shown);
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            if let Some(app) = weak.upgrade()
                && app.zoom_shown.get() == shown
            {
                app.ui.zoom_notice.set_visible(false);
            }
        });
    }

    fn install_actions(self: &Rc<Self>) {
        let group = gio::SimpleActionGroup::new();
        type Run = fn(&Rc<App>, String);
        let actions: [(&str, Run); 13] = [
            ("reactions", |app, id| app.open_reactors(id)),
            ("history", |app, id| app.open_history(&id)),
            ("reply", |app, id| app.row_action(Action::Reply(id), false)),
            ("thread", |app, id| {
                app.row_action(Action::Thread(id), false)
            }),
            ("copy-id", |app, id| app.ui.window.clipboard().set_text(&id)),
            ("block", |app, id| {
                let account = app.message_author_account(&id);
                if let Some(account) = account {
                    app.confirm_block(account, None);
                }
            }),
            ("unblock", |app, id| {
                let account = app.message_author_account(&id);
                if let Some(account) = account {
                    app.set_block(account, false, None);
                }
            }),
            ("forward", |app, id| app.open_forward(&id)),
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
            // As desktop: Enter does what the button can, nothing more.
            if code.len() == 6 && state.attempts_remaining != Some(0) {
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

    fn establish(&self, state: &mut State, token: String, account: Account, spaces: Spaces) {
        state.epoch += 1;
        state.token = Some(token);
        state.account = Some(account);
        state.set_spaces(spaces);
        state.page = Some(Page::Chat);
        state.layout += 1;
        // The DM list arrives with the account (`SignedInDirects`).
        self.refresh_unread(state);
        if let Some(token) = state.token.clone() {
            self.worker.send(Command::AssetUsage { token });
        }
        self.check_klipy(state);
        self.account_op(state, AccountOperation::LoadBlocks);
        state.settings = settings::AccountSettings::default();
        self.load_notifications(state);
        self.start_notifications(state);
        if state.needs_profile() {
            state.profile_setup = true;
        } else if let Some(space) = state.spaces.first().map(|space| space.id.clone()) {
            self.navigate(state, Some(space), None);
        }
    }

    /// Desktop's sign-out: stop the gateway, forget the saved session, revoke
    /// it on the server, and return to sign-in.
    fn sign_out(self: &Rc<Self>) {
        self.close_audio();
        self.clear_recording();
        if let Some(report) = self.report.borrow_mut().take() {
            report.window.destroy();
        }
        self.voice.borrow_mut().leave();
        self.voice.borrow_mut().error = None;
        self.close_forward();
        self.close_profile_editor();
        self.close_settings();
        self.close_inbox();
        for window in [
            self.admin_window.borrow_mut().take().map(|w| w.window),
            self.confirm_window.borrow_mut().take().map(|w| w.window),
        ]
        .into_iter()
        .flatten()
        {
            window.destroy();
        }
        {
            let mut state = self.state.borrow_mut();
            let token = state.token.take();
            let uploaded: Vec<String> = state
                .files
                .iter()
                .chain(state.voice_sends.iter().map(|send| &send.file))
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
            self.worker.send(Command::StopNotifications);
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
        state.visit_request = None;
        state.feedback_open = false;
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
        // Choosing a channel leaves narrow Browse; a space alone keeps it.
        if channel.is_some() {
            self.browsing.set(false);
        }
        // As desktop: snapshot the read position fresh on arrival, before
        // this visit marks anything read, so the divider stays put.
        state.unread_boundary = None;
        state.unread_visit_pending = false;
        state.thread_unread.clear();
        state.thread_unread_dirty = true;
        // The read snapshot goes out with the history, not after it.
        state.unread_snapshot = None;
        if let Some(token) = state.token.clone() {
            state.unread_snapshot = Some((state.navigation, None));
            self.worker.send(Command::LoadUnread {
                epoch: state.epoch,
                revision: state.unread_revision,
                visit: Some(state.navigation),
                token,
            });
        }
        state.restore_draft = Some(
            channel
                .as_ref()
                .and_then(|channel| state.drafts.remove(channel))
                .unwrap_or_default(),
        );
        self.prepare(state, space, channel);
    }

    /// Your notes don't exist yet: create the DM with yourself, which then
    /// opens as any created DM does.
    fn create_self_notes(self: &Rc<Self>) {
        let mut state = self.state.borrow_mut();
        let (Some(token), Some(username)) = (
            state.token.clone(),
            state
                .account
                .as_ref()
                .and_then(|account| account.username.clone()),
        ) else {
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

    fn open_direct(self: &Rc<Self>, id: String) {
        {
            let mut state = self.state.borrow_mut();
            if state.channel.as_deref() == Some(&id) {
                // Narrow Browse goes back to the open DM, as from the inbox.
                state.feedback_open = false;
                drop(state);
                self.set_browsing(false);
                return;
            }
            self.enter_direct(&mut state, id);
        }
        self.render();
    }

    fn enter_direct(&self, state: &mut State, id: String) {
        state.visit_request = None;
        state.feedback_open = false;
        self.leave_conversation(state);
        state.channel_name = state
            .directs
            .iter()
            .find(|direct| direct.id == id)
            .map(|direct| direct.peer.display_name.clone())
            .unwrap_or_default();
        state.channel = Some(id.clone());
        state.direct = Some(id.clone());
        self.browsing.set(false);
        state.restore_draft = Some(state.drafts.remove(&id).unwrap_or_default());
        self.load_people(state);
        self.prepare(state, None, Some(id));
    }

    fn leave_conversation(&self, state: &mut State) {
        self.ui.composer.rich.end();
        // The composer's unsent text stays with its conversation (an edit
        // in progress is not a draft).
        if let Some(channel) = state.channel.clone()
            && !matches!(state.compose, Some(Compose::Edit { .. }))
        {
            let buffer = self.ui.composer.view.buffer();
            let text = buffer.text(&buffer.start_iter(), &buffer.end_iter(), true);
            if text.trim().is_empty() {
                state.drafts.remove(&channel);
            } else {
                state.drafts.insert(channel, text.to_string());
            }
        }
        self.set_typing(state, false);
        *self.link_preview.borrow_mut() = Default::default();
        state.generation += 1;
        state.navigation += 1;
        state.loading_older = false;
        state.session = None;
        state.session_error = None;
        state.pending = None;
        state.compose = None;
        state.thread = None;
        state.focused = None;
        state.typers.clear();
        state.reactions.clear();
        state.reactors.clear();
        state.reactors_view = None;
        state.history = None;
        state.context_window = None;
        state.has_newer = false;
        state.context_loading = false;
        state.expanded_quotes.clear();
        state.files_revision += 1;
        state.direct = None;
        state.visit = None;
        state.report_thread = None;
        state.jump_to = None;
        if let Some(pins) = self.pins.borrow_mut().take() {
            pins.popover.popdown();
        }
        state.status = "Loading…".into();
        state.error = None;
        state.retry = None;
        state.send_error = None;
        state.layout += 1;
        // DMs started or answered elsewhere appear as you move around.
        self.load_directs(state);
        self.check_klipy(state);
    }

    fn prepare(&self, state: &mut State, space: Option<String>, channel: Option<String>) {
        // A voice message sending on its own belongs to its conversation.
        self.discard_voice_sends(state, channel.as_deref());
        // A report's conversation, read without joining it.
        let visit = space.is_none()
            && state
                .visit_request
                .as_ref()
                .is_some_and(|request| channel.as_ref() == Some(&request.target.channel));
        self.worker.send(Command::PrepareNavigation {
            generation: state.generation,
            navigation: state.navigation,
            token: state.token.clone(),
            space,
            channel,
            name: state.name(),
            cached: None,
            visit,
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
                state.restore_draft = Some(state.drafts.remove(&direct).unwrap_or_default());
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
            media: self.media_watch(state),
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
        close_on_escape(&window);
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
                thread_unread: &state.thread_unread,
                reactors: &state.reactors,
                expanded_quotes: &state.expanded_quotes,
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
        let root = self
            .state
            .borrow()
            .timeline
            .pinned_messages()
            .find(|message| message.id == id)
            .and_then(|message| message.thread_root_id.clone());
        self.jump_to_message(id, root);
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
            AccountResult::Blocks(Ok(blocks)) => {
                state.blocks = Some(blocks);
                state.settings.blocks_error = None;
            }
            AccountResult::Blocks(Err(error)) => state.settings.blocks_error = Some(error),
            AccountResult::Privacy(result) => match result {
                // A save in flight owns the shown value.
                Ok(value) if !state.settings.privacy_saving => state.settings.privacy = Some(value),
                Ok(_) => {}
                Err(error) => state.settings.privacy_error = Some(error),
            },
            AccountResult::PrivacySaved { previous, result } => {
                state.settings.privacy_saving = false;
                match result {
                    Ok(value) => {
                        state.settings.privacy = Some(value);
                        state.settings.privacy_error = None;
                    }
                    Err(error) => {
                        state.settings.privacy = previous;
                        state.settings.privacy_error = Some(format!("Could not save: {error}"));
                    }
                }
            }
            AccountResult::Notifications { revision, result } => {
                state.settings.notifications.finish_load(revision, result);
                state.layout += 1;
            }
            AccountResult::NotificationsSaved { scope, result } => {
                state.settings.notifications.finish_save(&scope, result);
                state.layout += 1;
            }
            result @ (AccountResult::Accepted { .. } | AccountResult::Declined { .. }) => {
                self.request_result(state, result)
            }
            AccountResult::Block {
                account,
                blocked,
                request,
                result,
            } => {
                state.block_busy = false;
                state.layout += 1;
                let open = state
                    .open_peer_direct()
                    .is_some_and(|direct| direct.peer.id == account.id);
                let card = state.card.as_mut().filter(|card| card.person == account.id);
                if let Err(error) = result {
                    match card {
                        Some(card) => card.error = Some(error),
                        // The request or blocked bar shows its own.
                        None if open => state.requests.error = Some(error),
                        None => state.error = Some(error),
                    }
                    return;
                }
                if open {
                    state.requests.error = None;
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
                if blocked {
                    // Blocking also declines their pending request.
                    let requests: Vec<String> = state
                        .incoming_requests()
                        .filter(|direct| direct.peer.id == account.id)
                        .map(|direct| direct.id.clone())
                        .chain(request)
                        .collect();
                    for id in requests {
                        self.leave_request(state, &id);
                    }
                }
            }
            result @ (AccountResult::DefaultInviteLink { .. }
            | AccountResult::InviteLinks { .. }
            | AccountResult::InviteLinkCreated { .. }
            | AccountResult::InviteLinkRevoked { .. }
            | AccountResult::InvitePreview { .. }
            | AccountResult::InviteJoined { .. }) => self.invite_result(state, result),
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
        // Reading a request must not look like engaging with it.
        // Nor does a jump's window away from the newest messages.
        // A feedback visit has no read marker to move.
        if state.unread_visit_pending
            || state.open_request().is_some()
            || state.has_newer
            || state.visiting().is_some()
        {
            return;
        }
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
                state.channel.as_ref() == Some(&file.channel)
                    && file.thread_root_id.as_deref() == root
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
                    path,
                    scratch,
                );
                self.worker
                    .send(file.command(token.clone(), settings.clone()));
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
        let voice = state.voice_sends.iter().any(|send| send.file.key == key);
        let file = state
            .files
            .iter_mut()
            .chain(
                state
                    .pending
                    .iter_mut()
                    .flat_map(|pending| pending.files.iter_mut()),
            )
            .chain(state.voice_sends.iter_mut().map(|send| &mut send.file))
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
        let storage_full = match &update {
            UploadUpdate::Finished(Err(error)) if error.storage_full => Some(error.message.clone()),
            _ => None,
        };
        if let Some(image) = file.upload.apply(update) {
            if let Some(attachment) = &file.attachment {
                self.images.media.insert_local(&attachment.id, &image);
            }
            file.thumbnail = Some(image);
        }
        if let Some(attachment) = &file.attachment
            && let Some(image) = &file.thumbnail
        {
            // You see your own copy without a download.
            self.images.media.insert_local(&attachment.id, image);
        }
        if storage_full.is_some() {
            file.error = storage_full;
        }
        state.files_revision += 1;
        if voice {
            self.send_voice_messages(state);
        }
    }

    fn chip_action(self: &Rc<Self>, action: ChipAction) {
        if let ChipAction::Edit(key) = action {
            let app = self.clone();
            glib::idle_add_local_once(move || app.view_draft(key));
            return;
        }
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
                    if let Some(file) = state.files.iter_mut().find(|file| file.key == key)
                        && file.upload.restart().is_some()
                    {
                        self.worker.send(file.command(token, settings));
                    }
                }
                ChipAction::Spoiler(key, spoiler) => {
                    if let Some(file) = state.files.iter_mut().find(|file| file.key == key) {
                        file.spoiler = spoiler;
                    }
                }
                ChipAction::Edit(_) => {}
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
        let paste: gtk::Button = named("GtkButton", "Paste from clipboard");
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        line.append(&self.images.icon_image(Icon::Copy, "#f3f4f5", 16));
        line.append(&label("Paste from clipboard", &[]));
        paste.set_child(Some(&line));
        paste.add_css_class("flat");
        paste.set_sensitive(attached < caper_client::uploads::MAX_ATTACHMENTS);
        let (app, popover_ref) = (self.clone(), popover.downgrade());
        paste.connect_clicked(move |_| {
            if let Some(popover) = popover_ref.upgrade() {
                popover.popdown();
            }
            if !app.paste_files(thread) {
                app.state.borrow_mut().send_error =
                    Some("Nothing to paste. Copy an image or files first.".into());
                app.render();
            }
        });
        menu.append(&paste);
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
                .filter(|file| {
                    state.channel.as_ref() == Some(&file.channel) && file.thread_root_id == root
                })
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
        let broadcast = thread && composer.broadcast.is_active();
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
                    .any(|file| file.channel == channel && file.thread_root_id == root);
            if text.is_empty() && !files {
                return;
            }
            if files {
                let mine = state
                    .files
                    .iter()
                    .filter(|file| file.channel == channel && file.thread_root_id == root);
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
                state.edit_failed = false;
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
                .partition(|file| file.channel == channel && file.thread_root_id == thread_root);
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
                gif: None,
                share: None,
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
                broadcast,
                reply_to_id,
                attachment_ids,
                spoiler_attachment_ids,
                gif: None,
                suppress_embeds: !thread && self.link_preview.borrow().removed(text),
            });
        }
        if !thread {
            self.follow.set(true);
        }
        composer.broadcast.set_active(false);
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

    /// Desktop's "Discard draft and load latest": fetches the message as it
    /// is now; its text replaces the draft (`Event::EditSnapshot`).
    fn reload_edit(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let Some(Compose::Edit { original, .. }) = state.compose.take() else {
                return;
            };
            state.edit_request += 1;
            let request = state.edit_request;
            self.worker.send(Command::ReloadMessage {
                generation: state.generation,
                request,
                token: state.token.clone(),
                channel: original.channel_id.clone(),
                message: original.id.clone(),
            });
            state.compose = Some(Compose::Edit { original, request });
            state.send_error = None;
            state.edit_failed = false;
        }
        self.ui.composer.view.grab_focus();
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
            state.edit_failed = false;
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
        self.open_thread_at(root, None);
    }

    /// Opens `root`'s thread with `focus`, one of its replies, brought into
    /// view and highlighted.
    fn open_thread_at(self: &Rc<Self>, root: String, focus: Option<String>) {
        self.thread_follow.set(focus.is_none());
        {
            let mut state = self.state.borrow_mut();
            state.focused = focus.clone();
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
                has_more: false,
                focus,
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
        // A narrow thread takes the conversation's place, drawer and all.
        if self.is_narrow() {
            self.ui.members_toggle.set_active(false);
        }
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
            Action::LoadReactors(id) => self.load_reactors(&id),
            Action::History(id) => self.open_history(&id),
            Action::Quote { id, root } => self.jump_to_message(id, root),
            Action::ExpandQuote(id) => {
                {
                    let mut state = self.state.borrow_mut();
                    if !state.expanded_quotes.remove(&id) {
                        state.expanded_quotes.insert(id);
                    }
                }
                self.render();
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
                    // Cleared by `render`: the buffer's handlers read the state.
                    if matches!(state.compose, Some(Compose::Edit { .. })) {
                        state.restore_draft = Some(String::new());
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
            Action::ImageMenu { attachment, anchor } => self.image_menu(attachment, &anchor),
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
            Action::ViewForward(id) => self.view_forward(&id),
        }
    }

    /// Who wrote `id`, when the menu may block or unblock them.
    fn message_author_account(&self, id: &str) -> Option<caper_client::model::BlockedAccount> {
        let state = self.state.borrow();
        let message = state.message(id)?;
        if message.is_deleted() {
            return None;
        }
        state.blockable_author(&message.author)
    }

    fn row_menu(self: &Rc<Self>, id: &str, anchor: &gtk::Widget) {
        let (mine, pinned, forwardable, joined, in_thread, reacted, edited) = {
            let state = self.state.borrow();
            let Some(message) = state.message(id) else {
                return;
            };
            (
                state.session.as_ref().is_some_and(|session| {
                    !session.author.is_guest && session.author.id == message.author.id
                }),
                message.pin.is_some(),
                forward::forwardable(message),
                state.session.is_some() && !message.is_deleted(),
                message.thread_root_id.is_some(),
                !message.is_deleted() && !state.shown_reactions(message).is_empty(),
                !message.is_deleted() && message.forward.is_none() && message.edited_at.is_some(),
            )
        };
        let block = self.message_author_account(id).map(|account| {
            let blocked = self.state.borrow().blocked(&account.id);
            let name = if account.username.is_empty() {
                account.display_name.clone()
            } else {
                format!("@{}", account.username)
            };
            (blocked, name)
        });
        let menu = gio::Menu::new();
        let item = |label: &str, action: &str| {
            let item = gio::MenuItem::new(Some(label), None);
            item.set_action_and_target_value(Some(action), Some(&id.to_variant()));
            item
        };
        let actions = gio::Menu::new();
        // In the desktop's order.
        if joined {
            actions.append_item(&item("Reply", "row.reply"));
        }
        if joined && !in_thread {
            actions.append_item(&item("Reply in thread", "row.thread"));
        }
        if forwardable {
            actions.append_item(&item("Forward message", "row.forward"));
        }
        if reacted {
            actions.append_item(&item("View reactions", "row.reactions"));
        }
        if mine {
            actions.append_item(&item("Edit message", "row.edit"));
        }
        if edited {
            actions.append_item(&item("Message history", "row.history"));
        }
        actions.append_item(&if pinned {
            item("Unpin message", "row.unpin")
        } else {
            item("Pin message", "row.pin")
        });
        actions.append_item(&item("Copy text", "row.copy"));
        actions.append_item(&item("Copy message ID", "row.copy-id"));
        menu.append_section(None, &actions);
        if mine || block.is_some() {
            let danger = gio::Menu::new();
            if mine {
                danger.append_item(&item("Delete message", "row.delete"));
            }
            if let Some((blocked, name)) = block {
                danger.append_item(&if blocked {
                    item(&format!("Unblock {name}"), "row.unblock")
                } else {
                    item(&format!("Block {name}"), "row.block")
                });
            }
            menu.append_section(None, &danger);
        }
        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popup(&popover, anchor);
    }

    /// Desktop's menu on an image: Copy image (the original, once loaded),
    /// Copy image link and Open image in browser, with what copying did.
    fn image_menu(
        self: &Rc<Self>,
        attachment: caper_client::model::Attachment,
        anchor: &gtk::Widget,
    ) {
        let attachment = self.images.media.resolve(&attachment);
        let popover = gtk::Popover::new();
        popover.add_css_class("menu");
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let status = label("", &["muted", "small", "image-menu-status"]);
        status.set_visible(false);
        for (text, item) in [
            ("Copy image", 0),
            ("Copy image link", 1),
            ("Open image in browser", 2),
        ] {
            let button = gtk::Button::with_label(text);
            button.add_css_class("flat");
            button.add_css_class("menu-item");
            if let Some(label) = button.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
            }
            let (app, menu, status, attachment) = (
                self.clone(),
                popover.clone(),
                status.clone(),
                attachment.clone(),
            );
            button.connect_clicked(move |button| match item {
                0 => {
                    status.set_label("Copying image…");
                    status.set_visible(true);
                    button.set_sensitive(false);
                    let key = if attachment.content_type == "image/gif" {
                        attachment.id.clone()
                    } else {
                        crate::media::Media::full_key(&attachment.id)
                    };
                    app.images.media.retry(&key);
                    let (weak, menu, status, button, attachment, generation) = (
                        Rc::downgrade(&app),
                        menu.downgrade(),
                        status.clone(),
                        button.clone(),
                        attachment.clone(),
                        app.state.borrow().generation,
                    );
                    glib::timeout_add_local(Duration::from_millis(50), move || {
                        let Some(app) = weak.upgrade() else {
                            return glib::ControlFlow::Break;
                        };
                        // Closing the menu or signing out cancels clipboard delivery.
                        if app.state.borrow().generation != generation
                            || menu.upgrade().is_none_or(|menu| !menu.is_visible())
                        {
                            return glib::ControlFlow::Break;
                        }
                        let file = app.images.media.resolve(&attachment);
                        let url = if file.content_type == "image/gif" {
                            caper_client::attachments::inline_source(&file)
                        } else {
                            file.url.as_deref()
                        };
                        match url
                            .ok_or(())
                            .and_then(|url| app.images.media.first_frame(&key, url))
                        {
                            Ok(Some(texture)) => {
                                button.clipboard().set_texture(&texture);
                                status.set_label("Image copied");
                            }
                            Ok(None) => return glib::ControlFlow::Continue,
                            Err(()) => status.set_label("Couldn’t copy image. Try again."),
                        }
                        button.set_sensitive(true);
                        glib::ControlFlow::Break
                    });
                }
                1 => {
                    if let Some(url) = &attachment.url {
                        button.clipboard().set_text(url);
                    }
                    menu.popdown();
                }
                _ => {
                    menu.popdown();
                    app.images.media.open(&attachment);
                }
            });
            list.append(&button);
        }
        list.append(&status);
        popover.set_child(Some(&list));
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
        crate::picker::show(
            &self.images,
            anchor,
            "Add a reaction",
            mine,
            move |emoji, active| {
                app.react(id.clone(), emoji, active);
            },
        );
    }

    // ---------------------------------------------------------------------
    // Worker events.

    /// Keeps the open channel's thread dots current: marks the open thread
    /// read through its newest reply, else asks for a fresh snapshot.
    fn sync_thread_unread(&self, state: &mut State) {
        if state.thread_unread_busy || state.direct.is_some() {
            return;
        }
        let (Some(token), Some(channel)) = (state.token.clone(), state.channel.clone()) else {
            return;
        };
        let read = state
            .thread
            .as_ref()
            .filter(|thread| !thread.loading)
            .and_then(|thread| {
                let unread = state.thread_unread.get(&thread.root)?.parse::<u64>().ok()?;
                let newest = state
                    .timeline
                    .messages()
                    .filter(|message| message.thread_root_id.as_deref() == Some(&thread.root))
                    .filter_map(|message| message.seq.parse::<u64>().ok())
                    .max()?;
                (newest >= unread).then(|| (thread.root.clone(), newest.to_string()))
            });
        if read.is_none() && !state.thread_unread_dirty {
            return;
        }
        if read.is_none() {
            state.thread_unread_dirty = false;
        }
        state.thread_unread_revision += 1;
        state.thread_unread_busy = true;
        self.worker.send(Command::ThreadUnread {
            scope: (state.epoch, state.navigation, channel),
            revision: state.thread_unread_revision,
            token,
            read,
        });
    }

    /// Drains the worker's events, then updates what they changed.
    fn receive(self: &Rc<Self>) {
        let mut typing = false;
        let mut forwards = Vec::new();
        let mut deferred = Vec::new();
        {
            let mut state = self.state.borrow_mut();
            while let Ok(event) = self.worker.events.try_recv() {
                typing |= matches!(event, Event::Gateway(GatewayEvent::Typing { .. }));
                match event {
                    Event::Forward {
                        generation,
                        request,
                        result,
                    } => forwards.push((generation, request, result)),
                    Event::NotificationActivity { epoch } if epoch == state.epoch => {
                        // Unread state changed somewhere; an open inbox refreshes.
                        state.inbox.refreshed = None;
                        self.refresh_unread(&mut state);
                        self.load_directs(&state);
                    }
                    Event::NotificationError { epoch, error } if epoch == state.epoch => {
                        state.error = Some(error);
                    }
                    event @ Event::InvitationLoaded {
                        epoch, navigation, ..
                    } if epoch == state.epoch && navigation == state.navigation => {
                        deferred.push(event)
                    }
                    event @ (Event::Profiled { .. }
                    | Event::AvatarHistory { .. }
                    | Event::Picture { .. }
                    | Event::OpenNotification { .. }
                    | Event::ActivityRead { .. }
                    | Event::Dictated { .. }) => deferred.push(event),
                    event => self.handle(&mut state, event),
                }
            }
            self.sync_thread_unread(&mut state);
        }
        for (generation, request, result) in forwards {
            self.forward_result(generation, request, result);
        }
        for event in deferred {
            match event {
                Event::Profiled { generation, result } => self.profiled(generation, result),
                Event::Dictated { key, result } => self.dictated(key, result),
                Event::InvitationLoaded {
                    space,
                    channel,
                    result,
                    ..
                } => self.invitation_loaded(space, channel, result),
                Event::AvatarHistory {
                    epoch,
                    request,
                    result,
                } if epoch == self.state.borrow().epoch => self.previous_pictures(request, result),
                Event::Picture {
                    epoch,
                    target,
                    result,
                } if epoch == self.state.borrow().epoch => self.picture_changed(target, result),
                Event::OpenNotification { epoch, alert } if epoch == self.state.borrow().epoch => {
                    self.open_alert(alert)
                }
                Event::ActivityRead {
                    epoch,
                    navigation,
                    item,
                    open,
                    result,
                } if epoch == self.state.borrow().epoch => {
                    self.inbox_read(navigation, item, open, result)
                }
                _ => {}
            }
        }
        self.poll_profile_picture();
        self.renew_reading();
        let setup = std::mem::take(&mut self.state.borrow_mut().profile_setup);
        if setup {
            self.open_profile_editor();
        }
        self.follow_forward();
        let arrived = self.state.borrow_mut().arrival_ready.take();
        if let Some((message, root)) = arrived {
            self.jump_to_message(message, root);
        }
        let thread = self.state.borrow_mut().forward_thread.take();
        if let Some((root, focus)) = thread {
            self.open_thread_at(root, focus);
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
        if std::mem::take(&mut self.state.borrow_mut().clear_code) {
            self.ui.code.set_text("");
        }
        let created = self.state.borrow_mut().open_created.take();
        if let Some(id) = created {
            self.close_start_direct();
            self.close_profile();
            self.open_direct(id);
        }
        self.render_profile();
        self.render_inbox();
        self.render_updates();
        self.poll_whats_new();
        self.render_settings();
        self.render_start_direct();
        self.start_membership_refresh();
        self.render_admin();
        self.receive_voice();
        self.render();
    }

    fn handle(&self, state: &mut State, event: Event) {
        match event {
            Event::Restored { generation, result } if generation == state.generation => {
                match result {
                    Ok(Some((token, account, spaces))) => {
                        self.establish(state, token, account, spaces)
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
                        state.attempts_remaining = None;
                        state.clear_code = true;
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
                state.attempts_remaining = attempts_remaining;
                match result {
                    Ok((token, account, spaces)) => {
                        state.challenge = None;
                        state.signin_status = None;
                        self.worker.send(Command::PersistCredential {
                            generation: state.generation,
                            token: token.clone(),
                        });
                        self.establish(state, token, account, spaces);
                    }
                    Err(error) => state.error = Some(error),
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
            } if generation == state.epoch => state.set_directs(directs),
            // Fetched with the account at sign-in.
            Event::SignedInDirects {
                token,
                result: Ok(directs),
            } if state.token.as_ref() == Some(&token) => state.set_directs(directs),
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
            Event::ActivityLoaded {
                epoch,
                request,
                more,
                result,
            } if epoch == state.epoch => state.inbox.finish(request, more, result),
            Event::ActivitySent {
                epoch,
                send,
                result,
            } if epoch == state.epoch => {
                self.inbox_reply_sent(state, &send, result);
            }
            Event::ActivityReacted {
                epoch,
                request,
                result,
            } if epoch == state.epoch => self.inbox_reacted(state, request, result),
            Event::ProfileStatus {
                epoch,
                person,
                status,
            } if epoch == state.epoch => {
                if let Some(card) = state.card.as_mut().filter(|card| card.person == person) {
                    card.status = Some(status);
                }
            }
            // The update review's report: it stays in the review.
            Event::ProfileSent {
                epoch,
                send,
                result,
                ..
            } if epoch == state.epoch
                && state
                    .update_report
                    .as_ref()
                    .is_some_and(|report| report.client_id == send.client_id) =>
            {
                if let Some(direct) = result.as_ref().ok().or(send.direct.as_ref()) {
                    state.directs.retain(|item| item.id != direct.id);
                    state.directs.push(direct.clone());
                    state.layout += 1;
                }
                if let Some(report) = state.update_report.as_mut() {
                    report.state = result.map(|_| true).map_err(|error| error.message);
                }
            }
            Event::WhatsNew(result) => self.whats_new_loaded(result),
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
                visit: Some(visit),
                result,
                ..
            } if epoch == state.epoch => {
                if matches!(&state.unread_snapshot, Some((navigation, None)) if *navigation == visit)
                {
                    // Its visit takes it once the history is in; after a
                    // failure the visit asks again.
                    state.unread_snapshot = result.ok().map(|unread| (visit, Some(unread)));
                } else if visit == state.navigation && state.unread_visit_pending {
                    state.unread_visit_pending = false;
                    if let (Ok(unread), Some(channel)) = (&result, &state.channel) {
                        state.unread_boundary = unread.boundary(channel);
                    }
                    if state.joined() {
                        self.mark_read(state);
                    }
                }
            }
            Event::UnreadLoaded {
                epoch,
                revision,
                result: Ok(unread),
                ..
            } if epoch == state.epoch && revision == state.unread_revision => {
                let mut reads = HashMap::new();
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
                                reads.insert(channel.id.clone(), (read, last));
                                let count = channel.unread_count.unwrap_or(u64::from(last > read));
                                (channel.id, (count, channel.mention_count.unwrap_or(0)))
                            })
                            .collect();
                        (space.id, channels)
                    })
                    .collect();
                state.reads = reads;
                if let Some(count) = unread.badge_count {
                    state.inbox.unread_count = count;
                }
                state.feedback_waiting = unread.feedback_waiting;
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
                        if let (Some(window), Some(first)) =
                            (&mut state.context_window, history.messages.first())
                        {
                            window.start = first.seq.parse().unwrap_or(window.start);
                        }
                        if let Err(error) = state.timeline.prepend(history.messages) {
                            state.error = Some(error);
                        }
                    }
                    Err(_) => {
                        state.error = Some("Couldn’t load older messages.".into());
                        state.retry = Some(Retry::Older);
                    }
                }
            }
            Event::ThreadUnread {
                scope,
                revision,
                read,
                result,
            } if scope
                == (
                    state.epoch,
                    state.navigation,
                    state.channel.clone().unwrap_or_default(),
                )
                && revision == state.thread_unread_revision =>
            {
                state.thread_unread_busy = false;
                match result {
                    Ok(threads) => {
                        if let Some((root, seq)) = read {
                            if state.thread_unread.get(&root).is_some_and(|unread| {
                                unread.parse::<u64>().unwrap_or(0)
                                    <= seq.parse::<u64>().unwrap_or(0)
                            }) {
                                state.thread_unread.remove(&root);
                            }
                            state.thread_unread_dirty = true;
                        } else {
                            state.thread_unread = threads;
                        }
                    }
                    // Tried again with the next change or visit.
                    Err(_) => state.thread_unread_dirty = true,
                }
            }
            Event::VoiceActivityLoaded { epoch, result } if epoch == state.epoch => {
                state.voice_activity_loading = false;
                // A failed read keeps the last badges.
                if let Ok(spaces) = result {
                    let spaces = spaces.into_iter().collect();
                    if spaces != state.voice_activity {
                        state.voice_activity = spaces;
                        state.layout += 1;
                    }
                }
            }
            Event::SessionCreated { generation, result } if generation == state.generation => {
                match result {
                    Ok(session) => {
                        state.session = Some(session);
                        state.session_error = None;
                    }
                    Err(error) => {
                        state.session_error = Some(error);
                        state.retry = Some(Retry::Session);
                    }
                }
                state.layout += 1;
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
                            thread.has_more = history.has_more;
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
                        } else if let Some(share) = pending.and_then(|pending| pending.share) {
                            // Best effort, once KLIPY's GIF is really sent.
                            share.send();
                        }
                        self.mark_read(state);
                        // A reply can answer a report.
                        self.sent_in_visit(state);
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
                // A voice message waiting its turn goes next.
                self.send_voice_messages(state);
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
            Event::ContextLoaded {
                generation,
                request,
                channel,
                root: None,
                anchor,
                newer,
                result,
            } if generation == state.generation
                && request == state.request
                && state.channel.as_deref() == Some(&channel) =>
            {
                self.accept_context(state, &anchor, newer, result);
                if !newer {
                    self.report_context_loaded(state, &anchor);
                }
            }
            Event::FeedbackLoaded {
                epoch,
                request,
                result,
            } if epoch == state.epoch => state.feedback.finish(request, result),
            // Successful writes leave the other rows and older pages be.
            Event::FeedbackArchived {
                epoch,
                result: Err(error),
            } if epoch == state.epoch => self.feedback_archive_failed(state, error),
            Event::MessageVersions {
                generation,
                request,
                channel,
                message,
                result,
            } if current(
                generation,
                state.generation,
                Some(&channel),
                state.channel.as_deref(),
            ) =>
            {
                self.accept_versions(state, request, &message, result)
            }
            Event::Reactors {
                generation,
                channel,
                message,
                revision,
                result,
            } if current(
                generation,
                state.generation,
                Some(&channel),
                state.channel.as_deref(),
            ) =>
            {
                if let Some(cached) = state.reactors.get_mut(&message) {
                    cached.loaded(&message, &revision, result);
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
                reloaded,
                result,
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
                    // The latest version replaces the draft, and the next
                    // save expects its revision.
                    Ok(snapshot) if ours && reloaded => {
                        let text = snapshot.content.text.clone();
                        state.compose = Some(Compose::Edit {
                            original: snapshot.clone(),
                            request: 0,
                        });
                        state.send_error = None;
                        // Written by `render`, outside this borrow.
                        state.restore_draft = Some(text);
                        if state.timeline.merge_edit_ack(*snapshot).is_err() {
                            self.reload(state);
                        }
                    }
                    Ok(snapshot) => {
                        if ours {
                            state.compose = None;
                            state.returned_draft = None;
                            // Cleared by `render`: the buffer's handlers read
                            // the state.
                            state.restore_draft = Some(String::new());
                        }
                        if state.timeline.merge_edit_ack(*snapshot).is_err() {
                            self.reload(state);
                        }
                    }
                    Err(error) if ours => {
                        state.send_error = Some(error);
                        state.edit_failed = true;
                    }
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
            // A dialog's change answers even after a navigation.
            Event::Admin { generation, result }
                if generation == state.generation || state.admin.busy =>
            {
                self.admin_result(state, result);
            }
            Event::MembershipRefreshed {
                epoch,
                request,
                revision,
                navigation,
                result,
            } if epoch == state.epoch => {
                self.membership_refreshed(state, request, revision, navigation, result);
            }
            Event::Klipy { token, config } if state.token.as_ref() == Some(&token) => {
                state.klipy = config;
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
            Event::Gateway(event) => {
                if let Some(event) = self.voice_gateway(state, event) {
                    self.gateway(state, event);
                }
            }
            Event::MediaStatus {
                generation,
                root,
                enabled,
            } if generation == state.epoch => self.media_status(state, root, enabled),
            Event::VoiceChecked {
                request,
                voice_generation,
                space,
                channel,
                result,
            } => self.voice_checked(state, request, voice_generation, &space, &channel, result),
            _ => {}
        }
    }

    fn navigation_prepared(
        &self,
        state: &mut State,
        result: Result<caper_client::worker::PreparedNavigation, caper_client::worker::LoadError>,
    ) {
        // `navigate` drops a visit for any other conversation.
        let visit_request = state.visit_request.take();
        let report_focus = state
            .report_focus
            .take()
            .filter(|(navigation, _)| *navigation == state.navigation)
            .map(|(_, target)| target);
        match result {
            Ok(prepared) => {
                if let Some((history, _)) = &prepared.conversation
                    && !self.visit_prepared(state, visit_request.clone(), history)
                {
                    return;
                }
                if let Some(detail) = prepared.detail {
                    state.detail = Some(detail);
                }
                match prepared.conversation {
                    Some((history, session)) => {
                        state.channel = Some(history.channel.id.clone());
                        if state.direct.is_none() || state.channel_name.is_empty() {
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
                            Err(error) => {
                                state.session_error = Some(error);
                                state.retry = Some(Retry::Session);
                            }
                        }
                        self.follow.set(true);
                        self.connect_gateway(state, history.channel.id);
                        self.worker.showing(state.channel.as_deref());
                        let snapshot = state.unread_snapshot.take();
                        if state.direct.is_none()
                            && let Some(token) = state.token.clone()
                        {
                            state.unread_visit_pending = true;
                            match snapshot {
                                // Asked for beside the history.
                                Some((navigation, Some(unread)))
                                    if navigation == state.navigation =>
                                {
                                    state.unread_boundary = state
                                        .channel
                                        .as_deref()
                                        .and_then(|channel| unread.boundary(channel));
                                    state.unread_visit_pending = false;
                                }
                                // Still on its way: it lands through the visit fence.
                                Some((navigation, None)) if navigation == state.navigation => {}
                                _ => self.worker.send(Command::LoadUnread {
                                    epoch: state.epoch,
                                    revision: state.unread_revision,
                                    visit: Some(state.navigation),
                                    token,
                                }),
                            }
                        }
                        if state.joined() {
                            self.mark_read(state);
                        }
                        self.clear_open_unread(state);
                        if let Some((navigation, channel, message, root)) = state.arrival.take()
                            && navigation == state.navigation
                            && state.channel.as_deref() == Some(&channel)
                        {
                            state.arrival_ready = Some((message, root));
                        }
                        if let Some((navigation, channel, root, focus)) = state.forward_focus.take()
                            && navigation == state.navigation
                            && state.channel.as_deref() == Some(&channel)
                        {
                            state.forward_thread = Some((root, focus));
                        }
                        if let Some(target) = report_focus
                            .or(visit_request.map(|request| request.target))
                            .filter(|target| state.channel.as_ref() == Some(&target.channel))
                        {
                            self.focus_report(state, &target);
                        }
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
                // A report that can't open shows why, not the inbox.
                if visit_request.is_some() {
                    state.feedback_open = false;
                }
                // The last conversation's messages don't stay under the new title.
                state.timeline = Timeline::default();
                state.markup.clear();
                state.status.clear();
                state.error = Some(error.message);
                state.retry = Some(Retry::Reload);
                state.layout += 1;
            }
        }
    }

    /// Browse channels: the space's channels outside your sidebar, searched
    /// by name. Choosing one opens its preview with Join.
    fn open_browse(self: &Rc<Self>) {
        let popover = gtk::Popover::new();
        popover.add_css_class("browse");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 6);
        content.set_size_request(280, -1);
        let search: gtk::SearchEntry = named("GtkSearchEntry", "Search channels");
        search.set_placeholder_text(Some("Search channels"));
        content.append(&search);
        let list: gtk::ListBox = named("GtkListBox", "Channels to browse");
        list.add_css_class("channels");
        content.append(&list);
        let empty = label("", &["browse-empty"]);
        empty.set_xalign(0.0);
        empty.set_wrap(true);
        content.append(&empty);
        popover.set_child(Some(&content));
        let ids: Rc<RefCell<Vec<String>>> = Rc::default();
        let fill = {
            let (app, list, empty, ids) = (self.clone(), list.clone(), empty.clone(), ids.clone());
            move |query: &str| {
                let state = app.state.borrow();
                // People often type the name as shown, with its `#`.
                let query = query.trim().trim_start_matches('#').trim().to_lowercase();
                let channels = state
                    .detail
                    .as_ref()
                    .map_or(&[][..], |detail| &detail.channels);
                clear_list(&list);
                let mut found = Vec::new();
                for channel in channels.iter().filter(|channel| {
                    !channel.joined && channel.name.to_lowercase().contains(&query)
                }) {
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    line.append(&app.images.icon_image(
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
                    line.append(&name);
                    list.append(&line);
                    found.push(channel.id.clone());
                }
                let message = if !found.is_empty() {
                    ""
                } else if !query.is_empty() {
                    "No channels match your search."
                } else if !channels.is_empty() {
                    "You’ve joined every channel in this space."
                } else {
                    "This space has no channels yet."
                };
                empty.set_label(message);
                empty.set_visible(!message.is_empty());
                list.set_visible(!found.is_empty());
                *ids.borrow_mut() = found;
            }
        };
        fill("");
        search.connect_search_changed(move |entry| fill(&entry.text()));
        let (app, popover_ref) = (self.clone(), popover.downgrade());
        list.connect_row_activated(move |_, row| {
            let id = usize::try_from(row.index())
                .ok()
                .and_then(|index| ids.borrow().get(index).cloned());
            if let Some(popover) = popover_ref.upgrade() {
                popover.popdown();
            }
            if let Some(id) = id {
                let app = app.clone();
                glib::idle_add_local_once(move || {
                    {
                        let mut state = app.state.borrow_mut();
                        let space = state.space.clone();
                        app.navigate(&mut state, space, Some(id));
                    }
                    app.render();
                });
            }
        });
        // Enter opens the first match.
        let list_ref = list.downgrade();
        search.connect_activate(move |_| {
            if let Some(row) = list_ref.upgrade().and_then(|list| list.row_at_index(0)) {
                row.emit_activate();
            }
        });
        let popover_ref = popover.downgrade();
        search.connect_stop_search(move |_| {
            if let Some(popover) = popover_ref.upgrade() {
                popover.popdown();
            }
        });
        popup(&popover, self.ui.space_header.upcast_ref());
        search.grab_focus();
    }

    /// Tracks the main draft's first link: asks for its card once the link
    /// settles, then shows it above the composer.
    fn update_link_preview(self: &Rc<Self>) {
        let draft = self.ui.composer.text();
        let token = self.state.borrow().token.clone();
        let (url, wait) =
            self.link_preview
                .borrow_mut()
                .due(&draft, token.is_some(), Instant::now());
        if let (Some(url), Some(token)) = (url, token) {
            let api = self.api.clone();
            let (sender, receiver) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let embed = api.embed_preview(&token, &url);
                let _ = sender.send_blocking((url, embed));
            });
            let weak = Rc::downgrade(self);
            glib::spawn_future_local(async move {
                if let (Ok((url, embed)), Some(app)) = (receiver.recv().await, weak.upgrade()) {
                    app.link_preview.borrow_mut().answer(url, embed);
                    app.render_link_preview();
                }
            });
        } else if let Some(wait) = wait {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local_once(wait, move || {
                if let Some(app) = weak.upgrade() {
                    app.update_link_preview();
                }
            });
        }
        self.render_link_preview();
    }

    fn render_link_preview(self: &Rc<Self>) {
        let preview = &self.ui.composer.preview;
        let card = self.link_preview.borrow().card().cloned();
        let url = card.as_ref().map(|card| card.url.clone());
        if *self.shown_preview.borrow() == url {
            return;
        }
        *self.shown_preview.borrow_mut() = url;
        clear(preview);
        preview.set_visible(card.is_some());
        let Some(card) = card else {
            return;
        };
        let weak = Rc::downgrade(self);
        let handler: rows::Handler = Rc::new(move |action| {
            if let Some(app) = weak.upgrade() {
                app.row_action(action, false);
            }
        });
        let content = crate::content::Content {
            embeds: vec![card],
            ..Default::default()
        };
        if let Some(view) = crate::content::view(&self.images, &content, &handler) {
            preview.append(&view);
        }
        let remove: gtk::Button = named("GtkButton", "Remove link preview");
        remove.set_child(Some(&self.images.icon_image(Icon::Close, "#b9bcbe", 14)));
        remove.set_tooltip_text(Some("Remove link preview"));
        remove.add_css_class("flat");
        remove.set_valign(gtk::Align::Start);
        let weak = Rc::downgrade(self);
        remove.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.link_preview.borrow_mut().remove();
                // The button goes with the card it belongs to.
                glib::idle_add_local_once(move || app.render_link_preview());
            }
        });
        preview.append(&remove);
    }

    /// The desktop's empty conversation and empty channel stages.
    fn render_empty(&self, state: &State) {
        let ui = &self.ui;
        // "Loading…" until the conversation arrives; then "Connecting…" or "Live".
        let settled = state.status != "Loading…" && state.error.is_none();
        let (title, note, stage) = if state.channel.is_none() {
            if !settled || state.space.is_some() && state.detail.is_none() {
                (String::new(), String::new(), false)
            } else if state.space.is_none() {
                (
                    "Select a direct message".to_owned(),
                    "Open a conversation from Direct messages.".to_owned(),
                    true,
                )
            } else {
                let note = if state.owner() {
                    "Create a channel to start a conversation."
                } else {
                    "Browse public channels or accept a private invitation."
                };
                ("No joined channels".to_owned(), note.to_owned(), true)
            }
        } else if settled
            && state.pending.is_none()
            && !state
                .timeline
                .messages()
                .any(|message| message.is_channel_message())
        {
            let direct = state
                .direct
                .as_ref()
                .and_then(|id| state.directs.iter().find(|direct| &direct.id == id));
            let me = state.account.as_ref().map(|account| account.id.as_str());
            match direct {
                Some(direct) if Some(direct.peer.id.as_str()) == me => (
                    String::new(),
                    "You can message yourself here to keep notes, reminders, and ideas.".to_owned(),
                    false,
                ),
                Some(direct) if direct.peer.team => (
                    "No messages yet.".to_owned(),
                    "Messages here go to the Caper team.".to_owned(),
                    false,
                ),
                Some(direct) => (
                    "No messages yet.".to_owned(),
                    format!(
                        "Only you and {} can read this conversation.",
                        direct.peer.display_name
                    ),
                    false,
                ),
                None if state.direct.is_some() => (
                    "No messages yet.".to_owned(),
                    "Only you and this person can read this conversation.".to_owned(),
                    false,
                ),
                None => (
                    "No messages yet.".to_owned(),
                    format!("Start the conversation in #{}.", state.channel_name),
                    false,
                ),
            }
        } else {
            (String::new(), String::new(), false)
        };
        ui.empty.set_visible(!title.is_empty() || !note.is_empty());
        ui.empty_title.set_label(&title);
        ui.empty_title.set_visible(!title.is_empty());
        ui.empty_note.set_label(&note);
        ui.empty_icon.set_visible(stage);
        if stage {
            ui.empty.add_css_class("stage");
        } else {
            ui.empty.remove_css_class("stage");
        }
        let create = stage && state.space.is_some() && state.owner();
        ui.empty_action.set_visible(create);
        ui.empty_action.set_sensitive(state.can_create_channel());
        ui.empty_action
            .set_tooltip_text(Some(&state.create_channel_tooltip()));
    }

    /// The error banner's button.
    fn retry(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let Some(retry) = state.retry.take() else {
                return;
            };
            state.error = None;
            match retry {
                Retry::Reload => self.reload(&mut state),
                Retry::Older => {}
                Retry::Session => {
                    let name = state
                        .account
                        .as_ref()
                        .and_then(|account| account.display_name.clone())
                        .filter(|name| !name.trim().is_empty())
                        .unwrap_or_else(|| "Guest".into());
                    state.session_error = None;
                    self.worker.send(Command::CreateSession {
                        generation: state.generation,
                        token: state.token.clone(),
                        name,
                    });
                }
            }
            if retry == Retry::Older {
                drop(state);
                self.load_older();
            }
        }
        self.render();
    }

    /// The open conversation shows no unread badge while you read it.
    fn clear_open_unread(&self, state: &mut State) {
        // What you read here no longer counts as new when you come back.
        if let Some(channel) = &state.channel
            && let Some((read, last)) = state.reads.get_mut(channel)
        {
            *read = (*read).max(*last);
        }
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
                    // Best effort, once KLIPY's GIF is really sent.
                    if let Some(share) = state.pending.take().and_then(|pending| pending.share) {
                        share.send();
                    }
                    // A reply can answer a report.
                    self.sent_in_visit(state);
                }
                // Their message ends their typing.
                state.typers.remove(&message.author.id);
                // Others' messages play the message sound; blocked authors don't.
                let remote = state
                    .session
                    .as_ref()
                    .is_some_and(|session| session.author.id != message.author.id)
                    && !state.blocked(&message.author.id)
                    && state.open_request().is_none();
                if message.thread_root_id.is_some() {
                    state.thread_unread_dirty = true;
                }
                let applied = state.timeline.apply(*message);
                if matches!(applied, Ok(Apply::Applied)) {
                    if remote {
                        self.play(caper_client::effects::Effect::Message);
                    }
                    if self.follow.get() {
                        self.mark_read(state);
                    }
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
        let draft = self
            .state
            .try_borrow_mut()
            .ok()
            .and_then(|mut state| state.restore_draft.take());
        if let Some(draft) = draft {
            self.restoring.set(true);
            self.ui.composer.view.buffer().set_text(&draft);
            self.restoring.set(false);
        }
        self.render_rows();
        self.render_voice();
        self.render_layout();
        self.render_files();
        self.render_members();
        self.render_typing();
        self.render_reactors();
        self.render_history();
        self.render_recording();
        let ui = &self.ui;
        let mut state = self.state.borrow_mut();
        let first_space = state.page() == Page::Chat && state.needs_first_space();
        let page = match state.page() {
            Page::Loading => "loading",
            Page::SignIn => "signin",
            Page::Chat if first_space => "first-space",
            Page::Chat => "chat",
        };
        let step = if state.challenge.is_some() {
            "code"
        } else {
            "email"
        };
        // As desktop: arriving at a step puts the caret in its field.
        let arrived = page == "signin"
            && (ui.pages.visible_child_name().as_deref() != Some(page)
                || ui.steps.visible_child_name().as_deref() != Some(step));
        ui.pages.set_visible_child_name(page);
        if first_space {
            self.render_first_space(&state);
        }
        ui.steps.set_visible_child_name(step);
        if arrived {
            if state.challenge.is_some() {
                ui.code.grab_focus();
            } else {
                ui.email.grab_focus();
            }
        }
        ui.code_hint.set_label(&format!(
            "Enter the six-character code sent to {}. It expires in 10 minutes.",
            state.email
        ));
        ui.signin_actions
            .set_visible_child_name(if state.challenge.is_some() {
                "code"
            } else {
                "email"
            });
        // As desktop: each action waits for something it can send.
        ui.email_button.set(
            if state.busy {
                "Sending…"
            } else {
                "Email me a code"
            },
            !state.busy && ui.email.text().contains('@'),
        );
        let exhausted = state.attempts_remaining == Some(0);
        let resends_left = state.resends < MAX_RESENDS;
        ui.code.set_sensitive(!exhausted);
        if exhausted {
            // Counts as a resend; at the limit the API would send nothing.
            ui.code_button.button.set_visible(resends_left);
            ui.code_button.set(
                if state.busy {
                    "Sending…"
                } else {
                    "Email me a new code"
                },
                !state.busy,
            );
        } else {
            let mut code = ui.code.text().to_string();
            caper_client::sign_in::clean_sign_in_code(&mut code);
            ui.code_button.button.set_visible(true);
            ui.code_button.set(
                if state.busy && !state.resending {
                    "Checking…"
                } else {
                    "Continue"
                },
                !state.busy && code.len() == 6,
            );
        }
        ui.last_attempt
            .set_visible(state.challenge.is_some() && state.attempts_remaining == Some(1));
        ui.spam_hint.set_visible(!resends_left);
        self.render_resend(&state);
        let error = state.error.as_deref().unwrap_or_default();
        ui.signin_error.set_label(error);
        ui.signin_error.set_visible(!error.is_empty());
        // Desktop shows one of the two: the error, else the status.
        let status = state
            .signin_status
            .filter(|_| error.is_empty() && state.challenge.is_some())
            .unwrap_or_default();
        ui.signin_status.set_label(status);
        ui.signin_status.set_visible(!status.is_empty());
        // A missing session shows in the banner when nothing else does. A
        // preview has none by design: its Join bar says so.
        let session = (error.is_empty() && state.retry == Some(Retry::Session) && state.joined())
            .then(|| state.session_error.clone())
            .flatten()
            .unwrap_or_default();
        let shown = if error.is_empty() { &session } else { error };
        ui.error.set_label(shown);
        ui.error.set_visible(!shown.is_empty());
        let retry = state.retry.filter(|_| !shown.is_empty());
        ui.error_retry.set_visible(retry.is_some());
        ui.error_retry.set_label(match retry {
            Some(Retry::Reload) => "Try again",
            Some(Retry::Session) => "Retry session",
            _ => "Retry",
        });

        // A visit is titled "#plans · Studio", or "Alice & Bob" for a DM.
        let visit = state.visiting();
        let direct = state.direct.is_some() && visit.is_none_or(|visit| visit.direct);
        let person = state
            .direct
            .as_ref()
            .filter(|_| visit.is_none())
            .and_then(|id| state.directs.iter().find(|item| &item.id == id));
        if let Some(person) = person {
            ui.channel_avatar.set(&self.images, &peer_avatar(person));
        }
        ui.channel_avatar.root.set_visible(person.is_some());
        ui.channel_title.set_label(&if let Some(visit) = visit {
            visit.title.clone()
        } else if state.channel_name.is_empty() {
            String::new()
        } else if direct {
            state.channel_name.clone()
        } else {
            format!("# {}", state.channel_name)
        });
        // As desktop: a live connection says nothing; only its absence shows.
        ui.status.set_label(if state.status == "Live" {
            ""
        } else {
            &state.status
        });
        // DMs are named after the other person, without a channel `#`.
        let destination = if direct {
            state.channel_name.clone()
        } else {
            format!("#{}", state.channel_name)
        };
        if let Some(label) = ui
            .thread_composer
            .broadcast
            .child()
            .and_downcast::<gtk::Label>()
        {
            label.set_markup(&format!(
                "Also send to <b>{}</b>",
                glib::markup_escape_text(&destination)
            ));
        }
        let narrow = self.is_narrow();
        ui.thread_close.set_visible(!narrow);
        ui.thread_back[0].set_visible(narrow && !direct);
        ui.thread_back[1].set_visible(narrow && direct);
        ui.thread_place.set_label(&format!(
            "{} {destination}",
            if direct { "with" } else { "in" }
        ));
        self.render_empty(&state);
        self.render_context_bar(&state);
        self.render_gifs(&state);
        self.place_thread_mic();
        let joined = state.joined();
        let can_send = state.session.is_some();
        let replaced = self.render_request_bar(&state);
        ui.composer
            .root
            .set_visible(joined && state.channel.is_some());
        ui.composer.view.set_sensitive(can_send);
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
        ui.join_bar.set_visible(
            !joined && !replaced && state.channel.is_some() && state.page() == Page::Chat,
        );
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
            None if direct => format!("Message {}", state.channel_name),
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
        ui.compose_reload.set_visible(
            state.edit_failed
                && state.send_error.is_some()
                && matches!(state.compose, Some(Compose::Edit { .. })),
        );
        // The feedback inbox takes the conversation's place, and its panes.
        let thread_open = state.thread.is_some() && !state.feedback_open;
        ui.thread_panel.set_visible(thread_open);
        ui.members_panel.set_visible(
            !state.feedback_open && ui.members_toggle.is_active() && ui.members_toggle.is_visible(),
        );
        ui.thread_composer.view.set_sensitive(can_send && joined);
        if let Some(send) = &ui.thread_composer.send {
            send.set_sensitive(can_send && joined);
        }
        ui.thread_composer.placeholder.set_label("Reply to thread…");
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
        let returned = state.returned_draft.take();
        let shown = (state.channel.is_some() || state.feedback_open, thread_open);
        self.render_feedback(&state);
        drop(state);
        if let Some((draft, thread)) = returned {
            let buffer = if thread {
                ui.thread_composer.view.buffer()
            } else {
                ui.composer.view.buffer()
            };
            if buffer.char_count() == 0 {
                buffer.set_text(&draft);
            }
        }
        self.render_narrow(shown);
    }

    fn render_resend(self: &Rc<Self>, state: &State) {
        let ui = &self.ui;
        ui.resend
            .set_visible(state.resends < MAX_RESENDS && state.attempts_remaining != Some(0));
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
                thread_unread: &state.thread_unread,
                reactors: &state.reactors,
                expanded_quotes: &state.expanded_quotes,
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
                                .filter_map(|file| file.send_attachment())
                                .map(|attachment| (attachment, false))
                                .collect(),
                            gif: pending.gif.clone(),
                            ..Default::default()
                        },
                    })
            };
            let (window, has_newer) = (state.context_window, state.has_newer);
            let main = rows::build(
                state.timeline.messages().filter(|message| {
                    message.is_channel_message()
                        && window.is_none_or(|window| window.contains(message))
                }),
                // A pending send shows at the live end only.
                draft(None).filter(|_| !has_newer),
                &mut state.markup,
                &viewer(false),
            );
            let mut main = main;
            // Hidden while older history could still hold earlier unread messages.
            if let Some((read, last)) = state.unread_boundary.filter(|_| state.direct.is_none())
                && (!state.has_more
                    || state.timeline.messages().next().is_some_and(|message| {
                        message.seq.parse::<u64>().is_ok_and(|seq| seq <= read + 1)
                    }))
                && let Some(first) = state
                    .timeline
                    .messages()
                    .filter(|message| message.is_channel_message())
                    .find(|message| {
                        message
                            .seq
                            .parse::<u64>()
                            .is_ok_and(|seq| seq > read && seq <= last)
                            && author.as_deref() != Some(message.author.id.as_str())
                            && me.as_deref() != Some(message.author.id.as_str())
                            && !blocked.contains(&message.author.id)
                    })
                && let Some(row) = main.iter_mut().find(|row| row.id == first.id)
            {
                row.unread = true;
            }
            if !state.has_more
                && let Some(first) = main.first_mut()
            {
                first.beginning = true;
            }
            // A jump's target stays highlighted until you move on.
            if let Some(focused) = &state.focused {
                for row in &mut main {
                    row.focused = row.id == *focused;
                }
            }
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
            let mut thread = thread;
            if let Some(focused) = &state.focused {
                for row in &mut thread {
                    row.focused = row.id == *focused;
                }
            }
            // As desktop, an empty thread invites the first reply under its
            // root.
            let answerable =
                state.joined() && state.open_request().is_none() && !state.open_blocked();
            if state.thread.as_ref().is_some_and(|thread| !thread.loading)
                && let [root] = thread.as_mut_slice()
            {
                root.footer = Some(
                    if answerable {
                        "No replies yet. Start the thread."
                    } else {
                        "No replies yet."
                    }
                    .into(),
                );
            }
            let thread_changed =
                rows::apply(&self.ui.thread_list.store, &state.thread_rows, &thread);
            state.thread_rows = thread;
            (changed, thread_changed)
        };
        if changed && self.follow.get() {
            self.pin_tail();
        }
        // A reply to focus scrolls into view once its page is here;
        // otherwise the thread follows its newest reply.
        let (focus, more) = {
            let state = self.state.borrow();
            match state.thread.as_ref() {
                Some(thread) if !thread.loading => (
                    thread
                        .focus
                        .as_ref()
                        .map(|focus| state.thread_rows.iter().position(|row| &row.id == focus)),
                    thread.has_more,
                ),
                _ => (None, false),
            }
        };
        match focus {
            Some(Some(position)) => {
                if let Some(thread) = &mut self.state.borrow_mut().thread {
                    thread.focus = None;
                }
                let _ = self.ui.thread_list.view.activate_action(
                    "list.scroll-to-item",
                    Some(&u32::try_from(position).unwrap_or_default().to_variant()),
                );
            }
            Some(None) if more => {
                let app = self.clone();
                glib::idle_add_local_once(move || app.load_older_replies());
            }
            Some(None) => {
                if let Some(thread) = &mut self.state.borrow_mut().thread {
                    thread.focus = None;
                }
            }
            None => {
                let count = self.ui.thread_list.store.n_items();
                if thread_changed && count > 0 && self.thread_follow.get() {
                    let _ = self
                        .ui
                        .thread_list
                        .view
                        .activate_action("list.scroll-to-item", Some(&(count - 1).to_variant()));
                }
            }
        }
    }

    /// The thread's next older page of replies.
    fn load_older_replies(self: &Rc<Self>) {
        let Ok(mut state) = self.state.try_borrow_mut() else {
            return;
        };
        let (generation, token) = (state.generation, state.token.clone());
        let Some(channel) = state.channel.clone() else {
            return;
        };
        state.request += 1;
        let request = state.request;
        let Some(root) = state.thread.as_ref().map(|thread| thread.root.clone()) else {
            return;
        };
        // Pages before the oldest reply loaded, as the main list does.
        let Some(before) = state
            .timeline
            .messages()
            .find(|message| message.thread_root_id.as_deref() == Some(root.as_str()))
            .map(|message| message.seq.clone())
        else {
            return;
        };
        let Some(thread) = state.thread.as_mut() else {
            return;
        };
        if thread.loading || !thread.has_more {
            return;
        }
        thread.loading = true;
        thread.request = request;
        self.worker.send(Command::LoadThread {
            generation,
            request,
            token,
            channel,
            root: thread.root.clone(),
            before: Some(before),
        });
    }

    /// A muted row's bell-slash, its tooltip saying until when.
    fn muted_icon(&self, label: &str) -> gtk::Image {
        let icon = self.images.icon_image(Icon::BellOff, "#b9bcbe", 13);
        icon.set_tooltip_text(Some(label));
        icon
    }

    fn render_layout(self: &Rc<Self>) {
        let state = self.state.borrow();
        if self.rendered_layout.replace(state.layout) == state.layout {
            return;
        }
        let expiring = state
            .settings
            .notifications
            .next_expiry(chrono::Utc::now())
            .is_some();
        if expiring {
            let app = self.clone();
            glib::idle_add_local_once(move || app.schedule_mute_expiry());
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
            let connected = {
                let voice = self.voice.borrow();
                voice.active_space.as_deref() == Some(&space.id)
                    && matches!(voice.state.phase, caper_client::state::Phase::Connected(_))
            };
            let voice =
                (connected || state.voice_activity.contains(&space.id)).then_some(if connected {
                    "Connected to voice in this space"
                } else {
                    "Voice active in this space"
                });
            let mut name = space.name.clone();
            if let Some(voice) = voice {
                name.push_str(&format!(", {}", voice.to_lowercase()));
            }
            if space_unread(&space.id) {
                name.push_str(" (unread)");
            }
            let button: gtk::Button = named("GtkButton", &name);
            // A ready icon fills the tile; the initials show while it loads
            // or when it fails.
            let icon = space.icon().map(|icon| crate::images::Avatar {
                name: space.name.clone(),
                mascot: None,
                picture: Some((
                    icon.asset.to_owned(),
                    icon.url.unwrap_or_default().to_owned(),
                )),
            });
            match icon.filter(|icon| self.images.avatar(icon).is_some()) {
                Some(icon) => {
                    let view = crate::images::AvatarView::new(40, false);
                    view.root.add_css_class("space-icon");
                    view.set(&self.images, &icon);
                    button.set_child(Some(&view.root));
                    button.add_css_class("icon");
                }
                None => button.set_label(&caper_client::space_initial(&space.name)),
            }
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
            if let Some(voice) = voice {
                let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                badge.add_css_class("rail-voice");
                if connected {
                    badge.add_css_class("connected");
                }
                badge.append(&self.images.icon_image(Icon::Volume, "#f3f4f5", 12));
                badge.set_halign(gtk::Align::End);
                badge.set_valign(gtk::Align::Start);
                badge.set_can_target(false);
                overlay.add_overlay(&badge);
                button.set_tooltip_text(Some(&format!("{}\n{voice}", space.name)));
            }
            let app = self.clone();
            let id = space.id.clone();
            button.connect_clicked(move |_| app.select_space(id.clone()));
            self.scope_menu_on(&button, Scope::Space(space.id.clone()));
            ui.rail.append(&overlay);
        }
        self.render_admin_sidebar(&state);

        let detail = state.detail.as_ref();
        ui.space_name
            .set_label(detail.map_or("", |detail| detail.space.name.as_str()));
        clear_list(&ui.channels);
        self.room_timers.borrow_mut().clear();
        let mut ids = Vec::new();
        let unread = detail.and_then(|detail| state.unread.get(&detail.space.id));
        let joined = detail.map_or(0, |detail| {
            detail
                .channels
                .iter()
                .filter(|channel| channel.joined)
                .count()
        });
        ui.channels_count.set_label(&joined.to_string());
        if state.channels_collapsed {
            ui.channels_toggle.add_css_class("collapsed");
        } else {
            ui.channels_toggle.remove_css_class("collapsed");
        }
        for channel in detail.map_or(&[][..], |detail| &detail.channels) {
            let open = state.direct.is_none() && state.channel.as_deref() == Some(&channel.id);
            let roster = self.channel_voice(&state, &channel.id);
            // As web's `joinedChannels`: Browse finds the rest. A channel
            // you're previewing stays while it's open.
            if !channel.joined && !open || state.channels_collapsed && roster.is_none() {
                continue;
            }
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
                // As desktop: the open channel's mark is terracotta.
                if open { "#db6849" } else { "#b9bcbe" },
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
            let scope = Scope::Channel {
                space: detail
                    .map(|detail| detail.space.id.clone())
                    .unwrap_or_default(),
                channel: channel.id.clone(),
            };
            // What the row says to assistive technologies: what it shows.
            let mut spoken = vec![channel.name.clone()];
            if channel.private {
                spoken.push("private".into());
            }
            if !channel.joined {
                spoken.push("not joined".into());
            }
            if let Some(muted) = state.muted_label(&scope) {
                name.add_css_class("muted");
                line.append(&self.muted_icon(&muted));
                spoken.push("muted".into());
            }
            if count > 0 {
                line.append(&badge(count, mentions > 0));
                spoken.push(format!("{count} unread"));
                if mentions > 0 {
                    spoken.push("mentions you".into());
                }
            }
            if let Some((_, people, _)) = &roster {
                spoken.push(format!("{people} in voice"));
            }
            // As desktop: a live room you're not in says how many are there.
            if let Some((_, people, false)) = &roster {
                let live = gtk::Box::new(gtk::Orientation::Horizontal, 4);
                live.add_css_class("room-live");
                live.append(&self.images.icon_image(Icon::AudioLines, "#4aa86b", 14));
                live.append(&label(&people.to_string(), &["room-live-count"]));
                live.set_tooltip_text(Some(&format!("{people} in voice")));
                line.append(&live);
            }
            self.scope_menu_on(&line, scope);
            let row: gtk::ListBoxRow = named("GtkListBoxRow", &spoken.join(", "));
            match roster {
                Some((roster, _, _)) => {
                    let entry = gtk::Box::new(gtk::Orientation::Vertical, 4);
                    entry.add_css_class("room-card");
                    entry.append(&line);
                    entry.append(&roster);
                    row.set_child(Some(&entry));
                }
                None => row.set_child(Some(&line)),
            }
            ui.channels.append(&row);
            if open && let Some(row) = ui.channels.row_at_index(ids.len() as i32) {
                ui.channels.select_row(Some(&row));
            }
            ids.push(channel.id.clone());
        }
        if state.direct.is_some() {
            ui.channels.unselect_all();
        }
        *self.channel_ids.borrow_mut() = ids;

        self.render_request_navigation(&state);
        clear_list(&ui.directs);
        let mut ids = Vec::new();
        let me = state.account.as_ref().map(|account| account.id.as_str());
        // As desktop: your own notes come first, whether or not that DM
        // exists yet (choosing it creates it).
        if let Some(account) = &state.account {
            let own = state
                .directs
                .iter()
                .find(|direct| Some(direct.peer.id.as_str()) == me);
            let display = account
                .display_name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .or_else(|| account.username.clone())
                .unwrap_or_else(|| "You".into());
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let avatar = AvatarView::new(22, false);
            avatar.set(
                &self.images,
                &Avatar::new(
                    &display,
                    account.avatar_id,
                    &account.avatar_asset,
                    &account.avatar_url,
                ),
            );
            line.append(&avatar.root);
            let name = label(&display, &[]);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let unread = own.is_some_and(|direct| direct.has_unread());
            if unread {
                name.add_css_class("unread");
            }
            line.append(&name);
            let you = label("you", &["muted", "direct-suffix"]);
            you.set_hexpand(true);
            you.set_xalign(0.0);
            line.append(&you);
            if unread {
                let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                dot.add_css_class("self-unread");
                dot.set_valign(gtk::Align::Center);
                line.append(&dot);
            }
            let row: gtk::ListBoxRow = named("GtkListBoxRow", &format!("{display} you"));
            row.set_child(Some(&line));
            ui.directs.append(&row);
            match own {
                Some(own) => {
                    if state.direct.as_deref() == Some(&own.id) {
                        ui.directs.select_row(Some(&row));
                    }
                    ids.push(own.id.clone());
                }
                None => ids.push(SELF_NOTES.into()),
            }
        }
        // Requests live under "Message requests".
        for direct in state.directs.iter().filter(|direct| {
            direct.status != DirectStatus::Incoming && Some(direct.peer.id.as_str()) != me
        }) {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let avatar = AvatarView::new(22, false);
            avatar.set(&self.images, &peer_avatar(direct));
            line.append(&avatar.root);
            let name = label(&direct.peer.display_name, &[]);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            name.set_hexpand(true);
            let blocked = direct.blocked || state.blocked(&direct.peer.id);
            let count = direct.unread_count.unwrap_or(0) * u64::from(!blocked);
            if count > 0 {
                name.add_css_class("unread");
            }
            line.append(&name);
            // As on web: you blocked them, or they haven't accepted yet.
            let suffix = if blocked {
                Some("Blocked")
            } else if direct.status == DirectStatus::Outgoing {
                Some("Request sent")
            } else {
                None
            };
            if let Some(suffix) = suffix {
                line.append(&label(suffix, &["muted", "direct-suffix"]));
            }
            let mut spoken = vec![direct.peer.display_name.clone()];
            spoken.extend(suffix.map(str::to_owned));
            let scope = Scope::Direct(direct.id.clone());
            if let Some(muted) = state.muted_label(&scope) {
                name.add_css_class("muted");
                line.append(&self.muted_icon(&muted));
                spoken.push("muted".into());
            }
            if count > 0 {
                line.append(&badge(count, true));
                spoken.push(format!("{count} unread"));
            }
            self.scope_menu_on(&line, scope);
            let row: gtk::ListBoxRow = named("GtkListBoxRow", &spoken.join(", "));
            row.set_child(Some(&line));
            ui.directs.append(&row);
            if state.direct.as_deref() == Some(&direct.id)
                && let Some(row) = ui.directs.row_at_index(ids.len() as i32)
            {
                ui.directs.select_row(Some(&row));
            }
            ids.push(direct.id.clone());
        }
        // As desktop: a quiet "New message" row ends the list.
        if state.account.is_some() {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.append(&self.images.icon_image(Icon::Plus, "#b9bcbe", 14));
            line.append(&label("New message", &["muted"]));
            // An action, not a place: choosing it keeps the open DM selected.
            let row: gtk::ListBoxRow = named("GtkListBoxRow", "New message");
            row.set_selectable(false);
            row.set_child(Some(&line));
            ui.directs.append(&row);
            ids.push(NEW_MESSAGE.into());
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
/// Escape closes a dialog window, as desktop's modals, unless the window
/// can't be closed (a required first-run step).
pub fn close_on_escape(window: &gtk::Window) {
    let keys = gtk::EventControllerKey::new();
    let weak = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape
            && let Some(window) = weak.upgrade()
            && window.is_deletable()
        {
            window.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
}

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
    // Escape gives focus back to what had it (the row or button that opened
    // the popover). Left to GTK it lands on the chat page's first control,
    // the rail, because the popover's parent is the page.
    let previous = anchor
        .root()
        .and_then(|root| root.focus())
        .map(|widget| widget.downgrade());
    let stale: Vec<gtk::EventController> = popover
        .observe_controllers()
        .iter::<glib::Object>()
        .flatten()
        .filter_map(|object| object.downcast::<gtk::EventController>().ok())
        .filter(|controller| controller.name().as_deref() == Some("restore-focus"))
        .collect();
    for controller in stale {
        popover.remove_controller(&controller);
    }
    // Only notes the key: a field inside may still take Escape for itself.
    let escaped = Rc::new(std::cell::Cell::new(false));
    let keys = gtk::EventControllerKey::new();
    keys.set_name(Some("restore-focus"));
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let pressed = escaped.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        pressed.set(key == gdk::Key::Escape);
        glib::Propagation::Proceed
    });
    popover.add_controller(keys);
    let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
    let once = handler.clone();
    let id = popover.connect_closed(move |popover| {
        if let Some(id) = once.borrow_mut().take() {
            popover.disconnect(id);
        }
        if let Some(widget) = previous
            .as_ref()
            .and_then(|widget| widget.upgrade())
            .filter(|_| escaped.get())
        {
            glib::idle_add_local_once(move || {
                if widget.is_mapped() {
                    widget.grab_focus();
                }
            });
        }
    });
    *handler.borrow_mut() = Some(id);
    popover.connect_closed(|popover| {
        let popover = popover.clone();
        glib::idle_add_local_once(move || popover.unparent());
    });
    popover.popup();
}
