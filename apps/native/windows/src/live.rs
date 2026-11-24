//! Real conversations through the shared client core's background worker
//! (`caper_client::worker`): restores the stored session, opens the
//! account's first space and its first joined channel, follows it live
//! through the gateway, and sends messages. Signs in by email code when
//! no session is stored. The server is `endpoint()`: production in release
//! builds, as in the egui client.
use crate::avatars::Face;
use crate::model::{self, Conversation, Person, Reaction, Span};
use caper_client::notifications::{Change, Scope};
use caper_client::reactions::{
    REACTOR_CACHE_LIMIT, Reacted, Reactions, ReactorCache, ReactorState,
};
use caper_client::state::{CallContext, Phase};
use caper_client::voice::{Voice, VoiceOperation};
use caper_client::worker as core_worker;
use caper_client::worker::{
    AccountOperation, AccountResult, AdminOperation, AdminResult, Command, Event, Worker,
};
use caper_client::{Host, api::Api, gateway, grouping, mentions, model as core};
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
    /// A delayed wake requested while draining is consumed by that same
    /// drain, not another immediate WM_LIVE (Voice requests its release
    /// deadline every time it is read).
    draining: bool,
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
        let post = self
            .requests
            .lock()
            .map(|mut r| {
                r.wake_after = Some(r.wake_after.map_or(delay, |d| d.min(delay)));
                !r.draining
            })
            .unwrap_or(false);
        if post {
            self.post();
        }
    }

    fn bring_to_front(&self) {
        self.request(|r| r.front = true);
    }

    fn set_title(&self, title: String) {
        self.request(|r| r.title = Some(title));
    }
}

/// Ends the header's error for a send waiting to go again.
pub const RESEND_NOTE: &str = "Your message will be sent again when the connection returns.";

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
    /// Who is typing in the open conversation changed.
    Typing,
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
    /// A leave or an invitation's answer asked for in a dialog finished:
    /// None once done (another space or channel may be opening), else
    /// why not.
    Left(Option<String>),
    /// Joining the previewed channel failed: its bar shows why.
    Joining,
    /// The Join with invite link dialog's request `request` answered:
    /// the link's preview, None once joined (the space is opening), or
    /// why not.
    InviteLink {
        request: u64,
        result: Result<Option<caper_client::invites::InvitePreview>, String>,
    },
    /// A notification opened an invitation: its dialog shows.
    Invitation(Invitation),
    /// Invite people's link or invitation answer.
    InvitePeople(Box<crate::invite_people::Answer>),
    /// A new private channel's settings open, as on web.
    ChannelSettings(String),
    /// Your space or channel invitations changed: the rail and sidebar
    /// redraw.
    Invitations,
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
    /// An edited replacement finished; the original survives any failure.
    AttachmentEdited(u64, Option<String>),
    /// A page of message `message`'s versions for request `request`, or
    /// why it failed (for its history).
    Versions {
        request: u64,
        message: String,
        result: Result<core::MessageVersions, String>,
    },
    /// The profile card changed; Some((person, clear_draft)) closes it.
    Profile(Option<(String, bool)>),
    /// Online account ids in the open space.
    Presence(HashSet<String>),
    /// The gateway's state while not live ("Connecting…"); None once live.
    Connection(Option<String>),
    Failed(String),
    /// A send was confirmed: a waiting resend's note can go.
    SendConfirmed,
}

fn invite_answer(answer: crate::invite_people::Answer) -> Update {
    Update::InvitePeople(Box::new(answer))
}

/// An invitation to answer: to a space, or to one of its private
/// channels.
#[derive(Clone)]
pub enum Invitation {
    Space(core::Space),
    Channel(core::ChannelInvitation),
}

/// How often invitations and memberships are read again (web's 15 s).
const MEMBERSHIP_EVERY: Duration = Duration::from_secs(15);

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

/// The open profile card's requests, as egui's `MentionCard` keeps them:
/// its send or open in flight and what it says, the invitation in flight,
/// and spaces already answered. Drafts and retries survive closing it.
#[derive(Default)]
pub struct CardState {
    pub person: String,
    pub sending: bool,
    pub status: Option<String>,
    pub error: Option<String>,
    pub choosing_space: bool,
    pub inviting: Option<(u64, String)>,
    pub invited: HashMap<String, &'static str>,
}

/// Recipient-scoped files and a failed write's exact payload. An uncertain
/// write cannot be edited into a different message under the same key.
#[derive(Default)]
pub struct ProfileDraft {
    pub files: Vec<caper_client::profile_dm::File>,
    pub retry: Option<Box<caper_client::profile_dm::Send>>,
    pub error: Option<String>,
}

pub struct Live {
    worker: Worker<WinHost>,
    /// When input last reported activity, so the server's phone push hold
    /// sees this session as active (GTK's `active`).
    input_reported: std::time::Instant,
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
    /// Who is typing in the open conversation, and your typing.
    pub typing: crate::typing::Typing,
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
    /// Mark unread this visit: (channel, thread root, message). The divider
    /// sits above that message and the conversation stops acknowledging
    /// reads until it is left.
    marked: Option<(String, Option<String>, String)>,
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
    edit_upload: Option<(u64, crate::uploads::Draft)>,
    /// The last dictation sent, so only its answer is used.
    dictations: u64,
    /// The main composer's link card, and its request in flight.
    pub preview: caper_client::embeds::ComposerPreview,
    preview_pending: Option<std::sync::mpsc::Receiver<(String, Option<core::Embed>)>>,
    /// The same for the open thread's reply box.
    pub thread_preview: caper_client::embeds::ComposerPreview,
    thread_preview_pending: Option<std::sync::mpsc::Receiver<(String, Option<core::Embed>)>>,
    pub uploads: Option<caper_client::compress::Compression>,
    pub uploads_checked: bool,
    local: Vec<(String, caper_client::Rgba)>,
    /// KLIPY's key once `GET /api/klipy` answers (GIFs stay hidden until
    /// then, as in egui), when it was last asked, and the shares to send
    /// once their GIF messages are confirmed (by client id).
    pub klipy: Option<caper_client::klipy::Config>,
    klipy_checked: Option<std::time::Instant>,
    shares: HashMap<String, caper_client::klipy::Share>,
    /// Message sends not yet confirmed, by client id, and whether the last
    /// attempt failed with an unknown outcome (a network drop or 5xx): those
    /// go again unchanged, same client message ID, when the gateway is live.
    outgoing: std::cell::RefCell<HashMap<String, (core_worker::Outgoing, bool)>>,
    /// Your DMs as the server last listed them, your account (for your own
    /// DM), and the open DM while one is open instead of a channel.
    pub directs: Vec<core::DirectConversation>,
    pub account_id: Option<String>,
    pub account_name: String,
    pub feedback_team: bool,
    pub debug_enabled: bool,
    pub feedback: caper_client::feedback::Inbox,
    pub feedback_waiting: u64,
    pub feedback_shown: bool,
    pub feedback_visit: Option<caper_client::feedback::Visit>,
    report_target: Option<(u64, caper_client::feedback::VisitTarget)>,
    /// Your avatar, as the account last said.
    pub account_avatar: crate::avatars::Avatar,
    /// The open profile card's requests, and the last invitation's number.
    pub card: Option<CardState>,
    pub profile_drafts: HashMap<String, ProfileDraft>,
    profile_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    card_request: u64,
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
    /// Account-wide activity, including spaces whose gateway isn't open.
    voice_spaces: HashSet<String>,
    voice_activity_at: Option<Instant>,
    voice_activity_pending: bool,
    /// Your call (the shared engine, as egui's `voice`).
    pub voice: Voice,
    pub preferences: crate::settings::preferences::Preferences,
    pub whats_new: caper_client::whats_new::WhatsNew,
    pub highlights: crate::updates::History,
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
    /// What voice last showed, so only changes redraw.
    voice_seen: String,
    /// A previewed channel's Join in flight, and why the last one failed.
    pub joining: bool,
    pub join_error: Option<String>,
    /// Spaces you're invited to (the rail's tiles), the last invite link
    /// request (answers to earlier ones drop), the background membership
    /// refresh (when it last went, the one in flight), and the channel
    /// invitation a notification opens once its space has loaded.
    pub invitations: Vec<core::Space>,
    /// The account's space and channel limits, as the space list said.
    limits: Option<core::SpaceLimits>,
    invite_request: u64,
    membership_at: Option<Instant>,
    membership_request: u64,
    membership_pending: bool,
    invite_focus: Option<(u64, String)>,
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
    /// Connecting, reconnecting, a lost microphone or connected (None).
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
    /// An explicit target; older pages load until it can be shown without
    /// leaving a gap between that reply and the latest history.
    focus: Option<String>,
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

/// The production server, which release builds use unless told otherwise.
pub const PRODUCTION: &str = "https://caper.chat";

/// The server, as egui picks it: `--api-url`, then `CAPER_API_URL`, then
/// production in release builds. Development builds without either show
/// the sample conversation instead (`None`).
pub fn endpoint() -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.windows(2)
        .find(|pair| pair[0] == "--api-url")
        .map(|pair| pair[1].clone())
        .or_else(|| std::env::var("CAPER_API_URL").ok())
        .or_else(|| (!cfg!(debug_assertions)).then(|| PRODUCTION.into()))
}

/// Whether this copy talks to production. Only then does it share the
/// installed Caper's OS registrations (launch at login), as egui keeps
/// fixture runs off them.
pub fn production() -> bool {
    endpoint().is_some_and(|u| u.trim_end_matches('/') == PRODUCTION)
}

/// Starts the worker and restores the stored session; `None` without a
/// server (`endpoint`).
pub fn start(hwnd: HWND) -> Option<Live> {
    let url = endpoint()?;
    let host = WinHost {
        hwnd: hwnd.0 as isize,
        requests: Arc::default(),
    };
    let api = match Api::new(&url) {
        Ok(api) => api,
        Err(e) => {
            eprintln!("Caper could not start: {e}");
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
    crate::avatars::connect(api.clone(), hwnd);
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
        crate::settings::preferences::Preferences::load(
            production()
                .then(|| std::env::var_os("APPDATA").map(std::path::PathBuf::from))
                .flatten(),
        ),
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
        preferences: crate::settings::preferences::Preferences,
    ) -> Self {
        let origin = media
            .api()
            .map(|a| a.base().origin().ascii_serialization())
            .unwrap_or_default();
        let mut whats_new = caper_client::whats_new::WhatsNew::new(&origin, "windows");
        whats_new.restore(preferences.get(whats_new.key()));
        Live {
            worker,
            input_reported: std::time::Instant::now(),
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
            typing: crate::typing::Typing::default(),
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
            marked: None,
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
            edit_upload: None,
            dictations: 0,
            preview: Default::default(),
            preview_pending: None,
            thread_preview: Default::default(),
            thread_preview_pending: None,
            uploads: None,
            uploads_checked: false,
            local: Vec::new(),
            klipy: None,
            klipy_checked: None,
            shares: HashMap::new(),
            outgoing: Default::default(),
            directs: Vec::new(),
            account_id: None,
            account_name: String::new(),
            feedback_team: false,
            debug_enabled: false,
            feedback: Default::default(),
            feedback_waiting: 0,
            feedback_shown: false,
            feedback_visit: None,
            report_target: None,
            account_avatar: crate::avatars::Avatar::default(),
            card: None,
            profile_drafts: HashMap::new(),
            profile_cancel: None,
            card_request: 0,
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
            voice_spaces: HashSet::new(),
            voice_activity_at: None,
            voice_activity_pending: false,
            voice,
            preferences,
            whats_new,
            highlights: Default::default(),
            availability: HashMap::new(),
            status_asked: HashSet::new(),
            join_request: 0,
            pending_join: None,
            prepared: HashMap::new(),
            voice_seen: String::new(),
            joining: false,
            join_error: None,
            invitations: Vec::new(),
            limits: None,
            invite_request: 0,
            membership_at: None,
            membership_request: 0,
            membership_pending: false,
            invite_focus: None,
        }
    }

    /// Signs out (egui's `logout`): the stored session is forgotten and the
    /// server revokes it. A signed-out Live replaces this one on the same
    /// worker, so nothing can restore the old session meanwhile.
    pub fn logout(mut self) -> Live {
        if let Some(cancel) = self.profile_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.profile_drafts.clear();
        self.discard_voice_sends();
        if let Some((_, draft)) = self.edit_upload.take() {
            draft.upload.abandon();
        }
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
            preferences,
            ..
        } = self;
        media.clear();
        voice.leave();
        voice.error = None;
        Live::fresh(
            worker,
            media,
            media_ready,
            host,
            generation,
            voice,
            preferences,
        )
    }

    pub fn load_highlights(&mut self, manual: bool) {
        if self.whats_new.begin(Instant::now(), manual) {
            self.highlights.loading = true;
            self.highlights.error = None;
            self.worker.send(Command::WhatsNew);
        }
    }

    pub fn save_highlights_read(&mut self) {
        if let Some(value) = self.whats_new.stored() {
            self.preferences.set(self.whats_new.key(), value);
        }
    }

    /// Keyboard or pointer input: tell the gateway, at most every 5 seconds.
    pub fn active(&mut self) {
        if self.input_reported.elapsed() > std::time::Duration::from_secs(5) {
            self.input_reported = std::time::Instant::now();
            self.worker.send(Command::Activity);
        }
    }

    pub fn account(&self, operation: AccountOperation) {
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

    pub fn unblock_account(&mut self, account: core::BlockedAccount) {
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
                    let p = person(&a.id, &a.display_name, a.avatar());
                    (
                        a.display_name.clone(),
                        a.username.clone(),
                        p.color,
                        p.initials,
                        p.avatar,
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
            AccountResult::Invitation {
                space,
                user,
                request,
                result,
            } => {
                if let Some(card) = self.card.as_mut().filter(|c| {
                    c.person == user && c.inviting.as_ref() == Some(&(request, space.id.clone()))
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
                    out.push(Update::Profile(None));
                }
            }
            AccountResult::DefaultInviteLink {
                space,
                request,
                result,
            } => {
                out.push(invite_answer(crate::invite_people::Answer::Default {
                    space,
                    request,
                    result,
                }));
                return;
            }
            AccountResult::InviteLinks {
                space,
                request,
                result,
            } => {
                out.push(invite_answer(crate::invite_people::Answer::Links {
                    space,
                    request,
                    result,
                }));
                return;
            }
            AccountResult::InviteLinkRevoked {
                space,
                request,
                code,
                result,
            } => {
                out.push(invite_answer(crate::invite_people::Answer::Revoked {
                    space,
                    request,
                    code,
                    result,
                }));
                return;
            }
            AccountResult::InviteLinkCreated {
                space,
                request,
                result,
            } => {
                out.push(invite_answer(crate::invite_people::Answer::Created {
                    space,
                    request,
                    result,
                }));
                return;
            }
            AccountResult::InvitePreview { request, result } => {
                if request == self.invite_request {
                    out.push(Update::InviteLink {
                        request,
                        result: result.map(Some),
                    });
                }
                return;
            }
            AccountResult::InviteJoined { request, result } => {
                if request == self.invite_request {
                    let result = result.map(|joined| {
                        self.joined_invite(joined, out);
                        None
                    });
                    out.push(Update::InviteLink { request, result });
                }
                return;
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

    /// A profile request belongs to this sign-in, not a navigation generation.
    pub fn profile_session(&self) -> Option<(u64, caper_client::api::Api, String)> {
        Some((
            self.account_epoch,
            self.media.api()?.clone(),
            self.token.clone()?,
        ))
    }

    /// Forward requests remain tied to the signed-in account.
    pub fn forward_session(&self) -> Option<(u64, caper_client::api::Api, String)> {
        self.profile_session()
    }

    /// Apply the saved identity to retained snapshots without navigating or
    /// resetting cursors, pending sends, or composer drafts.
    pub fn profile_saved(&mut self, account: &core::Account) {
        if self.account_id.as_deref() != Some(&account.id) {
            return;
        }
        let update = core::PersonUpdate::from(account);
        self.feedback_team = account.feedback_team;
        self.debug_enabled = account.debug_enabled;
        self.account_avatar = account.avatar();
        self.username.clone_from(&account.username);
        self.name = account
            .display_name
            .as_ref()
            .or(account.username.as_ref())
            .cloned()
            .unwrap_or_default();
        self.account_name.clone_from(&self.name);
        if let Some(author) = &mut self.author {
            author.apply_person(&update);
        }
        if let Some(detail) = &mut self.detail {
            for member in &mut detail.members {
                member.apply_person(&update);
            }
        }
        for person in &mut self.people {
            person.apply_person(&update);
        }
        for direct in &mut self.directs {
            direct.peer.apply_person(&update);
        }
        for person in self.blocks.iter_mut().flatten() {
            person.apply_person(&update);
        }
        for person in self
            .reactors
            .values_mut()
            .flat_map(|r| &mut r.groups)
            .flat_map(|g| &mut g.authors)
        {
            person.apply_person(&update);
        }
        for person in self.rosters.values_mut().flat_map(|(people, _)| people) {
            person.apply_person(&update);
        }
        for (message, pin) in self.mutations.pins.values_mut() {
            message.apply_person(&update);
            if let Some(pin) = pin {
                pin.author.apply_person(&update);
            }
        }
        self.timeline.apply_person(&update);
        self.queued.push(Update::Badges);
    }

    pub fn profile_snapshot(&self) -> Option<(Conversation, Ids)> {
        self.rebuilt()
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
        crate::actions::set_quick(account.quick_reactions());
        crate::actions::settle_quick();
        self.feedback_team = account.feedback_team;
        self.debug_enabled = account.debug_enabled;
        self.account_avatar = account.avatar();
        self.username.clone_from(&account.username);
        self.name = account
            .display_name
            .or(account.username)
            .unwrap_or_default();
        self.account_name.clone_from(&self.name);
        let space = spaces.spaces.first().map(|s| s.id.clone());
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
        self.voice_spaces.clear();
        self.voice_activity_at = None;
        self.voice_activity_pending = false;
        if let Some(token) = self.token.clone() {
            self.worker.send(Command::StartNotifications {
                epoch: self.account_epoch,
                token,
            });
        }
        self.invitations = spaces.invitations.clone();
        self.limits = spaces.limits.clone();
        self.membership_at = Some(Instant::now());
        self.host.wake_after(MEMBERSHIP_EVERY);
        if space.is_none() {
            out.push(Update::Loaded(Box::new(conversation(
                &spaces,
                None,
                None,
                &[],
                None,
                false,
                None,
                None,
            ))));
        }
        self.spaces = Some(spaces);
        if space.is_some() {
            self.navigate(space, None);
        }
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

    /// `id`'s profile card from what is loaded (the open space's members,
    /// people you share a space with, your DMs), as every client builds it.
    pub fn profile_of(
        &self,
        entry: &core::Mention,
        author: Option<&core::Author>,
    ) -> Option<caper_client::mentions::Profile> {
        let me = self.account_id.as_deref();
        let members = self
            .detail
            .as_ref()
            .map_or(&[][..], |d| d.members.as_slice());
        let mut profile = match author {
            Some(a) => Some(mentions::author_profile(
                a,
                me,
                members,
                &self.people,
                &self.directs,
            )),
            None => mentions::profile(entry, me, members, &self.people, &self.directs),
        }?;
        if profile.me {
            profile.username.clone_from(&self.username);
            profile.display_name = Some(self.account_name.clone());
            profile.avatar_id = self.account_avatar.mascot.map(i32::from);
            profile.avatar_asset = self.account_avatar.picture.as_ref().map(|p| p.0.clone());
            profile.avatar_url = self
                .account_avatar
                .picture
                .as_ref()
                .and_then(|p| p.1.clone());
        }
        Some(profile)
    }

    /// You blocked `id`, from Settings' list or their DM.
    pub fn blocked_person(&self, id: &str) -> bool {
        self.is_blocked(id) || self.directs.iter().any(|d| d.peer.id == id && d.blocked)
    }

    /// Sends the card's `text` to `person` (`open_only`: only opens their
    /// DM), creating the conversation when it is new, as egui's
    /// `profile_action`. False when it can't go now.
    pub fn profile_send(
        &mut self,
        person: &str,
        username: String,
        text: String,
        open_only: bool,
    ) -> bool {
        if self.token.is_none()
            || self
                .card
                .as_ref()
                .is_none_or(|c| c.person != person || c.sending || c.inviting.is_some())
        {
            return false;
        }
        let draft = self.profile_drafts.entry(person.into()).or_default();
        if !open_only && text.trim().is_empty() && draft.files.is_empty() && draft.retry.is_none() {
            return false;
        }
        if text.chars().count() > 4_000 && !open_only && draft.retry.is_none() {
            draft.error = Some("Messages can be at most 4,000 characters.".into());
            return false;
        }
        let send = if !open_only && draft.retry.is_some() {
            draft.retry.take().unwrap()
        } else {
            Box::new(caper_client::profile_dm::Send {
                person: person.into(),
                username,
                name: self.account_name.clone(),
                client_id: uuid::Uuid::new_v4().to_string(),
                text: if open_only { String::new() } else { text },
                files: if open_only {
                    Vec::new()
                } else {
                    draft.files.clone()
                },
                settings: self.uploads.clone(),
                cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
                retrying: false,
                open_only,
                voice_only: false,
                gif: None,
                direct: None,
            })
        };
        self.dispatch_profile(send)
    }

    pub fn profile_voice(
        &mut self,
        person: &str,
        username: String,
        file: caper_client::profile_dm::File,
    ) {
        if !self.preferences.voice_send_on_stop() {
            self.profile_drafts
                .entry(person.into())
                .or_default()
                .files
                .push(file);
            self.upload_profile_files(person, &username);
            return;
        }
        self.dispatch_profile(Box::new(caper_client::profile_dm::Send {
            person: person.into(),
            username,
            name: self.account_name.clone(),
            client_id: uuid::Uuid::new_v4().to_string(),
            text: String::new(),
            files: vec![file],
            settings: self.uploads.clone(),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            retrying: false,
            open_only: false,
            voice_only: true,
            gif: None,
            direct: None,
        }));
    }

    /// Eager uploads are account/recipient scoped, never the selected channel.
    pub fn upload_profile_files(&mut self, person: &str, username: &str) {
        let Some((_, api, token)) = self.profile_session() else {
            return;
        };
        let Some(settings) = &self.uploads else {
            return;
        };
        if let Some(draft) = self
            .profile_drafts
            .get_mut(person)
            .filter(|d| d.retry.is_none())
        {
            for file in &mut draft.files {
                if file.upload.is_none() && file.attachment.is_none() {
                    file.upload_eagerly(
                        &api,
                        &token,
                        (person, username, None),
                        settings,
                        &self.host,
                    );
                }
            }
        }
    }

    fn dispatch_profile(&mut self, send: Box<caper_client::profile_dm::Send>) -> bool {
        let Some(token) = self.token.clone() else {
            return false;
        };
        let Some(card) = self.card.as_mut().filter(|c| c.person == send.person) else {
            return false;
        };
        if card.sending || card.inviting.is_some() {
            return false;
        }
        self.profile_drafts
            .entry(send.person.clone())
            .or_default()
            .error = None;
        card.sending = true;
        card.error = None;
        card.status = Some(
            if send.open_only {
                "Opening\u{2026}"
            } else if send.voice_only {
                "Sending voice message\u{2026}"
            } else {
                "Sending\u{2026}"
            }
            .into(),
        );
        self.profile_cancel = Some(send.cancel.clone());
        self.worker.send(Command::SendProfile {
            epoch: self.account_epoch,
            person: send.person.clone(),
            token,
            send,
        });
        true
    }

    /// Who `@` can suggest, never yourself, and which specials apply, as on
    /// egui: a space channel offers its members, `everyone` and `here`; a
    /// DM, or a profile card's message to `peer`, offers `GET /api/people`
    /// plus the peer.
    pub fn mention_people(
        &self,
        peer: Option<mentions::Person>,
    ) -> (Vec<mentions::Person>, mentions::Specials) {
        let me = self.account_id.as_deref();
        let dm_peer = self
            .direct
            .as_ref()
            .and_then(|id| self.directs.iter().find(|d| &d.id == id))
            .map(|d| mentions::Person {
                username: d.peer.username.clone(),
                display_name: d.peer.display_name.clone(),
                avatar_id: d.peer.avatar_id,
                avatar_asset: d.peer.avatar_asset.clone(),
                avatar_url: d.peer.avatar_url.clone(),
                invited: false,
            });
        if peer.is_some() || self.direct.is_some() {
            let mut people: Vec<mentions::Person> = self
                .people
                .iter()
                .filter(|p| Some(p.id.as_str()) != me)
                .map(mentions::Person::from)
                .collect();
            if let Some(peer) = peer.or(dm_peer)
                && !people.iter().any(|p| p.username == peer.username)
                && self.username.as_deref() != Some(peer.username.as_str())
            {
                people.push(peer);
            }
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
            .filter(|d| self.space.as_ref() == Some(&d.space.id))
            .map_or_else(Vec::new, |d| {
                d.members
                    .iter()
                    .filter(|m| Some(m.id.as_str()) != me)
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

    /// Spaces you own that `id` is not in (the open one only if they
    /// aren't), for the card's Invite to space.
    pub fn invitable_spaces(&self, id: &str) -> Vec<core::Space> {
        let me = self.account_id.as_deref();
        let member_of_open = self
            .detail
            .as_ref()
            .filter(|d| d.members.iter().any(|m| m.id == id))
            .map(|d| d.space.id.as_str());
        self.spaces
            .iter()
            .flat_map(|s| &s.spaces)
            .filter(|s| {
                !s.demo && Some(s.owner_id.as_str()) == me && member_of_open != Some(s.id.as_str())
            })
            .cloned()
            .collect()
    }

    /// Invites the card's person (`username`) to `space`.
    pub fn profile_invite(&mut self, space: core::Space, username: String, user: String) {
        self.card_request += 1;
        let request = self.card_request;
        let Some(card) = self.card.as_mut().filter(|c| c.person == user) else {
            return;
        };
        card.inviting = Some((request, space.id.clone()));
        card.error = None;
        card.status = None;
        self.account(AccountOperation::InviteToSpace {
            space,
            username,
            user,
            request,
        });
    }

    /// Whether a block or unblock is in flight.
    pub fn block_busy(&self) -> bool {
        self.block_busy
    }

    /// Server-stamped Caper identities, which can't be blocked (egui's
    /// `protected_account`).
    pub fn protected(&self, id: &str) -> bool {
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
        if let Some(c) = self.open_preview() {
            return Some(Bar {
                kind: BarKind::Preview,
                name: c.name.clone(),
                username: String::new(),
                busy: self.joining.then_some(false),
                unblocking: false,
                protected: true,
                error: self.join_error.clone(),
            });
        }
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
                BarKind::Blocked | BarKind::Preview => self.block_error.clone(),
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
        if let Some(visit) = &self.feedback_visit {
            return Some(crate::model::Direct {
                name: format!("{} · Feedback visit", visit.title),
                own: false,
                note: "Visiting as the Caper team. Replies are visible in this conversation. Return through Feedback in the sidebar.".into(),
            });
        }
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

    /// The open channel's or DM's id.
    pub fn channel_id(&self) -> Option<&str> {
        self.channel.as_deref()
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

    /// Whether a profile card can send a GIF: the account has KLIPY.
    pub fn can_profile_gif(&self) -> bool {
        self.klipy.is_some() && self.token.is_some()
    }

    /// Sends a GIF chosen from `person`'s card on its own to their DM,
    /// keeping the card's draft. KLIPY hears of it once the send succeeds.
    pub fn profile_gif(
        &mut self,
        person: &str,
        username: String,
        gif: caper_client::klipy::Gif,
        share: caper_client::klipy::Share,
    ) -> bool {
        if self
            .profile_drafts
            .get(person)
            .is_some_and(|d| d.retry.is_some())
        {
            return false;
        }
        let client_id = uuid::Uuid::new_v4().to_string();
        self.shares.insert(client_id.clone(), share);
        let sent = self.dispatch_profile(Box::new(caper_client::profile_dm::Send {
            person: person.into(),
            username,
            name: self.account_name.clone(),
            client_id: client_id.clone(),
            text: String::new(),
            files: Vec::new(),
            settings: self.uploads.clone(),
            cancel: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            retrying: false,
            open_only: false,
            voice_only: false,
            gif: Some(gif),
            direct: None,
        }));
        if !sent {
            self.shares.remove(&client_id);
        }
        sent
    }

    /// Sends a GIF chosen in the picker as its own message, replying to
    /// `reply_to`; with `thread`, into that root's thread (and to the
    /// channel too when its flag is set). KLIPY hears of it once the
    /// server confirms the send.
    pub fn send_gif(
        &mut self,
        gif: caper_client::klipy::Gif,
        share: caper_client::klipy::Share,
        reply_to: Option<String>,
        thread: Option<(String, bool)>,
    ) {
        let (Some(channel), Some(chat_token)) = (&self.channel, &self.chat_token) else {
            return;
        };
        let outgoing = core_worker::Outgoing {
            broadcast: thread.as_ref().is_some_and(|(_, b)| *b),
            thread_root_id: thread.map(|(root, _)| root),
            reply_to_id: reply_to,
            gif: Some(gif),
            ..core_worker::Outgoing::new(channel.clone(), String::new())
        };
        self.shares.insert(outgoing.client_id.clone(), share);
        let chat_token = chat_token.clone();
        self.dispatch(outgoing, chat_token);
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

    /// Files for the next manual Send; automatic voice messages send alone.
    pub fn open_drafts(&self) -> Vec<&crate::uploads::Draft> {
        self.visible_drafts()
            .into_iter()
            .filter(|d| d.auto_send.is_none())
            .collect()
    }

    /// Also show uploading/sending voice messages without consuming the draft.
    pub fn visible_drafts(&self) -> Vec<&crate::uploads::Draft> {
        self.drafts
            .iter()
            .filter(|d| self.is_open_draft(d))
            .collect()
    }

    /// Manual-send files belong to exactly one channel or thread composer.
    pub fn composer_drafts(&self, root: Option<&str>) -> Vec<&crate::uploads::Draft> {
        self.drafts
            .iter()
            .filter(|d| {
                Some(d.upload.channel.as_str()) == self.channel.as_deref()
                    && d.upload.thread_root_id.as_deref() == root
                    && d.auto_send.is_none()
            })
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
        root: Option<&str>,
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
        let used = self.composer_drafts(root).len();
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
                root.map(str::to_owned),
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

    /// Uploads a recording now. When enabled, it sends alone with the reply.
    /// Returns whether the caller should consume its reply, never its text.
    pub fn stage_voice(
        &mut self,
        prepared: Arc<caper_client::uploads::Prepared>,
        reply: Option<String>,
    ) -> Result<bool, String> {
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
        let auto = self.preferences.voice_send_on_stop();
        let auto_send = if auto {
            Some(crate::uploads::AutoSend {
                client_id: uuid::Uuid::new_v4().to_string(),
                chat_token: self
                    .chat_token
                    .clone()
                    .ok_or("The conversation is not connected.")?,
                reply,
                sending: false,
                attempted: false,
            })
        } else {
            None
        };
        self.draft_keys += 1;
        let mut upload =
            caper_client::drafts::Upload::recorded(self.draft_keys, channel, None, prepared);
        upload.restart();
        self.worker.send(upload.command(token, settings));
        let mut draft = crate::uploads::Draft::new(upload);
        draft.auto_send = auto_send;
        self.drafts.push(draft);
        Ok(auto)
    }

    fn send_voice(&mut self, key: u64) {
        let Some(d) = self.drafts.iter_mut().find(|d| d.upload.key == key) else {
            return;
        };
        let Some(send) = d.auto_send.as_mut() else {
            return;
        };
        if send.sending || d.upload.uploading || d.upload.error.is_some() {
            return;
        }
        let Some(file) = &d.upload.attachment else {
            return;
        };
        send.sending = true;
        send.attempted = true;
        self.worker.send(Command::Send {
            generation: self.generation,
            token: self.token.clone(),
            chat_token: send.chat_token.clone(),
            channel: d.upload.channel.clone(),
            client_id: send.client_id.clone(),
            text: String::new(),
            thread_root_id: None,
            broadcast: false,
            reply_to_id: send.reply.clone(),
            attachment_ids: vec![file.id.clone()],
            spoiler_attachment_ids: Vec::new(),
            gif: None,
            suppress_embeds: false,
        });
    }

    /// Navigation/logout cancel unfinished uploads, but never delete an asset
    /// whose send has already reached the server (including a lost response).
    fn discard_voice_sends(&mut self) {
        let mut ids = Vec::new();
        self.drafts.retain(|d| {
            let Some(send) = &d.auto_send else {
                return true;
            };
            d.upload.abandon();
            if !send.attempted
                && let Some(file) = &d.upload.attachment
            {
                ids.push(file.id.clone());
            }
            false
        });
        self.cancel_assets(ids);
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

    /// The link card for `draft`, the main composer's or (with `thread`) the
    /// open thread's reply box: takes an answer that arrived, then asks
    /// about the draft's link once it has stayed for the debounce. Whether
    /// the card changed, and when to look again.
    pub fn preview_tick(&mut self, thread: bool, draft: &str) -> (bool, Option<Duration>) {
        let (preview, pending) = if thread {
            (&mut self.thread_preview, &mut self.thread_preview_pending)
        } else {
            (&mut self.preview, &mut self.preview_pending)
        };
        let before = preview.card().cloned();
        if let Some(receiver) = pending.as_ref()
            && let Ok((url, embed)) = receiver.try_recv()
        {
            *pending = None;
            preview.answer(url, embed);
        }
        let mut wait = None;
        match preview.due(draft, self.token.is_some(), std::time::Instant::now()) {
            (Some(url), _) => {
                if let (Some(api), Some(token)) = (self.media.api().cloned(), self.token.clone()) {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    let host = self.host.clone();
                    std::thread::spawn(move || {
                        let embed = api.embed_preview(&token, &url);
                        let _ = sender.send((url, embed));
                        host.wake();
                    });
                    *pending = Some(receiver);
                }
            }
            (None, after) => wait = after,
        }
        (before.as_ref() != preview.card(), wait)
    }

    /// Removes a composer file, stopping its upload and giving up the
    /// server's copy.
    pub fn remove_draft(&mut self, key: u64) {
        self.cancel_draft_edit(key);
        if let Some(i) = self.drafts.iter().position(|d| d.upload.key == key) {
            if self.drafts[i].auto_send.as_ref().is_some_and(|s| s.sending) {
                return;
            }
            let draft = self.drafts.remove(i);
            draft.upload.abandon();
            if !draft.auto_send.as_ref().is_some_and(|s| s.attempted)
                && let Some(file) = draft.upload.attachment
            {
                self.cancel_assets(vec![file.id]);
            }
        }
    }

    pub fn toggle_spoiler(&mut self, key: u64) {
        if let Some(d) = self.drafts.iter_mut().find(|d| d.upload.key == key) {
            d.upload.spoiler = !d.upload.spoiler;
        }
    }

    /// Stage an edited replacement separately. Never consume the original
    /// draft or its server reservation until preparation and upload succeed.
    pub fn save_draft_edit(
        &mut self,
        key: u64,
        edit: Option<caper_client::attachment_edit::Edit>,
    ) -> Result<(), &'static str> {
        let (Some(token), Some(settings)) = (self.token.clone(), self.uploads.clone()) else {
            return Err("Uploads are unavailable. Your original attachment is unchanged.");
        };
        if self.edit_upload.is_some() {
            return Err("An attachment edit is already uploading.");
        }
        let d = self
            .drafts
            .iter()
            .find(|d| d.upload.key == key)
            .ok_or("The attachment is no longer available.")?;
        let source = &d.upload;
        let path = source
            .local_path
            .clone()
            .ok_or("The original file is unavailable.")?;
        self.draft_keys += 1;
        let mut replacement = caper_client::drafts::Upload::picked(
            self.draft_keys,
            source.channel.clone(),
            source.thread_root_id.clone(),
            source.source_name.clone(),
            Some(path),
        );
        replacement.slot = source.slot;
        replacement.spoiler = source.spoiler;
        replacement.source_scratch = source.source_scratch.clone();
        replacement.edit = edit;
        self.worker.send(replacement.command(token, settings));
        self.edit_upload = Some((key, crate::uploads::Draft::new(replacement)));
        Ok(())
    }

    pub fn cancel_draft_edit(&mut self, key: u64) {
        if self
            .edit_upload
            .as_ref()
            .is_some_and(|(old, _)| *old == key)
            && let Some((_, replacement)) = self.edit_upload.take()
        {
            replacement.upload.abandon();
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
        if let Some(send) = &d.auto_send {
            if send.sending {
                return;
            }
            // Retry the exact send after an uncertain response. Reuploading or
            // deleting its asset could damage an already accepted message.
            if send.attempted {
                d.upload.error = None;
                self.send_voice(key);
                return;
            }
        }
        let Some(old) = d.upload.restart() else {
            return;
        };
        self.worker.send(d.upload.command(token, settings));
        if let Some(old) = old {
            self.cancel_assets(vec![old.id]);
        }
    }

    pub(crate) fn cancel_assets(&self, ids: Vec<String>) {
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
    pub fn send_with_files(
        &mut self,
        text: String,
        reply_to: Option<String>,
        thread: Option<(String, bool)>,
    ) -> Sent {
        let root = thread.as_ref().map(|(root, _)| root.as_str());
        let keys: Vec<u64> = self
            .composer_drafts(root)
            .iter()
            .map(|d| d.upload.key)
            .collect();
        if self
            .edit_upload
            .as_ref()
            .is_some_and(|(old, _)| keys.contains(old))
        {
            return Sent::Waiting;
        }
        for key in &keys {
            self.retry_draft(*key);
        }
        if self
            .composer_drafts(root)
            .iter()
            .any(|d| d.upload.uploading || d.upload.needs_upload())
        {
            return Sent::Waiting;
        }
        let (Some(channel), Some(chat_token)) = (&self.channel, &self.chat_token) else {
            return Sent::Waiting;
        };
        let files: Vec<_> = self
            .composer_drafts(root)
            .iter()
            .filter_map(|d| d.upload.send_attachment())
            .collect();
        // A removed card sends the message without previews.
        let suppress_embeds = if thread.is_some() {
            self.thread_preview.removed(&text)
        } else {
            self.preview.removed(&text)
        };
        let outgoing = core_worker::Outgoing {
            broadcast: thread.as_ref().is_some_and(|(_, broadcast)| *broadcast),
            thread_root_id: thread.map(|(root, _)| root),
            reply_to_id: reply_to,
            spoiler_attachment_ids: files
                .iter()
                .filter(|f| f.spoiler)
                .map(|f| f.id.clone())
                .collect(),
            attachment_ids: files.into_iter().map(|f| f.id).collect(),
            suppress_embeds,
            ..core_worker::Outgoing::new(channel.clone(), text)
        };
        let chat_token = chat_token.clone();
        self.dispatch(outgoing, chat_token);
        // The message owns the files now: nothing is cancelled.
        self.drafts.retain(|d| !keys.contains(&d.upload.key));
        Sent::Sent
    }

    /// Opens a space (its first joined channel unless `channel` is given)
    /// or a channel. The open conversation stays until the new one loads.
    pub fn navigate(&mut self, space: Option<String>, channel: Option<String>) {
        if let Some(typing) = self.typing.stop() {
            self.send_typing(typing);
        }
        self.navigation += 1;
        self.join_error = None;
        self.marked = None;
        let visiting = self.report_target.as_ref().is_some_and(|(n, t)| {
            *n == self.navigation && channel.as_deref() == Some(t.channel.as_str())
        });
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
            visit: visiting,
            preferred: None,
        });
    }

    /// The main composer changed: announces your typing, or its end.
    pub fn typed(&mut self, draft: &str) {
        if let Some(typing) = self.typing.edited(draft, Instant::now()) {
            self.send_typing(typing);
        }
    }

    /// Expires who is typing and ends yours after a pause. True when the
    /// row changed.
    pub fn typing_tick(&mut self) -> bool {
        let (send, changed) = self.typing.tick(Instant::now());
        if let Some(typing) = send {
            self.send_typing(typing);
        }
        changed
    }

    fn send_typing(&mut self, typing: bool) {
        let (Some(chat_token), Some(channel)) = (&self.chat_token, &self.channel) else {
            return;
        };
        self.worker.send(Command::Typing {
            token: self.token.clone(),
            chat_token: chat_token.clone(),
            channel: channel.clone(),
            typing,
        });
    }

    /// Sends `text` to the open channel, replying to message `reply_to`;
    /// with `thread`, into that root's thread (and to the channel too when
    /// its flag is set).
    pub fn send(&self, text: String, reply_to: Option<String>, thread: Option<(String, bool)>) {
        let (Some(channel), Some(chat_token)) = (&self.channel, &self.chat_token) else {
            return;
        };
        // A removed card sends without previews: the channel composer's,
        // or the reply box's for a thread reply.
        let suppress_embeds = if thread.is_some() {
            self.thread_preview.removed(&text)
        } else {
            self.preview.removed(&text)
        };
        let outgoing = core_worker::Outgoing {
            broadcast: thread.as_ref().is_some_and(|(_, b)| *b),
            thread_root_id: thread.map(|(root, _)| root),
            reply_to_id: reply_to,
            suppress_embeds,
            ..core_worker::Outgoing::new(channel.clone(), text)
        };
        self.dispatch(outgoing, chat_token.clone());
    }

    /// Sends `outgoing`, keeping it until the server confirms it.
    fn dispatch(&self, outgoing: core_worker::Outgoing, chat_token: String) {
        self.worker
            .send(outgoing.command(self.generation, self.token.clone(), chat_token));
        self.outgoing
            .borrow_mut()
            .insert(outgoing.client_id.clone(), (outgoing, false));
    }

    /// Back online: each send whose outcome was unknown goes again unchanged.
    fn resend_unconfirmed(&self) {
        let Some(chat_token) = &self.chat_token else {
            return;
        };
        for (outgoing, unconfirmed) in self.outgoing.borrow_mut().values_mut() {
            if *unconfirmed && self.channel.as_deref() == Some(outgoing.channel.as_str()) {
                *unconfirmed = false;
                self.worker.send(outgoing.command(
                    self.generation,
                    self.token.clone(),
                    chat_token.clone(),
                ));
            }
        }
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
    /// ends the history, and the "You're viewing older messages" toast
    /// always shows.
    pub fn has_newer(&self) -> bool {
        self.window.is_some_and(|(_, end)| end.is_some())
    }

    /// View original on a reply's quote: the original's thread for a thread
    /// reply, the shown row when it is loaded, or the history around it.
    pub fn view_original(&mut self, id: &str, thread_root: Option<&str>) {
        if let Some(root) = thread_root {
            self.open_thread_at(root, id);
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

    /// Leaves the window for the channel's latest history (the toast's
    /// Jump to latest, and End).
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
        let Some(m) = self.reactor_message(id) else {
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
        let channel = m.channel_id.clone();
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

    /// The live reaction snapshot, including a thread reply or a pinned row.
    pub fn reactor_message(&self, id: &str) -> Option<core::Message> {
        self.timeline
            .messages()
            .chain(self.timeline.pinned_messages())
            .find(|m| m.id == id)
            .map(|m| self.shown(m))
            .filter(|m| !m.is_deleted())
    }

    pub fn reactor_cache(&self, id: &str) -> Option<&ReactorCache> {
        self.reactors.get(id)
    }

    /// The open conversation's pins as egui's Pins dialog lists them, newest
    /// pin first with pending pins and unpins applied: one ungrouped card
    /// each, dated in full, with whether you pinned it and its thread root.
    pub fn pinned_view(&self) -> Option<(Conversation, Vec<(bool, Option<String>)>)> {
        self.channel.as_ref()?;
        let spaces = core::Spaces {
            spaces: Vec::new(),
            invitations: Vec::new(),
            limits: None,
        };
        let pins: Vec<core::Message> = self
            .mutations
            .pinned(&self.timeline)
            .iter()
            .map(|m| self.shown(m))
            .collect();
        let (mut c, ids) = conversation(
            &spaces,
            self.detail.as_ref(),
            self.channel.as_deref(),
            &pins,
            self.me.as_deref(),
            true,
            None,
            self.direct_head(),
        );
        let mut about = vec![(false, None); c.messages.len()];
        for m in &pins {
            let Some(&k) = ids.rows.get(&m.id) else {
                continue;
            };
            let shown = &mut c.messages[k];
            shown.grouped = false;
            shown.date = None;
            shown.edited_inline = false;
            shown.thread_replies = None;
            shown.thread_context = false;
            shown.time = chrono::DateTime::parse_from_rfc3339(&m.created_at).map_or_else(
                |_| m.created_at.clone(),
                |t| {
                    t.with_timezone(&chrono::Local)
                        .format("%b %-d, %Y · %-I:%M %p")
                        .to_string()
                },
            );
            if m.revision > 1 && !m.is_deleted() {
                shown.time.push_str(" (edited)");
            }
            about[k] = (
                m.pin
                    .as_ref()
                    .is_some_and(|p| self.me.as_deref() == Some(p.author.id.as_str())),
                m.thread_root_id.clone(),
            );
        }
        Some((c, about))
    }

    /// The open space's members' usernames by id, for member search.
    pub fn member_usernames(&self) -> HashMap<&str, &str> {
        self.detail.as_ref().map_or_else(HashMap::new, |d| {
            d.members
                .iter()
                .map(|m| (m.id.as_str(), m.username.as_str()))
                .collect()
        })
    }

    pub fn retry_reactors(&mut self, id: &str) {
        if let Some(cached) = self.reactors.get_mut(id) {
            cached.revision.clear();
        }
        self.load_reactors(id);
    }

    /// A pending own reaction may not have an identity in the loaded groups.
    pub fn reactor_self(&self) -> core::Reactor {
        core::Reactor {
            id: self
                .me
                .as_ref()
                .or(self.account_id.as_ref())
                .cloned()
                .unwrap_or_default(),
            username: self.username.clone(),
            display_name: Some(self.account_name.clone()),
            avatar_id: self.account_avatar.mascot.map(i32::from),
            avatar_asset: self.account_avatar.picture.as_ref().map(|p| p.0.clone()),
            avatar_url: self
                .account_avatar
                .picture
                .as_ref()
                .and_then(|p| p.1.clone()),
        }
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

    pub fn forward_sent(&mut self, message: core::Message) {
        if self.channel.as_deref() != Some(&message.channel_id) {
            return;
        }
        if self.timeline.merge_sent(message.clone()).is_err() || self.has_newer() {
            self.reload();
        } else if self.in_channel(&message) {
            self.queued.push(Update::Message(Box::new(message)));
            self.host.wake();
        }
    }

    /// Only a server-authorized ForwardSource can enter this path. A root
    /// opens its thread too, rather than merely focusing its channel row.
    pub fn open_forward_source(&mut self, source: core::ForwardSource) {
        let root = source
            .message
            .thread_root_id
            .clone()
            .unwrap_or_else(|| source.message.id.clone());
        if self.channel.as_deref() == Some(&source.message.channel_id) {
            self.open_thread_at(&root, &source.message.id);
        } else {
            self.navigate(source.space_id, Some(source.message.channel_id));
            self.focus = Some((self.navigation, source.message.id, Some(root)));
        }
    }

    /// Mark unread is offered for a signed-in reader's channel or DM message,
    /// and in a thread only to people who started or replied in it (only
    /// they have thread unread state).
    pub fn can_mark_unread(&self, m: &core::Message, in_thread: bool) -> bool {
        if self.token.is_none() || m.is_deleted() || self.feedback_visit.is_some() {
            return false;
        }
        if !in_thread {
            return (self.direct.is_some() && self.direct == self.channel) || self.space.is_some();
        }
        let (Some(me), Some(thread)) = (self.me.as_deref(), &self.thread) else {
            return false;
        };
        self.timeline.messages().any(|r| {
            (r.id == thread.root || r.thread_root_id.as_deref() == Some(thread.root.as_str()))
                && r.author.id == me
        })
    }

    /// Mark unread: moves the read position to just before `m`, synced
    /// through the server.
    pub fn mark_unread(&mut self, m: &core::Message, in_thread: bool) {
        if !self.can_mark_unread(m, in_thread) {
            return;
        }
        let (Some(token), Some(channel)) = (self.token.clone(), self.channel.clone()) else {
            return;
        };
        let seq = core::sequence(&m.seq)
            .unwrap_or(1)
            .saturating_sub(1)
            .to_string();
        let root = if in_thread {
            self.thread.as_ref().map(|t| t.root.clone())
        } else {
            None
        };
        let target = match (&root, &self.direct, &self.space) {
            (Some(root), ..) => caper_client::api::UnreadTarget::Thread {
                channel: channel.clone(),
                root: root.clone(),
            },
            (None, Some(direct), _) if *direct == channel => {
                caper_client::api::UnreadTarget::Direct {
                    conversation: channel.clone(),
                }
            }
            (None, _, Some(space)) => caper_client::api::UnreadTarget::Channel {
                space: space.clone(),
                channel: channel.clone(),
            },
            _ => return,
        };
        self.marked = Some((channel, root.clone(), m.id.clone()));
        if root.is_none() {
            self.queued.push(Update::Unread(m.id.clone()));
        }
        self.worker.send(Command::MarkUnread {
            epoch: self.account_epoch,
            token,
            target,
            seq,
        });
        self.host.wake();
    }

    /// A message link (`caper.chat/m/...`): asks the API whether this account
    /// may see it, then opens it like a notification.
    pub fn open_message_link(&mut self, channel: &str, message: &str) {
        let Some(token) = self.token.clone() else {
            return;
        };
        self.worker.send(Command::ResolveMessageLink {
            epoch: self.account_epoch,
            token,
            channel: channel.to_owned(),
            message: message.to_owned(),
        });
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

    /// Up in an empty composer: the message Edit message would open for the
    /// channel's rows, or for thread `root`'s root and replies.
    pub fn last_editable(&self, root: Option<&str>) -> Option<core::Message> {
        let me = self.author.as_ref()?.id.as_str();
        let shown = self
            .timeline
            .messages()
            .filter(|m| root.is_some() || self.in_channel(m));
        last_editable(shown, me, root)
            .filter(|m| self.can_edit(m))
            .cloned()
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
        let channel = self.channel.as_ref()?;
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
            self.detail.as_ref(),
            Some(channel),
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
            focus: None,
        });
        self.load_thread(false);
    }

    /// Opens an exact root/reply, including one older than the latest page.
    /// Pins, inbox navigation and forwarded sources share this path.
    pub fn open_thread_at(&mut self, root: &str, id: &str) {
        self.open_thread(root);
        if let Some(t) = &mut self.thread {
            t.focus = Some(id.to_owned());
        }
        self.advance_thread_focus();
        self.queued.push(Update::Thread);
    }

    fn advance_thread_focus(&mut self) {
        let Some(t) = &self.thread else { return };
        let Some(id) = &t.focus else { return };
        if t.loading || t.error.is_some() {
            return;
        }
        match self.message(id) {
            Some(m)
                if !m.is_deleted()
                    && (m.id == t.root || m.thread_root_id.as_deref() == Some(&t.root)) => {}
            None if t.has_more => self.load_thread(true),
            _ => {
                self.thread.as_mut().unwrap().focus = None;
                self.queued
                    .push(Update::Warning("This reply is no longer available.".into()));
            }
        }
    }

    pub fn take_thread_focus(&mut self) -> Option<String> {
        let t = self.thread.as_mut()?;
        if t.loading || t.error.is_some() {
            return None;
        }
        t.focus.take()
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
        let channel = self.channel.as_ref()?;
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
            self.detail.as_ref(),
            Some(channel),
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
        if let Some(visit) = &self.feedback_visit {
            self.open_feedback_report(caper_client::feedback::VisitTarget {
                channel: visit.channel.clone(),
                message: String::new(),
                thread: None,
                reporter: visit.reporter.clone(),
            });
            return;
        }
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
            self.refresh_membership();
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
        // Rows loaded ahead show at once and refresh in place.
        let ready = self.activity.ready();
        self.load_activity(false, ready);
    }

    pub fn close_activity(&mut self) {
        self.activity.shown = false;
    }

    pub fn load_feedback(&mut self, more: bool) {
        let Some(token) = self.token.clone().filter(|_| self.feedback_team) else {
            return;
        };
        if let Some((request, before)) = self.feedback.start(more) {
            self.worker.send(Command::LoadFeedback {
                epoch: self.account_epoch,
                request,
                token,
                before,
                archived: self.feedback.wanted(),
            });
        }
    }

    pub fn archive_feedback(&mut self, report: &caper_client::feedback::Report) {
        let Some(token) = self.token.clone().filter(|_| self.feedback_team) else {
            return;
        };
        let archived = !self.feedback.archived;
        self.feedback
            .remove_thread(&report.channel.id, &report.thread_root_id);
        self.worker.send(Command::ArchiveFeedback {
            epoch: self.account_epoch,
            token,
            channel: report.channel.id.clone(),
            thread_root: report.thread_root_id.clone(),
            archived,
        });
    }

    pub fn open_feedback_report(&mut self, target: caper_client::feedback::VisitTarget) {
        if !self.feedback_team || self.token.is_none() {
            return;
        }
        self.report_target = Some((self.navigation + 1, target.clone()));
        self.navigate(None, Some(target.channel.clone()));
        self.focus = (!target.message.is_empty()).then(|| {
            (
                self.navigation,
                target.message.clone(),
                target.thread.filter(|root| root != &target.message),
            )
        });
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
    /// the row's whole group reads at once in one request, and opening
    /// waits for the read, then opens its oldest item. Reading never
    /// accepts an invitation.
    pub fn activity_action(&mut self, k: usize, open: bool) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let Some(group) = self.activity.visible_groups().into_iter().nth(k) else {
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
            self.activity.opening = Some(group.id().to_owned());
        }
        self.activity.error = None;
        self.activity.mark_read(&group.items);
        self.worker.send(Command::ReadActivity {
            epoch: self.account_epoch,
            navigation: self.navigation,
            token,
            items: group.items,
            open,
        });
    }

    /// egui's `open_activity_item`: the inbox closes over the notification's
    /// conversation, at its message; an invitation's dialog opens.
    fn open_activity_item(&mut self, item: caper_client::activity::Item, out: &mut Vec<Update>) {
        self.activity.shown = false;
        if item.kind == "space.invitation" || item.kind == "channel.invitation" {
            match item.space_id {
                Some(space) => self.open_invitation(space, item.channel_id),
                None => out.push(Update::Warning(
                    "This invitation is no longer available.".into(),
                )),
            }
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

    /// Faces in a channel: your media roster when in it, otherwise the
    /// gateway's spectator-safe identities. Only your call has levels.
    pub fn voice_people(&self, id: &str) -> Vec<core::VoiceOccupant> {
        if !self.own_call(id) {
            return self
                .rosters
                .get(id)
                .map_or_else(Vec::new, |(p, _)| p.clone());
        }
        self.voice
            .participants
            .iter()
            .map(|p| {
                let own = p.id == self.voice.self_id;
                core::VoiceOccupant {
                    id: p.id.clone(),
                    avatar_id: p.avatar_id,
                    avatar_asset: p.avatar_asset.clone(),
                    avatar_url: p.avatar_url.clone(),
                    name: if own {
                        self.account_name.clone()
                    } else {
                        p.name.clone()
                    },
                    muted: if own {
                        self.voice.state.audio.muted
                    } else {
                        p.muted
                    },
                    deafened: if own {
                        self.voice.state.audio.deafened
                    } else {
                        p.deafened
                    },
                }
            })
            .collect()
    }

    pub fn voice_speaking(&self, channel: &str, p: &core::VoiceOccupant) -> bool {
        self.own_call(channel) && self.voice.speaking(&p.id, p.muted, Instant::now())
    }

    /// None: quiet, false: active elsewhere, true: connected here. Joining
    /// or reconnecting alone must not claim a working connection.
    pub fn voice_badge(&self, space: &str) -> Option<bool> {
        let connected = self.voice.active_space.as_deref() == Some(space)
            && matches!(self.voice.state.phase, Phase::Connected(_));
        (connected || self.voice_spaces.contains(space)).then_some(connected)
    }

    /// A local listening preference, never a remote microphone command.
    pub fn participant_playback(&mut self, id: &str, gain: u16, muted: bool) {
        self.voice.set_participant_playback(
            id,
            caper_client::voice_media::TrackPlayback {
                gain_percent: gain,
                muted,
            },
        );
        self.queued.push(Update::Voice);
    }

    fn poll_voice_activity(&mut self) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let elapsed = self.voice_activity_at.map(|at| at.elapsed());
        if !self.voice_activity_pending && elapsed.is_none_or(|e| e >= MEMBERSHIP_EVERY) {
            self.voice_activity_at = Some(Instant::now());
            self.voice_activity_pending = true;
            self.worker.send(Command::LoadVoiceActivity {
                epoch: self.account_epoch,
                token,
            });
        }
        let delay = MEMBERSHIP_EVERY
            .saturating_sub(
                self.voice_activity_at
                    .map_or(Duration::ZERO, |at| at.elapsed()),
            )
            .max(Duration::from_secs(1));
        self.host.wake_after(delay);
    }

    /// Whether your call (not a failed one) is in channel `id`.
    fn own_call(&self, id: &str) -> bool {
        self.voice.state.active_channel() == Some(id)
            && !matches!(self.voice.state.phase, Phase::Failed(_))
    }

    /// The open channel when you haven't joined it (a preview from Browse).
    fn open_preview(&self) -> Option<&core::Channel> {
        if self.direct.is_some() {
            return None;
        }
        let channel = self.channel.as_ref()?;
        self.detail
            .as_ref()?
            .channels
            .iter()
            .find(|c| c.id == *channel && !c.joined)
    }

    /// Browse channels for the open space: every channel, joined or
    /// not, with the unread dot of one with something new.
    pub fn browse(&self) -> Option<crate::browse::Browse> {
        let detail = self.detail.as_ref()?;
        let unread = |id: &str| {
            self.unread_spaces
                .iter()
                .any(|s| s.channels.iter().any(|c| c.id == id))
        };
        Some(crate::browse::Browse {
            owner: self.owns_space(),
            space: detail.space.id.clone(),
            space_name: detail.space.name.clone(),
            rows: detail
                .channels
                .iter()
                .map(|c| crate::browse::Row {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    private: c.private,
                    joined: c.joined,
                    unread: unread(&c.id),
                    detail: String::new(),
                    alternates: Vec::new(),
                    target: crate::browse::Target::Channel(detail.space.id.clone()),
                })
                .collect(),
            switcher: false,
            help: false,
        })
    }

    fn channel_unread(&self, space: &str, channel: &str) -> bool {
        self.unread_spaces
            .iter()
            .any(|s| s.id == space && s.channels.iter().any(|c| c.id == channel))
    }

    fn direct_unread(&self, d: &core::DirectConversation) -> bool {
        d.status != core::DirectStatus::Incoming
            && !d.blocked
            && !self.is_blocked(&d.peer.id)
            && d.has_unread()
    }

    /// The quick switcher (Ctrl+K): every channel of the open space, DMs,
    /// spaces and people you have no DM with, ranked as web's.
    pub fn switcher(&self) -> crate::browse::Browse {
        use crate::browse::{Row, Target};
        let me = self.account_id.as_deref();
        let mut rows = Vec::new();
        if let Some(detail) = &self.detail {
            for c in detail.channels.iter().filter(|c| c.joined) {
                rows.push(Row {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    private: c.private,
                    joined: true,
                    unread: self.channel_unread(&detail.space.id, &c.id),
                    detail: detail.space.name.clone(),
                    alternates: Vec::new(),
                    target: Target::Channel(detail.space.id.clone()),
                });
            }
        }
        for d in &self.directs {
            let own = Some(d.peer.id.as_str()) == me;
            rows.push(Row {
                id: d.id.clone(),
                name: if own {
                    format!("{} (you)", d.peer.display_name)
                } else {
                    d.peer.display_name.clone()
                },
                private: false,
                joined: true,
                unread: d.status == core::DirectStatus::Incoming || self.direct_unread(d),
                detail: format!("@{}", d.peer.username),
                alternates: vec![d.peer.username.clone()],
                target: Target::Direct,
            });
        }
        for space in self
            .spaces
            .iter()
            .flat_map(|s| &s.spaces)
            .filter(|s| !s.demo)
        {
            rows.push(Row {
                id: space.id.clone(),
                name: space.name.clone(),
                private: false,
                joined: true,
                unread: self.unread_spaces.iter().any(|s| s.id == space.id),
                detail: "Space".into(),
                alternates: Vec::new(),
                target: Target::Space,
            });
        }
        for p in &self.people {
            if Some(p.id.as_str()) == me
                || self.is_blocked(&p.id)
                || self.directs.iter().any(|d| d.peer.id == p.id)
            {
                continue;
            }
            rows.push(Row {
                id: p.id.clone(),
                name: p.display_name.clone(),
                private: false,
                joined: true,
                unread: false,
                detail: format!("@{}", p.username),
                alternates: vec![p.username.clone()],
                target: Target::Person(p.username.clone()),
            });
        }
        crate::browse::Browse {
            owner: false,
            space: self.space.clone().unwrap_or_default(),
            space_name: String::new(),
            rows,
            switcher: true,
            help: false,
        }
    }

    /// Opens a quick switcher row.
    pub fn switch_to(&mut self, row: &crate::browse::Row) {
        use crate::browse::Target;
        match &row.target {
            Target::Channel(space) => {
                if self.direct.is_some() || self.channel.as_deref() != Some(row.id.as_str()) {
                    self.navigate(Some(space.clone()), Some(row.id.clone()));
                }
            }
            Target::Direct => self.open_direct(row.id.clone()),
            Target::Space => {
                if self.direct.is_some() || self.space.as_deref() != Some(row.id.as_str()) {
                    self.navigate(Some(row.id.clone()), None);
                }
            }
            Target::Person(username) => self.create_direct(username.clone()),
        }
    }

    /// Alt+Up/Down: the previous or next conversation in the sidebar's
    /// order (the space's joined channels, then your notes and DMs); with
    /// `unread_only` (Alt+Shift), the previous or next unread one.
    pub fn step_conversation(&mut self, forward: bool, unread_only: bool) {
        let me = self.account_id.as_deref();
        let mut order: Vec<(Option<String>, String, bool)> = Vec::new();
        if let Some(detail) = &self.detail {
            for c in detail.channels.iter().filter(|c| c.joined) {
                let unread = self.channel_unread(&detail.space.id, &c.id);
                order.push((Some(detail.space.id.clone()), c.id.clone(), unread));
            }
        }
        let own = self
            .directs
            .iter()
            .filter(|d| Some(d.peer.id.as_str()) == me);
        let others = self
            .directs
            .iter()
            .filter(|d| Some(d.peer.id.as_str()) != me);
        for d in own
            .chain(others)
            .filter(|d| d.status != core::DirectStatus::Incoming)
        {
            order.push((None, d.id.clone(), self.direct_unread(d)));
        }
        let current = order.iter().position(|(space, id, _)| match space {
            Some(_) => self.direct.is_none() && self.channel.as_deref() == Some(id.as_str()),
            None => self.direct.as_deref() == Some(id.as_str()),
        });
        let unread: Vec<bool> = order.iter().map(|(_, _, unread)| *unread).collect();
        if let Some(next) = caper_client::switcher::adjacent(&unread, current, forward, unread_only)
        {
            let (space, id, _) = order.swap_remove(next);
            match space {
                Some(space) => self.navigate(Some(space), Some(id)),
                None => self.open_direct(id),
            }
        }
    }

    /// Escape in a conversation: marks it read through its newest message,
    /// even while reading further back. Whether there was anything to read.
    pub fn mark_open_read(&mut self) -> bool {
        if self.feedback_visit.is_some() {
            return false;
        }
        let (Some(channel), Some(token)) = (self.channel.clone(), self.token.clone()) else {
            return false;
        };
        if self.direct.as_ref() == Some(&channel) {
            let blocked = |d: &core::DirectConversation| self.is_blocked(&d.peer.id);
            let Some(index) = self.directs.iter().position(|d| {
                d.id == channel
                    && d.status != core::DirectStatus::Incoming
                    && d.has_unread()
                    && !blocked(d)
            }) else {
                return false;
            };
            let d = &mut self.directs[index];
            d.read_seq.clone_from(&d.last_seq);
            d.unread_count = Some(0);
            let seq = d.last_seq.clone();
            self.queued.push(Update::Badges);
            self.worker.send(Command::ReadDirect {
                token,
                id: channel,
                seq,
            });
            return true;
        }
        let Some(space) = self.space.clone() else {
            return false;
        };
        let Some(seq) = self
            .unread_spaces
            .iter()
            .filter(|s| s.id == space)
            .flat_map(|s| &s.channels)
            .find(|c| c.id == channel)
            .map(|c| c.last_seq.clone())
        else {
            return false;
        };
        for s in &mut self.unread_spaces {
            s.channels.retain(|c| c.id != channel);
        }
        self.unread_spaces.retain(|s| !s.channels.is_empty());
        self.reads.insert(channel.clone(), seq.clone());
        self.queued.push(Update::Badges);
        self.worker.send(Command::ReadChannel {
            epoch: 0,
            token,
            space,
            channel,
            seq,
        });
        true
    }

    /// Connected to a voice channel, where Ctrl+Shift+M and +D apply.
    pub fn in_call(&self) -> bool {
        matches!(self.voice.state.phase, Phase::Connected(_))
    }

    /// Joins the previewed channel; it opens joined once the server
    /// answers.
    pub fn join_channel(&mut self) {
        let (Some(token), Some(space), Some(channel)) = (
            self.token.clone(),
            self.space.clone(),
            self.open_preview().map(|c| c.id.clone()),
        ) else {
            return;
        };
        self.joining = true;
        self.join_error = None;
        self.worker.send(Command::Admin {
            generation: self.account_epoch,
            token,
            operation: AdminOperation::JoinChannel { space, channel },
        });
    }

    /// Invite people for the open space you own, from its menu or a
    /// public channel's (whose link then opens in that channel).
    pub fn invite_people(
        &self,
        channel: Option<String>,
    ) -> Option<crate::invite_people::InvitePeople> {
        let d = self.detail.as_ref().filter(|_| self.owns_space())?;
        let prefix = caper_client::invites::invite_url(self.media.api()?.base(), "");
        let public: Vec<_> = d
            .channels
            .iter()
            .filter(|c| !c.private)
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect();
        let channel = channel.filter(|id| public.iter().any(|(p, _)| p == id));
        Some(crate::invite_people::InvitePeople::new(
            d.space.id.clone(),
            d.space.name.clone(),
            channel,
            public,
            prefix,
            d.members.clone(),
        ))
    }

    /// Whether a menu offers Invite people: the open space you own, or
    /// one of its public channels (egui's).
    pub fn can_invite(&self, scope: &Scope) -> bool {
        let Some(d) = self.detail.as_ref().filter(|_| self.owns_space()) else {
            return false;
        };
        match scope {
            Scope::Space(space) => *space == d.space.id,
            Scope::Channel { space, channel } => {
                *space == d.space.id && d.channels.iter().any(|c| c.id == *channel && !c.private)
            }
            _ => false,
        }
    }

    /// Whether a menu offers Space settings or Channel settings: the open
    /// space you own, or any of its channels.
    pub fn can_manage(&self, scope: &Scope) -> bool {
        let Some(d) = self.detail.as_ref().filter(|_| self.owns_space()) else {
            return false;
        };
        match scope {
            Scope::Space(space) => *space == d.space.id,
            Scope::Channel { space, channel } => {
                *space == d.space.id && d.channels.iter().any(|c| c.id == *channel)
            }
            _ => false,
        }
    }

    /// Channel settings for one of the open space's channels, if you own it.
    pub fn channel_settings(&self, id: &str) -> Option<crate::invite_people::InvitePeople> {
        let c = self.detail.as_ref()?.channels.iter().find(|c| c.id == id)?;
        let owner = self.account_id.clone()?;
        Some(
            self.invite_people(None)?
                .channel_settings(c.id.clone(), &c.name, c.private, owner),
        )
    }

    /// Sends an owner's space or member change.
    pub fn admin(&self, operation: AdminOperation) {
        if let Some(token) = self.token.clone() {
            self.worker.send(Command::Admin {
                generation: self.account_epoch,
                token,
                operation,
            });
        }
    }

    /// Whether you own the open space (its owner manages it).
    pub fn owns_space(&self) -> bool {
        self.detail.as_ref().is_some_and(|d| {
            !d.space.demo && self.account_id.as_deref() == Some(d.space.owner_id.as_str())
        })
    }

    pub fn space_name(&self) -> String {
        self.detail
            .as_ref()
            .map_or_else(String::new, |d| d.space.name.clone())
    }

    /// Whether a space (`space`) or a channel in the open space can be
    /// created within the account's limits (egui's `can_create_space`
    /// and `can_create_channel`).
    pub fn can_create(&self, space: bool) -> bool {
        let Some(limits) = &self.limits else {
            return false;
        };
        if !space {
            return self
                .detail
                .as_ref()
                .is_some_and(|d| d.channels.len() < limits.channels_per_space);
        }
        let listed: Vec<&core::Space> = self
            .spaces
            .iter()
            .flat_map(|s| &s.spaces)
            .filter(|s| !s.demo)
            .collect();
        let owned = listed
            .iter()
            .filter(|s| self.account_id.as_deref() == Some(s.owner_id.as_str()))
            .count();
        owned < limits.owned_spaces && listed.len() < limits.total_spaces
    }

    /// Create space's or Create channel's name: web's title, or the
    /// limit that stops it.
    pub fn create_label(&self, space: bool) -> String {
        let limits = self.limits.as_ref();
        match (space, self.can_create(space)) {
            (true, true) => "Create space".into(),
            (false, true) => "Create channel".into(),
            (true, false) => {
                let (owned, total) = limits.map_or((20, 100), |l| (l.owned_spaces, l.total_spaces));
                format!("Space limit reached ({owned} owned, {total} total)")
            }
            (false, false) => format!(
                "Channel limit reached ({})",
                limits.map_or(100, |l| l.channels_per_space)
            ),
        }
    }

    /// Creates a space (Create a space) or a channel in the open space
    /// (Create a channel); `Update::Left` answers, and it opens.
    pub fn create(&mut self, name: String, channel: Option<bool>) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let operation = match (channel, self.space.clone()) {
            (None, _) => AdminOperation::CreateSpace { name },
            (Some(private), Some(space)) => AdminOperation::CreateChannel {
                space,
                name,
                private,
            },
            (Some(_), None) => return,
        };
        self.worker.send(Command::Admin {
            generation: self.account_epoch,
            token,
            operation,
        });
    }

    /// The open space's private channels you're invited to.
    pub fn channel_invitations(&self) -> &[core::ChannelInvitation] {
        self.detail
            .as_ref()
            .map_or(&[], |d| d.channel_invitations.as_slice())
    }

    /// Whether `invitation` is still offered (one answered elsewhere or
    /// withdrawn closes its dialog).
    pub fn offered(&self, invitation: &Invitation) -> bool {
        match invitation {
            Invitation::Space(s) => self.invitations.iter().any(|i| i.id == s.id),
            Invitation::Channel(c) => self
                .channel_invitations()
                .iter()
                .any(|i| i.channel.id == c.channel.id),
        }
    }

    /// Accepts or declines `invitation`; `Update::Left` answers.
    pub fn answer_invitation(&mut self, invitation: &Invitation, accept: bool) {
        let Some(token) = self.token.clone() else {
            return;
        };
        let operation = match (invitation, accept) {
            (Invitation::Space(s), true) => AdminOperation::AcceptInvitation {
                space: s.id.clone(),
            },
            (Invitation::Space(s), false) => AdminOperation::DeclineInvitation {
                space: s.id.clone(),
            },
            (Invitation::Channel(c), true) => AdminOperation::AcceptChannelInvitation {
                space: c.channel.space_id.clone(),
                channel: c.channel.id.clone(),
            },
            (Invitation::Channel(c), false) => AdminOperation::DeclineChannelInvitation {
                space: c.channel.space_id.clone(),
                channel: c.channel.id.clone(),
            },
        };
        self.worker.send(Command::Admin {
            generation: self.account_epoch,
            token,
            operation,
        });
    }

    /// Join with invite link: the pasted link's preview, then joining
    /// it. Returns the request, which `Update::InviteLink` answers.
    pub fn preview_invite(&mut self, code: String) -> u64 {
        self.invite_request += 1;
        let request = self.invite_request;
        self.account(AccountOperation::PreviewInvite { request, code });
        request
    }

    pub fn join_invite(&mut self, code: String) -> u64 {
        self.invite_request += 1;
        let request = self.invite_request;
        self.account(AccountOperation::JoinInvite { request, code });
        request
    }

    /// Web's invite join: the space joins the list (a member just opens
    /// it), then opens at the link's channel or its first joined one.
    fn joined_invite(
        &mut self,
        joined: caper_client::invites::JoinedInvite,
        out: &mut Vec<Update>,
    ) {
        let space = joined.space;
        self.invitations.retain(|s| s.id != space.id);
        if let Some(s) = &mut self.spaces {
            match s.spaces.iter_mut().find(|s| s.id == space.id) {
                Some(existing) => *existing = space.clone(),
                None => s.spaces.push(space.clone()),
            }
        }
        self.navigate(Some(space.id), joined.channel_id);
        out.push(Update::Invitations);
    }

    /// Web's membership reconcile, every 15 seconds and when the window
    /// regains focus: invitations made meanwhile appear.
    fn refresh_membership(&mut self) {
        let Some(token) = self.token.clone() else {
            return;
        };
        self.membership_at = Some(Instant::now());
        self.host.wake_after(MEMBERSHIP_EVERY);
        if self.membership_pending {
            return;
        }
        self.membership_pending = true;
        self.membership_request += 1;
        self.worker.send(Command::RefreshMembership {
            epoch: self.account_epoch,
            request: self.membership_request,
            revision: 0,
            navigation: self.navigation,
            token,
            space: self.space.clone().filter(|_| self.direct.is_none()),
        });
    }

    /// A notification's invitation (egui's `OpenInvitation`): the
    /// account's spaces are read again first, so a withdrawn one says so.
    fn open_invitation(&mut self, space: String, channel: Option<String>) {
        let Some(token) = self.token.clone() else {
            return;
        };
        self.navigation += 1;
        self.worker.send(Command::OpenInvitation {
            epoch: self.account_epoch,
            navigation: self.navigation,
            token,
            space,
            channel,
        });
    }

    /// The notification's invitation, still offered: a space's dialog
    /// shows; a channel's once its space is open.
    fn invitation_loaded(
        &mut self,
        space: String,
        channel: Option<String>,
        result: Result<(core::Spaces, Option<core::SpaceDetail>), String>,
        out: &mut Vec<Update>,
    ) {
        let gone = || Update::Warning("This invitation is no longer available.".into());
        let (spaces, detail) = match result {
            Ok(loaded) => loaded,
            Err(e) => return out.push(Update::Warning(e)),
        };
        self.invitations = spaces.invitations;
        out.push(Update::Invitations);
        let Some(channel) = channel else {
            match self.invitations.iter().find(|s| s.id == space) {
                Some(s) => out.push(Update::Invitation(Invitation::Space(s.clone()))),
                None => out.push(gone()),
            }
            return;
        };
        let Some(detail) = detail else {
            return out.push(gone());
        };
        let Some(invite) = detail
            .channel_invitations
            .iter()
            .find(|i| i.channel.id == channel)
            .cloned()
        else {
            return out.push(gone());
        };
        if self.direct.is_none() && self.space.as_deref() == Some(detail.space.id.as_str()) {
            self.detail = Some(detail);
            out.push(Update::Invitation(Invitation::Channel(invite)));
        } else {
            self.navigate(Some(space), None);
            self.invite_focus = Some((self.navigation, channel));
        }
    }

    /// Whether you can leave the open space: it is someone else's (an
    /// owner manages or deletes theirs), as in egui.
    pub fn can_leave_space(&self) -> bool {
        self.detail.as_ref().is_some_and(|d| {
            !d.space.demo && self.account_id.as_deref() != Some(d.space.owner_id.as_str())
        })
    }

    /// What leaving a menu's space or channel leaves, named for its
    /// dialog.
    pub fn leave_target(&self, scope: &Scope) -> Option<crate::overlay::Leave> {
        let detail = self.detail.as_ref()?;
        let owner = self.account_id.as_deref() == Some(detail.space.owner_id.as_str());
        match scope {
            Scope::Space(space) if *space == detail.space.id => Some(crate::overlay::Leave {
                space: space.clone(),
                channel: None,
                name: detail.space.name.clone(),
                private: false,
            }),
            Scope::Channel { space, channel } if *space == detail.space.id => {
                let c = detail.channels.iter().find(|c| c.id == *channel)?;
                Some(crate::overlay::Leave {
                    space: space.clone(),
                    channel: Some(channel.clone()),
                    name: c.name.clone(),
                    private: c.private && !owner,
                })
            }
            _ => None,
        }
    }

    /// Leaves a space or channel; `Update::Left` answers.
    pub fn leave(&mut self, leave: &crate::overlay::Leave) {
        let (Some(token), Some(member)) = (self.token.clone(), self.account_id.clone()) else {
            return;
        };
        let operation = match &leave.channel {
            Some(channel) => AdminOperation::LeaveChannel {
                space: leave.space.clone(),
                channel: channel.clone(),
            },
            None => AdminOperation::LeaveSpace {
                space: leave.space.clone(),
                member,
            },
        };
        self.worker.send(Command::Admin {
            generation: self.account_epoch,
            token,
            operation,
        });
    }

    /// Another space or channel opens after one was left: the next
    /// account space, else a DM (egui's `forget_open_space`).
    fn left(&mut self, result: AdminResult, out: &mut Vec<Update>) {
        match result {
            AdminResult::SpaceLeft(id) => {
                self.voice.revoke_space(&id);
                if let Some(s) = &mut self.spaces {
                    s.spaces.retain(|s| s.id != id);
                }
                // Fence late history/gateway responses and discard the departed
                // conversation even when there is nothing else to navigate to.
                self.discard_voice_sends();
                self.generation += 1;
                self.navigation += 1;
                self.worker.send(Command::StopGateway);
                self.detail = None;
                self.space = None;
                self.channel = None;
                self.direct = None;
                self.chat_token = None;
                self.timeline = core::Timeline::default();
                self.thread = None;
                self.thread_only.clear();
                self.window = None;
                self.context_loading = false;
                self.prefetch = None;
                self.focus = None;
                self.invite_focus = None;
                self.has_more = false;
                self.loading_older = false;
                self.older_error = None;
                self.rosters.clear();
                if let Some(spaces) = &self.spaces {
                    out.push(Update::Loaded(Box::new(conversation(
                        spaces,
                        None,
                        None,
                        &[],
                        self.me.as_deref(),
                        false,
                        None,
                        None,
                    ))));
                }
                let next = self
                    .spaces
                    .as_ref()
                    .and_then(|s| s.spaces.iter().find(|s| !s.demo))
                    .map(|s| s.id.clone());
                let direct = self.directs.first().map(|d| d.id.clone());
                match (next, direct) {
                    (Some(space), _) => self.navigate(Some(space), None),
                    (None, Some(direct)) => self.navigate(None, Some(direct)),
                    (None, None) => {}
                }
            }
            AdminResult::ChannelLeft(channel) => {
                self.voice.revoke_channel(&channel);
                // The space reloads without it; another channel opens
                // only when it was the open one.
                let open = self.channel.clone().filter(|c| *c != channel);
                self.navigate(self.space.clone(), open);
            }
            AdminResult::ChannelMembership {
                detail,
                channel,
                joined: true,
            } => {
                // It opens, now in the sidebar with its composer (joined
                // from a preview or an accepted invitation).
                self.joining = false;
                self.navigate(Some(detail.space.id.clone()), Some(channel));
            }
            AdminResult::SpaceCreated(space) => {
                if let Some(s) = &mut self.spaces {
                    s.spaces.push(space.clone());
                }
                self.navigate(Some(space.id), None);
            }
            AdminResult::ChannelCreated(channel) => {
                if let Some(d) = &mut self.detail {
                    d.channels.push(channel.clone());
                }
                // A new private channel's settings open, to add its people
                // (web's).
                let settings = channel.private.then(|| channel.id.clone());
                self.navigate(Some(channel.space_id), Some(channel.id));
                out.push(Update::Left(None));
                out.extend(settings.map(Update::ChannelSettings));
                return;
            }
            AdminResult::SpaceDeleted(id) => {
                // The dialog closes, and the space goes as a left one does.
                out.push(invite_answer(crate::invite_people::Answer::Deleted));
                return self.left(AdminResult::SpaceLeft(id), out);
            }
            AdminResult::ChannelDeleted(channel) => {
                if let Some(d) = &mut self.detail {
                    d.channels.retain(|c| c.id != channel);
                }
                out.push(invite_answer(crate::invite_people::Answer::Deleted));
                return self.left(AdminResult::ChannelLeft(channel), out);
            }
            AdminResult::SpaceUpdated(space) => {
                if let Some(d) = self.detail.as_mut().filter(|d| d.space.id == space.id) {
                    d.space.name = space.name.clone();
                }
                if let Some(s) = self
                    .spaces
                    .as_mut()
                    .and_then(|s| s.spaces.iter_mut().find(|s| s.id == space.id))
                {
                    s.name = space.name.clone();
                }
                out.push(invite_answer(crate::invite_people::Answer::SpaceSaved(
                    space,
                )));
                return;
            }
            AdminResult::ChannelUpdated(channel) => {
                if let Some(c) = self
                    .detail
                    .as_mut()
                    .and_then(|d| d.channels.iter_mut().find(|c| c.id == channel.id))
                {
                    c.name = channel.name.clone();
                    c.private = channel.private;
                }
                out.push(invite_answer(crate::invite_people::Answer::ChannelSaved(
                    channel,
                )));
                return;
            }
            AdminResult::Members {
                channel: Some(channel),
                members,
                invitations,
            } => {
                out.push(invite_answer(crate::invite_people::Answer::Members {
                    channel,
                    members,
                    invitations,
                }));
                return;
            }
            AdminResult::MemberAdded {
                channel: Some(channel),
                member,
            } => {
                out.push(invite_answer(
                    crate::invite_people::Answer::ChannelInvited { channel, member },
                ));
                return;
            }
            AdminResult::MemberRemoved {
                channel: None,
                member,
            } => {
                if let Some(d) = &mut self.detail {
                    d.members.retain(|m| m.id != member);
                }
                out.push(invite_answer(crate::invite_people::Answer::SpaceRemoved(
                    member,
                )));
                return;
            }
            AdminResult::MemberRemoved {
                channel: Some(channel),
                member,
            } => {
                out.push(invite_answer(
                    crate::invite_people::Answer::ChannelRemoved { channel, member },
                ));
                return;
            }
            AdminResult::InvitationAccepted(space) => {
                self.invitations.retain(|s| s.id != space.id);
                if let Some(s) = &mut self.spaces
                    && !s.spaces.iter().any(|s| s.id == space.id)
                {
                    s.spaces.push(space.clone());
                }
                self.navigate(Some(space.id), None);
                out.push(Update::Invitations);
            }
            AdminResult::InvitationDeclined(space) => {
                self.invitations.retain(|s| s.id != space);
                out.push(Update::Invitations);
            }
            AdminResult::Invitations(members) => {
                out.push(invite_answer(crate::invite_people::Answer::Invitations(
                    members,
                )));
                return;
            }
            AdminResult::InvitationCreated(member) => {
                out.push(invite_answer(crate::invite_people::Answer::Invited(member)));
                return;
            }
            AdminResult::InvitationCancelled(user) => {
                out.push(invite_answer(crate::invite_people::Answer::Cancelled(user)));
                return;
            }
            AdminResult::ChannelInvitationDeclined(channel) => {
                if let Some(d) = &mut self.detail {
                    d.channel_invitations.retain(|i| i.channel.id != channel);
                }
                out.push(Update::Invitations);
            }
            _ => return,
        }
        out.push(Update::Left(None));
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

    /// Leave, or Cancel while joining.
    pub fn leave_voice(&mut self) {
        self.pending_join = None;
        self.voice.leave();
        self.queued.push(Update::Voice);
    }

    pub fn toggle_mute(&mut self) {
        let muted = self.voice.state.audio.muted;
        self.voice.command(VoiceOperation::Mute(!muted));
        // An unmute that found no microphone says why; the call's card has
        // room only for its status.
        if muted && self.voice.state.audio.muted && self.voice.microphone_lost {
            self.queued.push(Update::Warning(
                caper_client::voice::MICROPHONE_DISCONNECTED_DETAIL.into(),
            ));
        }
        self.queued.push(Update::Voice);
    }

    pub fn toggle_deafen(&mut self) {
        let deafened = self.voice.state.audio.deafened;
        self.voice.command(VoiceOperation::Deafen(!deafened));
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
                // Connected, but muted because its input went away.
                Phase::Connected(_) if self.voice.microphone_lost => {
                    Some(caper_client::voice::MICROPHONE_DISCONNECTED)
                }
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

    /// Reads the call's reports (egui's `receive`). Whether what voice
    /// shows changed.
    fn receive_voice(&mut self) -> bool {
        let input = self.voice.preferences.input.clone();
        self.voice.receive();
        // A choice forgotten because its device is gone stays forgotten.
        if self.voice.preferences.input != input {
            crate::settings::audio::save_preferences(&self.voice.preferences);
        }
        if self
            .pending_join
            .as_ref()
            .is_some_and(|(_, generation, _)| *generation != self.voice.state.generation)
        {
            self.pending_join = None;
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
                (v.refreshing_devices, v.microphone_lost),
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
            // One row per group of neighbouring notifications, shown as
            // its newest.
            rows: a
                .visible_groups()
                .iter()
                .map(|group| (group, group.newest()))
                .map(|(group, item)| crate::settings::InboxRow {
                    actor: group.names(),
                    mark: group.mark_label(),
                    unread: item.unread,
                    opening: a.opening.as_deref() == Some(group.id()),
                    time: caper_client::feedback::relative_time(&item.created_at, now),
                    description: group.summary(),
                    body: item.body.clone(),
                    place: item.place(),
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
        if let Some((channel, None, id)) = &self.marked
            && self.channel.as_ref() == Some(channel)
        {
            return Some(id.clone());
        }
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
        if !self.foreground
            || self.unread_pending
            || self.window.is_some()
            || self.feedback_visit.is_some()
            || self
                .marked
                .as_ref()
                .is_some_and(|(c, root, _)| root.is_none() && self.channel.as_ref() == Some(c))
        {
            return;
        }
        let (Some(channel), Some(token)) = (self.channel.clone(), self.token.clone()) else {
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
        let Some(space) = self.space.clone() else {
            return;
        };
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
        if let Ok(mut r) = self.host.requests.lock() {
            r.draining = true;
        }
        // A notification's unread change, once its second has passed (the
        // window's timer rounds to whole milliseconds).
        if self
            .unread_due
            .is_some_and(|due| due <= Instant::now() + Duration::from_millis(15))
        {
            self.refresh_unread();
        }
        if self
            .membership_at
            .is_some_and(|at| at.elapsed() + Duration::from_millis(50) >= MEMBERSHIP_EVERY)
        {
            self.refresh_membership();
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
        self.poll_voice_activity();
        if let Ok(mut r) = self.host.requests.lock() {
            r.draining = false;
        }
        out
    }

    fn handle(&mut self, event: Event, out: &mut Vec<Update>) {
        let current = self.generation;
        match event {
            Event::WhatsNew(result) => {
                self.highlights.finish(&result);
                self.whats_new.finish(result);
                self.save_highlights_read();
                out.push(Update::Settings);
            }
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
                    self.discard_voice_sends();
                    self.generation += 1;
                    let current = self.generation;
                    // A DM has no space of its own: the open space's
                    // channels stay beside it, as in egui.
                    let direct = prepared.detail.is_none();
                    let detail = prepared.detail.or_else(|| self.detail.clone());
                    let Some((history, session)) = prepared.conversation else {
                        out.push(Update::Failed("No joined channel in this space.".into()));
                        return;
                    };
                    self.feedback_visit = self
                        .report_target
                        .take()
                        .filter(|(n, target)| {
                            *n == navigation && target.channel == history.channel.id
                        })
                        .map(|(_, target)| {
                            caper_client::feedback::Visit::new(
                                caper_client::feedback::VisitRequest {
                                    target,
                                    origin: caper_client::feedback::Origin::Inbox,
                                },
                                &history,
                            )
                        });
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
                    self.typing.clear();
                    out.push(Update::Typing);
                    self.detail = detail.clone();
                    let mut timeline = core::Timeline::default();
                    // The history's pinned messages seed Pins, as in egui.
                    if let Err(e) = timeline
                        .reset(history.messages, &history.cursor)
                        .and_then(|()| timeline.reset_pins(history.pinned_messages))
                    {
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
                    self.space = detail.as_ref().map(|d| d.space.id.clone());
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
                        detail.as_ref(),
                        Some(&channel),
                        &messages,
                        self.me.as_deref(),
                        false,
                        unread.as_deref(),
                        self.direct_head(),
                    );
                    // Whether each joined channel's voice can be joined,
                    // once per account (egui's `refresh_media_status`).
                    if !direct && let (Some(token), Some(detail)) = (self.token.clone(), &detail) {
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
                            .iter()
                            .flat_map(|d| d.channels.iter().map(move |c| (d, c)))
                            .filter(|(_, c)| c.joined && !quiet)
                            .take(gateway::MAX_MEDIA_CHANNELS)
                            .map(|(d, c)| gateway::MediaChannel {
                                id: c.id.clone(),
                                demo: d.space.demo,
                            })
                            .collect(),
                    };
                    out.push(Update::Loaded(Box::new((conversation, ids))));
                    if let Some((_, channel)) =
                        self.invite_focus.take().filter(|(n, _)| *n == navigation)
                    {
                        match detail
                            .iter()
                            .flat_map(|d| &d.channel_invitations)
                            .find(|i| i.channel.id == channel)
                        {
                            Some(i) => out.push(Update::Invitation(Invitation::Channel(i.clone()))),
                            None => out.push(Update::Warning(
                                "This invitation is no longer available.".into(),
                            )),
                        }
                    }
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
                        presence: detail.filter(|_| !direct).map(|d| (d.space.id, Vec::new())),
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
                            if t.focus.is_some()
                                && t.retry_older
                                && t.has_more
                                && before.as_ref().is_none_or(|next| {
                                    t.before.as_ref().is_none_or(|old| {
                                        core::sequence(next).unwrap_or(0)
                                            >= core::sequence(old).unwrap_or(0)
                                    })
                                })
                            {
                                t.error = Some(
                                    "This thread could not load earlier replies. Try again.".into(),
                                );
                            }
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
                self.advance_thread_focus();
                out.append(&mut self.queued);
                out.push(Update::Thread);
            }
            Event::Sent {
                generation,
                client_id,
                result,
                ..
            } if generation == current => {
                if let Some(index) = self.drafts.iter().position(|d| {
                    d.auto_send
                        .as_ref()
                        .is_some_and(|s| s.client_id == client_id)
                }) {
                    match &result {
                        Ok(_) => {
                            self.drafts.remove(index);
                        }
                        Err(failure) => {
                            let d = &mut self.drafts[index];
                            d.auto_send.as_mut().unwrap().sending = false;
                            if failure.status == Some(409)
                                && failure.code.as_deref() == Some("attachment_unavailable")
                            {
                                // A definitive reservation rejection: Retry may upload
                                // again, unlike an uncertain transport failure.
                                d.upload.upload_expired = true;
                                d.auto_send.as_mut().unwrap().attempted = false;
                            }
                            d.upload.error = Some(failure.message.clone());
                        }
                    }
                    out.push(Update::Drafts);
                }
                let unknown = result.as_ref().is_err_and(|f| !f.rejected());
                {
                    let mut outgoing = self.outgoing.borrow_mut();
                    match outgoing.get_mut(&client_id) {
                        Some((_, unconfirmed)) if unknown => *unconfirmed = true,
                        _ => {
                            outgoing.remove(&client_id);
                        }
                    }
                }
                match result {
                    Err(f) if unknown && self.outgoing.borrow().contains_key(&client_id) => {
                        out.push(Update::Failed(format!("{} {RESEND_NOTE}", f.message)));
                    }
                    Ok(m) => {
                        out.push(Update::SendConfirmed);
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
                }
            }
            Event::Gateway(gateway::GatewayEvent::Message {
                generation,
                channel,
                message,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                let id = message.id.clone();
                if self.typing.sent_message(&message.author.id) {
                    out.push(Update::Typing);
                }
                let old = self.seq();
                let applied = self.timeline.apply(*message);
                self.publish(old, applied, &id, out);
                self.mark_read();
            }
            Event::Gateway(gateway::GatewayEvent::Typing {
                generation,
                channel,
                author,
                typing,
                revision,
            }) if generation == current && self.channel.as_deref() == Some(channel.as_str()) => {
                // Yours never shows, nor anyone's you blocked, as on web.
                if self.me.as_deref() != Some(author.id.as_str())
                    && !self.is_blocked(&author.id)
                    && self.typing.update(
                        &author.id,
                        &author.name,
                        typing,
                        core::sequence(&revision).unwrap_or(0),
                        Instant::now(),
                    )
                {
                    out.push(Update::Typing);
                }
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
                    // Most 15 s answers repeat what is shown: nothing redraws.
                    let changed = unread.feedback_waiting != self.feedback_waiting
                        || unread.spaces != self.unread_spaces
                        || unread
                            .badge_count
                            .is_some_and(|count| count != self.activity.unread_count());
                    self.feedback_waiting = unread.feedback_waiting;
                    self.unread_spaces = unread.spaces;
                    // A closed inbox loads ahead when its count changes, so
                    // it opens on these notifications.
                    if let Some(count) = unread.badge_count
                        && self.activity.count_changed(count)
                    {
                        self.load_activity(false, true);
                    }
                    if changed {
                        out.extend([Update::Badges, Update::Inbox]);
                    }
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
            Event::MarkedUnread {
                epoch,
                target,
                result,
            } if epoch == self.account_epoch => match result {
                Ok(()) => {
                    // Unread counts, the inbox and the badge follow the server.
                    if let caper_client::api::UnreadTarget::Channel { channel, .. } = &target {
                        self.reads.remove(channel);
                    }
                    self.refresh_unread();
                    if matches!(target, caper_client::api::UnreadTarget::Direct { .. }) {
                        self.refresh_membership();
                    }
                    out.push(Update::Badges);
                }
                Err(_) => {
                    self.marked = None;
                    out.push(Update::Warning(
                        "Couldn\u{2019}t mark unread. Try again.".into(),
                    ));
                }
            },
            Event::MessageLinkResolved { epoch, result, .. } if epoch == self.account_epoch => {
                match result {
                    Ok(Some(source)) if source.message.is_deleted() => {
                        out.push(Update::Warning("That message was deleted.".into()));
                    }
                    Ok(Some(source)) => {
                        let source = *source;
                        let id = source.message.id.clone();
                        let thread = source.message.thread_root_id.clone();
                        if self.channel.as_deref() == Some(source.message.channel_id.as_str()) {
                            self.focus_target(&id, thread.as_deref(), out);
                        } else {
                            self.navigate(source.space_id, Some(source.message.channel_id));
                            self.focus = Some((self.navigation, id, thread));
                        }
                    }
                    Ok(None) => out.push(Update::Warning(
                        "You can\u{2019}t view that message.".into(),
                    )),
                    Err(_) => out.push(Update::Warning(
                        "Couldn\u{2019}t open that message. Try again.".into(),
                    )),
                }
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
            Event::VoiceActivityLoaded { epoch, result }
                if epoch == self.account_epoch && self.token.is_some() =>
            {
                self.voice_activity_pending = false;
                let spaces: HashSet<String> = result.unwrap_or_default().into_iter().collect();
                // Unchanged 15 s answers leave voice's surfaces alone.
                if spaces != self.voice_spaces {
                    self.voice_spaces = spaces;
                    out.push(Update::Voice);
                }
            }
            Event::MediaStatus {
                generation,
                root,
                enabled,
            } if generation == self.account_epoch => {
                // An unchanged answer redraws nothing.
                if self.availability.insert(root, enabled) != Some(enabled) {
                    out.push(Update::Voice);
                }
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
            }) if generation == current => {
                if !online {
                    self.typing.clear();
                    out.push(Update::Typing);
                } else {
                    self.resend_unconfirmed();
                }
                out.push(Update::Connection(
                    (!online && !detail.is_empty()).then_some(detail),
                ));
            }
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
            Event::ProfileStatus {
                epoch,
                person,
                status,
            } if epoch == self.account_epoch => {
                if let Some(card) = self.card.as_mut().filter(|c| c.person == person) {
                    card.status = Some(status);
                    out.push(Update::Profile(None));
                }
            }
            Event::ProfileSent {
                epoch,
                person,
                mut send,
                result,
            } if epoch == self.account_epoch => {
                self.profile_cancel = None;
                if let Some(direct) = result.as_ref().ok().or(send.direct.as_ref()) {
                    self.directs.retain(|d| d.id != direct.id);
                    self.directs.push(direct.clone());
                    out.push(Update::Badges);
                }
                if let Some(card) = self.card.as_mut().filter(|c| c.person == person) {
                    card.sending = false;
                    card.status = None;
                    let draft = self.profile_drafts.entry(person.clone()).or_default();
                    // Voice messages and GIFs go on their own, keeping the draft.
                    let alone = send.voice_only || send.gif.is_some();
                    match result {
                        Ok(direct) => {
                            if let Some(share) = self.shares.remove(&send.client_id) {
                                share.send();
                            }
                            if !alone && !send.open_only {
                                draft.files.clear();
                            }
                            if !send.open_only {
                                draft.retry = None;
                                draft.error = None;
                            }
                            if send.voice_only {
                                out.push(Update::Profile(None));
                            } else {
                                // Explicit Open and a GIF preserve the unsent draft.
                                self.card = None;
                                out.push(Update::Profile(Some((
                                    person,
                                    !send.open_only && !alone,
                                ))));
                                self.open_direct(direct.id);
                            }
                        }
                        Err(error) => {
                            if !send.open_only {
                                if !alone {
                                    draft.files = send.files.clone();
                                }
                                send.retrying = error.uncertain;
                                if error.uncertain || send.voice_only {
                                    draft.retry = Some(send);
                                } else {
                                    self.shares.remove(&send.client_id);
                                }
                            }
                            draft.error = Some(error.message);
                            out.push(Update::Profile(None));
                        }
                    }
                }
            }
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
            Event::Admin { generation, result } if generation == self.account_epoch => match result
            {
                Ok(result) => self.left(result, out),
                Err(e) if self.joining => {
                    self.joining = false;
                    self.join_error = Some(e);
                    out.push(Update::Joining);
                }
                Err(e) => out.push(Update::Left(Some(e))),
            },
            Event::MembershipRefreshed {
                epoch,
                request,
                navigation,
                result,
                ..
            } if epoch == self.account_epoch && request == self.membership_request => {
                self.membership_pending = false;
                if let Ok((spaces, detail)) = result {
                    let mut changed = spaces.invitations != self.invitations;
                    self.invitations = spaces.invitations;
                    if spaces.limits.is_some() {
                        changed |= spaces.limits != self.limits;
                        self.limits = spaces.limits;
                    }
                    if let (Some(fresh), Some(open)) = (detail, &mut self.detail)
                        && navigation == self.navigation
                        && fresh.space.id == open.space.id
                    {
                        changed |= fresh.channel_invitations != open.channel_invitations;
                        open.channel_invitations = fresh.channel_invitations;
                    }
                    // Unchanged 15 s answers redraw nothing.
                    if changed {
                        out.push(Update::Invitations);
                    }
                }
            }
            Event::InvitationLoaded {
                epoch,
                navigation,
                space,
                channel,
                result,
            } if epoch == self.account_epoch && navigation == self.navigation => {
                self.invitation_loaded(space, channel, result, out);
            }
            Event::Klipy { token, config } if self.token.as_ref() == Some(&token) => {
                self.klipy = config;
                out.push(Update::Drafts);
            }
            // Toolbars pick these up when the shown one moves or hides.
            Event::QuickReactions { token, quick } if self.token.as_ref() == Some(&token) => {
                crate::actions::set_quick(quick);
            }
            Event::AssetUsage { token, settings } if self.token.as_ref() == Some(&token) => {
                self.uploads = settings;
                self.uploads_checked = true;
                out.push(Update::Drafts);
            }
            Event::Upload { key, token, update } => {
                // Keys restart with a new account. An old session's reply
                // must never overwrite a new session's draft or edit.
                if self.token.as_ref() != Some(&token) {
                    if let caper_client::worker::UploadUpdate::Finished(Ok(file)) = update {
                        self.worker.send(Command::CancelAssets {
                            token,
                            ids: vec![file.id],
                        });
                    }
                    return;
                }
                if self
                    .edit_upload
                    .as_ref()
                    .is_some_and(|(_, d)| d.upload.key == key)
                {
                    let terminal =
                        matches!(&update, caper_client::worker::UploadUpdate::Finished(_));
                    let (_, d) = self.edit_upload.as_mut().unwrap();
                    if let Some(image) = d.upload.apply(update) {
                        d.set_thumbnail(image);
                    }
                    if terminal {
                        let (old_key, mut replacement) = self.edit_upload.take().unwrap();
                        let error = replacement.upload.error.clone();
                        if error.is_none() {
                            if let Some(i) =
                                self.drafts.iter().position(|d| d.upload.key == old_key)
                            {
                                // Spoiler may have changed while the request was in flight.
                                replacement.upload.spoiler = self.drafts[i].upload.spoiler;
                                let old = std::mem::replace(&mut self.drafts[i], replacement);
                                old.upload.abandon();
                                if let Some(file) = old.upload.attachment {
                                    self.cancel_assets(vec![file.id]);
                                }
                                let d = &self.drafts[i];
                                if let (Some(file), Some(image)) = (&d.upload.attachment, &d.thumb)
                                {
                                    self.local.push((file.id.clone(), image.clone()));
                                }
                            } else if let Some(file) = replacement.upload.attachment {
                                self.worker.send(Command::CancelAssets {
                                    token,
                                    ids: vec![file.id],
                                });
                            }
                        }
                        out.push(Update::AttachmentEdited(old_key, error));
                        out.push(Update::Drafts);
                    }
                    return;
                }
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
                if finished {
                    self.send_voice(key);
                }
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
            Event::FeedbackLoaded {
                epoch,
                request,
                result,
            } if epoch == self.account_epoch => {
                self.feedback.finish(request, result);
                out.push(Update::Settings);
            }
            Event::FeedbackArchived { epoch, result } if epoch == self.account_epoch => {
                if let Err(error) = result {
                    self.feedback.action_error = Some(error);
                    self.load_feedback(false);
                }
                self.refresh_unread();
                out.push(Update::Settings);
            }
            // A clicked toast opens its conversation, as in egui.
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
                    Some(Route::Invitation { space, channel }) => {
                        self.open_invitation(space, channel);
                    }
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
                mut items,
                open,
                result,
            } if epoch == self.account_epoch => {
                if open {
                    self.activity.opening = None;
                }
                if self.activity.read_finished(&items, result) {
                    self.refresh_unread();
                    // A group opens at its oldest, where the run starts.
                    if open
                        && navigation == self.navigation
                        && self.activity.shown
                        && let Some(oldest) = items.pop()
                    {
                        self.open_activity_item(oldest, out);
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
    fn set_directs(&mut self, mut directs: Vec<core::DirectConversation>, out: &mut Vec<Update>) {
        let reads: HashMap<&str, &str> = self
            .directs
            .iter()
            .map(|d| (d.id.as_str(), d.read_seq.as_str()))
            .collect();
        for d in &mut directs {
            if let Some(&read) = reads.get(d.id.as_str())
                && core::sequence(read).unwrap_or(0) > core::sequence(&d.read_seq).unwrap_or(0)
            {
                read.clone_into(&mut d.read_seq);
            }
        }
        // The 15 s refresh usually brings the same list: nothing redraws.
        if directs == self.directs {
            return;
        }
        self.directs = directs;
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

pub fn person(id: &str, name: &str, avatar: crate::avatars::Avatar) -> Person {
    Person {
        name: name.to_owned(),
        initials: initials(name),
        color: color_for(id),
        online: false,
        avatar,
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
    detail: Option<&core::SpaceDetail>,
    channel: Option<&str>,
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
        detail.map_or_else(Vec::new, |d| d.member_order(channel))
    };
    let mut ids = Ids {
        members: order.iter().map(|m| m.id.clone()).collect(),
        spaces: spaces.spaces.iter().map(|s| s.id.clone()).collect(),
        ..Ids::default()
    };
    let mut members: Vec<Person> = order
        .iter()
        .map(|m| person(&m.id, &m.display_name, m.avatar()))
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
            members.push(person(&m.author.id, &m.author.name, m.author.avatar()));
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
    let joined: Vec<&core::Channel> = detail
        .into_iter()
        .flat_map(|d| &d.channels)
        .filter(|c| c.joined)
        .collect();
    ids.channels = joined.iter().map(|c| c.id.clone()).collect();
    let channels = joined
        .iter()
        .map(|c| model::Channel {
            name: c.name.clone(),
            private: c.private,
        })
        .collect();
    ids.open = channel.map(str::to_owned);
    // A channel open from Browse that you haven't joined: no row is
    // selected and the header names it.
    let preview = detail
        .into_iter()
        .flat_map(|d| &d.channels)
        .find(|c| direct.is_none() && Some(c.id.as_str()) == channel && !c.joined)
        .map(|c| model::Channel {
            name: c.name.clone(),
            private: c.private,
        });
    let selected = if direct.is_some() || preview.is_some() {
        joined.len()
    } else {
        joined
            .iter()
            .position(|c| Some(c.id.as_str()) == channel)
            .unwrap_or(0)
    };
    let space = spaces
        .spaces
        .iter()
        .position(|s| Some(s.id.as_str()) == detail.map(|d| d.space.id.as_str()))
        .unwrap_or(0);
    let spaces = spaces
        .spaces
        .iter()
        .map(|s| model::Space {
            name: s.name.clone(),
            icon: crate::avatars::Avatar::icon(s),
        })
        .collect();
    let mut c = Conversation::live(spaces, space, channels, selected, members, out);
    c.listed_members = listed;
    c.direct = direct;
    c.preview = preview;
    (c, ids)
}

/// Message text as styled runs, as the egui client draws it: the shared
/// grammar (`shared/messages/formatting.rs`) removes its markers
/// first; then, in the text that remains, the mentions the server resolved
/// (`caper_client::mentions`) become pills and links
/// (`shared/messages/links.rs`) are found between them.
pub fn spans(text: &str, mentions: &[core::Mention]) -> Vec<Span> {
    let mut out = Vec::new();
    let blocks = caper_client::formatting::code::parse_blocks(text);
    for (idx, block) in blocks.into_iter().enumerate() {
        if idx > 0 {
            out.push(Span::Rich {
                text: "\n".into(),
                style: 0,
                mention: false,
                href: None,
            });
        }
        match block {
            caper_client::formatting::code::MessageBlock::Code(blk) => {
                out.push(Span::CodeBlock {
                    text: format!("{}\n", blk.language.display_name()),
                    token: 255,
                });
                let tokens = caper_client::formatting::code::highlight(blk.code, blk.language);
                if tokens.is_empty() {
                    out.push(Span::CodeBlock {
                        text: blk.code.to_owned(),
                        token: 0,
                    });
                } else {
                    for tok in tokens {
                        out.push(Span::CodeBlock {
                            text: blk.code[tok.start..tok.end].to_owned(),
                            token: tok.kind as u8,
                        });
                    }
                }
            }
            caper_client::formatting::code::MessageBlock::Prose { text: chunk, .. } => {
                let mut visible = String::with_capacity(chunk.len());
                let mut styles = Vec::new();
                crate::formatting::visit_runs(chunk, |range, style| {
                    let start = visible.len();
                    visible.push_str(&chunk[range]);
                    styles.push((start..visible.len(), style));
                });
                let in_code = |pos: usize| {
                    styles.iter().any(|(r, st)| {
                        st & (crate::formatting::CODE | crate::formatting::CODE_BLOCK) != 0
                            && r.contains(&pos)
                    })
                };
                let pills: Vec<_> = caper_client::mentions::highlights(&visible, mentions)
                    .into_iter()
                    .map(|(range, _)| range)
                    .filter(|r| !in_code(r.start))
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
                        if let Some(href) = segment.href
                            && !in_code(offset)
                        {
                            links.push((offset..next, href));
                        }
                        offset = next;
                    }
                    gap = end;
                }
                for (range, style) in styles {
                    if style & (crate::formatting::CODE | crate::formatting::CODE_BLOCK) != 0 {
                        out.push(Span::Rich {
                            text: visible[range].to_owned(),
                            style,
                            mention: false,
                            href: None,
                        });
                        continue;
                    }
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
                            style,
                            mention: pills.iter().any(|p| p.contains(&start)),
                            href: links
                                .iter()
                                .find(|(link, _)| link.contains(&start))
                                .map(|(_, href)| href.clone()),
                        });
                    }
                }
            }
        }
    }
    out
}

/// The newest of `messages` (oldest first) by `me` that is neither deleted
/// nor a forward: in thread `root`, its root or a reply; otherwise a channel
/// row (a top-level message or a reply also sent to the channel), as on web.
pub fn last_editable<'a>(
    messages: impl IntoIterator<Item = &'a core::Message>,
    me: &str,
    root: Option<&str>,
) -> Option<&'a core::Message> {
    messages
        .into_iter()
        .filter(|m| match root {
            Some(root) => m.id == root || m.thread_root_id.as_deref() == Some(root),
            None => m.is_channel_message(),
        })
        .filter(|m| m.author.id == me && !m.is_deleted() && m.forward.is_none())
        .last()
}

/// One displayed message. `me`
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
                style: crate::formatting::ITALIC,
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
            author: person(&r.author.id, &r.author.name, r.author.avatar()),
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
        forward: m.forward.clone().filter(|_| !deleted),
        forward_revealed: Default::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_pages_follow_account_permissions_and_keep_credentials_out() {
        use crate::settings::{Page, support};
        let api = Api::new("http://127.0.0.1:9").unwrap();
        let host = WinHost {
            hwnd: 0,
            requests: Arc::default(),
        };
        let voice = Voice::new(
            api.base().clone(),
            caper_client::voice::Repaint::new(|_| {}),
        );
        let mut live = Live::fresh(
            Worker::new(api, host.clone()),
            caper_client::media::Media::new(None, Arc::new(|| {})),
            Arc::default(),
            host,
            1,
            voice,
            crate::settings::preferences::Preferences::load(None),
        );
        live.token = Some("fixture-token".into());
        live.account_id = Some("team-member".into());
        assert!(support::snapshot(&live, Page::Feedback).is_none());
        assert!(support::snapshot(&live, Page::Diagnostics).is_none());
        assert!(
            !live
                .inbox_rows()
                .iter()
                .any(|r| r.kind == crate::directs::RowKind::Feedback)
        );
        live.feedback_team = true;
        live.feedback_waiting = 9;
        // Feedback sits in the account panel, not under Direct messages.
        assert!(
            live.direct_rows()
                .iter()
                .all(|r| r.kind != crate::directs::RowKind::Feedback)
        );
        let rows = live.inbox_rows();
        let row = rows
            .iter()
            .find(|r| r.kind == crate::directs::RowKind::Feedback)
            .unwrap();
        assert_eq!(row.count, Some(9));
        assert!(support::snapshot(&live, Page::Feedback).is_some());
        assert!(
            support::snapshot(&live, Page::Diagnostics).is_none(),
            "team permission does not grant diagnostics"
        );
        live.debug_enabled = true;
        let Some(support::Snapshot::Report {
            json: Some(json), ..
        }) = support::snapshot(&live, Page::Diagnostics)
        else {
            panic!("missing report")
        };
        let report: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(report.as_object().unwrap().len(), 2);
        assert!(report["processing"].is_null());
        assert!(!json.contains("fixture-token"));
    }

    #[test]
    fn profile_files_survive_open_and_voice_but_clear_only_after_their_own_send() {
        let api = Api::new("http://127.0.0.1:9").unwrap();
        let host = WinHost {
            hwnd: 0,
            requests: Arc::default(),
        };
        let voice = Voice::new(
            api.base().clone(),
            caper_client::voice::Repaint::new(|_| {}),
        );
        let mut live = Live::fresh(
            Worker::new(api, host.clone()),
            caper_client::media::Media::new(None, Arc::new(|| {})),
            Arc::default(),
            host,
            1,
            voice,
            crate::settings::preferences::Preferences::load(None),
        );
        let epoch = live.account_epoch;
        live.profile_drafts
            .entry("alex-id".into())
            .or_default()
            .files
            .push(caper_client::profile_dm::File::picked("alex.txt".into()));
        live.profile_drafts
            .entry("bob-id".into())
            .or_default()
            .files
            .push(caper_client::profile_dm::File::picked("bob.txt".into()));
        let send = |open_only, voice_only| {
            Box::new(caper_client::profile_dm::Send {
                person: "alex-id".into(),
                username: "alex".into(),
                name: "Me".into(),
                client_id: "same-key".into(),
                text: "kept payload".into(),
                files: vec![caper_client::profile_dm::File::picked(
                    "returned.txt".into(),
                )],
                settings: None,
                cancel: Arc::default(),
                retrying: false,
                open_only,
                voice_only,
                gif: None,
                direct: None,
            })
        };
        let direct = || {
            serde_json::from_value(serde_json::json!({
            "id": "dm-alex", "peer": {"id": "alex-id", "username": "alex", "displayName": "Alex"},
            "lastSeq": "1", "readSeq": "0", "status": "accepted"
        })).unwrap()
        };
        let card = || CardState {
            person: "alex-id".into(),
            sending: true,
            ..Default::default()
        };
        let mut updates = Vec::new();
        live.card = Some(card());
        live.handle(
            Event::ProfileSent {
                epoch,
                person: "alex-id".into(),
                send: send(false, false),
                result: Err(caper_client::profile_dm::Failure {
                    message: "Timed out".into(),
                    uncertain: true,
                }),
            },
            &mut updates,
        );
        let retry = live.profile_drafts["alex-id"].retry.as_ref().unwrap();
        assert!(retry.retrying);
        assert_eq!(retry.client_id, "same-key");
        assert_eq!(retry.text, "kept payload");
        assert_eq!(retry.files[0].name(), "returned.txt");
        assert!(!live.card.as_ref().unwrap().sending);

        // Closing and reopening, or navigating explicitly, must not silently
        // turn an uncertain send into a new editable payload.
        live.card = None;
        live.card = Some(card());
        live.handle(
            Event::ProfileSent {
                epoch,
                person: "alex-id".into(),
                send: send(true, false),
                result: Ok(direct()),
            },
            &mut updates,
        );
        assert!(live.profile_drafts["alex-id"].retry.is_some());
        assert_eq!(
            live.profile_drafts["alex-id"].files[0].name(),
            "returned.txt"
        );
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, Update::Profile(Some((id, false))) if id == "alex-id"))
        );

        live.profile_drafts.get_mut("alex-id").unwrap().retry = None;
        live.card = Some(card());
        live.handle(
            Event::ProfileSent {
                epoch,
                person: "alex-id".into(),
                send: send(false, true),
                result: Err(caper_client::profile_dm::Failure {
                    message: "Offline".into(),
                    uncertain: false,
                }),
            },
            &mut updates,
        );
        assert!(
            live.profile_drafts["alex-id"]
                .retry
                .as_ref()
                .unwrap()
                .voice_only
        );
        assert_eq!(
            live.profile_drafts["alex-id"].files[0].name(),
            "returned.txt"
        );
        live.handle(
            Event::ProfileSent {
                epoch,
                person: "alex-id".into(),
                send: send(false, true),
                result: Ok(direct()),
            },
            &mut updates,
        );
        assert!(live.profile_drafts["alex-id"].retry.is_none());
        assert!(live.card.is_some(), "voice alone keeps the card and draft");
        assert_eq!(live.profile_drafts["alex-id"].files.len(), 1);

        // A GIF goes alone too: it opens the DM but keeps the typed text
        // (`false`: not cleared) and the staged file.
        let mut gif = send(false, false);
        gif.text = String::new();
        gif.files.clear();
        gif.gif = Some(caper_client::klipy::Gif {
            kind: caper_client::klipy::Kind::Gif,
            slug: "wave".into(),
            title: "Wave".into(),
            width: 200,
            height: 200,
            gif: Some("https://static.klipy.com/wave.gif".into()),
            webp: None,
            mp4: None,
            still: None,
        });
        updates.clear();
        live.handle(
            Event::ProfileSent {
                epoch,
                person: "alex-id".into(),
                send: gif,
                result: Ok(direct()),
            },
            &mut updates,
        );
        assert!(live.card.is_none(), "a GIF opens the DM like Send");
        assert_eq!(live.profile_drafts["alex-id"].files.len(), 1);
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, Update::Profile(Some((id, false))) if id == "alex-id"))
        );
        live.card = Some(card());

        live.handle(
            Event::ProfileSent {
                epoch: epoch.wrapping_sub(1),
                person: "alex-id".into(),
                send: send(false, false),
                result: Ok(direct()),
            },
            &mut updates,
        );
        assert_eq!(
            live.profile_drafts["alex-id"].files.len(),
            1,
            "a prior account cannot clear this draft"
        );
        live.handle(
            Event::ProfileSent {
                epoch,
                person: "alex-id".into(),
                send: send(false, false),
                result: Ok(direct()),
            },
            &mut updates,
        );
        assert!(live.profile_drafts["alex-id"].files.is_empty());
        assert_eq!(live.profile_drafts["bob-id"].files[0].name(), "bob.txt");
        assert!(
            updates
                .iter()
                .any(|u| matches!(u, Update::Profile(Some((id, true))) if id == "alex-id"))
        );
    }

    #[test]
    fn thread_files_have_independent_limits_and_send_without_consuming_other_drafts() {
        let api = Api::new("http://127.0.0.1:9").unwrap();
        let host = WinHost {
            hwnd: 0,
            requests: Arc::default(),
        };
        let voice = Voice::new(
            api.base().clone(),
            caper_client::voice::Repaint::new(|_| {}),
        );
        let mut live = Live::fresh(
            Worker::new(api, host.clone()),
            caper_client::media::Media::new(None, Arc::new(|| {})),
            Arc::default(),
            host,
            1,
            voice,
            crate::settings::preferences::Preferences::load(None),
        );
        live.token = Some("fixture-token".into());
        live.channel = Some("channel".into());
        live.chat_token = Some("fixture-chat".into());
        live.uploads = Some(Default::default());
        for key in 1..=10 {
            live.drafts.push(crate::uploads::Draft::new(
                caper_client::drafts::Upload::picked(
                    key,
                    "channel".into(),
                    None,
                    format!("channel-{key}.txt"),
                    None,
                ),
            ));
        }
        for (key, channel, root) in [
            (11, "channel", "root-a"),
            (12, "channel", "root-b"),
            (13, "other-channel", "root-a"),
        ] {
            live.drafts.push(crate::uploads::Draft::new(
                caper_client::drafts::Upload::picked(
                    key,
                    channel.into(),
                    Some(root.into()),
                    format!("thread-{key}.txt"),
                    None,
                ),
            ));
        }
        live.draft_keys = 20;
        assert_eq!(live.composer_drafts(None).len(), 10);
        assert_eq!(live.composer_drafts(Some("root-a")).len(), 1);
        assert_eq!(
            live.add_files(vec![("next.txt".into(), None)], Some("root-a")),
            None
        );
        assert!(
            live.add_files(vec![("full.txt".into(), None)], None)
                .is_some()
        );
        assert_eq!(
            live.send_with_files(
                String::new(),
                Some("quote".into()),
                Some(("root-a".into(), true))
            ),
            Sent::Waiting
        );
        for d in &mut live.drafts {
            if d.upload.channel == "channel" && d.upload.thread_root_id.as_deref() == Some("root-a")
            {
                let attachment = core::Attachment {
                    id: format!("asset-{}", d.upload.key),
                    kind: core::AttachmentKind::File,
                    content_type: "text/plain".into(),
                    name: d.upload.name.clone(),
                    size: 1,
                    spoiler: false,
                    width: None,
                    height: None,
                    duration_ms: None,
                    preview: false,
                    url: None,
                    preview_url: None,
                    unavailable: false,
                    status: core::AttachmentStatus::Ready,
                    animated: false,
                    voice: false,
                    waveform: None,
                    transcript: None,
                };
                d.upload
                    .apply(caper_client::worker::UploadUpdate::Finished(Ok(attachment)));
            }
        }
        assert_eq!(
            live.send_with_files(
                String::new(),
                Some("quote".into()),
                Some(("root-a".into(), true))
            ),
            Sent::Sent
        );
        assert!(live.composer_drafts(Some("root-a")).is_empty());
        assert_eq!(live.composer_drafts(None).len(), 10);
        assert_eq!(live.composer_drafts(Some("root-b"))[0].upload.key, 12);
        live.channel = Some("other-channel".into());
        assert_eq!(live.composer_drafts(Some("root-a"))[0].upload.key, 13);
    }

    #[test]
    fn voice_faces_levels_badges_and_playback_follow_the_current_account_and_call() {
        let api = Api::new("http://127.0.0.1:9").unwrap();
        let host = WinHost {
            hwnd: 0,
            requests: Arc::default(),
        };
        let voice = Voice::new(
            api.base().clone(),
            caper_client::voice::Repaint::new(|_| {}),
        );
        let mut live = Live::fresh(
            Worker::new(api, host.clone()),
            caper_client::media::Media::new(None, Arc::new(|| {})),
            Arc::default(),
            host,
            1,
            voice,
            crate::settings::preferences::Preferences::load(None),
        );
        live.token = Some("fixture".into());
        live.account_epoch = 17;
        live.account_name = "My saved name".into();
        let call = CallContext {
            channel_id: "room".into(),
            channel_name: "Room".into(),
            space_name: "Space".into(),
        };
        live.voice.active_space = Some("space".into());
        let mut out = Vec::new();
        live.handle(
            Event::VoiceActivityLoaded {
                epoch: 16,
                result: Ok(vec!["space".into()]),
            },
            &mut out,
        );
        assert_eq!(
            live.voice_badge("space"),
            None,
            "a former account's answer is ignored"
        );
        live.handle(
            Event::VoiceActivityLoaded {
                epoch: 17,
                result: Ok(vec!["space".into(), "elsewhere".into()]),
            },
            &mut out,
        );
        for phase in [
            Phase::Idle,
            Phase::Joining(call.clone()),
            Phase::Reconnecting(call.clone()),
            Phase::Failed(call.clone()),
        ] {
            live.voice.state.phase = phase;
            assert_eq!(live.voice_badge("space"), Some(false));
        }
        live.voice.state.phase = Phase::Connected(call);
        live.handle(
            Event::VoiceActivityLoaded {
                epoch: 17,
                result: Err("offline".into()),
            },
            &mut out,
        );
        assert_eq!(
            live.voice_badge("space"),
            Some(true),
            "the local connection does not depend on polling"
        );
        assert_eq!(
            live.voice_badge("elsewhere"),
            None,
            "failed polls clear stale activity"
        );
        live.voice.self_id = "self".into();
        live.voice.state.audio.muted = true;
        live.voice.participants = serde_json::from_value(serde_json::json!([
            {"id":"self","name":"Old name","muted":false,"deafened":false,"tracks":[]},
            {"id":"other","name":"Other","avatarId":4,"avatarAsset":"picture","avatarUrl":"https://example.invalid/picture","muted":false,"deafened":true,"tracks":[]}
        ])).unwrap();
        let people = live.voice_people("room");
        assert_eq!(people[0].name, "My saved name");
        assert!(people[0].muted, "local mute wins over the delayed roster");
        assert_eq!(people[1].avatar_asset.as_deref(), Some("picture"));
        assert!(people[1].deafened);
        live.rosters
            .insert("spectating".into(), (people.clone(), None));
        live.voice.activity.local = Some(Instant::now());
        live.voice
            .activity
            .participants
            .insert("other".into(), Instant::now());
        assert!(!live.voice_speaking("room", &people[0]));
        assert!(live.voice_speaking("room", &people[1]));
        assert!(
            !live.voice_speaking("spectating", &people[1]),
            "spectator identities don't imply audio levels"
        );
        live.participant_playback("other", 137, true);
        live.participant_playback("self", 0, true);
        assert_eq!(live.voice.playback("self").gain_percent, 100);
        assert!(!live.voice.playback("self").muted);
        assert_eq!(live.voice.playback("other").gain_percent, 137);
        assert!(live.voice.playback("other").muted);
        assert!(
            !live.voice_people("room")[1].muted,
            "Mute for me never changes their microphone"
        );
        live.voice.participants.reverse();
        assert_eq!(live.voice.playback("other").gain_percent, 137);
        live.participant_playback("other", 999, false);
        assert_eq!(live.voice.playback("other").gain_percent, 200);
        assert!(!live.voice.playback("other").muted);
        live.leave_voice();
        assert_eq!(live.voice.playback("other").gain_percent, 100);
        assert_eq!(live.voice_badge("space"), None);
    }

    #[test]
    fn stopped_voice_sends_separately_only_when_enabled_and_can_be_discarded() {
        use caper_client::uploads::{Body, Prepared, VoiceUpload};
        let api = Api::new("http://127.0.0.1:9").unwrap();
        let host = WinHost {
            hwnd: 0,
            requests: Arc::default(),
        };
        let voice = Voice::new(
            api.base().clone(),
            caper_client::voice::Repaint::new(|_| {}),
        );
        let mut live = Live::fresh(
            Worker::new(api, host.clone()),
            caper_client::media::Media::new(None, Arc::new(|| {})),
            Arc::default(),
            host,
            1,
            voice,
            crate::settings::preferences::Preferences::load(None),
        );
        live.token = Some("fixture-token".into());
        live.chat_token = Some("fixture-chat".into());
        live.channel = Some("general".into());
        live.uploads = Some(Default::default());
        let prepared = Arc::new(Prepared {
            name: "Voice message.m4a".into(),
            content_type: "audio/mp4".into(),
            kind: core::AttachmentKind::Audio,
            source_size: 1,
            size: 1,
            body: Body::Memory(Arc::from([0])),
            width: None,
            height: None,
            duration_ms: Some(2000),
            preview: None,
            thumbnail: None,
            voice: Some(VoiceUpload { waveform: None }),
        });
        assert!(!live.stage_voice(prepared.clone(), None).unwrap());
        assert_eq!(live.open_drafts().len(), 1, "the default waits for Send");
        let staged = live.open_drafts()[0].upload.key;
        live.remove_draft(staged);
        let manual = caper_client::drafts::Upload::picked(
            100,
            "general".into(),
            None,
            "keep.txt".into(),
            None,
        );
        live.drafts.push(crate::uploads::Draft::new(manual));
        live.preferences.set("voice-send-on-stop-v1", "true".into());
        assert!(
            live.stage_voice(prepared.clone(), Some("quoted-message".into()))
                .unwrap()
        );
        assert_eq!(live.visible_drafts().len(), 2);
        assert_eq!(
            live.open_drafts()
                .iter()
                .map(|d| d.upload.source_name.as_str())
                .collect::<Vec<_>>(),
            ["keep.txt"]
        );
        let outgoing = live.visible_drafts()[1].upload.key;
        let cancelled = live.visible_drafts()[1].upload.cancel.clone();
        live.remove_draft(outgoing);
        assert!(cancelled.load(std::sync::atomic::Ordering::Relaxed));
        assert_eq!(live.visible_drafts().len(), 1);
        live.preferences
            .set("voice-send-on-stop-v1", "false".into());
        assert!(!live.stage_voice(prepared, None).unwrap());
        assert_eq!(live.open_drafts().len(), 2, "turning it off stages again");
        assert!(
            live.voice_note_blocked().is_some(),
            "one staged recording per message"
        );
    }

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
    fn up_edits_your_newest_live_message_in_the_composers_conversation() {
        let me = "member000001";
        let n = |k: u32, extra: serde_json::Value| {
            let mut m = serde_json::json!({
                "id": format!("b0000000000000{k}"), "seq": k.to_string(),
                "author": { "id": me, "name": "Me", "isGuest": false },
                "clientMessageId": format!("00000000-0000-4000-9000-00000000000{k}"),
            });
            for (key, v) in extra.as_object().unwrap() {
                m[key] = v.clone();
            }
            core_message(m)
        };
        let root = n(1, serde_json::json!({}));
        let messages = [
            root.clone(),
            n(
                2,
                serde_json::json!({ "threadRootId": root.id, "broadcast": true }),
            ),
            // Only the thread shows it.
            n(3, serde_json::json!({ "threadRootId": root.id })),
            n(
                4,
                serde_json::json!({ "author": { "id": "member000002", "name": "Alex", "isGuest": false } }),
            ),
            n(
                5,
                serde_json::json!({ "deletedAt": "2026-10-01T09:05:00Z" }),
            ),
            n(
                6,
                serde_json::json!({ "forward": { "seq": "9", "message": null } }),
            ),
        ];
        let id = |m: Option<&core::Message>| m.map(|m| m.id.clone());
        assert_eq!(
            id(last_editable(&messages, me, None)),
            Some(messages[1].id.clone())
        );
        assert_eq!(
            id(last_editable(&messages, me, Some(root.id.as_str()))),
            Some(messages[2].id.clone())
        );
        // Someone else's newest message is theirs to edit, not yours.
        assert_eq!(
            id(last_editable(&messages, "member000002", None)),
            Some(messages[3].id.clone())
        );
        assert_eq!(
            id(last_editable(&messages, me, Some(messages[3].id.as_str()))),
            None
        );
        assert_eq!(id(last_editable(&messages[4..], me, None)), None);
    }

    #[test]
    fn opening_an_old_thread_reply_keeps_recent_replies_and_stops_on_missing_targets() {
        let api = Api::new("http://127.0.0.1:9").unwrap();
        let host = WinHost {
            hwnd: 0,
            requests: Arc::default(),
        };
        let voice = Voice::new(
            api.base().clone(),
            caper_client::voice::Repaint::new(|_| {}),
        );
        let mut live = Live::fresh(
            Worker::new(api, host.clone()),
            caper_client::media::Media::new(None, Arc::new(|| {})),
            Arc::default(),
            host,
            1,
            voice,
            crate::settings::preferences::Preferences::load(None),
        );
        live.channel = Some("general".into());
        let root = core_message(serde_json::json!({}));
        let older = core_message(serde_json::json!({
            "id": "b00000000000002", "seq": "20", "threadRootId": root.id,
            "clientMessageId": "00000000-0000-4000-9000-000000000002"
        }));
        let recent = core_message(serde_json::json!({
            "id": "b00000000000003", "seq": "90", "threadRootId": root.id,
            "clientMessageId": "00000000-0000-4000-9000-000000000003"
        }));
        let deliver = |live: &mut Live, messages: Vec<core::Message>, has_more| {
            let mut updates = Vec::new();
            live.handle(
                Event::ThreadLoaded {
                    generation: live.generation,
                    request: live.thread.as_ref().unwrap().request,
                    channel: "general".into(),
                    root: root.id.clone(),
                    result: Ok(core::ThreadHistory {
                        root: root.clone(),
                        messages,
                        cursor: "90".into(),
                        has_more,
                    }),
                },
                &mut updates,
            );
            updates
        };
        live.open_thread_at(&root.id, &older.id);
        assert_eq!(live.take_thread_focus(), None, "wait for history");
        deliver(&mut live, vec![recent.clone()], true);
        assert_eq!(
            live.take_thread_focus(),
            None,
            "the latest page is not the target"
        );
        deliver(&mut live, vec![older.clone()], true);
        assert_eq!(live.take_thread_focus().as_deref(), Some(older.id.as_str()));
        assert_eq!(live.take_thread_focus(), None, "focus is consumed once");
        let (view, _, _) = live.thread_view().unwrap();
        assert_eq!(
            view.messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            [root.id.as_str(), older.id.as_str(), recent.id.as_str()]
        );

        live.open_thread_at(&root.id, "b00000000000004");
        deliver(&mut live, vec![older], true);
        assert!(
            live.thread.as_ref().unwrap().error.is_some(),
            "a repeated cursor stops automatic paging"
        );
        assert_eq!(live.take_thread_focus(), None);
        live.retry_thread();
        let updates = deliver(&mut live, Vec::new(), false);
        assert_eq!(live.take_thread_focus(), None);
        assert!(
            updates.iter().any(|u| matches!(u, Update::Warning(_))),
            "report an unavailable reply in the same update, not on a later wake"
        );
    }

    #[test]
    fn opening_a_conversation_shows_its_existing_pins() {
        let api = Api::new("http://127.0.0.1:9").unwrap();
        let host = WinHost {
            hwnd: 0,
            requests: Arc::default(),
        };
        let voice = Voice::new(
            api.base().clone(),
            caper_client::voice::Repaint::new(|_| {}),
        );
        let mut live = Live::fresh(
            Worker::new(api, host.clone()),
            caper_client::media::Media::new(None, Arc::new(|| {})),
            Arc::default(),
            host,
            1,
            voice,
            crate::settings::preferences::Preferences::load(None),
        );
        let pinned = serde_json::json!({
            "id": "b00000000000001", "channelId": "general", "seq": "1",
            "author": { "id": "member000002", "name": "Alex", "isGuest": false },
            "content": { "version": 1, "type": "text", "text": "pinned before opening" },
            "clientMessageId": "00000000-0000-4000-9000-000000000001",
            "createdAt": "2026-10-01T09:00:00Z",
            "pin": {
                "author": { "id": "member000001", "name": "Maya", "isGuest": false },
                "createdAt": "2026-10-01T09:05:00Z"
            },
            "pinSeq": "2",
        });
        let history: core::History = serde_json::from_value(serde_json::json!({
            "messages": [pinned],
            "pinnedMessages": [pinned],
            "cursor": "2",
            "hasMore": false,
            "space": { "id": "", "name": "Direct messages" },
            "channel": { "id": "general", "name": "general" },
        }))
        .unwrap();
        let mut updates = Vec::new();
        live.handle(
            Event::NavigationPrepared {
                generation: live.generation,
                navigation: live.navigation,
                result: Ok(caper_client::worker::PreparedNavigation {
                    detail: None,
                    conversation: Some((history, Err("no session in this test".into()))),
                }),
            },
            &mut updates,
        );
        let (pins, _) = live.pinned_view().unwrap();
        assert_eq!(
            pins.messages
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            ["b00000000000001"],
            "pins made before opening the conversation are listed"
        );
    }

    #[test]
    fn no_space_can_show_an_empty_account_or_a_direct_without_stale_channels() {
        let spaces = core::Spaces {
            spaces: Vec::new(),
            invitations: Vec::new(),
            limits: None,
        };
        let (empty, ids) = conversation(&spaces, None, None, &[], None, false, None, None);
        assert!(empty.spaces.is_empty() && empty.channels.is_empty() && empty.members.is_empty());
        assert!(!empty.has_conversation());
        assert_eq!(ids.open, None);
        assert_eq!(empty.empty_text().0, Some("Start a conversation"));

        let message = core_message(serde_json::json!({ "channelId": "dm" }));
        let (direct, ids) = conversation(
            &spaces,
            None,
            Some("dm"),
            &[message],
            Some("owner0000001"),
            false,
            None,
            Some(model::Direct {
                name: "Alex".into(),
                own: false,
                note: "".into(),
            }),
        );
        assert!(direct.has_conversation());
        assert!(direct.spaces.is_empty() && direct.channels.is_empty());
        assert_eq!(direct.listed_members, 0);
        assert_eq!(direct.title(), "Alex");
        assert_eq!(ids.open.as_deref(), Some("dm"));
        assert_eq!(direct.members[direct.messages[0].author].name, "Alex");
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

        let original = core_message(serde_json::json!({ "id": "original", "content": content }));
        let mut wrapper = core_message(serde_json::json!({
            "forward": { "seq": "9", "message": original },
        }));
        let shown = message(&wrapper, 0, false, None);
        assert!(
            shown.attachments.is_empty(),
            "the wrapper keeps its own media separate"
        );
        let forward = shown.forward.unwrap();
        assert_eq!(forward.seq, "9");
        let source = forward.message.unwrap();
        assert_eq!(source.id, "original");
        assert_eq!(source.content.gif.unwrap().title, "Wave");
        wrapper.forward.as_mut().unwrap().message = None;
        assert!(
            message(&wrapper, 0, false, None)
                .forward
                .unwrap()
                .message
                .is_none(),
            "unavailable is still a card"
        );
        wrapper.deleted_at = Some("2026-10-01T09:10:00Z".into());
        assert!(
            message(&wrapper, 0, false, None).forward.is_none(),
            "deleting the wrapper hides its forward"
        );

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
            matches!(&row.spans[..], [Span::Rich { text, style: crate::formatting::ITALIC, .. }] if text == "This message was deleted.")
        );
    }
    fn shown(spans: &[Span]) -> Vec<(String, bool, bool, Option<String>)> {
        spans
            .iter()
            .map(|s| match s {
                Span::Rich {
                    text, style, href, ..
                } => (
                    text.clone(),
                    style & crate::formatting::BOLD != 0,
                    style & crate::formatting::ITALIC != 0,
                    href.clone(),
                ),
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
    fn every_style_bit_reaches_its_run() {
        use crate::formatting::{QUOTE, SPOILER, STRIKETHROUGH, UNDERLINE};
        let runs: Vec<(String, i32)> = spans("__u__ ~~s~~ ||x||\n> q", &[])
            .into_iter()
            .map(|s| match s {
                Span::Rich { text, style, .. } => (text, style),
                _ => panic!("live messages are rich runs"),
            })
            .collect();
        for (text, style) in [
            ("u", UNDERLINE),
            ("s", STRIKETHROUGH),
            ("x", SPOILER),
            ("q", QUOTE),
        ] {
            assert!(runs.contains(&(text.to_owned(), style)), "{text}: {runs:?}");
        }
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

    #[test]
    fn inline_code_and_fenced_blocks_suppress_mentions_and_highlight() {
        let maya = core::Mention {
            kind: "user".into(),
            id: Some("member000001".into()),
            username: Some("maya".into()),
        };
        let inline = spans(
            "use `@maya` and `https://x.example`",
            std::slice::from_ref(&maya),
        );
        assert!(inline.iter().all(|s| !matches!(
            s,
            Span::Rich { mention: true, .. } | Span::Rich { href: Some(_), .. }
        )));
        let block = spans("```rust\nfn main() {}\n```", &[]);
        assert!(
            block
                .iter()
                .any(|s| matches!(s, Span::CodeBlock { text, token: 255 } if text == "Rust\n"))
        );
        assert!(
            block
                .iter()
                .any(|s| matches!(s, Span::CodeBlock { text, token: 1 } if text == "fn"))
        );
    }
}
