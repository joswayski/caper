//! Real conversations through the shared client core's background worker
//! (`caper_client::worker`): restores the stored session, opens the
//! account's first space and its first joined channel, follows it live
//! through the gateway, and sends messages. Signs in by email code when
//! no session is stored. Only used when `CAPER_API_URL` is set; the rest
//! of the egui client's commands come later.
use crate::model::{self, Conversation, Person, Reaction, Span};
use caper_client::effects::{Effect, Effects};
use caper_client::notifications::{Change, Scope};
use caper_client::reactions::{
    REACTOR_CACHE_LIMIT, Reacted, Reactions, ReactorCache, ReactorState,
};
use caper_client::state::{CallContext, Phase};
use caper_client::voice::{Voice, VoiceOperation};
use caper_client::worker::{AccountOperation, AccountResult, Command, Event, Worker};
use caper_client::{Host, api::Api, gateway, grouping, model as core};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// Posted to the window when the worker has news; drained on the UI thread.
pub const WM_LIVE: u32 = WM_APP + 1;

/// What the worker asked of the window, applied on the UI thread (timers
/// and titles belong to the window's own thread).
#[derive(Default)]
pub struct Requests {
    pub wake_after: Option<Duration>,
    pub front: bool,
    pub title: Option<String>,
}

/// The window as the worker's host: every call posts `WM_LIVE`.
#[derive(Clone)]
pub struct WinHost {
    hwnd: isize,
    pub requests: Arc<Mutex<Requests>>,
}

impl WinHost {
    fn post(&self) {
        unsafe {
            let _ = PostMessageW(Some(HWND(self.hwnd as _)), WM_LIVE, WPARAM(0), LPARAM(0));
        }
    }

    fn request(&self, f: impl FnOnce(&mut Requests)) {
        if let Ok(mut r) = self.requests.lock() {
            f(&mut r);
        }
        self.post();
    }
}

impl Host for WinHost {
    fn wake(&self) {
        self.post();
    }

    fn wake_after(&self, delay: Duration) {
        self.request(|r| r.wake_after = Some(r.wake_after.map_or(delay, |d| d.min(delay))));
    }

    fn bring_to_front(&self) {
        self.request(|r| r.front = true);
    }

    fn set_title(&self, title: String) {
        self.request(|r| r.title = Some(title));
    }
}

pub enum Update {
    /// No stored session (or restoring it failed, with the reason): sign in.
    SignedOut(Option<String>),
    /// A sign-in code was emailed (the challenge), or why not.
    CodeRequested(Result<String, String>),
    /// The code was wrong, expired or used up, with attempts left.
    VerifyFailed(String, Option<u64>),
    /// Signed in: the account's spaces are loading.
    SignedIn,
    Loaded(Box<(Conversation, Ids)>),
    /// An older page arrived: the whole history again, the new messages
    /// first and the shown ones after them, unchanged.
    Older(Box<(Conversation, Ids)>),
    /// Loading older messages started, finished or failed.
    OlderState,
    /// The history around a message (View original): shown afresh with
    /// that message centred and highlighted.
    Context(Box<(Conversation, Ids)>, String),
    /// Newer messages after the shown window: the same history with them
    /// after the shown rows.
    Newer(Box<(Conversation, Ids)>),
    /// A message already shown: scroll to it and highlight it.
    Focus(String),
    /// The read state arrived: the "New messages" divider goes above this
    /// shown message.
    Unread(String),
    /// Who reacted to a message loaded (for a chip's card).
    Reactors,
    /// The account's unread state changed: the rail's dots and mention
    /// counts and the sidebar's dots.
    Badges,
    /// The open DM's name or note changed (the DM list arrived).
    DirectHead,
    /// The New DM dialog's lookup `request` answered: who it found, or why
    /// not.
    DirectFound {
        request: u64,
        result: Result<core::Person, String>,
    },
    /// A DM you asked for opened (None), or why it couldn't.
    DirectCreated(Option<String>),
    /// Settings' account choices loaded, saved or failed.
    Settings,
    /// A notification was clicked: dialogs, settings and menus close over
    /// the conversation it opens.
    Notified,
    /// The notification inbox's rows, filters or unread count changed, or
    /// it closed to open one of its notifications.
    Inbox,
    /// Dictation `key`'s words, or the HTTP status of its failure.
    Dictated {
        key: u64,
        result: Result<String, Option<u16>>,
    },
    /// Voice changed: who is in a channel's voice, whether a channel can be
    /// joined, or your call's state.
    Voice,
    /// Something for the header's status line that leaves the sign-in
    /// screen alone (a toast that failed to show, say).
    Warning(String),
    /// A block asked for in the Block dialog finished: None once blocked,
    /// else why not.
    Blocked(Option<String>),
    /// The open thread's messages or state changed (or it closed): the
    /// pane rebuilds from `Live::thread_view`.
    Thread,
    /// A new message, or the latest state of a shown one (its reactions,
    /// text or deletion changed), whose row is then rebuilt.
    Message(Box<core::Message>),
    /// The reply to edit (or reload) request `request`: the message's new
    /// copy, or why it failed.
    Edited {
        request: u64,
        reloaded: bool,
        result: Result<Box<core::Message>, String>,
    },
    /// The reply to delete request `request`: None once deleted.
    Deleted(u64, Option<String>),
    /// Attachment images loaded or their URLs were refreshed: rows with
    /// attachments redraw.
    Media,
    /// An attachment to open in the browser or player, once its URL was
    /// refreshed.
    Open(String),
    /// The composer's files or whether uploads are available changed.
    Drafts,
    /// A page of message `message`'s versions for request `request`, or
    /// why it failed (for its history).
    Versions {
        request: u64,
        message: String,
        result: Result<core::MessageVersions, String>,
    },
    /// Online account ids in the open space.
    Presence(HashSet<String>),
    /// The gateway's state while not live ("Connecting…"); None once live.
    Connection(Option<String>),
    Failed(String),
}

/// Server ids behind the displayed conversation.
#[derive(Clone, Default)]
pub struct Ids {
    /// Account id of each `Conversation::members` entry.
    pub members: Vec<String>,
    /// Id of each `Conversation::spaces` and `Conversation::channels` entry.
    pub spaces: Vec<String>,
    pub channels: Vec<String>,
    /// The open conversation's id (a channel or a DM), for its draft.
    pub open: Option<String>,
    /// The newest row, for grouping the next message under it.
    last: Option<grouping::Row>,
    /// Messages shown, so a send's reply and its gateway echo appear once.
    seen: HashSet<String>,
    /// Each shown message's index in `Conversation::messages`.
    pub rows: HashMap<String, usize>,
    /// The newest row's local day, for the next date divider.
    last_date: Option<String>,
}

/// A timestamp's local day (its key) and divider label, as the egui client
/// formats them ("Thursday, November 5, 2026").
fn date_label(created_at: &str) -> Option<(String, String)> {
    let local = chrono::DateTime::parse_from_rfc3339(created_at)
        .ok()?
        .with_timezone(&chrono::Local);
    Some((
        local.format("%Y-%m-%d").to_string(),
        local.format("%A, %B %-d, %Y").to_string(),
    ))
}

impl Ids {
    /// Where a new message goes: its author's index (one past the end for a
    /// new author, whom the caller adds) and whether it groups under the
    /// previous one, and the date divider above it when it starts a new
    /// local day. `None` when it is already shown.
    pub fn place(&mut self, m: &core::Message) -> Option<(usize, bool, Option<String>)> {
        if !self.seen.insert(m.id.clone()) {
            return None;
        }
        self.rows.insert(m.id.clone(), self.rows.len());
        let author = self
            .members
            .iter()
            .position(|id| *id == m.author.id)
            .unwrap_or_else(|| {
                self.members.push(m.author.id.clone());
                self.members.len() - 1
            });
        // Replies and thread broadcasts keep their header, as in egui's history.
        let row = grouping::Row::new(
            &m.author.id,
            &m.created_at,
            m.thread_root_id.is_some() || m.reply_to.is_some(),
        );
        // As in egui's history: a divider at the first message and at each
        // new local day; it also ends the group above.
        let date =
            date_label(&m.created_at).filter(|(key, _)| self.last_date.as_ref() != Some(key));
        let grouped = date.is_none() && grouping::grouped(self.last.as_ref(), &row);
        self.last = Some(row);
        let date = date.map(|(key, label)| {
            self.last_date = Some(key);
            label
        });
        Some((author, grouped, date))
    }
}

pub struct Live {
    worker: Worker<WinHost>,
    /// Attachment images (the shared cache, as in egui), and whether one of
    /// its fetches finished since it was last polled.
    pub media: caper_client::media::Media<crate::files::D2d>,
    media_ready: Arc<std::sync::atomic::AtomicBool>,
    pub host: WinHost,
    generation: u64,
    /// The latest navigation; results for earlier ones are dropped.
    navigation: u64,
    token: Option<String>,
    name: String,
    spaces: Option<core::Spaces>,
    space: Option<String>,
    channel: Option<String>,
    chat_token: Option<String>,
    /// The signed-in author's public id, for "your" reactions.
    pub me: Option<String>,
    /// The open channel's history as the core keeps it: gateway messages,
    /// reactions and edits apply here in sequence, then reach the rows.
    timeline: core::Timeline,
    /// Your reactions not yet acknowledged, drawn over the server's counts
    /// (the shared queue: one request per message in flight).
    pub reactions: Reactions,
    /// Updates made on the UI thread (a reaction toggled), drained first.
    queued: Vec<Update>,
    /// Your edits and pin changes awaiting their reply, drawn over the
    /// server's copy (the core's projection, as in the egui client).
    mutations: core::MessageMutations,
    pending_pins: HashSet<String>,
    /// The last edit or delete request, so only its reply reaches a dialog.
    request: u64,
    /// The signed-in chat author (pins you make carry it).
    author: Option<core::Author>,
    /// The open space, for rebuilding the conversation after an older page.
    detail: Option<core::SpaceDetail>,
    /// Older history: whether the server has more, a request in flight,
    /// and why the last one failed.
    pub has_more: bool,
    pub loading_older: bool,
    pub older_error: Option<String>,
    /// The open thread, and the last thread request (results for earlier
    /// ones drop).
    pub thread: Option<Thread>,
    thread_request: u64,
    /// Channel messages the timeline holds only because a thread loaded
    /// them (its root, its broadcast replies): kept out of the channel's
    /// rows, as in the egui client, until an older page brings them.
    thread_only: HashSet<String>,
    /// The shown slice of the channel after View original jumped into its
    /// past: from sequence `start`, up to `end` while newer messages exist
    /// past it (egui's context window). None shows the latest history.
    window: Option<(u64, Option<u64>)>,
    /// Context (or newer messages) is loading.
    pub context_loading: bool,
    /// Read state, as in the egui client: this visit's unread range (read,
    /// last) for the "New messages" divider, whether its snapshot is still
    /// loading (reads wait for it), the visit and its channel, the last
    /// read acknowledged per channel, and whether the window has focus.
    unread: Option<(u64, u64)>,
    unread_pending: bool,
    visit: u64,
    visit_channel: Option<String>,
    /// The last visit id handed out.
    next_visit: u64,
    /// The unread snapshot a navigation started beside its history: its
    /// navigation, the visit id it was requested under, and once it
    /// arrived each channel's unread range.
    prefetch: Option<(u64, u64, Option<HashMap<String, (u64, u64)>>)>,
    reads: HashMap<String, String>,
    foreground: bool,
    /// Who reacted, per message, for the chips' cards (the shared cache).
    reactors: HashMap<String, ReactorCache>,
    /// Spaces and channels with anything new (and mentions of you), as the
    /// server last listed them; the last refresh (older replies drop).
    pub unread_spaces: Vec<core::UnreadSpace>,
    unread_revision: u64,
    /// Files staged in composers (every channel's, as in the egui client),
    /// the server's compression settings once uploads are known to work,
    /// and own decoded images to show for sent attachments.
    pub drafts: Vec<crate::uploads::Draft>,
    draft_keys: u64,
    /// The last dictation sent, so only its answer is used.
    dictations: u64,
    pub uploads: Option<caper_client::compress::Compression>,
    pub uploads_checked: bool,
    local: Vec<(String, caper_client::Rgba)>,
    /// KLIPY's key once `GET /api/klipy` answers (GIFs stay hidden until
    /// then, as in egui), when it was last asked, and the shares to send
    /// once their GIF messages are confirmed (by client id).
    pub klipy: Option<caper_client::klipy::Config>,
    klipy_checked: Option<std::time::Instant>,
    shares: HashMap<String, caper_client::klipy::Share>,
    /// Your DMs as the server last listed them, your account (for your own
    /// DM), and the open DM while one is open instead of a channel.
    pub directs: Vec<core::DirectConversation>,
    pub account_id: Option<String>,
    pub account_name: String,
    username: Option<String>,
    pub direct: Option<String>,
    /// The open DM's name as its history named it, until the list knows it.
    direct_name: String,
    /// People you share a space or a DM with, for the New DM dialog's exact
    /// match (egui's `people`), and the last username lookup.
    people: Vec<core::Person>,
    lookup: u64,
    /// "Message requests" opened in the sidebar; the open request's
    /// Accept or Decline in flight (true: declining) and why the last one
    /// failed.
    pub requests_open: bool,
    answer: Option<bool>,
    answer_error: Option<String>,
    /// Settings' account choices, as egui keeps them: who can start a DM,
    /// a save in flight and why the last load or save failed; your blocked
    /// accounts, an unblock in flight and why it failed; notifications.
    privacy: Option<String>,
    privacy_saving: bool,
    privacy_error: Option<String>,
    blocks: Option<Vec<core::BlockedAccount>>,
    blocks_error: Option<String>,
    block_busy: bool,
    block_error: Option<String>,
    notifications: caper_client::notifications::Notifications,
    /// When the unread badges are read again after a notification arrived.
    unread_due: Option<Instant>,
    /// The signed-in account's epoch, as egui's `account_epoch`: what its
    /// notifications and inbox results carry (`generation` moves on with
    /// every navigation).
    account_epoch: u64,
    /// The notification inbox, as every client keeps it.
    pub activity: caper_client::activity::Inbox,
    /// Where an opened notification points: its navigation, the message
    /// and its thread's root, shown once that navigation loads.
    focus: Option<(u64, String, Option<String>)>,
    /// Who is in voice in each joined channel of the open space, as the
    /// gateway last listed them (egui's `channel_rosters`), and when each
    /// session started.
    rosters: HashMap<String, (Vec<core::VoiceOccupant>, Option<u64>)>,
    /// Your call (the shared engine, as egui's `voice`) and its sounds.
    pub voice: Voice,
    pub effects: Effects,
    /// Whether each channel's voice can be joined, as the server said for
    /// this account (egui's `media_availability`), and which were asked.
    availability: HashMap<String, bool>,
    status_asked: HashSet<String>,
    /// The last join's membership check: its channel, the call generation
    /// it was made in and when Join was clicked (epoch ms).
    join_request: u64,
    pending_join: Option<(String, u64, u64)>,
    /// Joins prepared ahead of a click, by channel (at most every 4 s).
    prepared: HashMap<String, Instant>,
    /// The call generation whose connection was announced, and who else
    /// was in it, for the join and leave chimes.
    announced: Option<u64>,
    others: Option<(u64, Vec<String>)>,
    /// What voice last showed, so only changes redraw.
    voice_seen: String,
}

/// What a channel's voice offers now (egui's `VoiceAction`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VoiceAction {
    /// Join (or Switch, from a call elsewhere), and whether it can now.
    Join {
        switching: bool,
        enabled: bool,
    },
    Joining,
    Leave,
}

/// Your call as the sidebar's card shows it.
#[derive(Clone, PartialEq, Debug)]
pub struct CallView {
    /// Connecting, reconnecting or connected (None).
    pub status: Option<&'static str>,
    pub channel: String,
    pub space: String,
    pub names: String,
    pub count: usize,
    /// The session's running time.
    pub timer: Option<String>,
    pub joining: bool,
    pub muted: bool,
    pub deafened: bool,
}

/// What a send with composer files did.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sent {
    Sent,
    /// Files are still uploading (failed ones were restarted): the send
    /// waits for them.
    Waiting,
}

/// The thread beside the channel, as in the egui client's thread panel.
pub struct Thread {
    pub root: String,
    request: u64,
    pub loading: bool,
    /// Older replies exist before `before`.
    pub has_more: bool,
    before: Option<String>,
    pub error: Option<String>,
    /// The failed request was an older page, so Retry repeats it.
    retry_older: bool,
}

/// What the top of the history offers, as in the egui client.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Older {
    /// "Load older messages".
    More,
    /// "Loading…".
    Loading,
    /// "Couldn't load older messages." and Retry.
    Failed,
    /// "Beginning of conversation".
    Start,
}

/// Starts the worker and restores the stored session; `None` without
/// `CAPER_API_URL`.
pub fn start(hwnd: HWND) -> Option<Live> {
    let url = std::env::var("CAPER_API_URL").ok()?;
    let host = WinHost {
        hwnd: hwnd.0 as isize,
        requests: Arc::default(),
    };
    let api = match Api::new(&url) {
        Ok(api) => api,
        Err(e) => {
            eprintln!("CAPER_API_URL: {e}");
            return None;
        }
    };
    let media_ready = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let media = caper_client::media::Media::new(Some(api.clone()), {
        let (host, ready) = (host.clone(), media_ready.clone());
        Arc::new(move || {
            ready.store(true, std::sync::atomic::Ordering::Release);
            host.wake();
        })
    });
    // Voice wakes the window like the worker does.
    let mut voice = Voice::new(api.base().clone(), {
        let host = host.clone();
        caper_client::voice::Repaint::new(move |delay| match delay {
            None => host.wake(),
            Some(delay) => host.wake_after(delay),
        })
    });
    if let Some(saved) = crate::settings::audio::restore_preferences() {
        voice.preferences = saved;
    }
    let worker = Worker::new(api, host.clone());
    let generation = 1;
    worker.send(Command::Restore { generation });
    Some(Live::fresh(
        worker,
        media,
        media_ready,
        host,
        generation,
        voice,
        Effects::new(true),
    ))
}

impl Live {
    /// A signed-out Live around `worker`, at `generation`.
    fn fresh(
        worker: Worker<WinHost>,
        media: caper_client::media::Media<crate::files::D2d>,
        media_ready: Arc<std::sync::atomic::AtomicBool>,
        host: WinHost,
        generation: u64,
        voice: Voice,
        effects: Effects,
    ) -> Self {
        Live {
            worker,
            media,
            media_ready,
            host,
            generation,
            navigation: 0,
            token: None,
            name: String::new(),
            spaces: None,
            space: None,
            channel: None,
            chat_token: None,
            me: None,
            timeline: core::Timeline::default(),
            reactions: Reactions::default(),
            queued: Vec::new(),
            mutations: core::MessageMutations::default(),
            pending_pins: HashSet::new(),
            request: 0,
            author: None,
            detail: None,
            has_more: false,
            loading_older: false,
            older_error: None,
            thread: None,
            thread_request: 0,
            thread_only: HashSet::new(),
            window: None,
            context_loading: false,
            unread: None,
            unread_pending: false,
            visit: 0,
            visit_channel: None,
            next_visit: 0,
            prefetch: None,
            reads: HashMap::new(),
            foreground: false,
            reactors: HashMap::new(),
            unread_spaces: Vec::new(),
            unread_revision: 0,
            drafts: Vec::new(),
            draft_keys: 0,
            dictations: 0,
            uploads: None,
            uploads_checked: false,
            local: Vec::new(),
            klipy: None,
            klipy_checked: None,
            shares: HashMap::new(),
            directs: Vec::new(),
            account_id: None,
            account_name: String::new(),
            username: None,
            direct: None,
            direct_name: String::new(),
            people: Vec::new(),
            lookup: 0,
            requests_open: false,
            answer: None,
            answer_error: None,
            privacy: None,
            privacy_saving: false,
            privacy_error: None,
            blocks: None,
            blocks_error: None,
            block_busy: false,
            block_error: None,
            notifications: Default::default(),
            unread_due: None,
            account_epoch: 0,
            activity: Default::default(),
            focus: None,
            rosters: HashMap::new(),
            voice,
            effects,
            availability: HashMap::new(),
            status_asked: HashSet::new(),
            join_request: 0,
            pending_join: None,
            prepared: HashMap::new(),
            announced: None,
            others: None,
            voice_seen: String::new(),
        }
    }

    /// Signs out (egui's `logout`): the stored session is forgotten and the
    /// server revokes it. A signed-out Live replaces this one on the same
    /// worker, so nothing can restore the old session meanwhile.
    pub fn logout(self) -> Live {
        let generation = self.generation + 1;
        self.worker.send(Command::StopNotifications);
        self.worker.send(Command::StopGateway);
        self.worker.send(Command::ClearCredential { generation });
        if let Some(token) = self.token.clone() {
            self.worker.send(Command::Logout { generation, token });
        }
        let Live {
            worker,
            mut media,
            media_ready,
            host,
            mut voice,
            effects,
            ..
        } = self;
        media.clear();
        voice.leave();
        voice.error = None;
        Live::fresh(worker, media, media_ready, host, generation, voice, effects)
    }

    fn account(&self, operation: AccountOperation) {
        if let Some(token) = &self.token {
            self.worker.send(Command::Account {
                epoch: 0,
                token: token.clone(),
                operation,
            });
        }
    }

    /// Settings opened (egui's `open_settings`): privacy (unless a save is
    /// in flight), blocks, and notifications until they have loaded.
    pub fn load_settings(&mut self) {
        if !self.privacy_saving {
            self.load_privacy();
        }
        self.load_blocks();
        if !self.notifications.ready() {
            self.load_notifications();
        }
    }

    pub fn load_privacy(&mut self) {
        self.privacy_error = None;
        self.account(AccountOperation::LoadPrivacy);
    }

    pub fn load_blocks(&mut self) {
        self.blocks_error = None;
        self.account(AccountOperation::LoadBlocks);
    }

    pub fn load_notifications(&mut self) {
        if self.token.is_some()
            && let Some(revision) = self.notifications.start_load()
        {
            self.account(AccountOperation::LoadNotifications { revision });
        }
    }

    /// Applies a notification choice at once and saves it; a failed save
    /// reverts with an inline error.
    pub fn change_notifications(&mut self, scope: Scope, change: Change) {
        if self.token.is_some() && self.notifications.begin(&scope, &change) {
            self.account(AccountOperation::SaveNotifications { scope, change });
        }
    }

    /// How long until the earliest timed mute ends.
    pub fn next_mute_expiry(&self) -> Option<Duration> {
        self.notifications.next_expiry(chrono::Utc::now())
    }

    /// A DM's bell-slash label while it is muted.
    pub fn direct_muted(&self, id: &str) -> Option<String> {
        self.notifications
            .muted_label(&Scope::Direct(id.to_owned()), &chrono::Local::now())
    }

    /// Why a DM's last notification change failed.
    pub fn direct_notify_error(&self, id: &str) -> Option<String> {
        self.notifications
            .error(&Scope::Direct(id.to_owned()))
            .map(str::to_owned)
    }

    /// A space's, channel's or DM's bell-slash label while it is muted.
    pub fn muted(&self, scope: &Scope) -> Option<String> {
        self.notifications.muted_label(scope, &chrono::Local::now())
    }

    /// Why a space's, channel's or DM's last notification change failed.
    pub fn notify_error(&self, scope: &Scope) -> Option<String> {
        self.notifications.error(scope).map(str::to_owned)
    }

    /// A space's, channel's or DM's notification menu, as egui's
    /// `notification_items`: whether its choices can change now, and its
    /// items.
    pub fn notify_menu(&self, scope: &Scope) -> (bool, Vec<crate::directs::MenuItem>) {
        use crate::directs::MenuItem;
        use caper_client::model::NotificationLevel as Level;
        use caper_client::notifications::{MUTE_PRESETS, default_label, level_label, mute_label};
        let n = &self.notifications;
        let now = chrono::Local::now();
        let utc = now.with_timezone(&chrono::Utc);
        let enabled = n.ready() && !n.saving(scope);
        let noun = match scope {
            Scope::Space(_) => "space",
            Scope::Channel { .. } => "channel",
            _ => "conversation",
        };
        let level = n.level(scope);
        let mut items = Vec::new();
        if let Scope::Space(_) = scope {
            let voice = n.voice_started(scope);
            items.push(MenuItem::VoiceHeading);
            items.extend([None, Some(true), Some(false)].map(|v| MenuItem::Voice(v, voice == v)));
        }
        if let Scope::Direct(_) = scope {
            items.push(MenuItem::Toggle(level != Some(Level::Nothing)));
        } else {
            let inherited = n.inherited(scope);
            items.push(MenuItem::LevelHeading);
            items.extend(
                [
                    None,
                    Some(Level::All),
                    Some(Level::Mentions),
                    Some(Level::Nothing),
                ]
                .map(|choice| {
                    let label =
                        choice.map_or_else(|| default_label(inherited), |l| level_label(l).into());
                    MenuItem::Level(choice, label, level == choice)
                }),
            );
        }
        match n.mute(scope, utc) {
            Some(mute) => {
                items.push(MenuItem::Unmute(noun));
                items.push(MenuItem::Note(mute_label(mute, &now), false));
            }
            None => {
                items.push(MenuItem::MuteHeading(noun));
                items.extend((0..MUTE_PRESETS.len()).map(MenuItem::Mute));
            }
        }
        if let Scope::Channel { space, .. } = scope
            && n.mute(&Scope::Space(space.clone()), utc).is_some()
        {
            items.push(MenuItem::Note("Muted with the space".into(), false));
        }
        if !n.ready() {
            items.push(match n.load_error() {
                Some(e) => MenuItem::Note(e.to_owned(), true),
                None => MenuItem::Note("Loading\u{2026}".into(), false),
            });
        }
        if let Some(e) = n.error(scope) {
            items.push(MenuItem::Note(e.to_owned(), true));
        }
        (enabled, items)
    }

    /// A choice in a notification menu, applied at once and saved.
    pub fn menu_choose(&mut self, scope: &Scope, item: &crate::directs::MenuItem) {
        use crate::directs::MenuItem;
        use caper_client::notifications::{MUTE_PRESETS, mute_value};
        let change = match item {
            MenuItem::Toggle(on) => Change::Level(on.then_some(core::NotificationLevel::Nothing)),
            MenuItem::Level(choice, ..) => Change::Level(*choice),
            MenuItem::Voice(value, _) => Change::VoiceStarted(*value),
            MenuItem::Mute(k) => {
                Change::Mute(Some(mute_value(MUTE_PRESETS[*k].1, chrono::Utc::now())))
            }
            MenuItem::Unmute(_) => Change::Mute(None),
            _ => return,
        };
        self.change_notifications(scope.clone(), change);
    }

    /// Who can start a DM with you: shown at once, reverted if the save
    /// fails.
    pub fn save_privacy(&mut self, value: &str) {
        if self.privacy_saving || self.privacy.as_deref() == Some(value) {
            return;
        }
        let previous = self.privacy.replace(value.to_owned());
        self.privacy_saving = true;
        self.privacy_error = None;
        self.account(AccountOperation::SavePrivacy {
            value: value.to_owned(),
            previous,
        });
    }

    /// Unblocks the `k`th blocked account.
    pub fn unblock(&mut self, k: usize) {
        if let Some(account) = self.blocks.as_ref().and_then(|b| b.get(k)).cloned() {
            self.unblock_account(account);
        }
    }

    /// Unblocks the open DM's other person (its blocked bar's Unblock).
    pub fn unblock_open(&mut self) {
        if let Some(d) = self.open_direct_conversation() {
            let account = (&d.peer).into();
            self.unblock_account(account);
        }
    }

    fn unblock_account(&mut self, account: core::BlockedAccount) {
        self.block_busy = true;
        self.block_error = None;
        self.account(AccountOperation::SetBlock {
            account,
            blocked: false,
            request: None,
        });
    }

    /// The account's half of what Settings shows.
    pub fn settings_snapshot(&self, snap: &mut crate::settings::Snapshot) {
        let n = &self.notifications;
        snap.account = self.token.is_some();
        snap.level = n.account_level();
        snap.voice_started = n.account_voice_started();
        snap.notify_saving = n.saving(&Scope::Account);
        snap.notify_load_error = n.load_error().map(str::to_owned);
        snap.notify_error = n.error(&Scope::Account).map(str::to_owned);
        snap.privacy.clone_from(&self.privacy);
        snap.privacy_error.clone_from(&self.privacy_error);
        snap.blocks = self.blocks.as_ref().map(|list| {
            list.iter()
                .map(|a| {
                    let p = person(&a.id, &a.display_name);
                    (
                        a.display_name.clone(),
                        a.username.clone(),
                        p.color,
                        p.initials,
                    )
                })
                .collect()
        });
        snap.blocks_error.clone_from(&self.blocks_error);
        snap.block_busy = self.block_busy;
        snap.block_error.clone_from(&self.block_error);
    }

    /// Settings' Account results (egui's handling of each).
    fn account_settings(&mut self, result: AccountResult, out: &mut Vec<Update>) {
        match result {
            AccountResult::Blocks(Ok(blocks)) => {
                self.blocks = Some(blocks);
                self.blocks_error = None;
            }
            AccountResult::Blocks(Err(e)) => self.blocks_error = Some(e),
            AccountResult::Block {
                account,
                blocked,
                request,
                result,
            } => {
                self.block_busy = false;
                match result {
                    Err(e) => {
                        if blocked {
                            out.push(Update::Blocked(Some(e.clone())));
                        }
                        self.block_error = Some(e);
                    }
                    Ok(()) => {
                        self.block_error = None;
                        let blocks = self.blocks.get_or_insert_with(Vec::new);
                        blocks.retain(|b| b.id != account.id);
                        if blocked {
                            blocks.insert(0, account.clone());
                        }
                        for d in &mut self.directs {
                            if d.peer.id == account.id {
                                d.blocked = blocked;
                            }
                        }
                        if blocked {
                            // Blocking also declines their pending request.
                            let mut requests: Vec<String> = self
                                .directs
                                .iter()
                                .filter(|d| {
                                    d.peer.id == account.id
                                        && d.status == core::DirectStatus::Incoming
                                })
                                .map(|d| d.id.clone())
                                .collect();
                            requests.extend(request.filter(|r| !requests.contains(r)));
                            for id in requests {
                                self.leave_request(&id);
                            }
                            out.push(Update::Blocked(None));
                        }
                        out.extend([Update::Badges, Update::DirectHead]);
                    }
                }
            }
            // A save in flight owns the displayed value.
            AccountResult::Privacy(Ok(value)) if !self.privacy_saving => {
                self.privacy = Some(value);
            }
            AccountResult::Privacy(Ok(_)) => {}
            AccountResult::Privacy(Err(e)) => self.privacy_error = Some(e),
            AccountResult::PrivacySaved { previous, result } => {
                self.privacy_saving = false;
                match result {
                    Ok(value) => {
                        self.privacy = Some(value);
                        self.privacy_error = None;
                    }
                    Err(e) => {
                        self.privacy = previous;
                        self.privacy_error = Some(e);
                    }
                }
            }
            AccountResult::Notifications { revision, result } => {
                self.notifications.finish_load(revision, result);
            }
            AccountResult::NotificationsSaved { scope, result } => {
                self.notifications.finish_save(&scope, result);
            }
            _ => return,
        }
        out.push(Update::Settings);
    }
}

impl Live {
    /// Applies finished image loads and starts queued ones, and asks for
    /// fresh signed URLs of shown attachments about to expire. Returns
    /// whether an image arrived.
    pub fn pump_media(&mut self, tex: &mut crate::files::D2d) -> bool {
        let now = std::time::Instant::now();
        let ready = self
            .media_ready
            .swap(false, std::sync::atomic::Ordering::Acquire);
        self.media.poll(tex, now);
        for (id, image) in self.local.drain(..) {
            self.media.insert_local(tex, &id, image);
        }
        for draft in &mut self.drafts {
            draft.prepare(tex);
        }
        if let Some(token) = &self.token {
            let ids = self.media.take_refresh(now);
            if !ids.is_empty() {
                self.worker.send(Command::RefreshAttachmentUrls {
                    token: token.clone(),
                    ids,
                });
            }
        }
        ready
    }

    pub fn request_code(&self, email: String) {
        self.worker.send(Command::RequestCode {
            generation: self.generation,
            email,
        });
    }

    pub fn verify(&self, challenge: String, code: String) {
        self.worker.send(Command::VerifyCode {
            generation: self.generation,
            challenge,
            code,
        });
    }

    /// Opens the account's first space; its first joined channel follows.
    fn open(
        &mut self,
        token: String,
        account: core::Account,
        spaces: core::Spaces,
        out: &mut Vec<Update>,
    ) {
        self.account_id = Some(account.id.clone());
        self.username.clone_from(&account.username);
        self.name = account
            .display_name
            .or(account.username)
            .unwrap_or_default();
        self.account_name.clone_from(&self.name);
        let Some(space) = spaces.spaces.first().map(|s| s.id.clone()) else {
            out.push(Update::Failed("This account has no spaces.".into()));
            return;
        };
        // Uploads are offered once the server's usage answers.
        self.worker.send(Command::AssetUsage {
            token: token.clone(),
        });
        self.token = Some(token);
        self.check_klipy();
        // The DM list arrives with the account (`SignedInDirects`).
        // Blocked people's DMs show the blocked bar; muted ones their bell.
        self.load_blocks();
        self.load_notifications();
        // Mentions and DMs toast while the window isn't showing them.
        self.account_epoch = self.generation;
        if let Some(token) = self.token.clone() {
            self.worker.send(Command::StartNotifications {
                epoch: self.account_epoch,
                token,
            });
        }
        self.spaces = Some(spaces);
        self.navigate(Some(space), None);
    }

    /// Whether files can be attached to the open channel now.
    pub fn can_attach(&self) -> bool {
        self.token.is_some()
            && self.uploads.is_some()
            && self.channel.is_some()
            && self.chat_token.is_some()
    }

    /// Refreshes the DM list (with the unread badges, as egui does).
    fn load_directs(&self) {
        if let Some(token) = &self.token {
            self.worker.send(Command::LoadDirects {
                generation: self.generation,
                token: token.clone(),
            });
        }
    }

    /// Opens a DM in place of the channel; the space's channels stay.
    pub fn open_direct(&mut self, id: String) {
        if self.direct.as_deref() != Some(id.as_str()) {
            self.navigate(None, Some(id));
        }
    }

    /// Your own DM: opened, or created first (egui's
    /// `select_or_create_self_direct`).
    pub fn open_self_direct(&mut self) {
        let me = self.account_id.clone();
        if let Some(d) = self
            .directs
            .iter()
            .find(|d| Some(&d.peer.id) == me.as_ref())
        {
            let id = d.id.clone();
            return self.open_direct(id);
        }
        if let Some(username) = self.username.clone() {
            self.create_direct(username);
        }
    }

    /// Opens the DM with `username`, creating it first when it is new.
    pub fn create_direct(&mut self, username: String) {
        let Some(token) = self.token.clone() else {
            return;
        };
        self.navigation += 1;
        self.worker.send(Command::CreateDirect {
            generation: self.generation,
            navigation: self.navigation,
            token,
            username,
        });
    }

    /// The open DM, unless it is your own.
    fn open_direct_conversation(&self) -> Option<&core::DirectConversation> {
        let id = self.direct.as_ref()?;
        self.directs
            .iter()
            .find(|d| &d.id == id && Some(&d.peer.id) != self.account_id.as_ref())
    }

    pub fn is_blocked(&self, account: &str) -> bool {
        self.blocks
            .as_ref()
            .is_some_and(|b| b.iter().any(|a| a.id == account))
    }

    /// Server-stamped Caper identities, which can't be blocked (egui's
    /// `protected_account`).
    fn protected(&self, id: &str) -> bool {
        self.directs
            .iter()
            .any(|d| d.peer.id == id && (d.peer.admin || d.peer.team))
            || self.people.iter().any(|p| p.id == id && p.admin)
            || self
                .detail
                .iter()
                .flat_map(|d| &d.members)
                .any(|m| m.id == id && m.admin)
    }

    /// The bar replacing the open DM's composer: an incoming request's,
    /// or a blocked person's.
    pub fn dm_bar(&self) -> Option<crate::directs::Bar> {
        use crate::directs::{Bar, BarKind};
        let d = self.open_direct_conversation()?;
        let kind = if d.status == core::DirectStatus::Incoming {
            BarKind::Request
        } else if d.blocked || self.is_blocked(&d.peer.id) {
            BarKind::Blocked
        } else {
            return None;
        };
        Some(Bar {
            kind,
            name: d.peer.display_name.clone(),
            username: d.peer.username.clone(),
            busy: self.answer,
            unblocking: self.block_busy,
            protected: self.protected(&d.peer.id),
            error: match kind {
                BarKind::Request => self.answer_error.clone(),
                BarKind::Blocked => self.block_error.clone(),
            },
        })
    }

    /// Above your own request's composer, until they accept.
    pub fn waiting_note(&self) -> Option<String> {
        let d = self.open_direct_conversation()?;
        (d.status == core::DirectStatus::Outgoing && !d.blocked && !self.is_blocked(&d.peer.id))
            .then(|| {
                format!(
                    "Waiting for @{} to accept. They'll see your messages when they do.",
                    d.peer.username
                )
            })
    }

    /// Whom the open DM's Block would block, and the request it declines.
    pub fn block_target(&self) -> Option<(core::BlockedAccount, Option<String>)> {
        let d = self.open_direct_conversation()?;
        (!self.protected(&d.peer.id)).then(|| {
            (
                (&d.peer).into(),
                (d.status == core::DirectStatus::Incoming).then(|| d.id.clone()),
            )
        })
    }

    /// Blocks `account` (the Block dialog's confirmation), declining
    /// `request` too.
    pub fn block(&mut self, account: core::BlockedAccount, request: Option<String>) {
        if self.protected(&account.id) {
            return;
        }
        self.block_busy = true;
        self.block_error = None;
        self.account(AccountOperation::SetBlock {
            account,
            blocked: true,
            request,
        });
    }

    /// Accepts or declines the open request.
    pub fn answer_request(&mut self, accept: bool) {
        let (Some(token), Some(id)) = (self.token.clone(), self.direct.clone()) else {
            return;
        };
        if self.answer.is_some() {
            return;
        }
        self.answer = Some(!accept);
        self.answer_error = None;
        let operation = if accept {
            AccountOperation::Accept(id)
        } else {
            AccountOperation::Decline(id)
        };
        self.worker.send(Command::Account {
            epoch: 0,
            token,
            operation,
        });
    }

    /// A declined request leaves the list; when it was open, the next
    /// request opens, else the space's first channel (egui's
    /// `leave_request`).
    fn leave_request(&mut self, id: &str) {
        self.directs.retain(|d| d.id != id);
        if self.direct.as_deref() != Some(id) {
            return;
        }
        let next = self
            .directs
            .iter()
            .find(|d| d.status == core::DirectStatus::Incoming)
            .map(|d| d.id.clone());
        match next {
            Some(next) => {
                self.requests_open = true;
                self.open_direct(next);
            }
            None => self.navigate(self.space.clone(), None),
        }
    }

    /// Refreshes the people the New DM dialog matches (egui's `load_people`).
    pub fn load_people(&self) {
        if let Some(token) = &self.token {
            self.worker.send(Command::LoadPeople {
                generation: self.generation,
                token: token.clone(),
            });
        }
    }

    /// People you share a space or a DM with (`GET /api/people`).
    pub fn people(&self) -> &[core::Person] {
        &self.people
    }

    /// Someone you already know, by exact username.
    pub fn person_named(&self, username: &str) -> Option<&core::Person> {
        self.people.iter().find(|p| p.username == username)
    }

    /// Looks a username up (`POST /api/dms/lookup`); returns the request,
    /// which `Update::DirectFound` answers.
    pub fn lookup_direct(&mut self, username: String) -> u64 {
        self.lookup += 1;
        if let Some(token) = self.token.clone() {
            self.worker.send(Command::LookupDirect {
                generation: self.generation,
                navigation: self.navigation,
                request: self.lookup,
                token,
                username,
            });
        }
        self.lookup
    }

    /// The open DM's header and empty note.
    pub fn direct_head(&self) -> Option<crate::model::Direct> {
        let id = self.direct.as_ref()?;
        let d = self.directs.iter().find(|d| &d.id == id);
        Some(crate::model::Direct {
            name: d.map_or_else(|| self.direct_name.clone(), |d| d.peer.display_name.clone()),
            own: d.is_some_and(|d| Some(&d.peer.id) == self.account_id.as_ref()),
            note: d.map_or_else(
                || "Only you and this person can read this conversation.".into(),
                core::DirectConversation::empty_note,
            ),
        })
    }

    /// Asks for KLIPY's key while GIFs are unavailable, at most once a
    /// minute (egui's rule).
    fn check_klipy(&mut self) {
        let Some(token) = &self.token else { return };
        if self.klipy.is_none()
            && self
                .klipy_checked
                .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(60))
        {
            self.klipy_checked = Some(std::time::Instant::now());
            self.worker.send(Command::Klipy {
                token: token.clone(),
            });
        }
    }

    /// Whether a GIF can be sent to the open channel now.
    pub fn can_gif(&self) -> bool {
        self.klipy.is_some()
            && self.token.is_some()
            && self.channel.is_some()
            && self.chat_token.is_some()
    }

    /// Sends a GIF chosen in the picker as its own message, replying to
    /// `reply_to`; KLIPY hears of it once the server confirms the send.
    pub fn send_gif(
        &mut self,
        gif: caper_client::klipy::Gif,
        share: caper_client::klipy::Share,
        reply_to: Option<String>,
    ) {
        let (Some(channel), Some(chat_token)) = (&self.channel, &self.chat_token) else {
            return;
        };
        let client_id = uuid::Uuid::new_v4().to_string();
        self.shares.insert(client_id.clone(), share);
        self.worker.send(Command::Send {
            generation: self.generation,
            token: self.token.clone(),
            chat_token: chat_token.clone(),
            channel: channel.clone(),
            client_id,
            text: String::new(),
            broadcast: false,
            thread_root_id: None,
            reply_to_id: reply_to,
            attachment_ids: Vec::new(),
            spoiler_attachment_ids: Vec::new(),
            gif: Some(gif),
            suppress_embeds: false,
        });
    }

    /// Why Attach files is unavailable, as the egui client words it.
    pub fn attach_unavailable(&self) -> &'static str {
        if self.token.is_some() && !self.uploads_checked {
            "Checking file uploads\u{2026}"
        } else {
            "File uploads are unavailable"
        }
    }

    fn is_open_draft(&self, draft: &crate::uploads::Draft) -> bool {
        Some(&draft.upload.channel) == self.channel.as_ref()
            && draft.upload.thread_root_id.is_none()
    }

    /// The open channel's composer files, in order.
    pub fn open_drafts(&self) -> Vec<&crate::uploads::Draft> {
        self.drafts
            .iter()
            .filter(|d| self.is_open_draft(d))
            .collect()
    }

    /// Stages picked, dropped or pasted files in the open channel's
    /// composer and starts uploading them; at most ten. Returns the
    /// error to show when some (or all) did not fit.
    pub fn add_files(
        &mut self,
        files: Vec<(
            std::path::PathBuf,
            Option<Arc<caper_client::uploads::Scratch>>,
        )>,
    ) -> Option<String> {
        if files.is_empty() {
            return None;
        }
        let (Some(token), Some(settings), Some(channel)) = (
            self.token.clone(),
            self.uploads.clone(),
            self.channel.clone(),
        ) else {
            return Some(self.attach_unavailable().into());
        };
        let used = self.open_drafts().len();
        let room = caper_client::uploads::MAX_ATTACHMENTS.saturating_sub(used);
        if room == 0 {
            return Some(caper_client::drafts::room_error(0));
        }
        let error = (files.len() > room).then(|| caper_client::drafts::room_error(room));
        for (path, scratch) in files.into_iter().take(room) {
            self.draft_keys += 1;
            let name = if scratch.is_some() {
                "Pasted image.png".to_owned()
            } else {
                path.file_name()
                    .map_or_else(|| "file".into(), |n| n.to_string_lossy().into_owned())
            };
            let mut upload = caper_client::drafts::Upload::picked(
                self.draft_keys,
                channel.clone(),
                None,
                name,
                Some(path),
            );
            upload.source_scratch = scratch;
            self.worker
                .send(upload.command(token.clone(), settings.clone()));
            self.drafts.push(crate::uploads::Draft::new(upload));
        }
        error
    }

    /// Why a recording can't become a voice message here: one per
    /// message, within the attachment limit (egui's `voice_note_blocked`).
    pub fn voice_note_blocked(&self) -> Option<&'static str> {
        let drafts = self.open_drafts();
        (drafts.iter().any(|d| d.upload.voice().is_some())
            || drafts.len() >= caper_client::uploads::MAX_ATTACHMENTS)
            .then_some("Remove the voice message to record another")
    }

    /// Stages a stopped recording in the open channel's composer: it
    /// uploads now (so transcription starts) and goes with the next Send.
    pub fn stage_voice(
        &mut self,
        prepared: Arc<caper_client::uploads::Prepared>,
    ) -> Result<(), String> {
        let (Some(token), Some(settings), Some(channel)) = (
            self.token.clone(),
            self.uploads.clone(),
            self.channel.clone(),
        ) else {
            return Err(self.attach_unavailable().into());
        };
        if let Some(blocked) = self.voice_note_blocked() {
            return Err(blocked.into());
        }
        self.draft_keys += 1;
        let upload =
            caper_client::drafts::Upload::recorded(self.draft_keys, channel, None, prepared);
        self.worker.send(upload.command(token, settings));
        self.drafts.push(crate::uploads::Draft::new(upload));
        Ok(())
    }

    /// Transcribes a recording (`POST /api/dictation`); its key, or None
    /// when signed out.
    pub fn dictate(&mut self, audio: Arc<[u8]>, content_type: String) -> Option<u64> {
        let token = self.token.clone()?;
        self.dictations += 1;
        self.worker.send(Command::Dictate {
            token,
            key: self.dictations,
            audio,
            content_type,
        });
        Some(self.dictations)
    }

    /// Removes a composer file, stopping its upload and giving up the
    /// server's copy.
    pub fn remove_draft(&mut self, key: u64) {
        if let Some(i) = self.drafts.iter().position(|d| d.upload.key == key) {
            let draft = self.drafts.remove(i);
            draft.upload.abandon();
            if let Some(file) = draft.upload.attachment {
                self.cancel_assets(vec![file.id]);
            }
        }
    }

    pub fn toggle_spoiler(&mut self, key: u64) {
        if let Some(d) = self.drafts.iter_mut().find(|d| d.upload.key == key) {
            d.upload.spoiler = !d.upload.spoiler;
        }
    }

    /// Uploads a failed or expired file again.
    pub fn retry_draft(&mut self, key: u64) {
        let (Some(token), Some(settings)) = (self.token.clone(), self.uploads.clone()) else {
            return;
        };
        let Some(d) = self.drafts.iter_mut().find(|d| d.upload.key == key) else {
            return;
        };
        let Some(old) = d.upload.restart() else {
            return;
        };
        self.worker.send(d.upload.command(token, settings));
        if let Some(old) = old {
            self.cancel_assets(vec![old.id]);
        }
    }

    fn cancel_assets(&self, ids: Vec<String>) {
        if let Some(token) = &self.token
            && !ids.is_empty()
        {
            self.worker.send(Command::CancelAssets {
                token: token.clone(),
                ids,
            });
        }
    }

    /// Sends `text` with the open channel's composer files once they are
    /// all uploaded: failed or expired ones start again and the send waits
    /// (`Sent::Waiting`). Sent files leave the composer.
    pub fn send_with_files(&mut self, text: String, reply_to: Option<String>) -> Sent {
        let keys: Vec<u64> = self.open_drafts().iter().map(|d| d.upload.key).collect();
        for key in &keys {
            self.retry_draft(*key);
        }
        if self
            .open_drafts()
            .iter()
            .any(|d| d.upload.uploading || d.upload.needs_upload())
        {
            return Sent::Waiting;
        }
        let (Some(channel), Some(chat_token)) = (&self.channel, &self.chat_token) else {
            return Sent::Waiting;
        };
        let files: Vec<_> = self
            .open_drafts()
            .iter()
            .filter_map(|d| d.upload.send_attachment())
            .collect();
        self.worker.send(Command::Send {
            generation: self.generation,
            token: self.token.clone(),
            chat_token: chat_token.clone(),
            channel: channel.clone(),
            client_id: uuid::Uuid::new_v4().to_string(),
            text,
            broadcast: false,
            thread_root_id: None,
            reply_to_id: reply_to,
            spoiler_attachment_ids: files
                .iter()
                .filter(|f| f.spoiler)
                .map(|f| f.id.clone())
                .collect(),
            attachment_ids: files.into_iter().map(|f| f.id).collect(),
            gif: None,
            suppress_embeds: false,
        });
        // The message owns the files now: nothing is cancelled.
        self.drafts.retain(|d| !keys.contains(&d.upload.key));
        Sent::Sent
    }

    /// Opens a space (its first joined channel unless `channel` is given)
    /// or a channel. The open conversation stays until the new one loads.
    pub fn navigate(&mut self, space: Option<String>, channel: Option<String>) {
        self.navigation += 1;
        // A visit's unread snapshot loads beside its history, not after it.
        self.prefetch = None;
        if self.token.is_some() {
            self.next_visit += 1;
            self.prefetch = Some((self.navigation, self.next_visit, None));
            self.load_unread(Some(self.next_visit));
        }
        self.worker.send(Command::PrepareNavigation {
            generation: self.generation,
            navigation: self.navigation,
            token: self.token.clone(),
            space,
            channel,
            name: self.name.clone(),
            cached: None,
            visit: false,
            preferred: None,
        });
    }

    /// Sends `text` to the open channel, replying to message `reply_to`;
    /// with `thread`, into that root's thread (and to the channel too when
    /// its flag is set).
    pub fn send(&self, text: String, reply_to: Option<String>, thread: Option<(String, bool)>) {
        let (Some(channel), Some(chat_token)) = (&self.channel, &self.chat_token) else {
            return;
        };
        let client_id = uuid::Uuid::new_v4().to_string();
        self.worker.send(Command::Send {
            generation: self.generation,
            token: self.token.clone(),
            chat_token: chat_token.clone(),
            channel: channel.clone(),
            client_id,
            text,
            broadcast: thread.as_ref().is_some_and(|(_, b)| *b),
            thread_root_id: thread.map(|(root, _)| root),
            reply_to_id: reply_to,
            attachment_ids: Vec::new(),
            spoiler_attachment_ids: Vec::new(),
            gif: None,
            suppress_embeds: false,
        });
    }

    /// The open channel's gateway position.
    fn seq(&self) -> u64 {
        self.timeline.cursor().parse().unwrap_or(0)
    }

    /// After a gateway update for message `id`: the messages it (and any
    /// buffered ones it released) made visible, past sequence `old`, and the
    /// message's own new state. A gap the core cannot bridge reloads the
    /// channel, as the egui client does.
    fn publish(
        &mut self,
        old: u64,
        applied: Result<core::Apply, String>,
        id: &str,
        out: &mut Vec<Update>,
    ) {
        match applied {
            Ok(core::Apply::Resync) | Err(_) => return self.reload(),
            Ok(core::Apply::Buffered) => return,
            Ok(core::Apply::Applied | core::Apply::Duplicate) => {}
        }
        let mut root = None;
        for m in self.timeline.messages() {
            if m.id == id {
                root = m.thread_root_id.clone();
            }
            if m.seq.parse::<u64>().is_ok_and(|seq| seq > old) && m.id != id && self.in_channel(m) {
                out.push(Update::Message(Box::new(self.shown(m))));
            }
        }
        // A thread reply also changes its root's summary.
        for id in [Some(id), root.as_deref()].into_iter().flatten() {
            if let Some(m) = self
                .timeline
                .messages()
                .find(|m| m.id == id && self.in_channel(m))
            {
                out.push(Update::Message(Box::new(self.shown(m))));
            }
        }
        if self.in_thread(id) || root.is_some_and(|r| self.in_thread(&r)) {
            out.push(Update::Thread);
        }
    }

    /// Whether `m` is one of the channel's rows (not a thread-only reply or
    /// a message only a thread loaded).
    fn in_channel(&self, m: &core::Message) -> bool {
        m.is_channel_message() && !self.thread_only.contains(&m.id) && self.in_window(m)
    }

    /// Whether `m` falls inside the shown window (always, at the latest).
    fn in_window(&self, m: &core::Message) -> bool {
        let Some((start, end)) = self.window else {
            return true;
        };
        core::sequence(&m.seq).is_ok_and(|seq| seq >= start && end.is_none_or(|end| seq <= end))
    }

    /// Newer messages exist after the shown window: "Load newer messages"
    /// and "Back to latest messages" end the history.
    pub fn has_newer(&self) -> bool {
        self.window.is_some_and(|(_, end)| end.is_some())
    }

    /// View original on a reply's quote: the original's thread for a thread
    /// reply, the shown row when it is loaded, or the history around it.
    pub fn view_original(&mut self, id: &str, thread_root: Option<&str>) {
        if let Some(root) = thread_root {
            self.open_thread(root);
            return;
        }
        if self
            .timeline
            .messages()
            .any(|m| m.id == id && self.in_channel(m))
        {
            self.queued.push(Update::Focus(id.to_owned()));
            return;
        }
        self.load_context(id.to_owned(), false);
    }

    /// The next page after the shown window.
    pub fn load_newer(&mut self) {
        if self.context_loading {
            return;
        }
        if let Some((_, Some(end))) = self.window {
            self.load_context(end.to_string(), true);
        }
    }

    /// Leaves the window for the channel's latest history.
    pub fn back_to_latest(&mut self) {
        self.reload();
    }

    fn load_context(&mut self, anchor: String, newer: bool) {
        let Some(channel) = self.channel.clone() else {
            return;
        };
        // Older pages and context for the previous view drop.
        self.navigation += 1;
        self.loading_older = false;
        self.context_loading = true;
        self.worker.send(Command::LoadContext {
            generation: self.generation,
            request: self.navigation,
            token: self.token.clone(),
            channel,
            root: None,
            anchor,
            newer,
        });
        self.queued.push(Update::OlderState);
    }

    /// Whether message `id` is the open thread's root or one of its replies.
    fn in_thread(&self, id: &str) -> bool {
        let Some(t) = &self.thread else {
            return false;
        };
        id == t.root
            || self
                .timeline
                .messages()
                .any(|m| m.id == id && m.thread_root_id.as_deref() == Some(&t.root))
    }

    /// A message as the viewer sees it: your pending reactions drawn over
    /// the server's counts.
    fn shown(&self, m: &core::Message) -> core::Message {
        let mut m = self.mutations.project(m).into_owned();
        m.reactions = self
            .reactions
            .projected(&m.reactions, &m.id, self.me.as_deref());
        m
    }

    /// Message `id` again, after its pending reactions changed.
    fn republish(&mut self, id: &str) {
        if let Some(m) = self.timeline.messages().find(|m| m.id == id)
            && self.in_channel(m)
        {
            let shown = self.shown(m);
            self.queued.push(Update::Message(Box::new(shown)));
        }
        if self.in_thread(id) {
            self.queued.push(Update::Thread);
        }
    }

    /// You react with `emoji` to message `id` (or take it back): shown at
    /// once, sent through the shared queue.
    pub fn react(&mut self, id: &str, emoji: &str, active: bool) {
        if self.chat_token.is_none() || self.channel.is_none() {
            return;
        }
        self.reactions.set(id, emoji, active);
        self.send_next_reaction(id);
        self.republish(id);
    }

    /// Starts reading who reacted to message `id` unless its current
    /// reaction revision is known or already loading, as egui's hover card.
    pub fn load_reactors(&mut self, id: &str) {
        let Some(m) = self.timeline.messages().find(|m| m.id == id) else {
            return;
        };
        let revision = m.reaction_seq.clone().unwrap_or_else(|| "0".into());
        if self
            .reactors
            .get(id)
            .is_some_and(|cached| !cached.wants_request(&revision))
        {
            return;
        }
        let Some(channel) = self.channel.clone() else {
            return;
        };
        if self.reactors.len() >= REACTOR_CACHE_LIMIT && !self.reactors.contains_key(id) {
            self.reactors.clear();
        }
        let now = std::time::Instant::now();
        self.reactors
            .entry(id.to_owned())
            .and_modify(|cached| {
                cached.revision = revision.clone();
                cached.state = ReactorState::Loading(now);
            })
            .or_insert_with(|| ReactorCache {
                revision: revision.clone(),
                state: ReactorState::Loading(now),
                groups: Vec::new(),
            });
        self.worker.send(Command::LoadReactors {
            generation: self.generation,
            token: self.token.clone(),
            channel,
            message: id.to_owned(),
            revision,
        });
    }

    /// Who reacted to message `id` with `emoji`, as every client words it:
    /// known names (yours as "You"), else a count until they load.
    pub fn reactor_summary(&self, id: &str, emoji: &str) -> Option<String> {
        let m = self.timeline.messages().find(|m| m.id == id)?;
        let shown = self.shown(m);
        let reaction = shown.reactions.iter().find(|r| r.emoji == emoji)?;
        let me = self.me.as_deref();
        let name = caper_client::emoji::name(emoji);
        let groups = self
            .reactors
            .get(id)
            .map_or(&[][..], |cached| cached.groups.as_slice());
        Some(match core::reactor_list(groups, reaction, me) {
            Some(authors) => core::reactor_summary(&authors, me, name, emoji),
            None => core::reactor_fallback(&reaction.author_ids, me, name, emoji),
        })
    }

    /// A shown message as the server last sent it, by id.
    pub fn message(&self, id: &str) -> Option<core::Message> {
        self.timeline.messages().find(|m| m.id == id).cloned()
    }

    /// The signed-in, non-guest author of `m` may delete it (and edit it,
    /// unless it is a forward), as in the egui client.
    pub fn can_delete(&self, m: &core::Message) -> bool {
        !m.is_deleted()
            && self.chat_token.is_some()
            && self
                .author
                .as_ref()
                .is_some_and(|a| !a.is_guest && a.id == m.author.id)
    }

    pub fn can_edit(&self, m: &core::Message) -> bool {
        m.forward.is_none() && self.can_delete(m)
    }

    /// You can react and reply in the open channel.
    pub fn can_react(&self) -> bool {
        self.chat_token.is_some() && self.channel.is_some()
    }

    pub fn can_pin(&self, m: &core::Message) -> bool {
        !m.is_deleted() && self.chat_token.is_some()
    }

    pub fn pin_pending(&self, id: &str) -> bool {
        self.pending_pins.contains(id)
    }

    /// Saves `text` as the new content of `original`; returns the request
    /// its reply carries. The new text shows at once.
    pub fn edit(&mut self, original: &core::Message, text: String) -> u64 {
        self.request += 1;
        let (Some(chat_token), Some(_)) = (&self.chat_token, &self.channel) else {
            return self.request;
        };
        self.mutations
            .edits
            .insert(original.id.clone(), (text.clone(), original.revision));
        self.worker.send(Command::EditMessage {
            generation: self.generation,
            request: self.request,
            token: self.token.clone(),
            chat_token: chat_token.clone(),
            original: Box::new(original.clone()),
            text,
        });
        self.republish(&original.id);
        self.request
    }

    /// "Discard draft and load latest": reloads `original` from the server.
    pub fn reload_message(&mut self, original: &core::Message) -> u64 {
        self.request += 1;
        self.worker.send(Command::ReloadMessage {
            generation: self.generation,
            request: self.request,
            token: self.token.clone(),
            channel: original.channel_id.clone(),
            message: original.id.clone(),
        });
        self.request
    }

    /// Reads message `m`'s versions, newest first, or with `before` the
    /// page older than that revision; returns the request its reply carries.
    pub fn load_versions(&mut self, m: &core::Message, before: Option<u32>) -> u64 {
        self.request += 1;
        self.worker.send(Command::MessageVersions {
            generation: self.generation,
            request: self.request,
            token: self.token.clone(),
            channel: m.channel_id.clone(),
            message: m.id.clone(),
            before,
        });
        self.request
    }

    pub fn delete(&mut self, m: &core::Message) -> u64 {
        self.request += 1;
        if let Some(chat_token) = &self.chat_token {
            self.worker.send(Command::DeleteMessage {
                generation: self.generation,
                request: self.request,
                token: self.token.clone(),
                chat_token: chat_token.clone(),
                channel: m.channel_id.clone(),
                message: m.id.clone(),
            });
        }
        self.request
    }

    /// Pins or unpins `m`, shown at once.
    pub fn pin(&mut self, m: &core::Message, active: bool) {
        let (Some(chat_token), Some(channel), Some(author)) =
            (&self.chat_token, &self.channel, &self.author)
        else {
            return;
        };
        if self.pending_pins.contains(&m.id) || m.channel_id != *channel {
            return;
        }
        let pin = active.then(|| core::Pin {
            author: author.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
        });
        self.mutations.pins.insert(m.id.clone(), (m.clone(), pin));
        self.pending_pins.insert(m.id.clone());
        self.worker.send(Command::Pin {
            generation: self.generation,
            token: self.token.clone(),
            chat_token: chat_token.clone(),
            channel: channel.clone(),
            message: m.id.clone(),
            active,
        });
        self.republish(&m.id);
    }

    /// Retry under a failed reaction.
    pub fn retry_reaction(&mut self, id: &str) {
        if let Some((emoji, active)) = self.reactions.retry(id) {
            self.react(id, &emoji, active);
        }
    }

    /// Dismiss under a failed reaction.
    pub fn dismiss_reaction(&mut self, id: &str) {
        self.reactions.dismiss(id);
        self.republish(id);
    }

    fn send_next_reaction(&mut self, id: &str) {
        let (Some(channel), Some(chat_token)) = (&self.channel, &self.chat_token) else {
            return;
        };
        if let Some(send) = self.reactions.next(id) {
            self.worker.send(Command::React {
                generation: self.generation,
                token: self.token.clone(),
                chat_token: chat_token.clone(),
                channel: channel.clone(),
                message: send.message,
                emoji: send.emoji,
                active: send.active,
            });
        }
    }

    /// What the top of the history shows now.
    pub fn older(&self) -> Older {
        if self.loading_older {
            Older::Loading
        } else if self.older_error.is_some() {
            Older::Failed
        } else if self.has_more {
            Older::More
        } else {
            Older::Start
        }
    }

    /// Asks for the page before the oldest shown message; false when one is
    /// in flight or there is none.
    pub fn load_older(&mut self) -> bool {
        if self.loading_older || !self.has_more {
            return false;
        }
        let Some(channel) = self.channel.clone() else {
            return false;
        };
        let Some(before) = self
            .timeline
            .messages()
            .find(|m| self.in_channel(m))
            .map(|m| m.seq.clone())
        else {
            return false;
        };
        self.loading_older = true;
        self.older_error = None;
        self.worker.send(Command::LoadOlder {
            generation: self.generation,
            request: self.navigation,
            token: self.token.clone(),
            channel,
            before,
        });
        self.queued.push(Update::OlderState);
        true
    }

    /// The shown history as the viewer sees it, rebuilt from the timeline.
    fn rebuilt(&self) -> Option<(Conversation, Ids)> {
        let (detail, channel) = (self.detail.as_ref()?, self.channel.as_ref()?);
        let spaces = self.spaces.clone().unwrap_or(core::Spaces {
            spaces: Vec::new(),
            invitations: Vec::new(),
            limits: None,
        });
        let messages: Vec<core::Message> = self
            .timeline
            .messages()
            .filter(|m| !self.thread_only.contains(&m.id) && self.in_window(m))
            .map(|m| self.shown(m))
            .collect();
        Some(conversation(
            &spaces,
            detail,
            channel,
            &messages,
            self.me.as_deref(),
            false,
            self.unread_row().as_deref(),
            self.direct_head(),
        ))
    }

    /// Opens message `root`'s thread beside the channel.
    pub fn open_thread(&mut self, root: &str) {
        if self.thread.as_ref().is_some_and(|t| t.root == root) {
            return;
        }
        self.thread = Some(Thread {
            root: root.to_owned(),
            request: 0,
            loading: true,
            has_more: false,
            before: None,
            error: None,
            retry_older: false,
        });
        self.load_thread(false);
    }

    pub fn close_thread(&mut self) {
        self.thread_request += 1;
        if self.thread.take().is_some() {
            self.queued.push(Update::Thread);
        }
    }

    /// Loads the open thread's latest replies, or (with `older`) the page
    /// before the oldest shown one.
    pub fn load_thread(&mut self, older: bool) {
        let (Some(channel), Some(t)) = (self.channel.clone(), self.thread.as_mut()) else {
            return;
        };
        self.thread_request += 1;
        t.request = self.thread_request;
        t.loading = true;
        t.error = None;
        t.retry_older = older;
        self.worker.send(Command::LoadThread {
            generation: self.generation,
            request: t.request,
            token: self.token.clone(),
            channel,
            root: t.root.clone(),
            before: if older { t.before.clone() } else { None },
        });
        self.queued.push(Update::Thread);
    }

    /// Retry after a failed thread load repeats the page that failed.
    pub fn retry_thread(&mut self) {
        let older = self.thread.as_ref().is_some_and(|t| t.retry_older);
        self.load_thread(older);
    }

    /// The open thread's root and replies as rows (deleted replies left
    /// out, as in the egui client), with the root's reply count.
    pub fn thread_view(&self) -> Option<(Conversation, Ids, Option<u64>)> {
        let t = self.thread.as_ref()?;
        let (detail, channel) = (self.detail.as_ref()?, self.channel.as_ref()?);
        let spaces = core::Spaces {
            spaces: Vec::new(),
            invitations: Vec::new(),
            limits: None,
        };
        let root = self.timeline.messages().find(|m| m.id == t.root);
        let count = root
            .and_then(|m| m.thread.as_ref())
            .map(|s| s.reply_count)
            .filter(|n| *n > 0);
        let messages: Vec<core::Message> = self
            .timeline
            .messages()
            .filter(|m| {
                !m.is_deleted() && (m.id == t.root || m.thread_root_id.as_deref() == Some(&t.root))
            })
            .map(|m| self.shown(m))
            .collect();
        let (c, ids) = conversation(
            &spaces,
            detail,
            channel,
            &messages,
            self.me.as_deref(),
            true,
            None,
            self.direct_head(),
        );
        Some((c, ids, count))
    }

    /// Reloads the open channel from history.
    fn reload(&mut self) {
        if let Some(channel) = self.channel.clone() {
            let space = if self.direct.is_some() {
                None
            } else {
                self.space.clone()
            };
            self.navigate(space, Some(channel));
        }
    }

    /// The visible conversation, so the worker suppresses its notifications.
    pub fn showing(&mut self, visible: bool) {
        self.worker
            .showing(self.channel.as_deref().filter(|_| visible));
        // Coming back to the window reads what arrived meanwhile.
        let gained = visible && !self.foreground;
        self.foreground = visible;
        if gained {
            self.mark_read();
        }
    }

    /// A new visit to the open channel: its unread range loads before
    /// anything is marked read, as in the egui client.
    /// Takes this navigation's prefetched snapshot when there is one.
    fn begin_visit(&mut self) {
        self.visit_channel = self.channel.clone();
        self.unread = None;
        self.unread_pending = false;
        let prefetch = self
            .prefetch
            .take()
            .filter(|(navigation, ..)| *navigation == self.navigation);
        if let Some((_, visit, ranges)) = prefetch {
            self.visit = visit;
            self.unread_pending = true;
            if let Some(ranges) = ranges {
                self.apply_visit(&ranges);
            }
        } else if self.token.is_some() {
            self.next_visit += 1;
            self.visit = self.next_visit;
            self.unread_pending = true;
            self.load_unread(Some(self.visit));
        }
    }

    /// The visit's snapshot arrived: its "New messages" range; the
    /// channel may be marked read from now on.
    fn apply_visit(&mut self, ranges: &HashMap<String, (u64, u64)>) {
        self.unread_pending = false;
        self.unread = self
            .visit_channel
            .as_ref()
            .and_then(|c| ranges.get(c).copied());
    }

    /// Reads the account's unread state again (the badges); while a visit
    /// waits for its snapshot, this is that snapshot too.
    pub fn refresh_unread(&mut self) {
        self.load_unread(self.unread_pending.then_some(self.visit));
    }

    /// The inbox opened (egui's Notifications button): its first page
    /// loads.
    pub fn open_activity(&mut self) {
        self.activity.shown = true;
        self.load_activity(false, false);
    }

    pub fn close_activity(&mut self) {
        self.activity.shown = false;
    }

    /// egui's `load_activity_page`: the first page (`more`: the next older
    /// one); as a poll, the loaded rows again. A filter change keeps the
    /// shown rows, dimmed, until its page answers.
    pub fn load_activity(&mut self, more: bool, poll: bool) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let a = &mut self.activity;
        if poll && (a.loading || !a.reads.is_empty()) || more && (a.loading || a.cursor.is_none()) {
            return;
        }
        a.request += 1;
        a.loading = true;
        a.error = None;
        a.refreshed = Some(Instant::now());
        if !more && !poll {
            a.stale = !a.items.is_empty();
        }
        self.worker.send(Command::LoadActivity {
            epoch: self.account_epoch,
            request: a.request,
            token,
            filter: a.filter,
            unread: a.unread_only,
            cursor: if more { a.cursor.clone() } else { None },
            through: if poll { a.items.last().cloned() } else { None },
            more,
        });
    }

    /// The open inbox's 15 s refresh, or sooner after a notification.
    pub fn poll_activity(&mut self) {
        if self.activity.due() {
            self.load_activity(false, true);
        }
    }

    pub fn set_activity_filter(&mut self, filter: caper_client::activity::Filter) {
        if self.activity.filter != filter {
            self.activity.filter = filter;
            self.load_activity(false, false);
        }
    }

    pub fn toggle_unread_only(&mut self) {
        self.activity.unread_only = !self.activity.unread_only;
        self.load_activity(false, false);
    }

    /// Opens (`open`) or marks read the inbox's shown row `k`, as egui:
    /// the row reads at once, and opening waits for the read. Reading
    /// never accepts an invitation.
    pub fn activity_action(&mut self, k: usize, open: bool) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let Some(item) = self.activity.visible_items().into_iter().nth(k) else {
            return;
        };
        if open && self.activity.opening.is_some() {
            return;
        }
        self.unread_revision += 1;
        // A refresh in flight could bring the row back unread; a filter's
        // page replacing dimmed rows keeps going.
        if !self.activity.stale {
            self.activity.request += 1;
            self.activity.loading = false;
        }
        if open {
            self.activity.opening = Some(item.id.clone());
        }
        self.activity.error = None;
        self.activity.mark_read(&item);
        self.worker.send(Command::ReadActivity {
            epoch: self.account_epoch,
            navigation: self.navigation,
            token,
            item,
            open,
        });
    }

    /// egui's `open_activity_item`: the inbox closes over the notification's
    /// conversation, at its message. Invitations aren't answered here yet.
    fn open_activity_item(&mut self, item: caper_client::activity::Item, out: &mut Vec<Update>) {
        self.activity.shown = false;
        if item.kind == "space.invitation" || item.kind == "channel.invitation" {
            out.push(Update::Warning(
                "Invitations can't be answered in this app yet.".into(),
            ));
            return;
        }
        let Some(channel) = item.channel().map(str::to_owned) else {
            out.push(Update::Warning(
                "This notification is no longer available.".into(),
            ));
            return;
        };
        let target = item.message_id.map(|m| (m, item.thread_root_id));
        if self.channel.as_ref() == Some(&channel) {
            if let Some((message, thread)) = target {
                self.focus_target(&message, thread.as_deref(), out);
            }
            return;
        }
        self.navigate(item.space_id, Some(channel));
        self.focus = target.map(|(message, thread)| (self.navigation, message, thread));
    }

    /// Shows message `id` of the open channel: its thread for a reply in
    /// one, else the row (or the history around it).
    fn focus_target(&mut self, id: &str, thread: Option<&str>, out: &mut Vec<Update>) {
        self.view_original(id, thread);
        out.append(&mut self.queued);
    }

    /// Who is in voice in channel `id` (egui's `voice_people`): your call's
    /// own roster there, else the gateway's.
    pub fn voice_names(&self, id: &str) -> Vec<&str> {
        if self.own_call(id) {
            return self
                .voice
                .participants
                .iter()
                .map(|p| p.name.as_str())
                .collect();
        }
        self.rosters.get(id).map_or_else(Vec::new, |(people, _)| {
            people.iter().map(|p| p.name.as_str()).collect()
        })
    }

    /// Whether your call (not a failed one) is in channel `id`.
    fn own_call(&self, id: &str) -> bool {
        self.voice.state.active_channel() == Some(id)
            && !matches!(self.voice.state.phase, Phase::Failed(_))
    }

    /// Whether a call is under way anywhere.
    pub fn in_voice(&self) -> bool {
        !matches!(self.voice.state.phase, Phase::Idle | Phase::Failed(_))
    }

    /// Why channel `id`'s voice can't be joined now (egui's
    /// `join_unavailable`).
    pub fn join_unavailable(&self, id: &str) -> Option<&'static str> {
        match self.availability.get(id) {
            Some(true) => None,
            Some(false) => Some("Joining is not available at this time."),
            None => Some("Checking voice availability\u{2026}"),
        }
    }

    /// Where a join of channel `id` goes: a joined channel of the open
    /// space, signed in and available.
    fn voice_target(&self, id: &str) -> Option<(CallContext, String)> {
        let detail = self.detail.as_ref()?;
        let target = detail.channels.iter().find(|c| c.id == id && c.joined)?;
        if self.token.is_none() || detail.space.demo || self.join_unavailable(id).is_some() {
            return None;
        }
        Some((
            CallContext {
                channel_id: target.id.clone(),
                channel_name: target.name.clone(),
                space_name: detail.space.name.clone(),
            },
            detail.space.id.clone(),
        ))
    }

    /// egui's `voice_action`: what voice offers in channel `id` now.
    pub fn voice_action(&self, id: &str) -> VoiceAction {
        let own = self.own_call(id);
        let authorizing = self.pending_join.as_ref().map(|(c, ..)| c.as_str());
        let connecting = matches!(
            self.voice.state.phase,
            Phase::Joining(_) | Phase::Reconnecting(_)
        );
        if own && matches!(self.voice.state.phase, Phase::Connected(_)) {
            VoiceAction::Leave
        } else if authorizing == Some(id) || own && connecting {
            VoiceAction::Joining
        } else {
            VoiceAction::Join {
                switching: self.in_voice(),
                enabled: authorizing.is_none() && !connecting && self.voice_target(id).is_some(),
            }
        }
    }

    /// Web prepares a join as the pointer nears Join; the API keeps it 8 s,
    /// so it is asked again at most every 4 s.
    pub fn prepare_voice(&mut self, id: &str) {
        let (Some(token), Some(_)) = (self.token.clone(), self.voice_target(id)) else {
            return;
        };
        if self.own_call(id)
            || self
                .prepared
                .get(id)
                .is_some_and(|at| at.elapsed() < Duration::from_secs(4))
        {
            return;
        }
        self.prepared.insert(id.to_owned(), Instant::now());
        self.worker.send(Command::PrepareVoice {
            token,
            channel: id.to_owned(),
        });
    }

    /// Join or Switch (egui's `join_voice_channel`): the space membership
    /// is checked, then the call joins.
    pub fn join_voice(&mut self, id: &str) {
        if self
            .pending_join
            .as_ref()
            .is_some_and(|(_, generation, _)| *generation == self.voice.state.generation)
            || matches!(
                self.voice.state.phase,
                Phase::Joining(_) | Phase::Reconnecting(_)
            )
            || self.own_call(id)
        {
            return;
        }
        let (Some(token), Some((_, space))) = (self.token.clone(), self.voice_target(id)) else {
            return;
        };
        let clicked = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.join_request += 1;
        self.pending_join = Some((id.to_owned(), self.voice.state.generation, clicked));
        self.worker.send(Command::CheckVoice {
            request: self.join_request,
            voice_generation: self.voice.state.generation,
            token,
            space,
            channel: id.to_owned(),
        });
        self.queued.push(Update::Voice);
    }

    fn accept_join(
        &mut self,
        request: u64,
        voice_generation: u64,
        space: &str,
        channel: &str,
        result: Result<(), caper_client::worker::LoadError>,
    ) {
        if request != self.join_request {
            return;
        }
        let Some((_, _, clicked)) = self.pending_join.take() else {
            return;
        };
        if voice_generation != self.voice.state.generation {
            return;
        }
        let Some((context, target)) = self.voice_target(channel) else {
            return;
        };
        if target != space {
            return;
        }
        match result {
            Ok(()) => {
                let started = self.rosters.get(channel).and_then(|(_, s)| *s);
                let name = Some(self.account_name.clone())
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| "Guest".into());
                self.voice.join(
                    context,
                    Some(target),
                    self.token.clone(),
                    name,
                    clicked,
                    started,
                );
            }
            Err(e) => {
                if e.access_denied {
                    self.availability.insert(channel.to_owned(), false);
                    self.rosters.remove(channel);
                }
                self.voice.error = Some(e.message);
            }
        }
    }

    /// Leave, or Cancel while joining; only a connected call plays the
    /// disconnect sound.
    pub fn leave_voice(&mut self) {
        if matches!(self.voice.state.phase, Phase::Connected(_)) {
            self.effects.play(Effect::Disconnect);
        }
        self.pending_join = None;
        self.voice.leave();
        self.queued.push(Update::Voice);
    }

    pub fn toggle_mute(&mut self) {
        let muted = self.voice.state.audio.muted;
        self.voice.command(VoiceOperation::Mute(!muted));
        self.effects.toggle(muted);
        self.queued.push(Update::Voice);
    }

    pub fn toggle_deafen(&mut self) {
        let deafened = self.voice.state.audio.deafened;
        self.voice.command(VoiceOperation::Deafen(!deafened));
        self.effects.toggle(deafened);
        self.queued.push(Update::Voice);
    }

    pub fn dismiss_voice_error(&mut self) {
        self.voice.error = None;
        self.queued.push(Update::Voice);
    }

    /// Your call as its card shows it, while one is under way.
    pub fn call_view(&self) -> Option<CallView> {
        let (Phase::Joining(call) | Phase::Connected(call) | Phase::Reconnecting(call)) =
            &self.voice.state.phase
        else {
            return None;
        };
        let names: Vec<&str> = self
            .voice
            .participants
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
        Some(CallView {
            status: match self.voice.state.phase {
                Phase::Connected(_) => None,
                Phase::Joining(_) => Some("Connecting\u{2026}"),
                _ => Some("Reconnecting\u{2026}"),
            },
            channel: call.channel_name.clone(),
            space: call.space_name.clone(),
            names: caper_client::voice::room_names(&names),
            count: names.len(),
            timer: self
                .voice
                .session_started_at
                .filter(|_| matches!(self.voice.state.phase, Phase::Connected(_)))
                .map(|started| caper_client::voice::session_duration(started, now)),
            joining: matches!(self.voice.state.phase, Phase::Joining(_)),
            muted: self.voice.state.audio.muted,
            deafened: self.voice.state.audio.deafened,
        })
    }

    /// Reads the call's reports (egui's `receive`): a connection plays the
    /// join sound, others arriving or leaving their chimes. Whether what
    /// voice shows changed.
    fn receive_voice(&mut self) -> bool {
        self.voice.receive();
        if self
            .pending_join
            .as_ref()
            .is_some_and(|(_, generation, _)| *generation != self.voice.state.generation)
        {
            self.pending_join = None;
        }
        let generation = self.voice.state.generation;
        if matches!(self.voice.state.phase, Phase::Connected(_)) {
            if self.announced != Some(generation) {
                self.announced = Some(generation);
                self.effects.play(Effect::Join);
            }
            let me = &self.voice.self_id;
            let others: Vec<String> = self
                .voice
                .participants
                .iter()
                .filter(|p| p.id != *me)
                .map(|p| p.id.clone())
                .collect();
            if let Some((g, before)) = &self.others
                && *g == generation
                && !me.is_empty()
            {
                if before.iter().any(|id| !others.contains(id)) {
                    self.effects.play(Effect::Leave);
                } else if others.iter().any(|id| !before.contains(id)) {
                    self.effects.play(Effect::Join);
                }
            }
            self.others = Some((generation, others));
        } else {
            self.others = None;
        }
        // And what the Audio test shows.
        let v = &self.voice;
        let seen = format!(
            "{:?}{:?}{:?}{:?}{:?}{:?}",
            self.voice.state.phase,
            self.voice.state.audio,
            self.voice
                .participants
                .iter()
                .map(|p| (&p.id, &p.name, p.muted, p.deafened))
                .collect::<Vec<_>>(),
            self.voice.error,
            self.pending_join,
            (
                (&v.microphone, &v.microphone_error, v.speaker_testing),
                (v.speaker_failed, &v.inputs, &v.outputs, &v.preferences),
                (&v.default_input, &v.default_output, &v.device_error),
                v.refreshing_devices,
            ),
        );
        if seen == self.voice_seen {
            return false;
        }
        self.voice_seen = seen;
        true
    }

    /// What the inbox card shows (egui's `activity_dialog` rows).
    pub fn inbox_snapshot(&self) -> crate::settings::InboxSnap {
        let a = &self.activity;
        let now = chrono::Utc::now();
        crate::settings::InboxSnap {
            filter: caper_client::activity::Filter::ALL
                .iter()
                .position(|f| *f == a.filter)
                .unwrap_or(0),
            unread_only: a.unread_only,
            unread_count: a.unread_count,
            rows: a
                .visible_items()
                .iter()
                .map(|item| crate::settings::InboxRow {
                    actor: item.actor_name.clone(),
                    unread: item.unread,
                    opening: a.opening.as_ref() == Some(&item.id),
                    time: caper_client::feedback::relative_time(&item.created_at, now),
                    description: match item.kind.as_str() {
                        "reaction" => Some(format!(
                            "Reacted {} to your message",
                            item.emoji.as_deref().unwrap_or_default()
                        )),
                        "direct.request" => Some("Wants to send you messages".into()),
                        "space.invitation" => Some("Invited you to a space".into()),
                        "channel.invitation" => Some("Invited you to a private channel".into()),
                        _ => None,
                    },
                    body: item.body.clone(),
                    place: [item.space_name.as_deref(), item.channel_name.as_deref()]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" \u{b7} "),
                })
                .collect(),
            loading: a.loading,
            stale: a.stale,
            error: a.error.clone(),
            more: a.cursor.is_some(),
            retry: !a.loading && a.reads.is_empty(),
        }
    }

    /// Reads the unread state, as the snapshot of `visit` when given.
    fn load_unread(&mut self, visit: Option<u64>) {
        self.unread_due = None;
        self.check_klipy();
        self.load_directs();
        let Some(token) = self.token.clone() else {
            return;
        };
        self.unread_revision += 1;
        self.worker.send(Command::LoadUnread {
            epoch: 0,
            revision: self.unread_revision,
            visit,
            token,
        });
    }

    /// The shown message the "New messages" divider goes above: the first
    /// unread one from someone else, once the history reaches back to the
    /// boundary, and not in a View original window.
    fn unread_row(&self) -> Option<String> {
        let (read, last) = self.unread?;
        if self.window.is_some() {
            return None;
        }
        let first = self.timeline.messages().next()?;
        if self.has_more && !core::sequence(&first.seq).is_ok_and(|s| s <= read.saturating_add(1)) {
            return None;
        }
        self.timeline
            .messages()
            .find(|m| {
                self.in_channel(m)
                    && core::sequence(&m.seq).is_ok_and(|s| s > read && s <= last)
                    && self.me.as_deref() != Some(m.author.id.as_str())
            })
            .map(|m| m.id.clone())
    }

    /// Marks the open channel read up to its newest message while you see
    /// it: not before the visit's unread range loaded, nor in the past.
    fn mark_read(&mut self) {
        if !self.foreground || self.unread_pending || self.window.is_some() {
            return;
        }
        let (Some(space), Some(channel), Some(token)) =
            (self.space.clone(), self.channel.clone(), self.token.clone())
        else {
            return;
        };
        let seq = self.timeline.cursor();
        if self.direct.as_ref() == Some(&channel) {
            // A DM's cursor: never backwards, and a request is not read by
            // looking at it.
            let Some(d) = self.directs.iter_mut().find(|d| d.id == channel) else {
                return;
            };
            if d.status == core::DirectStatus::Incoming
                || core::sequence(&d.read_seq).unwrap_or(0) >= core::sequence(&seq).unwrap_or(0)
            {
                return;
            }
            d.read_seq.clone_from(&seq);
            self.queued.push(Update::Badges);
            self.worker.send(Command::ReadDirect {
                token,
                id: channel,
                seq,
            });
            return;
        }
        let read = self
            .reads
            .get(&channel)
            .and_then(|r| core::sequence(r).ok());
        if core::sequence(&seq).unwrap_or(0) <= read.unwrap_or(0) {
            return;
        }
        self.worker.send(Command::ReadChannel {
            epoch: 0,
            token,
            space,
            channel,
            seq,
        });
    }

    /// Handles the worker's pending events; returns what the UI must show.
    pub fn drain(&mut self) -> Vec<Update> {
        // A notification's unread change, once its second has passed (the
        // window's timer rounds to whole milliseconds).
        if self
            .unread_due
            .is_some_and(|due| due <= Instant::now() + Duration::from_millis(15))
        {
            self.refresh_unread();
        }
        let voice = self.receive_voice();
        let events: Vec<Event> = self.worker.events.try_iter().collect();
        let mut out = std::mem::take(&mut self.queued);
        if voice {
            out.push(Update::Voice);
        }
        for event in events {
            self.handle(event, &mut out);
        }
        out
    }

    fn handle(&mut self, event: Event, out: &mut Vec<Update>) {
        let current = self.generation;
        match event {
            Event::Restored { generation, result } if generation == current => match result {
                Ok(Some((token, account, spaces))) => self.open(token, account, spaces, out),
                Ok(None) => out.push(Update::SignedOut(None)),
                Err(e) => out.push(Update::SignedOut(Some(e))),
            },
            Event::CodeRequested { generation, result } if generation == current => {
                out.push(Update::CodeRequested(result));
            }
            Event::Verified {
                generation,
                result,
                attempts_remaining,
            } if generation == current => match result {
                Ok((token, account, spaces)) => {
                    out.push(Update::SignedIn);
                    self.worker.send(Command::PersistCredential {
                        generation: current,
                        token: token.clone(),
                    });
                    self.open(token, account, spaces, out);
                }
                Err(e) => out.push(Update::VerifyFailed(e, attempts_remaining)),
            },
            Event::Credential {
                generation,
                result: Err(e),
            } if generation == current => {
                out.push(Update::Failed(format!("Could not save the session: {e}")));
            }
            Event::NavigationPrepared {
                generation,
                navigation,
                result,
            } if generation == current && navigation == self.navigation => match result {
                Ok(prepared) => {
                    // Events still in flight for the previous conversation
                    // (its gateway, sends) carry the old generation and drop.
                    self.generation += 1;
                    let current = self.generation;
                    // A DM has no space of its own: the open space's
                    // channels stay beside it, as in egui.
                    let direct = prepared.detail.is_none();
                    let (Some(detail), Some((history, session))) = (
                        prepared.detail.or_else(|| self.detail.clone()),
                        prepared.conversation,
                    ) else {
                        out.push(Update::Failed("No joined channel in this space.".into()));
                        return;
                    };
                    self.reactions.clear();
                    self.mutations = core::MessageMutations::default();
                    self.pending_pins.clear();
                    self.has_more = history.has_more;
                    self.loading_older = false;
                    self.older_error = None;
                    self.thread = None;
                    self.thread_request += 1;
                    self.thread_only.clear();
                    self.window = None;
                    self.context_loading = false;
                    self.reactors.clear();
                    self.detail = Some(detail.clone());
                    let mut timeline = core::Timeline::default();
                    if let Err(e) = timeline.reset(history.messages, &history.cursor) {
                        out.push(Update::Failed(e));
                        return;
                    }
                    match session {
                        Ok(s) => {
                            self.chat_token = Some(s.token);
                            self.me = Some(s.author.id.clone());
                            self.author = Some(s.author);
                        }
                        Err(e) => out.push(Update::Failed(e)),
                    }
                    let channel = history.channel.id.clone();
                    self.channel = Some(channel.clone());
                    self.space = Some(detail.space.id.clone());
                    self.direct = direct.then(|| channel.clone());
                    self.direct_name.clone_from(&history.channel.name);
                    let messages: Vec<core::Message> = timeline.messages().cloned().collect();
                    let cursor = timeline.cursor();
                    self.timeline = timeline;
                    if self.visit_channel.as_ref() != self.channel.as_ref() {
                        self.begin_visit();
                    }
                    let unread = self.unread_row();
                    let spaces = self.spaces.as_ref().map_or_else(
                        || core::Spaces {
                            spaces: Vec::new(),
                            invitations: Vec::new(),
                            limits: None,
                        },
                        Clone::clone,
                    );
                    let (conversation, ids) = conversation(
                        &spaces,
                        &detail,
                        &channel,
                        &messages,
                        self.me.as_deref(),
                        false,
                        unread.as_deref(),
                        self.direct_head(),
                    );
                    // Whether each joined channel's voice can be joined,
                    // once per account (egui's `refresh_media_status`).
                    if !direct && let Some(token) = self.token.clone() {
                        for c in detail.channels.iter().filter(|c| c.joined) {
                            if self.status_asked.insert(c.id.clone()) {
                                self.worker.send(Command::MediaStatus {
                                    generation: self.account_epoch,
                                    root: c.id.clone(),
                                    token: Some(token.clone()),
                                    channel: Some(c.id.clone()),
                                });
                            }
                        }
                    }
                    // The space's joined channels' voice rosters, as egui
                    // watches them; none in a request or a blocked DM.
                    self.rosters.clear();
                    let quiet = direct
                        && self.directs.iter().any(|d| {
                            d.id == channel
                                && (d.status == core::DirectStatus::Incoming
                                    || d.blocked
                                    || self.is_blocked(&d.peer.id))
                        });
                    let media = gateway::MediaWatch {
                        epoch: current,
                        channels: detail
                            .channels
                            .iter()
                            .filter(|c| c.joined && !quiet)
                            .take(gateway::MAX_MEDIA_CHANNELS)
                            .map(|c| gateway::MediaChannel {
                                id: c.id.clone(),
                                demo: detail.space.demo,
                            })
                            .collect(),
                    };
                    out.push(Update::Loaded(Box::new((conversation, ids))));
                    if let Some((_, message, thread)) =
                        self.focus.take().filter(|(n, ..)| *n == navigation)
                    {
                        self.focus_target(&message, thread.as_deref(), out);
                    }
                    self.mark_read();
                    self.worker.send(Command::Connect {
                        generation: current,
                        token: self.token.clone(),
                        channel,
                        cursor,
                        presence: (!direct).then(|| (detail.space.id.clone(), Vec::new())),
                        media,
                    });
                }
                Err(e) => out.push(Update::Failed(e.message)),
            },
            Event::OlderLoaded {
                generation,
                request,
                channel,
                result,
            } if generation == current
                && request == self.navigation
                && self.channel.as_deref() == Some(channel.as_str()) =>
            {
                self.loading_older = false;
                match result {
                    Ok(page) => {
                        self.has_more = page.has_more;
                        for m in &page.messages {
                            self.thread_only.remove(&m.id);
                        }
                        // The window reaches back over the new page.
                        if let (Some((start, _)), Some(first)) =
                            (&mut self.window, page.messages.first())
                        {
                            *start = core::sequence(&first.seq).unwrap_or(*start).min(*start);
                        }
                        if self.timeline.prepend(page.messages).is_err() {
                            return self.reload();
                        }
                        match self.rebuilt() {
                            Some(rebuilt) => out.push(Update::Older(Box::new(rebuilt))),
                            None => out.push(Update::OlderState),
                        }
                    }
                    Err(e) if e.access_denied => self.reload(),
                    Err(e) => {
                        self.older_error = Some(e.message);
                        out.push(Update::OlderState);
                    }
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
            } if generation == current
                && request == self.navigation
                && self.channel.as_deref() == Some(channel.as_str()) =>
            {
                self.context_loading = false;
                let page = match result {
                    Ok(page)
                        if page
                            .messages
                            .iter()
                            .all(|m| m.channel_id == channel && m.is_channel_message())
                            && (newer || page.messages.iter().any(|m| m.id == anchor)) =>
                    {
                        page
                    }
                    Ok(_) => {
                        out.push(Update::Failed(
                            "Caper returned invalid message context.".into(),
                        ));
                        out.push(Update::OlderState);
                        return;
                    }
                    Err(e) if e.access_denied => return self.reload(),
                    Err(e) => {
                        out.push(Update::Failed(e.message));
                        out.push(Update::OlderState);
                        return;
                    }
                };
                let seq = |m: Option<&core::Message>| m.and_then(|m| core::sequence(&m.seq).ok());
                let end = seq(page.messages.last()).filter(|_| page.has_newer);
                let start = seq(page.messages.first());
                for m in &page.messages {
                    self.thread_only.remove(&m.id);
                }
                if self.timeline.prepend(page.messages).is_err() {
                    return self.reload();
                }
                if newer {
                    if let Some((_, e)) = &mut self.window {
                        *e = end;
                    }
                } else {
                    self.window = Some((start.unwrap_or(0), end));
                    self.has_more = page.has_more;
                    self.older_error = None;
                }
                match self.rebuilt() {
                    Some(rebuilt) if newer => out.push(Update::Newer(Box::new(rebuilt))),
                    Some(rebuilt) => out.push(Update::Context(Box::new(rebuilt), anchor)),
                    None => out.push(Update::OlderState),
                }
            }
            Event::ThreadLoaded {
                generation,
                request,
                channel,
                root,
                result,
            } if generation == current
                && self.channel.as_deref() == Some(channel.as_str())
                && self
                    .thread
                    .as_ref()
                    .is_some_and(|t| t.request == request && t.root == root) =>
            {
                match result {
                    Ok(page)
                        if page.root.id == root
                            && page.root.channel_id == channel
                            && page.root.thread_root_id.is_none()
                            && page.messages.iter().all(|m| {
                                m.channel_id == channel
                                    && m.thread_root_id.as_deref() == Some(root.as_str())
                            }) =>
                    {
                        let before = page.messages.first().map(|m| m.seq.clone());
                        let rows: Vec<core::Message> =
                            std::iter::once(page.root).chain(page.messages).collect();
                        let loaded: HashSet<String> =
                            self.timeline.messages().map(|m| m.id.clone()).collect();
                        self.thread_only.extend(
                            rows.iter()
                                .filter(|m| m.is_channel_message() && !loaded.contains(&m.id))
                                .map(|m| m.id.clone()),
                        );
                        let merged = self.timeline.prepend(rows);
                        if let Some(t) = &mut self.thread {
                            t.loading = false;
                            t.has_more = page.has_more;
                            t.before = before.or(t.before.take());
                            if let Err(e) = merged {
                                t.error = Some(e);
                            }
                        }
                    }
                    Ok(_) => {
                        if let Some(t) = &mut self.thread {
                            t.loading = false;
                            t.error = Some("This thread could not be loaded.".into());
                        }
                    }
                    Err(e) if e.access_denied => {
                        self.thread = None;
                        out.push(Update::Failed(e.message));
                    }
                    Err(e) => {
                        if let Some(t) = &mut self.thread {
                            t.loading = false;
                            t.error = Some(e.message);
                        }
                    }
                }
                out.push(Update::Thread);
            }
            Event::Sent {
                generation,
                client_id,
                result,
                ..
            } if generation == current => match result {
                Ok(m) => {
                    // Best effort, once KLIPY's GIF is really sent.
                    if let Some(share) = self.shares.remove(&client_id) {
                        share.send();
                    }
                    if self.timeline.merge_sent(m.clone()).is_err() {
                        self.reload();
                    }
                    let thread = m
                        .thread_root_id
                        .as_deref()
                        .is_some_and(|r| self.in_thread(r));
                    if m.is_channel_message() && self.has_newer() {
                        // As in egui: your message shows at the latest history.
                        self.reload();
                    } else if self.in_channel(&m) {
                        out.push(Update::Message(Box::new(m)));
                    }
                    if thread {
                        out.push(Update::Thread);
                    }
                }
                Err(f) => {
                    self.shares.remove(&client_id);
                    out.push(Update::Failed(f.message));
                }
            },
            Event::Gateway(gateway::GatewayEvent::Message {
                generation,
                channel,
                message,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                let id = message.id.clone();
                let old = self.seq();
                let applied = self.timeline.apply(*message);
                self.publish(old, applied, &id, out);
                self.mark_read();
            }
            Event::UnreadLoaded {
                revision,
                visit,
                result,
                ..
            } => {
                // The visit's snapshot, taken as the channel opened, so the
                // divider marks what was new then even as reads move on. A
                // prefetched one waits for its history.
                let ranges = || -> HashMap<String, (u64, u64)> {
                    let Ok(unread) = &result else {
                        return HashMap::new();
                    };
                    unread
                        .spaces
                        .iter()
                        .flat_map(|space| &space.channels)
                        .filter_map(|c| {
                            let read = core::sequence(c.read_seq.as_deref()?).ok()?;
                            let last = core::sequence(&c.last_seq).ok()?;
                            (read < last).then(|| (c.id.clone(), (read, last)))
                        })
                        .collect()
                };
                if let Some((_, id, stored @ None)) = &mut self.prefetch
                    && visit == Some(*id)
                {
                    *stored = Some(ranges());
                } else if visit.is_some_and(|v| v == self.visit) && self.unread_pending {
                    self.apply_visit(&ranges());
                    if let Some(id) = self.unread_row() {
                        out.push(Update::Unread(id));
                    }
                    self.mark_read();
                }
                // Any snapshot a later read has not made stale feeds the
                // rail and sidebar badges.
                if revision == self.unread_revision
                    && let Ok(unread) = result
                {
                    self.unread_spaces = unread.spaces;
                    if let Some(count) = unread.badge_count {
                        self.activity.unread_count = count;
                    }
                    out.extend([Update::Badges, Update::Inbox]);
                }
            }
            Event::Reactors {
                generation,
                channel,
                message,
                revision,
                result,
            } if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                if let Some(cached) = self.reactors.get_mut(&message) {
                    cached.loaded(&message, &revision, result);
                }
                out.push(Update::Reactors);
            }
            Event::ChannelRead {
                space,
                channel,
                seq,
                result: Ok(()),
                ..
            } => {
                // As in egui: a refresh already in flight predates the read.
                self.unread_revision += 1;
                let n = core::sequence(&seq).unwrap_or(0);
                for entry in &mut self.unread_spaces {
                    if entry.id == space {
                        entry.channels.retain(|c| {
                            c.id != channel || core::sequence(&c.last_seq).unwrap_or(0) > n
                        });
                    }
                }
                self.unread_spaces
                    .retain(|entry| !entry.channels.is_empty());
                out.push(Update::Badges);
                let read = self.reads.entry(channel).or_insert_with(|| "0".into());
                if n > core::sequence(read).unwrap_or(0) {
                    *read = seq;
                }
            }
            Event::Reacted {
                generation,
                channel,
                message,
                emoji,
                active,
                result,
            } if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                match self
                    .reactions
                    .reacted(&channel, &message, &emoji, active, result)
                {
                    Reacted::Acknowledged(update) => {
                        if self.timeline.merge_reaction_ack(update).is_err() {
                            self.reload();
                        }
                        self.send_next_reaction(&message);
                    }
                    Reacted::Reload => self.reload(),
                    Reacted::ClearChannel(error) => {
                        out.push(Update::Failed(error));
                        self.reload();
                    }
                    Reacted::Failed => self.send_next_reaction(&message),
                }
                self.republish(&message);
                out.append(&mut self.queued);
            }
            Event::EditSnapshot {
                generation,
                request,
                channel,
                message,
                reloaded,
                result,
            } if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                self.mutations.edits.remove(&message);
                if let Ok(snapshot) = &result
                    && self.timeline.merge_edit_ack((**snapshot).clone()).is_err()
                {
                    self.reload();
                }
                self.republish(&message);
                out.append(&mut self.queued);
                out.push(Update::Edited {
                    request,
                    reloaded,
                    result,
                });
            }
            Event::Deleted {
                generation,
                request,
                channel,
                message,
                result,
            } if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                if let Ok(tombstone) = &result
                    && self.timeline.merge_edit_ack((**tombstone).clone()).is_err()
                {
                    self.reload();
                }
                self.republish(&message);
                out.append(&mut self.queued);
                out.push(Update::Deleted(request, result.err()));
            }
            Event::MessageVersions {
                generation,
                request,
                channel,
                message,
                result,
            } if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                out.push(Update::Versions {
                    request,
                    message,
                    result,
                });
            }
            Event::Pinned {
                generation,
                channel,
                message,
                result,
                ..
            } if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                self.pending_pins.remove(&message);
                self.mutations.pins.remove(&message);
                match result {
                    Ok(update) if update.channel_id == channel && update.message.id == message => {
                        if self.timeline.merge_pin_ack(update).is_err() {
                            self.reload();
                        }
                    }
                    Ok(_) => self.reload(),
                    Err(error) => {
                        out.push(Update::Failed(error.message));
                        if matches!(error.status, Some(401 | 403 | 404))
                            && !caper_client::worker::dm_refusal(error.code.as_deref())
                        {
                            self.reload();
                        }
                    }
                }
                self.republish(&message);
                out.append(&mut self.queued);
            }
            Event::Gateway(gateway::GatewayEvent::Reactions {
                generation,
                channel,
                update,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                let id = update.message_id.clone();
                let old = self.seq();
                let applied = self.timeline.apply_reactions(update);
                self.publish(old, applied, &id, out);
            }
            // Link cards arrive after the send, unsequenced; a file's
            // processing finishing is sequenced. Either rebuilds its row.
            Event::Gateway(gateway::GatewayEvent::Embeds {
                generation,
                channel,
                update,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                let id = update.message_id.clone();
                if self.timeline.apply_embeds(*update) {
                    let old = self.seq();
                    self.publish(old, Ok(core::Apply::Applied), &id, out);
                }
            }
            Event::Gateway(gateway::GatewayEvent::Attachments {
                generation,
                channel,
                update,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                let id = update.message_id.clone();
                let old = self.seq();
                let applied = self.timeline.apply_attachments(update);
                self.publish(old, applied, &id, out);
            }
            Event::Gateway(gateway::GatewayEvent::Pin {
                generation,
                channel,
                update,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                let id = update.message.id.clone();
                let old = self.seq();
                let applied = self.timeline.apply_pin(*update);
                self.publish(old, applied, &id, out);
            }
            Event::Gateway(gateway::GatewayEvent::Edit {
                generation,
                channel,
                update,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                let id = update.message.id.clone();
                let old = self.seq();
                let applied = self.timeline.apply_edit(*update);
                self.publish(old, applied, &id, out);
            }
            Event::Gateway(gateway::GatewayEvent::VoiceRoster {
                generation,
                channel,
                participants,
                session_started_at,
            }) if generation == current => {
                let started = session_started_at.filter(|_| !participants.is_empty());
                self.rosters.insert(channel, (participants, started));
                out.push(Update::Voice);
            }
            // A channel whose voice you lost access to; a call in it ends.
            Event::Gateway(gateway::GatewayEvent::VoiceUnavailable {
                generation,
                channel,
                revoked,
            }) if generation == current => {
                self.rosters.remove(&channel);
                if revoked {
                    self.voice.revoke_channel(&channel);
                }
                out.push(Update::Voice);
            }
            Event::Gateway(gateway::GatewayEvent::VoiceReset { generation })
                if generation == current =>
            {
                self.rosters.clear();
                out.push(Update::Voice);
            }
            Event::MediaStatus {
                generation,
                root,
                enabled,
            } if generation == self.account_epoch => {
                self.availability.insert(root, enabled);
                out.push(Update::Voice);
            }
            Event::VoiceChecked {
                request,
                voice_generation,
                space,
                channel,
                result,
            } => {
                self.accept_join(request, voice_generation, &space, &channel, result);
                out.push(Update::Voice);
            }
            Event::Gateway(gateway::GatewayEvent::Presence {
                generation,
                members,
                ..
            }) if generation == current => {
                out.push(Update::Presence(
                    members
                        .into_iter()
                        .filter(|p| p.status != "offline")
                        .map(|p| p.user_id)
                        .collect(),
                ));
            }
            Event::Gateway(gateway::GatewayEvent::Status {
                generation,
                online,
                detail,
                ..
            }) if generation == current => out.push(Update::Connection(
                (!online && !detail.is_empty()).then_some(detail),
            )),
            Event::DirectsLoaded {
                result: Ok(directs),
                ..
            } if self.token.is_some() => self.set_directs(directs, out),
            // Fetched with the account at sign-in; a failure waits for the
            // next unread refresh, as the list is not needed to sign in.
            Event::SignedInDirects {
                token,
                result: Ok(directs),
            } if self.token.as_ref() == Some(&token) => self.set_directs(directs, out),
            Event::DirectCreated {
                navigation, result, ..
            } => match result {
                Ok(direct) => {
                    self.directs.retain(|d| d.id != direct.id);
                    let id = direct.id.clone();
                    self.directs.push(direct);
                    out.push(Update::Badges);
                    if navigation == self.navigation {
                        self.open_direct(id);
                        out.push(Update::DirectCreated(None));
                    }
                }
                Err(e) if navigation == self.navigation => out.push(Update::DirectCreated(Some(e))),
                Err(_) => {}
            },
            Event::LoggedOut {
                generation,
                result: Err(e),
            } if generation == current => out.push(Update::SignedOut(Some(format!(
                "Signed out locally; server revocation failed: {e}"
            )))),
            Event::PeopleLoaded {
                generation,
                result: Ok(people),
            } if generation == current => self.people = people,
            Event::Account {
                result: AccountResult::Accepted { id, result },
                ..
            } => {
                self.answer = None;
                match result {
                    Ok(direct) => {
                        self.answer_error = None;
                        match self.directs.iter_mut().find(|d| d.id == id) {
                            Some(d) => *d = direct,
                            None => self.directs.push(direct),
                        }
                        if !self
                            .directs
                            .iter()
                            .any(|d| d.status == core::DirectStatus::Incoming)
                        {
                            self.requests_open = false;
                        }
                        // Accepted, it is read like any DM you are looking at.
                        self.mark_read();
                    }
                    Err(e) => self.answer_error = Some(e),
                }
                out.extend([Update::Badges, Update::DirectHead]);
            }
            Event::Account {
                result: AccountResult::Declined { id, result },
                ..
            } => {
                self.answer = None;
                match result {
                    Ok(()) => {
                        self.answer_error = None;
                        self.leave_request(&id);
                    }
                    Err(e) => self.answer_error = Some(e),
                }
                out.extend([Update::Badges, Update::DirectHead]);
            }
            Event::Account { result, .. } => self.account_settings(result, out),
            Event::DirectLookedUp {
                generation,
                request,
                result,
                ..
            } if generation == current && request == self.lookup => {
                out.push(Update::DirectFound { request, result });
            }
            Event::Dictated { key, result } if key == self.dictations => {
                out.push(Update::Dictated { key, result });
            }
            Event::Klipy { token, config } if self.token.as_ref() == Some(&token) => {
                self.klipy = config;
                out.push(Update::Drafts);
            }
            Event::AssetUsage { token, settings } if self.token.as_ref() == Some(&token) => {
                self.uploads = settings;
                self.uploads_checked = true;
                out.push(Update::Drafts);
            }
            Event::Upload { key, token, update } => {
                let Some(d) = self.drafts.iter_mut().find(|d| d.upload.key == key) else {
                    // Removed while it uploaded: give up the server's copy.
                    if let caper_client::worker::UploadUpdate::Finished(Ok(file)) = update {
                        self.worker.send(Command::CancelAssets {
                            token,
                            ids: vec![file.id],
                        });
                    }
                    return;
                };
                let finished =
                    matches!(update, caper_client::worker::UploadUpdate::Finished(Ok(_)));
                if let Some(image) = d.upload.apply(update) {
                    d.set_thumbnail(image);
                }
                // The sender sees its own copy without a download.
                if (finished || d.thumb_changed)
                    && let (Some(file), Some(image)) = (&d.upload.attachment, &d.thumb)
                {
                    self.local.push((file.id.clone(), image.clone()));
                }
                d.thumb_changed = false;
                out.push(Update::Drafts);
            }
            Event::AttachmentUrls {
                ids,
                result: Ok(urls),
                ..
            } => {
                if let Some(url) = self.media.refreshed(&ids, urls) {
                    out.push(Update::Open(url));
                }
                out.push(Update::Media);
            }
            // A clicked toast opens its conversation, as in egui. This
            // client has no feedback-team visits, so every conversation
            // counts as known.
            Event::OpenNotification { epoch, alert }
                if epoch == self.account_epoch && self.token.is_some() =>
            {
                use caper_client::os_notifications::Route;
                match alert.route(false, true) {
                    Some(Route::Direct(id)) => {
                        // A new DM may arrive before the DM list.
                        self.open_direct(id);
                        self.load_directs();
                    }
                    Some(Route::Channel { space, channel }) => {
                        self.navigate(Some(space), Some(channel));
                    }
                    Some(Route::Invitation { .. }) => out.push(Update::Warning(
                        "Invitations can't be answered in this app yet.".into(),
                    )),
                    Some(Route::Visit(_)) | None => {}
                }
                out.push(Update::Notified);
            }
            // Unread state changed: the badges are read again a second
            // later, once for a burst.
            Event::NotificationActivity { epoch } if epoch == self.account_epoch => {
                // An open inbox loads again at its next tick, as in egui.
                self.activity.refreshed = None;
                if self.unread_due.is_none() {
                    let wait = Duration::from_secs(1);
                    self.unread_due = Some(Instant::now() + wait);
                    self.host.wake_after(wait);
                }
            }
            Event::NotificationError { epoch, error } if epoch == self.account_epoch => {
                out.push(Update::Warning(error));
            }
            Event::ActivityLoaded {
                epoch,
                request,
                more,
                result,
            } if epoch == self.account_epoch => {
                self.activity.finish(request, more, result);
                out.push(Update::Inbox);
            }
            // A row reads before it opens, as in egui; a failed read puts
            // it back unread and opens nothing.
            Event::ActivityRead {
                epoch,
                navigation,
                item,
                open,
                result,
            } if epoch == self.account_epoch => {
                if open {
                    self.activity.opening = None;
                }
                if self.activity.read_finished(&item, result) {
                    self.refresh_unread();
                    if open && navigation == self.navigation && self.activity.shown {
                        self.open_activity_item(item, out);
                    }
                }
                out.push(Update::Inbox);
            }
            // Everything else (reactions, edits, threads, voice...) is not
            // shown by this client yet.
            _ => {}
        }
    }
}

impl Live {
    /// A fresh DM list; a read acknowledged here is never undone by an
    /// older list.
    fn set_directs(&mut self, directs: Vec<core::DirectConversation>, out: &mut Vec<Update>) {
        let reads: HashMap<String, String> = self
            .directs
            .iter()
            .map(|d| (d.id.clone(), d.read_seq.clone()))
            .collect();
        self.directs = directs;
        for d in &mut self.directs {
            if let Some(read) = reads.get(&d.id)
                && core::sequence(read).unwrap_or(0) > core::sequence(&d.read_seq).unwrap_or(0)
            {
                d.read_seq.clone_from(read);
            }
        }
        out.push(Update::Badges);
        if self.direct.is_some() {
            // The header and empty note follow the list.
            out.push(Update::DirectHead);
        }
    }
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

fn color_for(id: &str) -> u32 {
    let h = id
        .bytes()
        .fold(0u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    model::AUTHOR_COLORS[h as usize % model::AUTHOR_COLORS.len()]
}

pub fn person(id: &str, name: &str) -> Person {
    Person {
        name: name.to_owned(),
        initials: initials(name),
        color: color_for(id),
        online: false,
    }
}

fn time_label(created_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%-I:%M %p")
                .to_string()
        })
        .unwrap_or_default()
}

/// The displayed conversation for one channel's history.
pub fn conversation(
    spaces: &core::Spaces,
    detail: &core::SpaceDetail,
    channel: &str,
    messages: &[core::Message],
    me: Option<&str>,
    thread: bool,
    unread: Option<&str>,
    direct: Option<model::Direct>,
) -> (Conversation, Ids) {
    // A DM lists no members: its authors are known from its messages.
    let order = if direct.is_some() {
        Vec::new()
    } else {
        detail.member_order(Some(channel))
    };
    let mut ids = Ids {
        members: order.iter().map(|m| m.id.clone()).collect(),
        spaces: spaces.spaces.iter().map(|s| s.id.clone()).collect(),
        ..Ids::default()
    };
    let mut members: Vec<Person> = order
        .iter()
        .map(|m| person(&m.id, &m.display_name))
        .collect();
    let listed = members.len();
    let mut out = Vec::with_capacity(messages.len());
    // Thread-only replies live in their thread, not the channel. In the
    // thread pane the root and replies are shown without their thread links.
    for m in messages.iter().filter(|m| thread || m.is_channel_message()) {
        let Some((author, grouped, date)) = ids.place(m) else {
            continue;
        };
        if author == members.len() {
            members.push(person(&m.author.id, &m.author.name));
        }
        // The "New messages" divider ends the group above, as in egui.
        let new = unread == Some(m.id.as_str());
        let mut row = message(m, author, grouped && !new, me);
        row.date = date;
        row.unread = new;
        if thread {
            row.thread_replies = None;
            row.thread_context = false;
        }
        out.push(row);
    }
    let joined: Vec<&core::Channel> = detail.channels.iter().filter(|c| c.joined).collect();
    ids.channels = joined.iter().map(|c| c.id.clone()).collect();
    let channels = joined
        .iter()
        .map(|c| model::Channel {
            name: c.name.clone(),
            private: c.private,
        })
        .collect();
    ids.open = Some(channel.to_owned());
    let selected = if direct.is_some() {
        joined.len()
    } else {
        joined.iter().position(|c| c.id == channel).unwrap_or(0)
    };
    let space = spaces
        .spaces
        .iter()
        .position(|s| s.id == detail.space.id)
        .unwrap_or(0);
    let spaces = spaces
        .spaces
        .iter()
        .map(|s| model::Space {
            name: s.name.clone(),
        })
        .collect();
    let mut c = Conversation::live(spaces, space, channels, selected, members, out);
    c.listed_members = listed;
    c.direct = direct;
    (c, ids)
}

/// Message text as styled runs, as the egui client draws it: the shared
/// bold/italic grammar (`shared/messages/formatting.rs`) removes its markers
/// first; then, in the text that remains, the mentions the server resolved
/// (`caper_client::mentions`) become pills and links
/// (`shared/messages/links.rs`) are found between them.
pub fn spans(text: &str, mentions: &[core::Mention]) -> Vec<Span> {
    let mut visible = String::with_capacity(text.len());
    let mut styles = Vec::new();
    crate::formatting::visit_runs(text, |range, bold, italic| {
        let start = visible.len();
        visible.push_str(&text[range]);
        styles.push((start..visible.len(), bold, italic));
    });
    let pills: Vec<_> = caper_client::mentions::highlights(&visible, mentions)
        .into_iter()
        .map(|(range, _)| range)
        .collect();
    let mut links = Vec::new();
    let mut gap = 0;
    for (start, end) in pills
        .iter()
        .map(|p| (p.start, p.end))
        .chain([(visible.len(), visible.len())])
    {
        let mut offset = gap;
        for segment in crate::links::segments(&visible[gap..start]) {
            let next = offset + segment.text.len();
            if let Some(href) = segment.href {
                links.push((offset..next, href));
            }
            offset = next;
        }
        gap = end;
    }
    let mut out = Vec::new();
    for (range, bold, italic) in styles {
        let mut cuts = vec![range.start, range.end];
        for span in pills.iter().chain(links.iter().map(|(link, _)| link)) {
            for edge in [span.start, span.end] {
                if range.start < edge && edge < range.end {
                    cuts.push(edge);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for piece in cuts.windows(2) {
            let (start, end) = (piece[0], piece[1]);
            out.push(Span::Rich {
                text: visible[start..end].to_owned(),
                bold,
                italic,
                mention: pills.iter().any(|p| p.contains(&start)),
                href: links
                    .iter()
                    .find(|(link, _)| link.contains(&start))
                    .map(|(_, href)| href.clone()),
            });
        }
    }
    out
}

/// One displayed message (attachments and embeds are not drawn yet). `me`
/// is the signed-in author, whose reactions are highlighted.
pub fn message(
    m: &core::Message,
    author: usize,
    grouped: bool,
    me: Option<&str>,
) -> model::Message {
    let deleted = m.is_deleted();
    // As in the egui client: a deleted message keeps its place as a muted
    // notice without reactions; an edit marks the time, or follows the text
    // of a grouped row.
    let edited = m.revision > 1 && !deleted;
    let mut time = time_label(&m.created_at);
    if edited && !grouped {
        time.push_str(" (edited)");
    }
    model::Message {
        id: m.id.clone(),
        author,
        time,
        grouped,
        spans: if deleted {
            vec![Span::Rich {
                text: "This message was deleted.".into(),
                bold: false,
                italic: true,
                mention: false,
                href: None,
            }]
        } else {
            spans(&m.content.text, &m.content.mentions)
        },
        picture: None,
        reactions: if deleted {
            Vec::new()
        } else {
            m.reactions
                .iter()
                .filter(|r| !r.author_ids.is_empty())
                .map(|r| Reaction {
                    emoji: r.emoji.clone(),
                    count: r.author_ids.len() as u32,
                    mine: me.is_some_and(|me| r.author_ids.iter().any(|id| id == me)),
                })
                .collect()
        },
        edited_inline: edited && grouped,
        unread: false,
        deleted,
        reply: m.reply_to.as_ref().map(|r| model::Reply {
            id: r.id.clone(),
            thread_root: r.thread_root_id.clone(),
            author: person(&r.author.id, &r.author.name),
            text: if r.deleted {
                String::new()
            } else {
                let text = r.full_text.as_deref().unwrap_or(&r.text);
                if !text.is_empty() {
                    text.to_owned()
                } else if r.has_attachments {
                    core::attachments_label(&r.attachments).to_owned()
                } else {
                    "Message".to_owned()
                }
            },
            deleted: r.deleted,
        }),
        thread_replies: m
            .thread
            .as_ref()
            .map(|t| t.reply_count)
            .filter(|n| *n > 0 && m.thread_root_id.is_none()),
        thread_context: m.thread_root_id.is_some() && m.broadcast,
        reaction_error: None,
        date: None,
        pinned_by: m.pin.as_ref().filter(|_| !deleted).map(|p| {
            if me == Some(p.author.id.as_str()) {
                "you".to_owned()
            } else {
                p.author.name.clone()
            }
        }),
        // A GIF from the picker shows as its KLIPY image, like a file.
        attachments: if deleted {
            Vec::new()
        } else {
            m.content
                .attachments
                .iter()
                .cloned()
                .chain(m.content.gif.as_ref().map(caper_client::klipy::viewer_file))
                .collect()
        },
        revealed: Default::default(),
        embeds: if deleted {
            Vec::new()
        } else {
            m.content.embeds.clone()
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core_message(extra: serde_json::Value) -> core::Message {
        let mut m = serde_json::json!({
            "id": "b00000000000001",
            "channelId": "general",
            "seq": "1",
            "author": { "id": "member000002", "name": "Alex", "isGuest": false },
            "content": { "version": 1, "type": "text", "text": "hello **there**" },
            "clientMessageId": "00000000-0000-4000-9000-000000000001",
            "createdAt": "2026-10-01T09:00:00Z",
        });
        for (k, v) in extra.as_object().unwrap() {
            m[k] = v.clone();
        }
        serde_json::from_value(m).unwrap()
    }

    #[test]
    fn link_cards_and_picker_gifs_reach_the_row_until_it_is_deleted() {
        let content = serde_json::json!({
            "version": 1, "type": "text", "text": "look https://a.test/x",
            "gif": {
                "provider": "klipy", "kind": "gif", "slug": "wave", "title": "Wave",
                "width": 200, "height": 100,
                "gif": "https://static.klipy.com/wave.gif",
                "still": "https://static.klipy.com/wave.jpg",
            },
            "embeds": [{
                "version": 1, "url": "https://a.test/x",
                "site": { "name": "A", "domain": "a.test" },
                "blocks": [{ "type": "title", "text": "A page" }],
            }],
        });
        let row = message(
            &core_message(serde_json::json!({ "content": content })),
            0,
            false,
            None,
        );
        assert_eq!(row.embeds.len(), 1);
        assert_eq!(row.embeds[0].url, "https://a.test/x");
        let [gif] = row.attachments.as_slice() else {
            panic!("the GIF shows as one image: {:?}", row.attachments);
        };
        assert_eq!(
            gif.url.as_deref(),
            Some("https://static.klipy.com/wave.gif")
        );
        assert_eq!(gif.content_type, "image/gif");
        assert_eq!(gif.name, "Wave");

        let deleted = core_message(serde_json::json!({
            "content": content, "deletedAt": "2026-10-01T09:10:00Z",
        }));
        let row = message(&deleted, 0, false, None);
        assert!(row.embeds.is_empty() && row.attachments.is_empty());
    }

    #[test]
    fn dates_divide_the_first_message_and_each_new_day_and_end_groups() {
        let at =
            |id: &str, time: &str| core_message(serde_json::json!({ "id": id, "createdAt": time }));
        let mut ids = Ids::default();
        let first = ids.place(&at("a", "2026-10-01T12:00:00Z")).unwrap();
        let same_day = ids.place(&at("b", "2026-10-01T12:02:00Z")).unwrap();
        let next_day = ids.place(&at("c", "2026-10-02T12:03:00Z")).unwrap();
        assert!(first.2.is_some() && !first.1);
        assert!(
            same_day.2.is_none() && same_day.1,
            "same author and day groups"
        );
        let label = next_day.2.expect("a new day gets a divider");
        assert!(label.contains("October 2, 2026"), "{label}");
        assert!(!next_day.1, "a divider ends the group");
        assert!(ids.place(&at("c", "2026-10-02T12:03:00Z")).is_none());
    }

    #[test]
    fn rows_show_reactions_edits_deletions_replies_and_threads_as_egui_does() {
        let reacted = core_message(serde_json::json!({ "reactions": [
            { "emoji": "👍", "authorIds": ["member000002", "owner0000001"] },
            { "emoji": "🎉", "authorIds": ["member000002"] },
        ]}));
        let row = message(&reacted, 0, false, Some("owner0000001"));
        let chips: Vec<_> = row
            .reactions
            .iter()
            .map(|r| (r.emoji.as_str(), r.count, r.mine))
            .collect();
        assert_eq!(chips, [("👍", 2, true), ("🎉", 1, false)]);
        assert!(!row.time.contains("edited") && !row.edited_inline && !row.deleted);
        assert!(
            row != message(&reacted, 0, false, Some("member000003")),
            "mine changes the row"
        );

        let edited = core_message(serde_json::json!({
            "revision": 2, "editedAt": "2026-10-01T09:05:00Z", "editSeq": "2",
        }));
        assert!(message(&edited, 0, false, None).time.ends_with(" (edited)"));
        let grouped = message(&edited, 0, true, None);
        assert!(grouped.edited_inline && !grouped.time.contains("edited"));

        let deleted = core_message(serde_json::json!({
            "revision": 2, "editedAt": "2026-10-01T09:05:00Z", "editSeq": "2",
            "deletedAt": "2026-10-01T09:05:00Z",
            "content": { "version": 1, "type": "text", "text": "" },
            "reactions": [{ "emoji": "👍", "authorIds": ["member000002"] }],
        }));
        let row = message(&deleted, 0, false, None);
        assert!(row.deleted && row.reactions.is_empty() && !row.time.contains("edited"));
        assert!(row.reply.is_none() && row.thread_replies.is_none() && !row.thread_context);

        let quoted = |extra: serde_json::Value| {
            let mut reply = serde_json::json!({
                "id": "b00000000000002",
                "author": { "id": "member000001", "name": "Maya", "isGuest": false },
                "text": "short",
                "fullText": "the whole original",
                "hasAttachments": false,
            });
            for (k, v) in extra.as_object().unwrap() {
                reply[k] = v.clone();
            }
            message(
                &core_message(serde_json::json!({ "replyTo": reply })),
                0,
                false,
                None,
            )
            .reply
            .unwrap()
        };
        let reply = quoted(serde_json::json!({}));
        assert_eq!(
            (reply.author.name.as_str(), reply.text.as_str()),
            ("Maya", "the whole original")
        );
        assert!(
            quoted(serde_json::json!({ "deleted": true, "text": "", "fullText": null })).deleted
        );
        assert_eq!(
            quoted(serde_json::json!({ "text": "", "fullText": null })).text,
            "Message"
        );

        let root = core_message(serde_json::json!({
            "thread": { "replyCount": 2, "participants": [], "seq": "5" },
        }));
        assert_eq!(message(&root, 0, false, None).thread_replies, Some(2));
        let pin = |by: &str| {
            core_message(serde_json::json!({ "pin": {
                "author": { "id": by, "name": "Alex", "isGuest": false },
                "createdAt": "2026-10-01T09:01:00Z",
            }}))
        };
        let me = Some("owner0000001");
        assert_eq!(
            message(&pin("owner0000001"), 0, false, me)
                .pinned_by
                .as_deref(),
            Some("you")
        );
        assert_eq!(
            message(&pin("member000002"), 0, false, me)
                .pinned_by
                .as_deref(),
            Some("Alex")
        );
        let broadcast = core_message(serde_json::json!({
            "threadRootId": "b00000000000009", "broadcast": true,
        }));
        let shared = message(&broadcast, 0, false, None);
        assert!(shared.thread_context && shared.thread_replies.is_none());
        assert!(
            matches!(&row.spans[..], [Span::Rich { text, italic: true, .. }] if text == "This message was deleted.")
        );
    }
    fn shown(spans: &[Span]) -> Vec<(String, bool, bool, Option<String>)> {
        spans
            .iter()
            .map(|s| match s {
                Span::Rich {
                    text,
                    bold,
                    italic,
                    href,
                    ..
                } => (text.clone(), *bold, *italic, href.clone()),
                _ => panic!("live messages are rich runs"),
            })
            .collect()
    }

    #[test]
    fn styles_and_links_split_into_runs_without_their_markers() {
        let runs = shown(&spans("**see https://caper.chat/x** and _this_", &[]));
        let text: String = runs.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(text, "see https://caper.chat/x and this");
        let link = runs.iter().find(|r| r.3.is_some()).expect("a link run");
        assert_eq!(link.0, "https://caper.chat/x");
        assert!(link.1, "the link keeps the bold around it");
        assert!(runs.iter().any(|r| r.0 == "this" && r.2 && !r.1));
        assert!(runs.iter().any(|r| r.0 == "see " && r.1 && r.3.is_none()));
    }

    #[test]
    fn resolved_mentions_become_pills_and_unresolved_ones_stay_text() {
        let maya = core::Mention {
            kind: "user".into(),
            id: Some("member000001".into()),
            username: Some("maya".into()),
        };
        let text = "hi @maya and @nobody, see **@maya's** https://x.example";
        let pills: Vec<String> = spans(text, std::slice::from_ref(&maya))
            .into_iter()
            .filter_map(|s| match s {
                Span::Rich {
                    text,
                    mention: true,
                    ..
                } => Some(text),
                _ => None,
            })
            .collect();
        assert_eq!(pills, ["@maya", "@maya"]);
    }
}
