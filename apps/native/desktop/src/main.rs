#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod api;
mod avatar_images;
mod blocking;
mod credentials;
mod daily_icon;
mod edits;
mod effects;
mod emoji;
mod forwarding;
mod gateway;
#[path = "../voice-spike/src/media.rs"]
mod media;
#[path = "../voice-spike/src/media_gateway.rs"]
mod media_gateway;
mod mentions;
mod model;
mod navigation;
mod notifications;
mod startup;
#[path = "../voice-spike/src/state.rs"]
mod state;
mod updates;
mod voice;
mod worker;

use chrono::{DateTime, Local, TimeZone};
use effects::{Effect, Effects};
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};
use gateway::GatewayEvent;
use model::{
    Account, Author, ChatSession, Member, NotificationLevel, Presence, SpaceDetail, SpaceLimits,
    Spaces, Timeline,
};
use notifications::{Change, Scope};
use state::{CallContext, Phase};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};
use voice::{MicrophoneState, Recorded, Voice, VoiceOperation};
use worker::{
    AccountOperation, AccountResult, AdminOperation, AdminResult, Command, Event, Worker, current,
};

const BLACKOUT: Color32 = Color32::from_rgb(12, 13, 15);
const SURFACE: Color32 = Color32::from_rgb(21, 23, 25);
const RAISED: Color32 = Color32::from_rgb(28, 31, 33);
const SIDEBAR: Color32 = Color32::from_rgb(21, 28, 30);
const CONVERSATION: Color32 = Color32::from_rgb(25, 33, 35);
const COMPOSER: Color32 = Color32::from_rgb(40, 49, 51);
const BORDER: Color32 = Color32::from_rgb(52, 56, 59);
const TEXT: Color32 = Color32::from_rgb(243, 244, 245);
const MUTED: Color32 = Color32::from_rgb(185, 188, 190);
const TERRACOTTA: Color32 = Color32::from_rgb(182, 77, 50);
const TERRACOTTA_BRIGHT: Color32 = Color32::from_rgb(219, 104, 73);
const CAPER: Color32 = Color32::from_rgb(99, 122, 67);
const VOICE_SESSION_GREEN: Color32 = Color32::from_rgb(74, 168, 107);
const ERROR: Color32 = Color32::from_rgb(255, 155, 130);
const MESSAGE_TEXT: Color32 = Color32::from_rgb(222, 223, 224);
const MEMBER_PAGE_SIZE: usize = 25;

#[derive(Clone, Copy)]
enum NavIcon {
    Chevron,
    ChevronRight,
    Close,
    More,
    Plus,
    Settings,
    Hash,
    Lock,
    Users,
    Speech,
    Mic,
    MicOff,
    Headphones,
    VolumeX,
    Menu,
    PhoneOff,
    HeadphoneOff,
    AudioLines,
    BellOff,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingSend {
    id: String,
    text: String,
    created_at: String,
    sending: bool,
    rejection: Option<String>,
    thread_root_id: Option<String>,
    broadcast: bool,
}

impl PendingSend {
    fn prepare(previous: Option<&Self>, draft: &str) -> Self {
        previous.map_or_else(
            || Self {
                id: uuid::Uuid::new_v4().to_string(),
                text: draft.into(),
                created_at: Local::now().to_rfc3339(),
                sending: true,
                rejection: None,
                thread_root_id: None,
                broadcast: false,
            },
            |pending| Self {
                id: pending.id.clone(),
                text: pending.text.clone(),
                created_at: pending.created_at.clone(),
                sending: true,
                rejection: None,
                thread_root_id: pending.thread_root_id.clone(),
                broadcast: pending.broadcast,
            },
        )
    }

    fn confirmed_by(&self, message: &model::Message, author: &str) -> bool {
        self.id == message.client_message_id
            && message.author.id == author
            && self.thread_root_id == message.thread_root_id
            && self.broadcast == message.broadcast
    }
}

struct ThreadView {
    root: String,
    loading: bool,
    has_more: bool,
    before: Option<String>,
    error: Option<String>,
}

fn permanent_send_rejection(status: Option<u16>) -> bool {
    matches!(status, Some(400 | 404 | 409 | 413 | 422))
}

/// The composer token that opens suggestions. Their grammars never overlap,
/// so at most one kind is active at a caret.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ComposerToken {
    Emoji(emoji::Token),
    Mention(mentions::Token),
}

impl ComposerToken {
    fn at(text: &str, caret: usize) -> Option<Self> {
        emoji::token(text, caret)
            .map(Self::Emoji)
            .or_else(|| mentions::token(text, caret).map(Self::Mention))
    }

    fn end(&self) -> usize {
        match self {
            Self::Emoji(token) => token.end,
            Self::Mention(token) => token.end,
        }
    }

    fn insert(&self, text: &str, choice: &Suggestion) -> Option<(String, usize)> {
        match (self, choice) {
            (Self::Emoji(token), Suggestion::Emoji(entry)) => {
                emoji::insert(text, token, &entry.emoji)
            }
            (Self::Mention(token), Suggestion::Mention(candidate)) => {
                mentions::insert(text, token, candidate.name())
            }
            _ => None,
        }
    }
}

#[derive(Clone)]
enum Suggestion {
    Emoji(&'static emoji::Entry),
    Mention(mentions::Candidate),
}

/// A person pill that was clicked or activated with Enter/Space.
#[derive(Clone, Debug)]
struct PillClick {
    /// The pill's focusable widget, which takes focus back on Escape.
    id: egui::Id,
    rect: egui::Rect,
    entry: model::Mention,
}

/// The profile card opened from a person's mention pill.
#[derive(Clone, Debug)]
struct MentionCard {
    pill: PillClick,
    /// The `CreateDirect` navigation while **Message** is opening a DM.
    opening: Option<u64>,
    error: Option<String>,
    /// Opened this pass: the click that opened it is not "outside".
    fresh: bool,
}

#[derive(Clone)]
enum Dialog {
    SignIn,
    Profile,
    Settings,
    Audio,
    Connection,
    Diagnostics,
    CreateSpace,
    ManageSpace,
    Invitation(model::Space),
    LeaveSpace {
        id: String,
        name: String,
    },
    LeaveChannel {
        space: String,
        channel: String,
        name: String,
    },
    ConfirmDelete {
        space: String,
        channel: Option<String>,
        name: String,
    },
    CreateChannel,
    ManageChannel(String),
    StartDirect,
    /// Confirm blocking; `request` is an incoming DM request to drop with it.
    Block {
        account: model::BlockedAccount,
        request: Option<String>,
    },
}

#[derive(Clone)]
struct Typer {
    author: Author,
    typing: bool,
    revision: u64,
    expires: Instant,
}

#[derive(Clone)]
struct NavigationTarget {
    space: Option<String>,
    channel: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SelfDirectTarget {
    Existing(model::DirectConversation),
    Create(String),
}

#[derive(Clone)]
struct PendingReaction {
    desired: bool,
    sent: bool,
    visible: bool,
    superseded: bool,
}

/// Who reacted to one message, read at the snapshot's `reactionSeq`.
struct ReactorCache {
    revision: String,
    state: ReactorState,
}

enum ReactorState {
    Loading(Instant),
    Loaded(Vec<model::ReactorGroup>),
    /// Hovering again retries after `REACTOR_RETRY`.
    Failed(Instant),
}

impl ReactorCache {
    /// Whether hovering should (re)start the request for `revision`.
    fn wants_request(&self, revision: &str) -> bool {
        self.revision != revision
            || match self.state {
                ReactorState::Loaded(_) => false,
                // The API client times out after 15 s, so this request is lost.
                ReactorState::Loading(at) => at.elapsed() >= Duration::from_secs(20),
                ReactorState::Failed(at) => at.elapsed() >= REACTOR_RETRY,
            }
    }
}

const REACTOR_RETRY: Duration = Duration::from_secs(5);
const REACTOR_CACHE_LIMIT: usize = 256;
/// Discord-style hover card: large emoji, then the summary.
const REACTOR_TOOLTIP_WIDTH: f32 = 280.0;
const REACTOR_TOOLTIP_EMOJI: f32 = 36.0;

fn projected_reactions(
    reactions: &[model::Reaction],
    message: &str,
    author: Option<&str>,
    pending: &BTreeMap<(String, String), PendingReaction>,
) -> Vec<model::Reaction> {
    let mut projected = reactions.to_vec();
    let Some(author) = author else {
        return projected;
    };
    for ((pending_message, emoji), intent) in pending {
        if pending_message != message || !intent.visible {
            continue;
        }
        if let Some(reaction) = projected.iter_mut().find(|item| item.emoji == *emoji) {
            reaction.author_ids.retain(|id| id != author);
            if intent.desired {
                reaction.author_ids.push(author.to_owned());
            }
        } else if intent.desired {
            projected.push(model::Reaction {
                emoji: emoji.clone(),
                author_ids: vec![author.to_owned()],
            });
        }
    }
    projected.retain(|reaction| !reaction.author_ids.is_empty());
    projected
}

struct CaperApp {
    daily_icon: Option<daily_icon::DailyIcon>,
    worker: Worker,
    voice: Voice,
    effects: Effects,
    sound_effects: bool,
    launch_at_login: bool,
    startup_error: Option<String>,
    announced_voice: Option<u64>,
    /// Who else was in the call last frame, for web's join/leave chimes.
    voice_roster: Option<(u64, String, BTreeSet<String>)>,
    generation: u64,
    loading: bool,
    loading_older: bool,
    older_error: Option<String>,
    /// The first history load failed; shown in the pane with "Try again".
    load_error: Option<String>,
    /// When the live state last changed, for web's 1 s status delay.
    live_changed: Instant,
    was_live: bool,
    /// Auto-loading older history waits until the list has settled once.
    older_armed: bool,
    /// History height before older messages were prepended, to keep position.
    older_anchor: Option<f32>,
    history_height: f32,
    history_offset: Option<f32>,
    has_more: bool,
    error: Option<String>,
    warning: Option<String>,
    token: Option<String>,
    account: Option<Account>,
    spaces: Vec<model::Space>,
    directs: Vec<model::DirectConversation>,
    /// `GET /api/people` for DM `@` suggestions; kept while it refreshes.
    people: Option<Vec<model::Person>>,
    /// Bumps on sign-in and sign-out; fences request, block, privacy and
    /// notification results.
    account_epoch: u64,
    /// `GET /api/blocks`, newest first; `None` until loaded.
    blocks: Option<Vec<model::BlockedAccount>>,
    /// Blocked-message runs shown in place, by first message id (memory only).
    revealed_blocked: BTreeSet<String>,
    requests_open: bool,
    /// The open request bar's Accept/Decline/Block in flight, and its error.
    request_busy: bool,
    request_error: Option<String>,
    /// Block, unblock, or confirmation in flight, and the last failure.
    block_busy: bool,
    block_error: Option<String>,
    /// Who can start a DM with you: `anyone`, `spaces` or `nobody`.
    privacy: Option<String>,
    privacy_saving: bool,
    privacy_error: Option<String>,
    /// Notification levels and mutes; the controls only, for phone push.
    notifications: notifications::Notifications,
    selected_direct: Option<String>,
    directs_refreshed: Instant,
    foreground: bool,
    invitations: Vec<model::Space>,
    limits: Option<SpaceLimits>,
    selected_space: Option<String>,
    detail: Option<SpaceDetail>,
    selected_channel: Option<String>,
    session: Option<ChatSession>,
    session_error: Option<String>,
    timeline: Timeline,
    forwarding: forwarding::Forwarding,
    live: String,
    email: String,
    challenge: Option<String>,
    attempts_remaining: Option<u64>,
    code: String,
    username: String,
    display_name: String,
    draft: String,
    suggestion_token: Option<ComposerToken>,
    suggestion_selected: usize,
    suggestion_dismissed: Option<(String, usize)>,
    ime_composing: bool,
    pending: Option<PendingSend>,
    mention_card: Option<MentionCard>,
    thread_view: Option<ThreadView>,
    thread_request: u64,
    thread_drafts: BTreeMap<String, (String, bool)>,
    thread_only_rows: BTreeSet<String>,
    reaction_picker: Option<String>,
    reaction_search: String,
    reaction_search_focus: bool,
    reaction_textures: emoji::Textures,
    pending_reactions: BTreeMap<(String, String), PendingReaction>,
    reaction_errors: BTreeMap<String, String>,
    /// Who reacted, by message ID, for reaction chip hover cards.
    reactors: BTreeMap<String, ReactorCache>,
    pending_pins: BTreeSet<String>,
    mutations: model::MessageMutations,
    pin_errors: BTreeMap<String, (bool, String)>,
    showing_pins: bool,
    message_editor: Option<edits::Editor>,
    edit_history: Option<edits::History>,
    edit_request: u64,
    typers: BTreeMap<String, Typer>,
    typing_sent: bool,
    typing_edited: Instant,
    typing_pulse: Instant,
    presence: BTreeMap<String, String>,
    member_page: usize,
    members_visible: bool,
    narrow_members_visible: bool,
    channels_expanded: bool,
    browse_channels: bool,
    channel_search: String,
    roster_generation: u64,
    voice_join_request: u64,
    pending_voice_join: Option<(String, u64, u64)>,
    channel_rosters: BTreeMap<String, Vec<model::VoiceOccupant>>,
    voice_session_starts: BTreeMap<String, u64>,
    unavailable_rosters: BTreeSet<String>,
    /// Web's `/status` answers by media root; absent while checking.
    media_availability: BTreeMap<String, bool>,
    media_status_roots: BTreeSet<String>,
    /// When web's `media.prepare` was last sent per channel.
    prepared_voice: BTreeMap<String, Instant>,
    expanded_rosters: BTreeSet<String>,
    sidebar_width: f32,
    navigation_open: bool,
    navigation: u64,
    opening: bool,
    navigation_target: Option<NavigationTarget>,
    navigation_prefetch: Option<navigation::Target>,
    navigation_error: Option<String>,
    navigation_cache: navigation::NavigationCache,
    navigation_cache_generation: u64,
    dialog: Option<Dialog>,
    form_name: String,
    form_private: bool,
    member_username: String,
    member_error: Option<&'static str>,
    managed_members: Vec<Member>,
    managed_invitations: Vec<Member>,
    managed_channel: Option<String>,
    persist_preferences: bool,
    connection_copy_status: &'static str,
    diagnostics_copied: bool,
    updates: updates::Updates,
}

impl CaperApp {
    fn new(context: &egui::Context, api: api::Api, fixture: Option<&str>) -> Self {
        configure(context);
        let voice = Voice::new(api.base().clone(), context.clone());
        let worker = Worker::new(api, context.clone());
        let now = Instant::now();
        let mut app = Self {
            daily_icon: None,
            worker,
            voice,
            effects: Effects::new(fixture.is_none()),
            sound_effects: true,
            launch_at_login: false,
            startup_error: None,
            announced_voice: None,
            voice_roster: None,
            generation: 1,
            loading: fixture.is_none(),
            loading_older: false,
            older_error: None,
            load_error: None,
            live_changed: now,
            was_live: false,
            older_armed: false,
            older_anchor: None,
            history_height: 0.0,
            history_offset: None,
            has_more: false,
            error: None,
            warning: None,
            token: None,
            account: None,
            spaces: Vec::new(),
            directs: Vec::new(),
            people: None,
            account_epoch: 0,
            blocks: None,
            revealed_blocked: BTreeSet::new(),
            requests_open: false,
            request_busy: false,
            request_error: None,
            block_busy: false,
            block_error: None,
            privacy: None,
            privacy_saving: false,
            privacy_error: None,
            notifications: notifications::Notifications::default(),
            selected_direct: None,
            directs_refreshed: now - Duration::from_secs(15),
            foreground: false,
            invitations: Vec::new(),
            limits: None,
            selected_space: None,
            detail: None,
            selected_channel: None,
            session: None,
            session_error: None,
            timeline: Timeline::default(),
            forwarding: forwarding::Forwarding::default(),
            live: "Connecting…".into(),
            email: String::new(),
            challenge: None,
            attempts_remaining: None,
            code: String::new(),
            username: String::new(),
            display_name: String::new(),
            draft: String::new(),
            suggestion_token: None,
            suggestion_selected: 0,
            suggestion_dismissed: None,
            ime_composing: false,
            pending: None,
            mention_card: None,
            thread_view: None,
            thread_request: 0,
            thread_drafts: BTreeMap::new(),
            thread_only_rows: BTreeSet::new(),
            reaction_picker: None,
            reaction_search: String::new(),
            reaction_search_focus: false,
            reaction_textures: emoji::Textures::default(),
            pending_reactions: BTreeMap::new(),
            reaction_errors: BTreeMap::new(),
            reactors: BTreeMap::new(),
            pending_pins: BTreeSet::new(),
            mutations: model::MessageMutations::default(),
            pin_errors: BTreeMap::new(),
            showing_pins: false,
            message_editor: None,
            edit_history: None,
            edit_request: 0,
            typers: BTreeMap::new(),
            typing_sent: false,
            typing_edited: now,
            typing_pulse: now,
            presence: BTreeMap::new(),
            member_page: 0,
            members_visible: true,
            narrow_members_visible: false,
            channels_expanded: true,
            browse_channels: false,
            channel_search: String::new(),
            roster_generation: 0,
            voice_join_request: 0,
            pending_voice_join: None,
            channel_rosters: BTreeMap::new(),
            voice_session_starts: BTreeMap::new(),
            unavailable_rosters: BTreeSet::new(),
            media_availability: BTreeMap::new(),
            media_status_roots: BTreeSet::new(),
            prepared_voice: BTreeMap::new(),
            expanded_rosters: BTreeSet::new(),
            sidebar_width: 280.0,
            navigation_open: false,
            navigation: 0,
            opening: false,
            navigation_target: None,
            navigation_prefetch: None,
            navigation_error: None,
            navigation_cache: navigation::NavigationCache::default(),
            navigation_cache_generation: 1,
            dialog: None,
            form_name: String::new(),
            form_private: false,
            member_username: String::new(),
            member_error: None,
            managed_members: Vec::new(),
            managed_invitations: Vec::new(),
            managed_channel: None,
            persist_preferences: fixture.is_none(),
            connection_copy_status: "",
            diagnostics_copied: false,
            updates: updates::Updates::start(context, fixture.is_none()),
        };
        if let Some(name) = fixture.filter(|name| name.starts_with("parity-update")) {
            app.updates = updates::Updates::preview(updates::Available {
                version: "0.1.42".into(),
                notes: "Clearer update check feedback".into(),
                changelog: (36..=42)
                    .rev()
                    .map(|build| updates::ChangelogEntry {
                        version: format!("0.1.{build}"),
                        notes: "• Clearer update check feedback\n• Faster channel navigation with retained drafts\n• Improved native audio device selection".into(),
                    })
                    .collect(),
                history_complete: name != "parity-update-incomplete",
                can_apply: name != "parity-update-download",
            });
            if name == "parity-update-error" {
                app.updates.error = Some("Could not start the update. Please try again.".into());
            }
        }
        match fixture {
            Some("error" | "login-error") => {
                app.dialog = Some(Dialog::SignIn);
                app.email = "fixture@example.test".into();
                app.error =
                    Some("Sign-in is temporarily unavailable. Please try again later.".into())
            }
            Some("login") => app.dialog = Some(Dialog::SignIn),
            Some(name) if name.starts_with("parity") => {
                app.install_fixture();
                if name.starts_with("parity-settings") {
                    app.dialog = Some(Dialog::Settings);
                    app.updates = updates::Updates::preview_status(match name {
                        "parity-settings-checking" => updates::Status::Checking,
                        "parity-settings-current" => updates::Status::UpToDate,
                        "parity-settings-offline" => {
                            updates::Status::Failed("Offline. Try again when connected.".into())
                        }
                        _ => updates::Status::Idle,
                    });
                }
                if name == "parity-reactions" {
                    app.session = Some(ChatSession {
                        token: "fixture-token".into(),
                        author: Author {
                            id: "fixture-owner".into(),
                            name: "Fixture Owner".into(),
                            is_guest: false,
                            avatar_id: Some(0),
                        },
                    });
                    let mut messages: Vec<_> = app.timeline.messages().cloned().collect();
                    messages[1].reactions = vec![
                        model::Reaction {
                            emoji: "👍".into(),
                            author_ids: vec!["fixture-owner".into(), "fixture-maya".into()],
                        },
                        model::Reaction {
                            emoji: "❤️".into(),
                            author_ids: vec!["fixture-maya".into()],
                        },
                        model::Reaction {
                            emoji: "🎉".into(),
                            author_ids: vec!["fixture-alex".into()],
                        },
                    ];
                    messages[1].reaction_seq = Some("4".into());
                    app.timeline
                        .reset(messages, "4")
                        .expect("valid reaction fixture");
                } else if name == "parity-mentions" {
                    // Labelled sample entries as the server would resolve them:
                    // you (Fixture Owner) are named once and by Alex's @everyone.
                    let entry =
                        |kind: &str, id: Option<&str>, username: Option<&str>| model::Mention {
                            kind: kind.into(),
                            id: id.map(Into::into),
                            username: username.map(Into::into),
                        };
                    let mut messages: Vec<_> = app.timeline.messages().cloned().collect();
                    messages[1].content.text =
                        "@fixture_owner, the same conversation should feel familiar on every platform."
                            .into();
                    messages[1].content.mentions =
                        vec![entry("user", Some("fixture-owner"), Some("fixture_owner"))];
                    messages[2].content.text =
                        "Keep the space rail and audio controls in their usual places, @alex. @nobody stays plain."
                            .into();
                    messages[2].content.mentions =
                        vec![entry("user", Some("fixture-alex"), Some("alex"))];
                    messages[3].content.text =
                        "Agreed, @everyone. Let’s check the narrow layout and the management dialogs too."
                            .into();
                    messages[3].content.mentions = vec![entry("everyone", None, None)];
                    app.timeline
                        .reset(messages, "4")
                        .expect("valid mentions fixture");
                } else if name.starts_with("parity-edits") {
                    let mut message = app.timeline.messages().nth(1).unwrap().clone();
                    let original = message.clone();
                    message.content.text =
                        "The same edited conversation should feel familiar on every platform."
                            .into();
                    message.revision = 2;
                    message.edited_at = Some("2026-10-06T09:44:00Z".into());
                    message.edit_seq = Some("5".into());
                    app.timeline
                        .apply_edit(model::EditUpdate {
                            kind: "message.edited".into(),
                            schema_version: 1,
                            channel_id: message.channel_id.clone(),
                            seq: "5".into(),
                            message: message.clone(),
                        })
                        .expect("valid edit fixture");
                    if name == "parity-edits-history" {
                        app.edit_history = Some(edits::History {
                            message: message.clone(),
                            versions: vec![
                                model::MessageVersion {
                                    revision: 2,
                                    content: message.content.clone(),
                                    created_at: message.edited_at.clone().unwrap(),
                                },
                                model::MessageVersion {
                                    revision: 1,
                                    content: original.content.clone(),
                                    created_at: original.created_at.clone(),
                                },
                            ],
                            selected: None,
                            loading: false,
                            older: false,
                            more: false,
                            error: None,
                            request: 0,
                            requested_revision: 2,
                        });
                    }
                    if name == "parity-edits-editor" {
                        app.session = Some(ChatSession {
                            token: "fixture-token".into(),
                            author: message.author.clone(),
                        });
                        app.open_editor(&message);
                    }
                } else if name.starts_with("parity-requests") || name.starts_with("parity-blocked")
                {
                    // Labelled previews of message requests and blocking.
                    let peer =
                        |id: &str, username: &str, name: &str, avatar: i32| model::DirectPeer {
                            id: id.into(),
                            username: username.into(),
                            display_name: format!("TEST FIXTURE {name}"),
                            avatar_id: Some(avatar),
                        };
                    let direct =
                        |id: &str, peer: model::DirectPeer, status| model::DirectConversation {
                            id: id.into(),
                            peer,
                            last_seq: "1".into(),
                            read_seq: "1".into(),
                            status,
                            blocked: false,
                        };
                    let maya = model::BlockedAccount {
                        id: "fixture-maya".into(),
                        username: "maya".into(),
                        display_name: "Maya".into(),
                        avatar_id: Some(15),
                    };
                    app.directs = vec![
                        direct(
                            "dm0000000001",
                            peer("fixture-maya", "maya", "Maya", 15),
                            model::DirectStatus::Accepted,
                        ),
                        direct(
                            "dm0000000003",
                            peer("stranger0001", "jordan", "Jordan", 412),
                            model::DirectStatus::Incoming,
                        ),
                        direct(
                            "dm0000000004",
                            peer("stranger0002", "sam", "Sam", 300),
                            model::DirectStatus::Outgoing,
                        ),
                    ];
                    let open = match name {
                        "parity-requests" => Some((
                            "dm0000000003",
                            "stranger0001",
                            "TEST FIXTURE Jordan",
                            412,
                            "TEST FIXTURE — Hi! Could we talk about the mural?",
                        )),
                        "parity-requests-outgoing" => Some((
                            "dm0000000004",
                            "fixture-owner",
                            "Fixture Owner",
                            0,
                            "TEST FIXTURE — Hi Sam, are you joining Saturday?",
                        )),
                        "parity-blocked-dm" => Some((
                            "dm0000000001",
                            "fixture-maya",
                            "Maya",
                            15,
                            "TEST FIXTURE — A message from Maya.",
                        )),
                        _ => None,
                    };
                    if name.starts_with("parity-blocked") {
                        app.blocks = Some(vec![maya]);
                        app.directs[0].blocked = true;
                    }
                    app.requests_open = name == "parity-requests";
                    if let Some((id, author, author_name, avatar, text)) = open {
                        let mut message = app.timeline.messages().next().unwrap().clone();
                        message.channel_id = id.into();
                        message.author = Author {
                            id: author.into(),
                            avatar_id: Some(avatar),
                            name: author_name.into(),
                            is_guest: false,
                        };
                        message.content.text = text.into();
                        message.seq = "1".into();
                        app.timeline
                            .reset(vec![message], "1")
                            .expect("valid DM fixture");
                        app.selected_channel = Some(id.into());
                        app.selected_direct = Some(id.into());
                    }
                } else if matches!(
                    name,
                    "parity-direct" | "parity-direct-new" | "parity-direct-no-spaces"
                ) {
                    let id = "dm0000000001".to_owned();
                    app.directs = vec![model::DirectConversation {
                        id: id.clone(),
                        peer: model::DirectPeer {
                            id: "fixture-maya".into(),
                            username: "maya".into(),
                            display_name: "TEST FIXTURE Maya".into(),
                            avatar_id: None,
                        },
                        last_seq: "2".into(),
                        read_seq: "2".into(),
                        status: model::DirectStatus::Accepted,
                        blocked: false,
                    }];
                    let mut messages: Vec<_> = app.timeline.messages().take(2).cloned().collect();
                    for message in &mut messages {
                        message.channel_id = id.clone();
                    }
                    app.timeline.reset(messages, "2").expect("valid DM fixture");
                    app.selected_channel = Some(id.clone());
                    app.selected_direct = Some(id);
                    // As `GET /api/people` answers: space members and DM peers, not you.
                    app.people = app.detail.as_ref().map(|detail| {
                        detail
                            .members
                            .iter()
                            .filter(|member| member.id != "fixture-owner")
                            .map(|member| model::Person {
                                id: member.id.clone(),
                                username: member.username.clone(),
                                display_name: member.display_name.clone(),
                                avatar_id: member.avatar_id,
                            })
                            .collect()
                    });
                    if name == "parity-direct-new" {
                        app.dialog = Some(Dialog::StartDirect);
                    } else if name == "parity-direct-no-spaces" {
                        app.spaces.clear();
                        app.detail = None;
                        app.selected_space = None;
                        app.selected_direct = None;
                        app.clear_channel_state();
                    }
                } else if name == "parity-muted" {
                    // Labelled mutes: a muted space in the rail, a muted
                    // channel, a muted DM with unread messages (no dot) and
                    // a DM with notifications off (still dotted).
                    let quiet = model::Space {
                        id: "space0000002".into(),
                        name: "TEST FIXTURE · Quiet Room".into(),
                        owner_id: "fixture-maya".into(),
                        inviter: None,
                        demo: false,
                    };
                    app.spaces.push(quiet);
                    let direct = |id: &str, peer: &str, name: &str| model::DirectConversation {
                        id: id.into(),
                        peer: model::DirectPeer {
                            id: peer.into(),
                            username: name.to_lowercase(),
                            display_name: format!("TEST FIXTURE {name}"),
                            avatar_id: None,
                        },
                        last_seq: "4".into(),
                        read_seq: "1".into(),
                        status: model::DirectStatus::Accepted,
                        blocked: false,
                    };
                    app.directs = vec![
                        direct("dm0000000001", "fixture-maya", "Maya"),
                        direct("dm0000000005", "fixture-alex", "Alex"),
                    ];
                    let until = notifications::mute_value(Some(2 * 60), chrono::Utc::now());
                    app.notifications = notifications::Notifications::with_settings(
                        serde_json::from_value(serde_json::json!({
                            "level": "mentions",
                            "mobile": "whenInactive",
                            "overrides": [
                                {"spaceId": "space0000002", "level": null, "mutedUntil": "forever"},
                                {"spaceId": "space0000001", "channelId": "chan00000002", "level": null, "mutedUntil": until},
                                {"conversationId": "dm0000000001", "level": null, "mutedUntil": "forever"},
                                {"conversationId": "dm0000000005", "level": "nothing", "mutedUntil": null}
                            ]
                        }))
                        .expect("valid notification fixture"),
                    );
                } else if matches!(name, "parity-invitation" | "parity-invitation-narrow") {
                    let invitation = model::Space {
                        id: "invite000001".into(),
                        name: "TEST FIXTURE · Invited Studio".into(),
                        owner_id: "fixture-inviter".into(),
                        demo: false,
                        inviter: Some(model::Inviter {
                            username: "fixture_host".into(),
                            display_name: "TEST FIXTURE host".into(),
                        }),
                    };
                    app.spaces.clear();
                    app.detail = None;
                    app.selected_space = None;
                    app.clear_channel_state();
                    app.invitations.push(invitation.clone());
                    app.dialog = Some(Dialog::Invitation(invitation));
                } else if name.starts_with("parity-channel-preview")
                    || name.starts_with("parity-channel-directory")
                {
                    app.account.as_mut().unwrap().id = "fixture-maya".into();
                    let detail = app.detail.as_mut().unwrap();
                    detail.channels[1].joined = false;
                    let mut private = detail.channels.pop().unwrap();
                    private.joined = false;
                    detail.channel_invitations.push(model::ChannelInvitation {
                        channel: private,
                        inviter: model::Inviter {
                            username: "fixture_host".into(),
                            display_name: "TEST FIXTURE host".into(),
                        },
                    });
                    if name.starts_with("parity-channel-preview") {
                        app.selected_channel = Some("chan00000002".into());
                        app.session = None;
                        app.session_error = None;
                    } else {
                        app.browse_channels = true;
                        app.navigation_open = name.ends_with("-narrow");
                    }
                } else if matches!(name, "parity-admin" | "parity-admin-invitations") {
                    app.form_name = "Fixture Studio".into();
                    app.managed_members = app
                        .detail
                        .as_ref()
                        .map_or_else(Vec::new, |detail| detail.members.clone());
                    if name == "parity-admin-invitations" {
                        app.managed_invitations.push(Member {
                            id: "invited00001".into(),
                            avatar_id: None,
                            username: "fixture_invitee".into(),
                            display_name: "TEST FIXTURE invitee".into(),
                            owner: false,
                        });
                    }
                    app.dialog = Some(Dialog::ManageSpace);
                } else if name == "parity-channel" {
                    app.form_name = "planning".into();
                    app.form_private = true;
                    app.managed_channel = Some("chan00000003".into());
                    app.managed_members = app
                        .detail
                        .as_ref()
                        .map_or_else(Vec::new, |detail| detail.members[..2].to_vec());
                    app.dialog = Some(Dialog::ManageChannel("chan00000003".into()));
                } else if matches!(name, "parity-no-channels" | "parity-no-channels-member") {
                    app.detail.as_mut().unwrap().channels.clear();
                    app.selected_channel = None;
                    if name.ends_with("-member") {
                        app.account.as_mut().unwrap().id = "fixture-maya".into();
                    }
                } else if name == "parity-create-channel" {
                    app.dialog = Some(Dialog::CreateChannel);
                } else if name == "parity-create-space" {
                    app.dialog = Some(Dialog::CreateSpace);
                } else if name == "parity-channel-dirty" {
                    app.form_name = "planning-notes".into();
                    app.form_private = true;
                    app.managed_channel = Some("chan00000003".into());
                    app.dialog = Some(Dialog::ManageChannel("chan00000003".into()));
                } else if name == "parity-voice-checking" {
                    app.media_availability.clear();
                } else if name == "parity-voice-unavailable" {
                    app.media_availability.insert("chan00000001".into(), false);
                } else if name == "parity-voice-error" {
                    app.voice.error = Some(media::mic_test::CAPTURE_START_ERROR.into());
                } else if name == "parity-browse" {
                    app.navigation_open = true;
                } else if matches!(name, "parity-voice-rosters" | "parity-voice-rosters-narrow") {
                    app.channel_rosters.insert(
                        "chan00000002".into(),
                        vec![
                            model::VoiceOccupant {
                                id: "fixture-maya".into(),
                                avatar_id: Some(15),
                                name: "Maya".into(),
                                muted: false,
                                deafened: false,
                            },
                            model::VoiceOccupant {
                                id: "fixture-alex".into(),
                                avatar_id: Some(16),
                                name: "Alex".into(),
                                muted: true,
                                deafened: false,
                            },
                        ],
                    );
                    app.voice_session_starts.insert(
                        "chan00000002".into(),
                        chrono::Utc::now().timestamp_millis().max(0) as u64 - 1_701_000,
                    );
                    app.navigation_open = name.ends_with("-narrow");
                } else if name == "parity-typing" {
                    app.typers.insert(
                        "fixture-maya".into(),
                        Typer {
                            author: Author {
                                id: "fixture-maya".into(),
                                avatar_id: Some(15),
                                name: "Maya".into(),
                                is_guest: false,
                            },
                            typing: true,
                            revision: 1,
                            expires: Instant::now() + Duration::from_secs(3_600),
                        },
                    );
                    app.draft = "a".repeat(3_760);
                } else if name == "parity-load-error" {
                    app.timeline = Timeline::default();
                    app.load_error =
                        Some("Could not reach Caper. Check your connection and try again.".into());
                    app.live = "Offline".into();
                    app.live_changed = Instant::now() - Duration::from_secs(2);
                } else if name == "parity-loading" {
                    app.timeline = Timeline::default();
                    app.loading = true;
                } else if matches!(
                    name,
                    "parity-opening" | "parity-opening-narrow" | "parity-opening-error"
                ) {
                    app.navigation_target = Some(NavigationTarget {
                        space: app.selected_space.clone(),
                        channel: Some("chan00000002".into()),
                    });
                    app.draft = "TEST FIXTURE · loading preview".into();
                    app.opening = name != "parity-opening-error";
                    if !app.opening {
                        app.navigation_error = Some(
                            "Could not reach Caper. Check your connection and try again.".into(),
                        );
                    }
                } else if name == "parity-older-error" {
                    app.has_more = true;
                    app.older_error = Some("History unavailable".into());
                    app.live = "Reconnecting…".into();
                    app.live_changed = Instant::now() - Duration::from_secs(2);
                } else if name == "parity-rejected" {
                    let mut pending = PendingSend::prepare(
                        None,
                        "This fixture message was rejected. Edit or dismiss it without losing your next draft.",
                    );
                    pending.sending = false;
                    pending.rejection =
                        Some("Message could not be accepted (test fixture).".into());
                    app.pending = Some(pending);
                } else if name == "parity-profile" {
                    app.username = "fixture_owner".into();
                    app.display_name = "Fixture Owner".into();
                    app.dialog = Some(Dialog::Profile);
                } else if name == "parity-onboarding" {
                    let account = app.account.as_mut().unwrap();
                    account.username = None;
                    account.display_name = None;
                    app.dialog = Some(Dialog::Profile);
                } else if name == "parity-member" {
                    app.account.as_mut().unwrap().id = "fixture-maya".into();
                    app.account.as_mut().unwrap().display_name = Some("Maya".into());
                } else if name == "parity-audio-debug" {
                    app.account.as_mut().unwrap().debug_enabled = true;
                    app.dialog = Some(Dialog::Diagnostics);
                } else if matches!(
                    name,
                    "parity-audio" | "parity-audio-recorded" | "parity-audio-error"
                ) {
                    app.dialog = Some(Dialog::Audio);
                    if name == "parity-audio-recorded" {
                        app.voice.microphone = MicrophoneState::Ready(Recorded {
                            seconds: 3.6,
                            silent: false,
                        });
                    } else if name == "parity-audio-error" {
                        app.voice.microphone_error =
                            Some(media::mic_test::CAPTURE_START_ERROR.into());
                    }
                } else if matches!(
                    name,
                    "parity-voice-joining"
                        | "parity-voice-connected"
                        | "parity-voice-speaking"
                        | "parity-connection"
                ) {
                    // Explicit visual fixtures only: no media transport is started.
                    let call = CallContext {
                        channel_id: "chan00000001".into(),
                        channel_name: "general".into(),
                        space_name: "Fixture Studio".into(),
                    };
                    app.voice.active_space = Some("space0000001".into());
                    if name == "parity-voice-joining" {
                        app.voice.session_started_at = Some(
                            (chrono::Utc::now().timestamp_millis().max(0) as u64)
                                .saturating_sub(2_000),
                        );
                    }
                    app.voice.state.phase = if name != "parity-voice-joining" {
                        app.voice.self_id = "fixture-owner".into();
                        app.voice.participants = app
                            .detail
                            .as_ref()
                            .unwrap()
                            .members
                            .iter()
                            .map(|member| media::Participant {
                                id: member.id.clone(),
                                avatar_id: member.avatar_id,
                                name: member.display_name.clone(),
                                muted: false,
                                deafened: false,
                                tracks: vec![media::Track {
                                    id: format!("fixture-track-{}", member.id),
                                    kind: "microphone".into(),
                                }],
                            })
                            .collect();
                        app.voice.diagnostics = Some((
                            media::Diagnostics {
                                received_bytes: 4000,
                                sent_bytes: 6000,
                                receive_bitrate: 8000.0,
                                send_bitrate: 16000.0,
                                packets_lost: 2,
                                max_jitter_ms: 17.0,
                                round_trip_ms: 42.0,
                                route: "relay",
                                checks: Some("3 sent · 3 answered".into()),
                            },
                            Instant::now(),
                        ));
                        app.voice.join_times = Some(voice::JoinTimes {
                            joined_ms: 812.0,
                            session_ms: 356.0,
                            transport_ms: 204.0,
                            ice_ms: Some(188.0),
                            roster_ms: 41.0,
                        });
                        if name == "parity-connection" {
                            app.dialog = Some(Dialog::Connection);
                        }
                        if name == "parity-voice-speaking" {
                            // Fixture levels: you and Maya lit, Alex quiet.
                            let lit = Instant::now() + Duration::from_secs(3_600);
                            app.voice.activity = media::VoiceActivity {
                                local: Some(lit),
                                participants: [("fixture-maya".to_owned(), lit)].into(),
                            };
                        }
                        Phase::Connected(call)
                    } else {
                        Phase::Joining(call)
                    };
                }
                if matches!(name, "parity-narrow" | "parity-browse") {
                    app.members_visible = false;
                }
            }
            Some(_) => {}
            None => app.worker.send(Command::Restore {
                generation: app.generation,
            }),
        }
        app
    }

    fn install_fixture(&mut self) {
        use model::{Channel, Content, Message, Space};
        let demo = Space {
            id: "demo00000002".into(),
            name: "General".into(),
            owner_id: String::new(),
            inviter: None,
            demo: true,
        };
        let space = Space {
            id: "space0000001".into(),
            name: "Fixture Studio".into(),
            owner_id: "fixture-owner".into(),
            inviter: None,
            demo: false,
        };
        let channel = Channel {
            id: "chan00000001".into(),
            space_id: space.id.clone(),
            name: "general".into(),
            private: false,
            joined: true,
        };
        self.account = Some(Account {
            id: "fixture-owner".into(),
            avatar_id: Some(0),
            username: Some("fixture_owner".into()),
            display_name: Some("Fixture Owner".into()),
            debug_enabled: false,
        });
        let channels = vec![
            channel.clone(),
            Channel {
                id: "chan00000002".into(),
                space_id: space.id.clone(),
                name: "design".into(),
                private: false,
                joined: true,
            },
            Channel {
                id: "chan00000003".into(),
                space_id: space.id.clone(),
                name: "planning".into(),
                private: true,
                joined: true,
            },
        ];
        self.spaces = vec![demo, space.clone()];
        self.limits = Some(model::SpaceLimits {
            owned_spaces: 20,
            total_spaces: 100,
            channels_per_space: 100,
        });
        self.selected_space = Some(space.id.clone());
        self.selected_channel = Some(channel.id.clone());
        self.detail = Some(SpaceDetail {
            space,
            channels,
            members: vec![
                Member {
                    id: "fixture-owner".into(),
                    avatar_id: Some(0),
                    username: "fixture_owner".into(),
                    display_name: "Fixture Owner".into(),
                    owner: true,
                },
                Member {
                    id: "fixture-maya".into(),
                    avatar_id: Some(15),
                    username: "maya".into(),
                    display_name: "Maya".into(),
                    owner: false,
                },
                Member {
                    id: "fixture-alex".into(),
                    avatar_id: Some(16),
                    username: "alex".into(),
                    display_name: "Alex".into(),
                    owner: false,
                },
            ],
            channel_invitations: vec![],
        });
        self.presence
            .insert("fixture-owner".into(), "online".into());
        self.presence.insert("fixture-maya".into(), "online".into());
        self.presence.insert("fixture-alex".into(), "idle".into());
        let rows = [
            (
                "fixture-owner",
                "Fixture Owner",
                "09:40",
                "TEST FIXTURE — local sample data, not a live conversation.",
            ),
            (
                "fixture-maya",
                "Maya",
                "09:41",
                "The same conversation should feel familiar on every platform.",
            ),
            (
                "fixture-maya",
                "Maya",
                "09:42",
                "Keep the space rail, channel list, and audio controls in their usual places.",
            ),
            (
                "fixture-alex",
                "Alex",
                "09:43",
                "Agreed. Let’s check the narrow layout and the management dialogs too.",
            ),
        ];
        let messages = rows
            .into_iter()
            .enumerate()
            .map(|(index, (id, name, time, text))| Message {
                id: format!("fixture-message-{index}"),
                channel_id: channel.id.clone(),
                seq: (index + 1).to_string(),
                created_at: format!("2026-09-23T{time}:00Z"),
                client_message_id: format!("fixture-client-{index}"),
                author: Author {
                    id: id.into(),
                    avatar_id: Some(index as i32),
                    name: name.into(),
                    is_guest: false,
                },
                content: Content {
                    version: 1,
                    kind: "text".into(),
                    text: text.into(),
                    mentions: Vec::new(),
                },
                reactions: Vec::new(),
                reaction_seq: None,
                pin: None,
                pin_seq: None,
                thread_root_id: None,
                broadcast: false,
                thread: None,
                forward: None,
                forward_seq: None,
                revision: 1,
                edited_at: None,
                edit_seq: None,
            })
            .collect();
        self.timeline.reset(messages, "4").expect("valid fixture");
        // Static previews never load account state over the network.
        self.privacy = Some("anyone".into());
        self.blocks = Some(Vec::new());
        self.notifications =
            notifications::Notifications::with_settings(model::NotificationSettings {
                level: model::NotificationLevel::All,
                mobile: "whenInactive".into(),
                overrides: Vec::new(),
            });
        self.live = "Live".into();
        // Fixtures never contact a media service; voice reads as enabled.
        for root in ["general", "chan00000001", "chan00000002", "chan00000003"] {
            self.media_availability.insert(root.into(), true);
        }
    }

    /// Voice actions use their own channel's media root, independently of text.
    fn media_root(&self, channel: &str) -> (String, Option<String>) {
        match &self.detail {
            Some(detail) if !detail.space.demo => (channel.into(), Some(channel.into())),
            _ => ("general".into(), None),
        }
    }

    /// Query each listed channel once per account/access epoch, not per frame.
    fn refresh_media_status(&mut self) {
        if self.selected_direct.is_some() || !self.persist_preferences {
            return;
        }
        let channels: Vec<_> = self.detail.as_ref().map_or_else(Vec::new, |detail| {
            detail
                .channels
                .iter()
                .filter(|channel| channel.joined)
                .map(|channel| channel.id.clone())
                .collect()
        });
        for id in channels {
            let (root, channel) = self.media_root(&id);
            if self.media_status_roots.insert(root.clone()) {
                self.worker.send(Command::MediaStatus {
                    generation: self.navigation_cache_generation,
                    root,
                    token: channel.as_ref().and(self.token.clone()),
                    channel,
                });
            }
        }
    }

    fn join_unavailable(&self, channel: &str) -> Option<&'static str> {
        match self.media_availability.get(&self.media_root(channel).0) {
            Some(true) => None,
            Some(false) => Some("Joining is not available at this time."),
            None => Some("Checking voice availability…"),
        }
    }

    /// Web plays channel-leave when someone else leaves and channel-join when
    /// someone else arrives, comparing rosters within one call only.
    fn chime_roster_changes(&mut self) {
        if !matches!(self.voice.state.phase, Phase::Connected(_)) || self.voice.self_id.is_empty() {
            self.voice_roster = None;
            return;
        }
        let generation = self.voice.state.generation;
        let self_id = self.voice.self_id.clone();
        let others: BTreeSet<String> = self
            .voice
            .participants
            .iter()
            .map(|participant| participant.id.clone())
            .filter(|id| *id != self_id)
            .collect();
        if let Some((previous_generation, previous_self, previous)) = &self.voice_roster
            && *previous_generation == generation
            && *previous_self == self_id
        {
            if previous.iter().any(|id| !others.contains(id)) {
                self.effects.play(Effect::Leave);
            } else if others.iter().any(|id| !previous.contains(id)) {
                self.effects.play(Effect::Join);
            }
        }
        self.voice_roster = Some((generation, self_id, others));
    }

    fn receive(&mut self) {
        self.voice.receive();
        if self
            .pending_voice_join
            .as_ref()
            .is_some_and(|(_, generation, _)| *generation != self.voice.state.generation)
        {
            self.pending_voice_join = None;
        }
        if matches!(self.voice.state.phase, Phase::Connected(_))
            && self.announced_voice != Some(self.voice.state.generation)
        {
            self.announced_voice = Some(self.voice.state.generation);
            self.effects.play(Effect::Join);
        }
        self.chime_roster_changes();
        let events: Vec<_> = self.worker.events.try_iter().collect();
        for event in events {
            match event {
                Event::DirectsLoaded {
                    generation,
                    result: Ok(directs),
                } if generation == self.generation => {
                    self.directs = directs;
                }
                // A failed refresh keeps the previous list (or the peer fallback).
                Event::PeopleLoaded {
                    generation,
                    result: Ok(people),
                } if generation == self.generation => {
                    self.people = Some(people);
                }
                Event::DirectCreated {
                    generation,
                    navigation,
                    result,
                } if generation == self.generation => {
                    self.loading = false;
                    // A mention card's Message keeps its own pending state and
                    // error; opening the DM closes the card with the old channel.
                    let card = self
                        .mention_card
                        .as_mut()
                        .filter(|card| card.opening == Some(navigation));
                    let from_card = card.is_some();
                    if let Some(card) = card {
                        card.opening = None;
                        card.error = result.as_ref().err().cloned();
                    }
                    match result {
                        Ok(direct) => {
                            self.directs.retain(|item| item.id != direct.id);
                            self.directs.push(direct.clone());
                            if navigation == self.navigation {
                                self.dialog = None;
                                self.select_direct(direct);
                            }
                        }
                        Err(error) if navigation == self.navigation && !from_card => {
                            self.error = Some(error)
                        }
                        Err(_) => {}
                    }
                }
                Event::Restored { generation, result } if generation == self.generation => {
                    self.loading = false;
                    match result {
                        Ok(Some((token, account, spaces))) => {
                            self.establish(token, account, spaces)
                        }
                        Ok(None) => {}
                        Err(error) => {
                            self.warning = Some(error);
                        }
                    }
                }
                Event::CodeRequested { generation, result } if generation == self.generation => {
                    self.loading = false;
                    match result {
                        Ok(challenge) => {
                            self.challenge = Some(challenge);
                            self.code.clear();
                            self.attempts_remaining = None;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::Verified {
                    generation,
                    result,
                    attempts_remaining,
                } if generation == self.generation => {
                    self.loading = false;
                    self.attempts_remaining = attempts_remaining;
                    match result {
                        Ok((token, account, spaces)) => {
                            self.establish(token.clone(), account, spaces);
                            self.worker.send(Command::PersistCredential {
                                generation: self.generation,
                                token,
                            });
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::Profiled { generation, result } if generation == self.generation => {
                    self.loading = false;
                    match result {
                        Ok((account, spaces)) => self.profiled(account, spaces),
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::NavigationPrepared {
                    generation,
                    navigation,
                    result,
                } => self.accept_navigation(generation, navigation, result),
                Event::NavigationPrefetched {
                    generation,
                    request,
                    space,
                    channel,
                    result,
                } if generation == self.navigation_cache_generation => {
                    let target = navigation::Target { space, channel };
                    match result {
                        Ok(read) => {
                            self.navigation_cache.finish_prefetch(
                                &target,
                                request,
                                read,
                                Instant::now(),
                            );
                        }
                        Err(error) if error.access_denied => {
                            self.navigation_cache
                                .cancel_prefetch(&target, request, Instant::now());
                            if error.space_access_denied {
                                if let Some(space) = &target.space {
                                    self.navigation_cache.forget_space(space);
                                }
                            } else if let Some(channel) = &target.channel {
                                self.navigation_cache.forget_channel(channel);
                            }
                        }
                        Err(_) => {
                            self.navigation_cache
                                .cancel_prefetch(&target, request, Instant::now());
                        }
                    }
                }
                Event::ChannelLoaded {
                    generation,
                    channel,
                    general,
                    result,
                } if generation == self.generation
                    && (general || self.selected_channel.as_deref() == Some(&channel)) =>
                {
                    self.loading = false;
                    match result {
                        Ok((history, session)) => {
                            self.accept_channel(history, session, general, &channel)
                        }
                        Err(error) if error.access_denied => self.clear_channel(&error.message),
                        Err(error) => self.load_error = Some(error.message),
                    }
                }
                Event::SessionCreated { generation, result } if generation == self.generation => {
                    match result {
                        Ok(session) => {
                            self.session = Some(session);
                            self.session_error = None;
                        }
                        Err(error) => self.session_error = Some(error),
                    }
                }
                Event::OlderLoaded {
                    generation,
                    channel,
                    result,
                } if generation == self.generation
                    && self.selected_channel.as_deref() == Some(&channel) =>
                {
                    self.loading_older = false;
                    self.accept_older(&channel, result);
                }
                Event::ThreadLoaded {
                    generation,
                    request,
                    channel,
                    root,
                    result,
                } if generation == self.generation
                    && request == self.thread_request
                    && self.selected_channel.as_deref() == Some(&channel)
                    && self
                        .thread_view
                        .as_ref()
                        .is_some_and(|thread| thread.root == root) =>
                {
                    match result {
                        Ok(page)
                            if page.root.id == root
                                && page.root.channel_id == channel
                                && page.root.thread_root_id.is_none()
                                && page.root.validate().is_ok()
                                && page.messages.iter().all(|message| {
                                    message.channel_id == channel
                                        && message.thread_root_id.as_deref() == Some(&root)
                                        && message.validate().is_ok()
                                }) =>
                        {
                            let before = page.messages.first().map(|message| message.seq.clone());
                            let loaded: BTreeSet<_> = self
                                .timeline
                                .messages()
                                .map(|message| message.id.clone())
                                .collect();
                            let rows: Vec<_> =
                                std::iter::once(page.root).chain(page.messages).collect();
                            self.thread_only_rows.extend(
                                rows.iter()
                                    .filter(|message| {
                                        message.is_channel_message()
                                            && !loaded.contains(&message.id)
                                    })
                                    .map(|message| message.id.clone()),
                            );
                            if let Err(error) = self.timeline.prepend(rows) {
                                self.error = Some(error);
                            }
                            if let Some(thread) = &mut self.thread_view {
                                thread.loading = false;
                                thread.has_more = page.has_more;
                                thread.before = before.or(thread.before.take());
                            }
                        }
                        Err(error) if error.access_denied => {
                            self.thread_view = None;
                            self.reload_channel();
                        }
                        result => {
                            if let Some(thread) = &mut self.thread_view {
                                thread.loading = false;
                                thread.error =
                                    Some(result.err().map(|error| error.message).unwrap_or_else(
                                        || "Caper returned an invalid thread.".into(),
                                    ));
                            }
                        }
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
                    self.generation,
                    Some(&channel),
                    self.selected_channel.as_deref(),
                ) =>
                {
                    self.accept_edit(request, &message, reloaded, result);
                }
                Event::MessageVersions {
                    generation,
                    request,
                    channel,
                    message,
                    result,
                } if current(
                    generation,
                    self.generation,
                    Some(&channel),
                    self.selected_channel.as_deref(),
                ) =>
                {
                    self.accept_versions(request, &message, result);
                }
                Event::MediaStatus {
                    generation,
                    root,
                    enabled,
                } if generation == self.navigation_cache_generation => {
                    self.media_availability.insert(root, enabled);
                }
                Event::VoiceChecked {
                    request,
                    voice_generation,
                    space,
                    channel,
                    result,
                } => {
                    self.accept_voice_target(request, voice_generation, &space, &channel, result);
                }
                Event::Sent {
                    generation,
                    channel,
                    client_id,
                    result,
                } if current(
                    generation,
                    self.generation,
                    Some(&channel),
                    self.selected_channel.as_deref(),
                ) && self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.id == client_id) =>
                {
                    self.sent(result)
                }
                Event::Forward {
                    generation,
                    request,
                    result,
                } if generation == self.generation => {
                    if let Some(message) = self.forwarding.receive(request, result)
                        && self.selected_channel.as_deref() == Some(&message.channel_id)
                    {
                        let _ = self.timeline.merge_sent(message);
                    }
                }
                Event::Reacted {
                    generation,
                    channel,
                    message,
                    emoji,
                    active,
                    result,
                } if current(
                    generation,
                    self.generation,
                    Some(&channel),
                    self.selected_channel.as_deref(),
                ) =>
                {
                    let key = (message.clone(), emoji.clone());
                    match result {
                        Ok(update)
                            if update.channel_id == channel && update.message_id == message =>
                        {
                            if self.timeline.merge_reaction_ack(update).is_err() {
                                self.reload_channel();
                            }
                            if self
                                .pending_reactions
                                .get(&key)
                                .is_some_and(|pending| pending.desired == active)
                            {
                                self.pending_reactions.remove(&key);
                            } else if let Some(pending) = self.pending_reactions.get_mut(&key) {
                                pending.sent = false;
                            }
                            self.send_next_reaction(&message);
                        }
                        Ok(_) => self.reload_channel(),
                        Err(error) if matches!(error.status, Some(401 | 403 | 404)) => {
                            self.pending_reactions.remove(&key);
                            self.clear_channel(&error.message);
                        }
                        Err(error) => {
                            if let Some(pending) = self.pending_reactions.get_mut(&key) {
                                pending.sent = false;
                                if !pending.superseded {
                                    pending.visible = false;
                                    self.reaction_errors.insert(message.clone(), error.message);
                                }
                            }
                            self.send_next_reaction(&message);
                        }
                    }
                }
                Event::Reactors {
                    generation,
                    channel,
                    message,
                    revision,
                    result,
                } if current(
                    generation,
                    self.generation,
                    Some(&channel),
                    self.selected_channel.as_deref(),
                ) =>
                {
                    // A newer revision may have started its own request since.
                    if let Some(cached) = self.reactors.get_mut(&message).filter(|cached| {
                        cached.revision == revision
                            && matches!(cached.state, ReactorState::Loading(_))
                    }) {
                        cached.state = match result {
                            Ok(list) if list.message_id == message => {
                                ReactorState::Loaded(list.reactions)
                            }
                            _ => ReactorState::Failed(Instant::now()),
                        };
                    }
                }
                Event::Pinned {
                    generation,
                    channel,
                    message,
                    active,
                    result,
                } if current(
                    generation,
                    self.generation,
                    Some(&channel),
                    self.selected_channel.as_deref(),
                ) =>
                {
                    self.pending_pins.remove(&message);
                    self.mutations.pins.remove(&message);
                    match result {
                        Ok(update)
                            if update.channel_id == channel && update.message.id == message =>
                        {
                            if self.timeline.merge_pin_ack(update).is_err() {
                                self.reload_channel();
                            }
                            self.pin_errors.remove(&message);
                        }
                        Ok(_) => self.reload_channel(),
                        Err(error) if matches!(error.status, Some(401 | 403 | 404)) => {
                            self.clear_channel(&error.message)
                        }
                        Err(error) => {
                            self.pin_errors.insert(message, (active, error.message));
                        }
                    }
                }
                Event::Admin { generation, result } if generation == self.generation => {
                    self.loading = false;
                    match result {
                        Ok(result) => self.admin_result(result),
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::Credential {
                    generation,
                    result: Err(error),
                } if generation == self.generation => self.warning = Some(error),
                Event::LoggedOut {
                    generation,
                    result: Err(error),
                } if generation == self.generation => {
                    self.warning = Some(format!(
                        "Signed out locally; server revocation failed: {error}"
                    ))
                }
                Event::Gateway(event) => self.gateway(event),
                Event::Account { epoch, result } if epoch == self.account_epoch => {
                    self.account_result(result);
                }
                _ => {}
            }
        }
    }

    fn establish(&mut self, token: String, account: Account, spaces: Spaces) {
        self.invalidate_navigation_cache();
        self.reset_account_state();
        self.people = None;
        self.detail = None;
        self.selected_space = None;
        self.clear_channel_state();
        self.token = Some(token);
        self.username = account.username.clone().unwrap_or_default();
        self.display_name = account.display_name.clone().unwrap_or_default();
        let needs_profile = account.username.is_none() || account.display_name.is_none();
        self.account = Some(account);
        self.set_spaces(spaces);
        self.refresh_directs();
        self.account_op(AccountOperation::LoadBlocks);
        self.load_notifications();
        self.error = None;
        self.dialog = None;
        if needs_profile {
            self.dialog = Some(Dialog::Profile);
        } else if let Some(space) = self.spaces.first() {
            self.select_space(space.id.clone());
        } else {
            self.detail = None;
            self.selected_space = None;
            self.clear_channel_state();
        }
    }

    fn profiled(&mut self, account: Account, spaces: Spaces) {
        self.invalidate_navigation_cache();
        self.account = Some(account);
        self.set_spaces(spaces);
        self.refresh_directs();
        self.dialog = None;
        if let Some(space) = self.spaces.first() {
            self.select_space(space.id.clone());
        } else {
            self.detail = None;
            self.selected_space = None;
            self.clear_channel_state();
        }
    }

    fn set_spaces(&mut self, spaces: Spaces) {
        self.spaces = spaces.spaces;
        self.invitations = spaces.invitations;
        self.limits = spaces.limits;
    }

    fn refresh_directs(&mut self) {
        if let Some(token) = self.token.clone() {
            self.directs_refreshed = Instant::now();
            self.worker.send(Command::LoadDirects {
                generation: self.generation,
                token,
            });
        }
    }

    /// Sends a request, block, privacy or notification operation for the
    /// signed-in account.
    fn account_op(&mut self, operation: AccountOperation) {
        if let Some(token) = self.token.clone() {
            self.worker.send(Command::Account {
                epoch: self.account_epoch,
                token,
                operation,
            });
        }
    }

    fn reset_account_state(&mut self) {
        self.account_epoch += 1;
        self.blocks = None;
        self.revealed_blocked.clear();
        self.requests_open = false;
        self.request_busy = false;
        self.request_error = None;
        self.block_busy = false;
        self.block_error = None;
        self.privacy = None;
        self.privacy_saving = false;
        self.privacy_error = None;
        self.notifications = notifications::Notifications::default();
    }

    fn is_blocked(&self, account: &str) -> bool {
        self.blocks
            .as_ref()
            .is_some_and(|blocks| blocks.iter().any(|blocked| blocked.id == account))
    }

    fn blocked_ids(&self) -> BTreeSet<String> {
        self.blocks
            .iter()
            .flatten()
            .map(|blocked| blocked.id.clone())
            .collect()
    }

    fn selected_direct_conversation(&self) -> Option<&model::DirectConversation> {
        let id = self.selected_direct.as_ref()?;
        self.directs.iter().find(|direct| &direct.id == id)
    }

    /// The open conversation when it is an incoming message request.
    fn selected_request(&self) -> Option<&model::DirectConversation> {
        self.selected_direct_conversation()
            .filter(|direct| direct.status == model::DirectStatus::Incoming)
    }

    fn incoming_requests(&self) -> Vec<model::DirectConversation> {
        self.directs
            .iter()
            .filter(|direct| direct.status == model::DirectStatus::Incoming)
            .cloned()
            .collect()
    }

    /// Someone else's signed-in account behind a message, for Block/Unblock.
    fn blockable_author(&self, author: &Author) -> Option<model::BlockedAccount> {
        let me = self.account.as_ref().map(|account| account.id.as_str());
        if author.is_guest || Some(author.id.as_str()) == me {
            return None;
        }
        let username = self
            .detail
            .iter()
            .flat_map(|detail| &detail.members)
            .find(|member| member.id == author.id)
            .map(|member| member.username.clone())
            .or_else(|| {
                self.directs
                    .iter()
                    .find(|direct| direct.peer.id == author.id)
                    .map(|direct| direct.peer.username.clone())
            })
            .unwrap_or_default();
        Some(model::BlockedAccount {
            id: author.id.clone(),
            username,
            display_name: author.name.clone(),
            avatar_id: author.avatar_id,
        })
    }

    fn peer_account(peer: &model::DirectPeer) -> model::BlockedAccount {
        model::BlockedAccount {
            id: peer.id.clone(),
            username: peer.username.clone(),
            display_name: peer.display_name.clone(),
            avatar_id: peer.avatar_id,
        }
    }

    /// Blocking always confirms first; unblocking does not.
    fn confirm_block(&mut self, account: model::BlockedAccount, request: Option<String>) {
        self.block_error = None;
        self.dialog = Some(Dialog::Block { account, request });
    }

    fn unblock(&mut self, account: model::BlockedAccount) {
        self.block_busy = true;
        self.block_error = None;
        self.account_op(AccountOperation::SetBlock {
            account,
            blocked: false,
            request: None,
        });
    }

    /// Drops a declined or blocked request. If it was open, show the next
    /// request, else the first space (or nothing).
    fn leave_request(&mut self, id: &str) {
        self.directs.retain(|direct| direct.id != id);
        if self.selected_direct.as_deref() != Some(id) {
            return;
        }
        if let Some(next) = self.incoming_requests().into_iter().next() {
            self.requests_open = true;
            self.select_direct(next);
        } else if let Some(space) = self.spaces.first().map(|space| space.id.clone()) {
            self.selected_direct = None;
            self.select_space(space);
        } else {
            self.selected_direct = None;
            self.clear_channel_state();
        }
    }

    fn account_result(&mut self, result: AccountResult) {
        match result {
            AccountResult::Blocks(Ok(blocks)) => self.blocks = Some(blocks),
            AccountResult::Blocks(Err(error)) => self.block_error = Some(error),
            AccountResult::Block {
                account,
                blocked,
                request,
                result,
            } => {
                self.block_busy = false;
                self.request_busy = false;
                if let Err(error) = result {
                    self.block_error = Some(error);
                    return;
                }
                self.block_error = None;
                let blocks = self.blocks.get_or_insert_with(Vec::new);
                blocks.retain(|entry| entry.id != account.id);
                if blocked {
                    blocks.insert(0, account.clone());
                }
                for direct in &mut self.directs {
                    if direct.peer.id == account.id {
                        direct.blocked = blocked;
                    }
                }
                if matches!(self.dialog, Some(Dialog::Block { .. })) {
                    self.dialog = None;
                }
                if blocked {
                    self.typers.remove(&account.id);
                    // Blocking also declines their pending request.
                    let requests: Vec<_> = self
                        .directs
                        .iter()
                        .filter(|direct| {
                            direct.peer.id == account.id
                                && direct.status == model::DirectStatus::Incoming
                        })
                        .map(|direct| direct.id.clone())
                        .chain(request)
                        .collect();
                    for id in requests {
                        self.leave_request(&id);
                    }
                }
            }
            AccountResult::Accepted { id, result } => {
                self.request_busy = false;
                match result {
                    Ok(direct) => {
                        self.request_error = None;
                        match self.directs.iter_mut().find(|item| item.id == id) {
                            Some(existing) => *existing = direct,
                            None => self.directs.push(direct),
                        }
                        if self.incoming_requests().is_empty() {
                            self.requests_open = false;
                        }
                    }
                    Err(error) => self.request_error = Some(error),
                }
            }
            AccountResult::Declined { id, result } => {
                self.request_busy = false;
                match result {
                    Ok(()) => {
                        self.request_error = None;
                        self.leave_request(&id);
                    }
                    Err(error) => self.request_error = Some(error),
                }
            }
            AccountResult::Privacy(result) => match result {
                // A save in flight owns the displayed value.
                Ok(value) if !self.privacy_saving => self.privacy = Some(value),
                Ok(_) => {}
                Err(error) => self.privacy_error = Some(error),
            },
            AccountResult::PrivacySaved { previous, result } => {
                self.privacy_saving = false;
                match result {
                    Ok(value) => {
                        self.privacy = Some(value);
                        self.privacy_error = None;
                    }
                    Err(error) => {
                        self.privacy = previous;
                        self.privacy_error = Some(format!("Could not save: {error}"));
                    }
                }
            }
            AccountResult::Notifications { revision, result } => {
                self.notifications.finish_load(revision, result);
            }
            AccountResult::NotificationsSaved { scope, result } => {
                self.notifications.finish_save(&scope, result);
            }
        }
    }

    /// `GET /api/notifications/settings`, after sign-in and when Settings or
    /// a notification menu opens.
    fn load_notifications(&mut self) {
        if self.token.is_some()
            && let Some(revision) = self.notifications.start_load()
        {
            self.account_op(AccountOperation::LoadNotifications { revision });
        }
    }

    /// Applies a notification choice at once and saves it; a failed save
    /// reverts with an inline error.
    fn change_notifications(&mut self, scope: Scope, change: Change) {
        if self.token.is_some() && self.notifications.begin(&scope, &change) {
            self.account_op(AccountOperation::SaveNotifications { scope, change });
        }
    }

    /// The bell-slash tooltip for a muted space, channel or DM: its own mute,
    /// or "Muted with the space" for a channel in a muted space.
    fn muted_label(&self, scope: &Scope) -> Option<String> {
        let now = chrono::Utc::now();
        match self.notifications.mute(scope, now) {
            Some(mute) => Some(notifications::mute_label(mute, &now.with_timezone(&Local))),
            None => self
                .notifications
                .muted(scope, now)
                .then(|| "Muted with the space".into()),
        }
    }

    /// A failed notification change, inline under its sidebar row.
    fn notification_error(&self, ui: &mut egui::Ui, scope: &Scope) {
        if let Some(error) = self.notifications.error(scope) {
            ui.label(RichText::new(error).size(11.0).color(ERROR));
        }
    }

    /// The Notifications and Mute items of the space, channel and DM menus.
    /// `noun` names the scope: `space`, `channel` or `conversation`.
    fn notification_items(&mut self, ui: &mut egui::Ui, scope: &Scope, noun: &str) {
        let now = chrono::Utc::now();
        let enabled = self.notifications.ready() && !self.notifications.saving(scope);
        let mut change = None;
        ui.add_enabled_ui(enabled, |ui| {
            let level = self.notifications.level(scope);
            if let Scope::Direct(_) = scope {
                let off = level == Some(NotificationLevel::Nothing);
                if ui
                    .button(if off {
                        "Turn on notifications"
                    } else {
                        "Turn off notifications"
                    })
                    .clicked()
                {
                    change = Some(Change::Level((!off).then_some(NotificationLevel::Nothing)));
                    ui.close();
                }
            } else {
                let choices = [
                    None,
                    Some(NotificationLevel::All),
                    Some(NotificationLevel::Mentions),
                    Some(NotificationLevel::Nothing),
                ];
                let inherited = self.notifications.inherited(scope);
                ui.menu_button("Notifications", |ui| {
                    for choice in choices {
                        let label = choice.map_or_else(
                            || notifications::default_label(inherited),
                            |level| notifications::level_label(level).into(),
                        );
                        if ui.selectable_label(level == choice, label).clicked() {
                            change = Some(Change::Level(choice));
                            ui.close();
                        }
                    }
                });
            }
            if let Some(mute) = self.notifications.mute(scope, now) {
                if ui.button(format!("Unmute {noun}")).clicked() {
                    change = Some(Change::Mute(None));
                    ui.close();
                }
                ui.label(
                    RichText::new(notifications::mute_label(mute, &now.with_timezone(&Local)))
                        .size(12.0)
                        .color(MUTED),
                );
            } else {
                ui.menu_button(format!("Mute {noun}"), |ui| {
                    for (label, minutes) in notifications::MUTE_PRESETS {
                        if ui.button(label).clicked() {
                            change = Some(Change::Mute(Some(notifications::mute_value(
                                minutes,
                                chrono::Utc::now(),
                            ))));
                            ui.close();
                        }
                    }
                });
            }
            if let Scope::Channel { space, .. } = scope
                && self
                    .notifications
                    .mute(&Scope::Space(space.clone()), now)
                    .is_some()
            {
                ui.label(
                    RichText::new("Muted with the space")
                        .size(12.0)
                        .color(MUTED),
                );
            }
        });
        if let Some(change) = change {
            self.change_notifications(scope.clone(), change);
        }
        if !self.notifications.ready() {
            match self.notifications.load_error() {
                Some(error) => ui.colored_label(ERROR, error),
                None => ui.label(RichText::new("Loading…").size(12.0).color(MUTED)),
            };
        }
        if let Some(error) = self.notifications.error(scope) {
            ui.colored_label(ERROR, error.to_owned());
        }
    }

    /// The channel options menu: notifications, then settings and leave.
    fn channel_menu(
        &mut self,
        ui: &mut egui::Ui,
        space: &str,
        channel: &str,
        name: &str,
        private: bool,
    ) {
        let scope = Scope::Channel {
            space: space.into(),
            channel: channel.into(),
        };
        self.notification_items(ui, &scope, "channel");
        ui.separator();
        if self.owner() && ui.button("Channel settings").clicked() {
            self.open_manage_channel(channel, name, private);
            ui.close();
        }
        if ui
            .button(RichText::new("Leave channel").color(ERROR))
            .clicked()
        {
            self.dialog = Some(Dialog::LeaveChannel {
                space: space.into(),
                channel: channel.into(),
                name: name.into(),
            });
            ui.close();
        }
    }

    fn save_privacy(&mut self, value: &str) {
        if self.privacy_saving || self.privacy.as_deref() == Some(value) {
            return;
        }
        let previous = self.privacy.replace(value.to_owned());
        self.privacy_saving = true;
        self.privacy_error = None;
        self.account_op(AccountOperation::SavePrivacy {
            value: value.to_owned(),
            previous,
        });
    }

    fn select_direct(&mut self, direct: model::DirectConversation) {
        self.request_busy = false;
        self.request_error = None;
        self.remember_conversation();
        self.selected_direct = Some(direct.id.clone());
        self.reload_selected_channel(direct.id, false);
        // After the reload's generation bump, so the answer is not discarded.
        if let Some(token) = self.token.clone() {
            self.worker.send(Command::LoadPeople {
                generation: self.generation,
                token,
            });
        }
    }

    fn select_or_create_self_direct(&mut self) {
        let Some(target) = self.self_direct_target() else {
            return;
        };
        match target {
            SelfDirectTarget::Existing(direct) => self.select_direct(direct),
            SelfDirectTarget::Create(username) => {
                let Some(token) = self.token.clone() else {
                    return;
                };
                if self.loading {
                    return;
                }
                self.loading = true;
                self.error = None;
                self.navigation += 1;
                self.opening = false;
                self.navigation_target = None;
                self.worker.send(Command::CreateDirect {
                    generation: self.generation,
                    navigation: self.navigation,
                    token,
                    username,
                });
            }
        }
    }

    fn self_direct_target(&self) -> Option<SelfDirectTarget> {
        let account = self.account.as_ref()?;
        if let Some(direct) = self
            .directs
            .iter()
            .find(|direct| direct.peer.id == account.id)
        {
            return Some(SelfDirectTarget::Existing(direct.clone()));
        }
        account.username.clone().map(SelfDirectTarget::Create)
    }

    fn open_manage_space(&mut self) {
        self.form_name = self
            .detail
            .as_ref()
            .map_or_else(String::new, |detail| detail.space.name.clone());
        self.managed_members = self
            .detail
            .as_ref()
            .map_or_else(Vec::new, |detail| detail.members.clone());
        self.managed_channel = None;
        self.managed_invitations.clear();
        self.member_username.clear();
        self.member_error = None;
        if let (Some(token), Some(space)) = (self.token.clone(), self.selected_space.clone()) {
            self.worker.send(Command::Admin {
                generation: self.generation,
                token: token.clone(),
                operation: AdminOperation::LoadMembers {
                    space: space.clone(),
                    channel: None,
                },
            });
            self.worker.send(Command::Admin {
                generation: self.generation,
                token,
                operation: AdminOperation::LoadInvitations { space },
            });
        }
        self.dialog = Some(Dialog::ManageSpace);
    }

    fn open_direct_action(&mut self) {
        if self.owner() {
            self.open_manage_space();
        } else {
            self.member_username.clear();
            self.error = None;
            self.dialog = Some(Dialog::StartDirect);
        }
    }

    fn accept_channel(
        &mut self,
        mut history: model::History,
        session: crate::worker::SessionResult,
        general: bool,
        requested_channel: &str,
    ) {
        if !general && history.channel.id != requested_channel {
            self.clear_channel("Caper returned history for another channel.");
            return;
        }
        if history
            .messages
            .iter()
            .any(|message| message.channel_id != history.channel.id)
        {
            self.clear_channel("Caper returned messages from another channel.");
            return;
        }
        self.older_armed = false;
        self.older_anchor = None;
        self.load_error = None;
        // Every missing event must be a refreshed message. An unaccounted
        // sequence may be a reaction on an older cached row.
        let applied = model::sequence(&self.timeline.cursor()).unwrap_or(0);
        let mut accounted = applied;
        let contiguous = !history.messages.is_empty()
            && history.messages.iter().all(|message| {
                model::sequence(&message.seq).is_ok_and(|next| {
                    if next <= applied {
                        return true;
                    }
                    if next != accounted + 1 {
                        return false;
                    }
                    accounted = next;
                    true
                })
            })
            && model::sequence(&history.cursor) == Ok(accounted);
        let retained_older = contiguous
            && self
                .timeline
                .messages()
                .find(|message| {
                    message.is_channel_message() && !self.thread_only_rows.contains(&message.id)
                })
                .is_some_and(|oldest| {
                    history.messages.first().is_some_and(|first| {
                        model::sequence(&oldest.seq).ok() < model::sequence(&first.seq).ok()
                    })
                });
        // Timeline::merge takes incoming metadata and the higher reaction revision.
        // Keep newer snapshots on overlapping rows even when older pages must reload.
        let fresh_ids: BTreeSet<_> = history.messages.iter().map(|message| &message.id).collect();
        let mut messages: Vec<_> = self
            .timeline
            .messages()
            .filter(|message| {
                (contiguous
                    && message.is_channel_message()
                    && !self.thread_only_rows.contains(&message.id))
                    || fresh_ids.contains(&message.id)
            })
            .cloned()
            .collect();
        self.thread_only_rows.clear();
        messages.extend(std::mem::take(&mut history.messages));
        if let Err(error) = self.timeline.reset(messages, &history.cursor) {
            self.clear_channel(&error);
            return;
        }
        if let Err(error) = self
            .timeline
            .reset_pins(std::mem::take(&mut history.pinned_messages))
        {
            self.clear_channel(&error);
            return;
        }
        if !retained_older {
            self.has_more = history.has_more;
        }
        self.selected_channel = Some(history.channel.id.clone());
        if general {
            let demo = model::Space {
                id: history.space.id.clone(),
                name: history.space.name.clone(),
                owner_id: String::new(),
                inviter: None,
                demo: true,
            };
            self.spaces.retain(|space| !space.demo);
            self.spaces.insert(0, demo.clone());
            self.selected_space = Some(demo.id.clone());
            self.detail = Some(SpaceDetail {
                space: demo,
                channels: vec![model::Channel {
                    id: history.channel.id.clone(),
                    space_id: history.space.id,
                    name: history.channel.name,
                    private: false,
                    joined: true,
                }],
                members: vec![],
                channel_invitations: vec![],
            });
        }
        self.session_error = session.as_ref().err().cloned();
        self.session = session.ok();
        self.live = "Connecting…".into();
        self.connect_gateway();
        self.mark_selected_direct_read();
        if self.thread_view.is_some() {
            self.load_thread(false);
        }
    }

    fn mark_selected_direct_read(&mut self) {
        if !self.foreground || self.selected_channel != self.selected_direct {
            return;
        }
        // Reading a request must not look like engaging with it.
        if self.selected_request().is_some() {
            return;
        }
        if let (Some(id), Some(token)) = (self.selected_direct.clone(), self.token.clone()) {
            let seq = self.timeline.cursor();
            if let Some(direct) = self.directs.iter_mut().find(|item| item.id == id) {
                if model::sequence(&direct.read_seq).unwrap_or(0)
                    >= model::sequence(&seq).unwrap_or(0)
                {
                    return;
                }
                direct.read_seq = seq.clone();
            }
            self.worker.send(Command::ReadDirect { token, id, seq });
        }
    }

    fn selected_is_joined(&self) -> bool {
        if self.selected_direct.is_some() && self.selected_direct == self.selected_channel {
            // An incoming request stays read-only until it is accepted.
            return self.selected_request().is_none();
        }
        self.selected_channel.as_ref().is_some_and(|id| {
            self.detail.as_ref().is_some_and(|detail| {
                detail
                    .channels
                    .iter()
                    .any(|channel| channel.id == *id && channel.joined)
            })
        })
    }

    fn connect_gateway(&mut self) {
        let Some(channel) = self.selected_channel.clone() else {
            return;
        };
        self.roster_generation += 1;
        self.channel_rosters.clear();
        self.voice_session_starts.clear();
        self.unavailable_rosters.clear();
        let media = gateway::MediaWatch {
            epoch: self.roster_generation,
            channels: self.detail.as_ref().map_or_else(Vec::new, |detail| {
                detail
                    .channels
                    .iter()
                    .filter(|channel| self.selected_is_joined() && channel.joined)
                    .take(gateway::MAX_MEDIA_CHANNELS)
                    .map(|channel| gateway::MediaChannel {
                        id: channel.id.clone(),
                        demo: detail.space.demo,
                    })
                    .collect()
            }),
        };
        let presence = if self.token.is_some() {
            self.detail
                .as_ref()
                .map(|detail| {
                    let users = member_page_ids(detail, self.member_page);
                    (detail.space.id.clone(), users)
                })
                .filter(|(_, users): &(String, Vec<String>)| !users.is_empty())
        } else {
            None
        };
        self.worker.send(Command::Connect {
            generation: self.generation,
            token: self.token.clone(),
            channel,
            cursor: self.timeline.cursor(),
            presence,
            media,
        });
    }

    fn select_space(&mut self, id: String) {
        if self.token.is_none() {
            self.dialog = Some(Dialog::SignIn);
            return;
        }
        self.navigate(NavigationTarget {
            space: Some(id),
            channel: None,
        });
    }

    fn navigate(&mut self, target: NavigationTarget) {
        let mut cache_target = navigation::Target {
            space: target.space.clone(),
            channel: target.channel.clone(),
        };
        cache_target = self.navigation_cache.resolve(&cache_target);
        let target = NavigationTarget {
            space: cache_target.space.clone(),
            channel: cache_target.channel.clone(),
        };
        let selected = self.selected_channel.as_ref().is_some_and(|channel| {
            target.channel.as_ref() == Some(channel) && target.space == self.selected_space
        });
        // Joining the displayed preview still needs a participating session.
        let joined_preview = self.selected_is_joined()
            && self.session_error.as_deref() == Some("Join this channel to chat.");
        if selected && !joined_preview {
            // Self-DM creation uses `loading` while the displayed chat stays open.
            if self.opening || self.loading || self.navigation_error.is_some() {
                self.navigation += 1;
                self.opening = false;
                self.navigation_target = None;
                self.navigation_prefetch = None;
                self.navigation_error = None;
            }
            return;
        }
        if self.opening
            && self.navigation_target.as_ref().is_some_and(|pending| {
                pending.space == target.space && pending.channel == target.channel
            })
        {
            return;
        }
        self.remember_conversation();
        self.navigation += 1;
        self.opening = true;
        self.navigation_error = None;
        self.navigation_target = Some(target);
        self.navigation_prefetch = None;
        if self
            .navigation_cache
            .prefetch_state(&cache_target, Instant::now())
            == navigation::PrefetchState::Pending
        {
            self.navigation_prefetch = Some(cache_target);
            return;
        }
        self.prepare_navigation(cache_target);
    }

    fn navigation_target_matches(&self, target: &navigation::Target) -> bool {
        self.opening
            && self.navigation_target.as_ref().is_some_and(|pending| {
                pending.space == target.space && pending.channel == target.channel
            })
    }

    fn prepare_navigation(&mut self, cache_target: navigation::Target) {
        self.navigation_prefetch = None;
        let prefetched = self
            .navigation_cache
            .take_prefetch(&cache_target, Instant::now());
        let cached = self.navigation_cache.history(&cache_target).or_else(|| {
            prefetched.and_then(|read| {
                let _fresh_detail = read.detail;
                read.history
            })
        });
        self.worker.send(Command::PrepareNavigation {
            generation: self.generation,
            navigation: self.navigation,
            token: self.token.clone(),
            space: cache_target.space,
            channel: cache_target.channel,
            name: self.identity_name(),
            cached,
        });
    }

    fn prefetch(&mut self, target: NavigationTarget) {
        let target = self.navigation_cache.resolve(&navigation::Target {
            space: target.space,
            channel: target.channel,
        });
        if self.selected_channel.as_ref() == target.channel.as_ref()
            && self.selected_space == target.space
        {
            return;
        }
        let Some(request) = self
            .navigation_cache
            .begin_prefetch(target.clone(), Instant::now())
        else {
            return;
        };
        self.worker.send(Command::PrefetchNavigation {
            generation: self.navigation_cache_generation,
            request,
            token: self.token.clone(),
            space: target.space,
            channel: target.channel,
        });
    }

    fn remember_conversation(&mut self) {
        if self.session.is_none() {
            return;
        }
        let (Some(channel), Some(detail)) = (&self.selected_channel, &self.detail) else {
            return;
        };
        let Some(item) = detail.channels.iter().find(|item| item.id == *channel) else {
            return;
        };
        self.navigation_cache.remember(
            navigation::Target {
                space: if detail.space.demo {
                    None
                } else {
                    Some(detail.space.id.clone())
                },
                channel: if detail.space.demo {
                    None
                } else {
                    Some(channel.clone())
                },
            },
            model::History {
                messages: self
                    .timeline
                    .messages()
                    .filter(|message| {
                        message.is_channel_message() && !self.thread_only_rows.contains(&message.id)
                    })
                    .cloned()
                    .collect(),
                pinned_messages: self.timeline.pinned_messages().cloned().collect(),
                cursor: self.timeline.cursor(),
                has_more: self.has_more,
                space: model::HistoryPlace {
                    id: detail.space.id.clone(),
                    name: detail.space.name.clone(),
                },
                channel: model::HistoryPlace {
                    id: item.id.clone(),
                    name: item.name.clone(),
                },
            },
        );
    }

    fn invalidate_navigation_cache(&mut self) {
        self.voice_join_request += 1;
        self.pending_voice_join = None;
        self.media_status_roots.clear();
        self.media_availability.clear();
        self.navigation_cache_generation += 1;
        self.navigation_cache.clear();
        self.navigation += 1;
        self.opening = false;
        self.navigation_target = None;
    }

    fn accept_navigation(
        &mut self,
        generation: u64,
        navigation: u64,
        result: Result<worker::PreparedNavigation, worker::LoadError>,
    ) {
        if generation != self.generation || navigation != self.navigation {
            return;
        }
        self.opening = false;
        match result {
            Ok(prepared) => {
                let space_changed = self.selected_space.as_deref()
                    != prepared
                        .detail
                        .as_ref()
                        .map(|detail| detail.space.id.as_str());
                if space_changed {
                    self.voice_join_request += 1;
                    self.presence.clear();
                    self.member_page = 0;
                }
                self.generation += 1;
                self.clear_channel_state();
                self.selected_direct = None;
                self.loading = false;
                self.error = None;
                self.navigation_error = None;
                self.navigation_target = None;
                self.navigation_open = false;
                let general = prepared.detail.is_none();
                self.selected_space = prepared
                    .detail
                    .as_ref()
                    .map(|detail| detail.space.id.clone());
                self.detail = prepared.detail;
                if let Some((history, session)) = prepared.conversation {
                    let channel = history.channel.id.clone();
                    self.voice.state.browse(channel.clone());
                    self.accept_channel(history, session, general, &channel);
                }
            }
            Err(error) => {
                if error.access_denied
                    && let Some(target) = &self.navigation_target
                {
                    if error.space_access_denied {
                        if let Some(space) = &target.space {
                            self.navigation_cache.forget_space(space);
                            if error.message == "This space is no longer available." {
                                self.spaces.retain(|entry| entry.id != *space);
                            }
                        }
                    } else if let Some(channel) = &target.channel {
                        self.navigation_cache.forget_channel(channel);
                    }
                }
                if error.access_denied
                    && self.navigation_target.as_ref().is_some_and(|target| {
                        target.space == self.selected_space
                            && (error.space_access_denied
                                || target.channel.is_none()
                                || target.channel == self.selected_channel)
                    })
                {
                    if error.space_access_denied {
                        if let Some(space) = &self.selected_space {
                            self.voice.revoke_space(space);
                        }
                        self.selected_space = None;
                        self.detail = None;
                        self.presence.clear();
                    }
                    self.clear_channel(&error.message);
                }
                self.navigation_error = Some(error.message);
            }
        }
    }

    fn select_channel(&mut self, id: String, general: bool) {
        self.navigate(NavigationTarget {
            space: if general {
                None
            } else {
                self.selected_space.clone()
            },
            channel: Some(id),
        });
    }

    fn reload_selected_channel(&mut self, id: String, general: bool) {
        self.navigation += 1;
        self.opening = false;
        self.navigation_target = None;
        self.navigation_error = None;
        self.voice.state.browse(id.clone());
        self.generation += 1;
        self.selected_channel = Some(id.clone());
        self.session = None;
        self.session_error = None;
        self.loading_older = false;
        self.older_error = None;
        self.has_more = false;
        self.timeline = Timeline::default();
        self.thread_view = None;
        self.thread_request += 1;
        self.thread_drafts.clear();
        self.thread_only_rows.clear();
        self.older_armed = false;
        self.older_anchor = None;
        self.load_error = None;
        self.pending = None;
        self.mention_card = None;
        self.reaction_picker = None;
        self.pending_reactions.clear();
        self.reaction_errors.clear();
        self.reactors.clear();
        self.pending_pins.clear();
        self.mutations = model::MessageMutations::default();
        self.draft.clear();
        self.typers.clear();
        self.error = None;
        self.loading = true;
        self.navigation_open = false;
        self.worker.send(Command::StopGateway);
        self.worker.send(Command::LoadChannel {
            generation: self.generation,
            token: self.token.clone(),
            channel: id,
            name: self.identity_name(),
            general,
        });
    }

    fn identity_name(&self) -> String {
        self.account
            .as_ref()
            .and_then(|account| account.display_name.clone())
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| "Guest".into())
    }

    fn voice_target(&self, channel: &str) -> Option<(CallContext, Option<String>)> {
        let detail = self.detail.as_ref()?;
        let target = detail.channels.iter().find(|item| item.id == channel)?;
        if !target.joined
            || self.unavailable_rosters.contains(channel)
            || (!detail.space.demo && self.token.is_none())
            || self.join_unavailable(channel).is_some()
        {
            return None;
        }
        let context = CallContext {
            channel_id: target.id.clone(),
            channel_name: target.name.clone(),
            space_name: detail.space.name.clone(),
        };
        Some((
            context,
            (!detail.space.demo).then(|| detail.space.id.clone()),
        ))
    }

    /// Web prepares a signed-in member's join when the pointer nears Join:
    /// the API keeps the session 8 s, so reissue at most every 4 s. The public
    /// demo creates on join.
    fn prepare_voice_join(&mut self, channel: &str) {
        let Some((_, Some(_))) = self.voice_target(channel) else {
            return;
        };
        let Some(token) = self.token.clone() else {
            return;
        };
        if !self.persist_preferences
            || self.voice.state.active_channel() == Some(channel)
            || self
                .prepared_voice
                .get(channel)
                .is_some_and(|sent| sent.elapsed() < Duration::from_secs(4))
        {
            return;
        }
        self.prepared_voice.insert(channel.into(), Instant::now());
        self.worker.send(Command::PrepareVoice {
            token,
            channel: channel.into(),
        });
    }

    fn join_voice_channel(&mut self, channel: &str) {
        if self
            .pending_voice_join
            .as_ref()
            .is_some_and(|(_, generation, _)| *generation == self.voice.state.generation)
            || matches!(
                self.voice.state.phase,
                Phase::Joining(_) | Phase::Reconnecting(_)
            )
            || self.voice.state.active_channel() == Some(channel)
                && !matches!(self.voice.state.phase, Phase::Failed(_))
        {
            return;
        }
        let Some((context, space)) = self.voice_target(channel) else {
            return;
        };
        let clicked = chrono::Utc::now().timestamp_millis().max(0) as u64;
        self.voice_join_request += 1;
        if let Some(space) = space {
            self.pending_voice_join = Some((channel.into(), self.voice.state.generation, clicked));
            self.worker.send(Command::CheckVoice {
                request: self.voice_join_request,
                voice_generation: self.voice.state.generation,
                token: self.token.clone().unwrap_or_default(),
                space,
                channel: channel.into(),
            });
        } else {
            self.voice.join(
                context,
                None,
                self.token.clone(),
                self.identity_name(),
                clicked,
                self.voice_session_starts.get(channel).copied(),
            );
        }
    }

    fn accept_voice_target(
        &mut self,
        request: u64,
        voice_generation: u64,
        space: &str,
        channel: &str,
        result: Result<(), worker::LoadError>,
    ) {
        if request != self.voice_join_request {
            return;
        }
        let Some((_, _, clicked)) = self.pending_voice_join.take() else {
            return;
        };
        if voice_generation != self.voice.state.generation {
            return;
        }
        let Some((context, target_space)) = self.voice_target(channel) else {
            return;
        };
        if target_space.as_deref() != Some(space) {
            return;
        }
        match result {
            Ok(()) => self.voice.join(
                context,
                target_space,
                self.token.clone(),
                self.identity_name(),
                clicked,
                self.voice_session_starts.get(channel).copied(),
            ),
            Err(error) => {
                if error.access_denied {
                    self.unavailable_rosters.insert(channel.into());
                    self.channel_rosters.remove(channel);
                    self.voice_session_starts.remove(channel);
                }
                self.voice.error = Some(error.message);
            }
        }
    }

    fn gateway(&mut self, event: GatewayEvent) {
        match event {
            GatewayEvent::VoiceRoster {
                generation,
                channel,
                participants,
                session_started_at,
            } if generation == self.roster_generation
                && self.selected_channel.is_some()
                && self.detail.as_ref().is_some_and(|detail| {
                    detail.channels.iter().any(|item| item.id == channel)
                }) =>
            {
                self.unavailable_rosters.remove(&channel);
                if let Some(started) = session_started_at.filter(|_| !participants.is_empty()) {
                    self.voice_session_starts.insert(channel.clone(), started);
                } else {
                    self.voice_session_starts.remove(&channel);
                }
                self.channel_rosters.insert(channel, participants);
            }
            GatewayEvent::VoiceUnavailable {
                generation,
                channel,
                revoked,
            } if generation == self.roster_generation => {
                self.channel_rosters.remove(&channel);
                self.voice_session_starts.remove(&channel);
                self.unavailable_rosters.insert(channel.clone());
                if revoked {
                    self.voice.revoke_channel(&channel);
                }
            }
            GatewayEvent::VoiceReset { generation } if generation == self.roster_generation => {
                self.channel_rosters.clear();
                self.voice_session_starts.clear();
            }
            GatewayEvent::Status {
                generation,
                channel,
                online,
                detail,
            } if current(
                generation,
                self.generation,
                Some(&channel),
                self.selected_channel.as_deref(),
            ) =>
            {
                self.live = if online { "Live".into() } else { detail };
                if !online {
                    self.typers.clear();
                }
            }
            GatewayEvent::Message {
                generation,
                channel,
                message,
            } if current(
                generation,
                self.generation,
                Some(&channel),
                self.selected_channel.as_deref(),
            ) =>
            {
                if self.pending.as_ref().is_some_and(|pending| {
                    self.session
                        .as_ref()
                        .is_some_and(|session| pending.confirmed_by(&message, &session.author.id))
                }) {
                    self.pending = None;
                    self.error = None;
                }
                // Requests and blocked authors never play message sounds.
                let remote = self
                    .session
                    .as_ref()
                    .is_some_and(|session| session.author.id != message.author.id)
                    && !self.is_blocked(&message.author.id)
                    && self.selected_request().is_none();
                let author_id = message.author.id.clone();
                match self.timeline.apply(*message) {
                    Ok(model::Apply::Applied) if remote => {
                        if let Some(typer) = self.typers.get_mut(&author_id) {
                            typer.typing = false;
                        }
                        self.effects.play(Effect::Message);
                        self.mark_selected_direct_read();
                    }
                    Ok(model::Apply::Applied | model::Apply::Buffered) => {
                        if let Some(typer) = self.typers.get_mut(&author_id) {
                            typer.typing = false;
                        }
                    }
                    Ok(model::Apply::Resync) | Err(_) => self.reload_channel(),
                    _ => {}
                }
            }
            GatewayEvent::Reactions {
                generation,
                channel,
                update,
            } if current(
                generation,
                self.generation,
                Some(&channel),
                self.selected_channel.as_deref(),
            ) =>
            {
                match self.timeline.apply_reactions(update) {
                    Ok(model::Apply::Applied) => self.mark_selected_direct_read(),
                    Ok(model::Apply::Resync) | Err(_) => self.reload_channel(),
                    _ => {}
                }
            }
            GatewayEvent::Pin {
                generation,
                channel,
                update,
            } if current(
                generation,
                self.generation,
                Some(&channel),
                self.selected_channel.as_deref(),
            ) =>
            {
                self.pin_errors.remove(&update.message.id);
                match self.timeline.apply_pin(*update) {
                    Ok(model::Apply::Applied) => self.mark_selected_direct_read(),
                    Ok(model::Apply::Resync) | Err(_) => self.reload_channel(),
                    _ => {}
                }
            }
            GatewayEvent::Forward {
                generation,
                channel,
                update,
            } if current(
                generation,
                self.generation,
                Some(&channel),
                self.selected_channel.as_deref(),
            ) =>
            {
                match self.timeline.apply_forward(*update) {
                    Ok(model::Apply::Applied) => self.mark_selected_direct_read(),
                    Ok(model::Apply::Resync) | Err(_) => self.reload_channel(),
                    _ => {}
                }
            }
            GatewayEvent::Edit {
                generation,
                channel,
                update,
            } if current(
                generation,
                self.generation,
                Some(&channel),
                self.selected_channel.as_deref(),
            ) =>
            {
                match self.timeline.apply_edit(*update) {
                    Ok(model::Apply::Applied) => self.mark_selected_direct_read(),
                    Ok(model::Apply::Resync) | Err(_) => self.reload_channel(),
                    _ => {}
                }
            }
            GatewayEvent::Typing {
                generation,
                channel,
                author,
                typing,
                revision,
            } if current(
                generation,
                self.generation,
                Some(&channel),
                self.selected_channel.as_deref(),
            ) =>
            {
                let revision = model::sequence(&revision).unwrap_or(0);
                if author.id
                    != self
                        .session
                        .as_ref()
                        .map_or("", |session| session.author.id.as_str())
                    && !self.is_blocked(&author.id)
                {
                    let replace = self
                        .typers
                        .get(&author.id)
                        .is_none_or(|entry| revision > entry.revision);
                    if replace && (self.typers.contains_key(&author.id) || self.typers.len() < 64) {
                        self.typers.insert(
                            author.id.clone(),
                            Typer {
                                author,
                                typing,
                                revision,
                                expires: Instant::now() + Duration::from_secs(6),
                            },
                        );
                    }
                }
            }
            GatewayEvent::Presence {
                generation,
                space,
                members,
            } if generation == self.generation
                && self.selected_space.as_deref() == Some(&space) =>
            {
                for Presence { user_id, status } in members {
                    self.presence.insert(user_id, status);
                }
            }
            GatewayEvent::Resync {
                generation,
                channel,
            } if current(
                generation,
                self.generation,
                Some(&channel),
                self.selected_channel.as_deref(),
            ) =>
            {
                self.reload_channel()
            }
            GatewayEvent::AccessDenied {
                generation,
                channel,
                detail,
            } if current(
                generation,
                self.generation,
                Some(&channel),
                self.selected_channel.as_deref(),
            ) =>
            {
                self.clear_channel(&detail)
            }
            _ => {}
        }
    }

    fn sent(&mut self, result: Result<model::Message, worker::SendFailure>) {
        match result {
            Ok(message) => {
                let confirmed = self.selected_channel.as_deref() == Some(&message.channel_id)
                    && self.pending.as_ref().is_some_and(|pending| {
                        self.session.as_ref().is_some_and(|session| {
                            pending.confirmed_by(&message, &session.author.id)
                        })
                    });
                if confirmed && self.timeline.merge_sent(message).is_ok() {
                    self.pending = None;
                } else {
                    if let Some(pending) = &mut self.pending {
                        pending.sending = false;
                    }
                    self.error = Some("Caper returned an invalid send confirmation. Retry keeps the same message ID.".into());
                }
            }
            // A block (either way) is a definitive rejection, not an expired session.
            Err(error)
                if matches!(
                    error.code.as_deref(),
                    Some("dm_blocked" | "dm_not_accepted")
                ) =>
            {
                if let Some(pending) = &mut self.pending {
                    pending.sending = false;
                    pending.rejection = Some(error.message);
                }
                self.error = None;
                if error.code.as_deref() == Some("dm_blocked") {
                    self.account_op(AccountOperation::LoadBlocks);
                    self.refresh_directs();
                }
            }
            Err(error) if matches!(error.status, Some(401 | 403)) => self
                .clear_channel("Your messaging session expired. Select the channel to reconnect."),
            Err(error) if permanent_send_rejection(error.status) => {
                if let Some(pending) = &mut self.pending {
                    pending.sending = false;
                    pending.rejection = Some(error.message);
                }
                self.error = None;
            }
            Err(error) => {
                if let Some(pending) = &mut self.pending {
                    pending.sending = false;
                }
                self.error = Some(format!(
                    "{} Retry sends the same message ID.",
                    error.message
                ));
            }
        }
    }

    fn reload_channel(&mut self) {
        if let Some(channel) = self.selected_channel.clone() {
            let general = self.detail.as_ref().is_some_and(|detail| detail.space.demo);
            let mut pending = self.pending.take();
            let draft = std::mem::take(&mut self.draft);
            let timeline = std::mem::take(&mut self.timeline);
            let thread = self.thread_view.take();
            let drafts = std::mem::take(&mut self.thread_drafts);
            let rows = std::mem::take(&mut self.thread_only_rows);
            let has_more = self.has_more;
            if let Some(pending) = &mut pending {
                pending.sending = false;
            }
            self.reload_selected_channel(channel, general);
            self.pending = pending;
            self.draft = draft;
            self.timeline = timeline;
            self.thread_view = thread;
            self.thread_drafts = drafts;
            self.thread_only_rows = rows;
            self.has_more = has_more;
        }
    }

    fn discard_rejected(&mut self) -> Option<String> {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| !pending.sending && pending.rejection.is_some())
        {
            self.pending.take().map(|pending| pending.text)
        } else {
            None
        }
    }

    fn accept_older(
        &mut self,
        requested_channel: &str,
        result: Result<model::History, worker::LoadError>,
    ) {
        self.older_error = None;
        match result {
            Ok(history) if history.channel.id != requested_channel => {
                self.clear_channel("Caper returned history for another channel.")
            }
            Ok(history)
                if history
                    .messages
                    .iter()
                    .all(|message| message.channel_id == requested_channel) =>
            {
                self.has_more = history.has_more;
                self.older_anchor = Some(self.history_height);
                for message in &history.messages {
                    self.thread_only_rows.remove(&message.id);
                }
                if let Err(error) = self.timeline.prepend(history.messages) {
                    self.older_error = Some(error);
                }
            }
            Ok(_) => {
                self.older_error = Some("Caper returned messages from another channel.".into())
            }
            Err(error) if error.access_denied => self.clear_channel(&error.message),
            Err(error) => self.older_error = Some(error.message),
        }
    }

    fn clear_channel(&mut self, message: &str) {
        self.invalidate_navigation_cache();
        if let Some(channel) = &self.selected_channel {
            self.voice.revoke_channel(channel);
        }
        self.clear_channel_state();
        self.error = Some(message.into());
    }

    fn clear_channel_state(&mut self) {
        self.worker.send(Command::StopGateway);
        self.roster_generation += 1;
        self.channel_rosters.clear();
        self.voice_session_starts.clear();
        self.unavailable_rosters.clear();
        self.selected_channel = None;
        self.session = None;
        self.session_error = None;
        self.loading_older = false;
        self.older_error = None;
        self.has_more = false;
        self.timeline = Timeline::default();
        self.forwarding.close();
        self.older_armed = false;
        self.older_anchor = None;
        self.load_error = None;
        self.pending = None;
        self.mention_card = None;
        self.reaction_picker = None;
        self.pending_reactions.clear();
        self.reaction_errors.clear();
        self.reactors.clear();
        self.pending_pins.clear();
        self.mutations = model::MessageMutations::default();
        self.pin_errors.clear();
        self.showing_pins = false;
        self.message_editor = None;
        self.edit_history = None;
        self.draft.clear();
        self.typers.clear();
        self.live = "Offline".into();
    }

    fn logout(&mut self) {
        self.voice.leave();
        let token = self.token.take();
        self.generation += 1;
        self.worker.send(Command::StopGateway);
        self.worker.send(Command::ClearCredential {
            generation: self.generation,
        });
        if let Some(token) = token {
            self.worker.send(Command::Logout {
                generation: self.generation,
                token,
            });
        }
        self.account = None;
        self.reset_account_state();
        self.invalidate_navigation_cache();
        self.spaces.clear();
        self.directs.clear();
        self.people = None;
        self.selected_direct = None;
        self.invitations.clear();
        self.managed_invitations.clear();
        self.detail = None;
        self.selected_space = None;
        self.selected_channel = None;
        self.clear_channel_state();
        self.challenge = None;
        self.code.clear();
        self.attempts_remaining = None;
        self.loading = false;
        self.error = None;
        self.dialog = None;
    }

    fn send_message(&mut self) {
        self.send_message_to(None, false);
    }

    fn open_thread(&mut self, root: String) {
        self.thread_view = Some(ThreadView {
            root,
            loading: true,
            has_more: false,
            before: None,
            error: None,
        });
        self.load_thread(false);
    }

    fn load_thread(&mut self, older: bool) {
        let (Some(thread), Some(channel)) = (&mut self.thread_view, &self.selected_channel) else {
            return;
        };
        thread.loading = true;
        thread.error = None;
        self.thread_request += 1;
        self.worker.send(Command::LoadThread {
            generation: self.generation,
            request: self.thread_request,
            token: self.token.clone(),
            channel: channel.clone(),
            root: thread.root.clone(),
            before: if older { thread.before.clone() } else { None },
        });
    }

    fn send_message_to(&mut self, root: Option<String>, broadcast: bool) {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.thread_root_id != root)
        {
            return;
        }
        let Some(chat_token) = self.session.as_ref().map(|session| session.token.clone()) else {
            return;
        };
        let Some(channel) = self.selected_channel.clone() else {
            return;
        };
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.sending || pending.rejection.is_some())
        {
            return;
        }
        let text = self.pending.as_ref().map_or_else(
            || {
                root.as_ref()
                    .map(|root| {
                        self.thread_drafts
                            .entry(root.clone())
                            .or_default()
                            .0
                            .clone()
                    })
                    .unwrap_or_else(|| self.draft.clone())
            },
            |pending| pending.text.clone(),
        );
        let count = text.chars().count();
        if text.trim().is_empty()
            || count > 4_000
            || text
                .chars()
                .any(|character| character.is_control() && character != '\n' && character != '\t')
        {
            self.error = Some(
                if count > 4_000 {
                    "Messages can be at most 4,000 characters."
                } else if text.trim().is_empty() {
                    "Write a message first."
                } else {
                    "Messages cannot contain control characters."
                }
                .into(),
            );
            return;
        }
        let mut pending = PendingSend::prepare(self.pending.as_ref(), &text);
        if self.pending.is_none() {
            pending.thread_root_id = root.clone();
            pending.broadcast = broadcast;
        }
        let id = pending.id.clone();
        if self.pending.is_none() {
            if let Some(root) = &root {
                self.thread_drafts
                    .entry(root.clone())
                    .or_default()
                    .0
                    .clear();
            } else if self.draft == text {
                self.draft.clear();
            }
        }
        let thread_root_id = pending.thread_root_id.clone();
        let broadcast = pending.broadcast;
        self.pending = Some(pending);
        self.error = None;
        self.set_typing(false);
        self.worker.send(Command::Send {
            generation: self.generation,
            token: self.token.clone(),
            chat_token,
            channel,
            client_id: id,
            text,
            thread_root_id,
            broadcast,
        });
    }

    fn set_typing(&mut self, typing: bool) {
        if self.typing_sent == typing {
            return;
        }
        let (Some(session), Some(channel)) = (&self.session, &self.selected_channel) else {
            return;
        };
        self.typing_sent = typing;
        self.typing_pulse = Instant::now();
        self.worker.send(Command::Typing {
            token: self.token.clone(),
            chat_token: session.token.clone(),
            channel: channel.clone(),
            typing,
        });
    }

    fn load_older(&mut self) {
        if self.loading || self.loading_older || !self.has_more {
            return;
        }
        let Some(channel) = self.selected_channel.clone() else {
            return;
        };
        let Some(before) = self
            .timeline
            .messages()
            .find(|message| {
                message.is_channel_message() && !self.thread_only_rows.contains(&message.id)
            })
            .map(|message| message.seq.clone())
        else {
            return;
        };
        self.loading_older = true;
        self.older_error = None;
        self.worker.send(Command::LoadOlder {
            generation: self.generation,
            token: self.token.clone(),
            channel,
            before,
        });
    }

    fn admin(&mut self, operation: AdminOperation) {
        let Some(token) = self.token.clone() else {
            self.dialog = Some(Dialog::SignIn);
            return;
        };
        self.navigation += 1;
        self.invalidate_navigation_cache();
        self.opening = false;
        self.navigation_target = None;
        self.navigation_error = None;
        self.loading = true;
        self.error = None;
        self.worker.send(Command::Admin {
            generation: self.generation,
            token,
            operation,
        });
    }

    fn admin_result(&mut self, result: AdminResult) {
        if matches!(
            result,
            AdminResult::SpaceDeleted(_) | AdminResult::ChannelDeleted(_)
        ) {
            self.effects.play(Effect::Delete);
        }
        match result {
            AdminResult::SpaceCreated(space) => {
                self.spaces.push(space.clone());
                self.dialog = None;
                self.select_space(space.id);
            }
            AdminResult::SpaceUpdated(space) => {
                if let Some(item) = self.spaces.iter_mut().find(|item| item.id == space.id) {
                    *item = space.clone();
                }
                if let Some(detail) = &mut self.detail {
                    detail.space = space;
                }
                self.dialog = None;
            }
            AdminResult::SpaceDeleted(id) | AdminResult::SpaceLeft(id) => {
                self.voice.revoke_space(&id);
                self.invalidate_navigation_cache();
                self.spaces.retain(|space| space.id != id);
                self.dialog = None;
                self.detail = None;
                self.selected_space = None;
                self.managed_members.clear();
                self.managed_invitations.clear();
                self.presence.clear();
                if self.selected_direct.is_none() {
                    self.generation += 1;
                    self.clear_channel_state();
                }
                // General is retired, so only an account space can follow.
                if let Some(space) = self.spaces.iter().find(|space| !space.demo) {
                    self.select_space(space.id.clone());
                }
            }
            AdminResult::ChannelCreated(channel) => {
                if let Some(detail) = &mut self.detail {
                    detail.channels.push(channel.clone());
                }
                self.dialog = None;
                self.select_channel(channel.id.clone(), false);
                // Web opens a new private channel's Overview to add members.
                if channel.private {
                    self.open_manage_channel(&channel.id, &channel.name, true);
                }
            }
            AdminResult::ChannelUpdated(channel) => {
                if let Some(detail) = &mut self.detail
                    && let Some(item) = detail
                        .channels
                        .iter_mut()
                        .find(|item| item.id == channel.id)
                {
                    *item = channel;
                }
                self.dialog = None;
            }
            AdminResult::ChannelDeleted(id) => {
                self.voice.revoke_channel(&id);
                if let Some(detail) = &mut self.detail {
                    detail.channels.retain(|channel| channel.id != id);
                }
                self.dialog = None;
                if self.selected_channel.as_deref() != Some(&id) {
                    self.connect_gateway();
                    return;
                }
                self.clear_channel_state();
                if let Some(next) = self
                    .detail
                    .as_ref()
                    .and_then(|detail| detail.channels.first())
                {
                    self.select_channel(next.id.clone(), false);
                } else {
                    self.clear_channel_state();
                }
            }
            AdminResult::Members {
                channel,
                members,
                invitations,
            } => {
                if channel == self.managed_channel {
                    self.managed_members = members;
                    self.managed_invitations = invitations;
                }
            }
            AdminResult::Invitations(members) => self.managed_invitations = members,
            AdminResult::InvitationCreated(member) => {
                self.managed_invitations.retain(|item| item.id != member.id);
                self.managed_invitations.push(member);
                self.member_username.clear();
            }
            AdminResult::InvitationCancelled(user) => {
                self.managed_invitations.retain(|item| item.id != user);
            }
            AdminResult::InvitationAccepted(space) => {
                self.invitations.retain(|item| item.id != space.id);
                self.spaces.push(space.clone());
                self.dialog = None;
                self.select_space(space.id);
            }
            AdminResult::InvitationDeclined(space) => {
                self.invitations.retain(|item| item.id != space);
                self.dialog = None;
            }
            AdminResult::MemberAdded { channel, member } => {
                if channel == self.managed_channel {
                    self.managed_invitations.retain(|item| item.id != member.id);
                    self.managed_invitations.push(member);
                    self.member_username.clear();
                }
            }
            AdminResult::MemberRemoved { channel, member } => {
                if channel == self.managed_channel {
                    self.managed_members.retain(|item| item.id != member);
                    self.managed_invitations.retain(|item| item.id != member);
                }
            }
            AdminResult::ChannelLeft(channel) => {
                if let Some(mut detail) = self.detail.clone() {
                    let owner = self.owner();
                    detail
                        .channels
                        .retain(|item| item.id != channel || !item.private || owner);
                    for item in &mut detail.channels {
                        if item.id == channel {
                            item.joined = false;
                        }
                    }
                    self.admin_result(AdminResult::ChannelMembership {
                        detail,
                        channel,
                        joined: false,
                    });
                }
            }
            AdminResult::ChannelMembership {
                detail,
                channel,
                joined,
            } => {
                self.detail = Some(detail);
                if !joined && self.voice.state.active_channel() == Some(&channel) {
                    self.voice.leave();
                }
                if !joined && self.selected_channel.as_deref() == Some(&channel) {
                    self.generation += 1;
                    self.clear_channel_state();
                }
                self.loading = false;
                self.invalidate_navigation_cache();
                if self
                    .detail
                    .as_ref()
                    .is_some_and(|detail| detail.channels.iter().any(|item| item.id == channel))
                {
                    self.select_channel(channel, false);
                } else if let Some(next) = self
                    .detail
                    .as_ref()
                    .and_then(|detail| detail.channels.iter().find(|item| item.joined))
                    .map(|item| item.id.clone())
                {
                    self.select_channel(next, false);
                } else {
                    self.clear_channel_state();
                }
            }
            AdminResult::ChannelInvitationDeclined(channel) => {
                if let Some(detail) = &mut self.detail {
                    detail
                        .channel_invitations
                        .retain(|invite| invite.channel.id != channel);
                }
                self.loading = false;
            }
        }
    }

    fn periodic(&mut self, context: &egui::Context) {
        let now = Instant::now();
        if let Some(target) = self.navigation_prefetch.clone() {
            if !self.navigation_target_matches(&target) {
                self.navigation_prefetch = None;
            } else if self.navigation_cache.prefetch_state(&target, now)
                == navigation::PrefetchState::Pending
            {
                context.request_repaint_after(Duration::from_millis(100));
            } else {
                // Expired/evicted hover work must fall back to a normal load,
                // not leave navigation waiting for a discarded completion.
                self.prepare_navigation(target);
            }
        }
        if self.token.is_some()
            && now.duration_since(self.directs_refreshed) >= Duration::from_secs(15)
        {
            self.refresh_directs();
        }
        let live = self.live == "Live";
        if live != self.was_live {
            self.was_live = live;
            self.live_changed = now;
        }
        self.typers.retain(|_, typer| typer.expires > now);
        let active = !self.draft.trim().is_empty()
            && now.duration_since(self.typing_edited) < Duration::from_millis(600);
        if active
            && (!self.typing_sent
                || now.duration_since(self.typing_pulse) >= Duration::from_millis(500))
        {
            self.typing_sent = false;
            self.set_typing(true);
        } else if !active {
            self.set_typing(false);
        }
        if self.typing_sent || !self.typers.is_empty() {
            context.request_repaint_after(Duration::from_millis(100));
        }
    }

    fn restore_preferences(&mut self, storage: &dyn eframe::Storage) {
        if !self.persist_preferences {
            return;
        }
        if let Some(json) = storage.get_string("audio-preferences-v1") {
            self.voice.preferences = voice::Preferences::restore(&json);
        }
        if let Some(enabled) = storage
            .get_string("sound-effects-v1")
            .and_then(|value| value.parse::<bool>().ok())
        {
            self.sound_effects = enabled;
            self.effects = Effects::new(enabled);
        }
        if let Some(width) = storage
            .get_string("sidebar-width-v1")
            .and_then(|value| value.parse::<f32>().ok())
            .filter(|width| (220.0..=440.0).contains(width))
        {
            self.sidebar_width = width;
        }
    }
}

impl eframe::App for CaperApp {
    fn persist_egui_memory(&self) -> bool {
        self.persist_preferences
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Some(icon) = &self.daily_icon {
            storage.set_string("daily-icon-day-v1", icon.day.clone());
            storage.set_string("daily-icon-index-v1", icon.index.to_string());
        }
        if self.persist_preferences {
            storage.set_string("sidebar-width-v1", self.sidebar_width.to_string());
            storage.set_string("sound-effects-v1", self.sound_effects.to_string());
        }
        if self.persist_preferences
            && let Ok(json) = serde_json::to_string(&self.voice.preferences)
        {
            storage.set_string("audio-preferences-v1", json);
        }
    }

    fn update(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        if let Some(icon) = &mut self.daily_icon {
            context.request_repaint_after(Duration::from_secs(60));
            icon.refresh(chrono::Utc::now());
        }
        if context.input(|input| !input.events.is_empty()) {
            self.worker.send(Command::Activity);
        }
        let was_foreground = self.foreground;
        self.foreground = context.input(|input| input.viewport().focused.unwrap_or(false));
        self.receive();
        if self.foreground && !was_foreground {
            self.mark_selected_direct_read();
        }
        self.refresh_media_status();
        self.periodic(context);
        self.page(context);
        self.update_notice(context);
        let messages = self
            .timeline
            .messages()
            .chain(self.timeline.pinned_messages())
            .cloned()
            .collect::<Vec<_>>();
        self.forwarding.show(
            context,
            &self.worker,
            self.generation,
            self.token.as_deref(),
            &messages,
            &mut self.reaction_textures,
        );
        if !matches!(self.dialog, Some(Dialog::Audio)) {
            if !matches!(self.voice.microphone, MicrophoneState::Idle) {
                self.voice.stop_mic_test();
            }
            if self.voice.speaker_testing {
                self.voice.stop_speaker_test();
            }
        }
    }
}

impl CaperApp {
    fn page(&mut self, context: &egui::Context) {
        if self.account.is_none() || matches!(self.dialog, Some(Dialog::SignIn)) {
            self.login_page(context);
        } else if self.onboarding() {
            self.onboarding_page(context);
        } else if self.needs_first_space() {
            self.first_space_page(context);
        } else {
            let dialog_was_open = self.dialog.is_some();
            self.shell(context);
            self.dialogs(context, dialog_was_open);
            self.message_edit_dialogs(context);
        }
        // egui's buttons and custom click targets do not set a hand cursor.
        // Only supply a fallback: text fields and resize handles keep theirs.
        if context.output(|output| output.cursor_icon == egui::CursorIcon::Default) {
            let hovered = context.interaction_snapshot(|snapshot| snapshot.hovered.clone());
            if hovered.into_iter().any(|id| {
                context.read_response(id).is_some_and(|response| {
                    response.enabled() && response.hovered() && response.sense.senses_click()
                })
            }) {
                context.set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
    }

    /// Only the release notes scroll; the install action always stays visible.
    fn update_notice(&mut self, context: &egui::Context) {
        let Some(update) = self.updates.available() else {
            return;
        };
        let viewport = context.viewport_rect();
        egui::Window::new("Update available")
            .id(egui::Id::new("app-update"))
            .default_width((viewport.width() - 72.0).min(520.0))
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 16.0))
            .frame(
                egui::Frame::new()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(8)
                    .inner_margin(20),
            )
            .show(context, |ui| {
                ui.set_width((viewport.width() - 72.0).min(520.0));
                ui.label(bold("Update available").size(20.0));
                ui.label(RichText::new(format!("Caper {}", update.version)).color(MUTED));
                ui.add_space(12.0);
                egui::Frame::new()
                    .fill(BLACKOUT)
                    .corner_radius(8)
                    .inner_margin(14)
                    .show(ui, |ui| {
                        ui.label(bold("WHAT’S NEW").color(MUTED).size(12.0));
                        ui.add_space(8.0);
                        egui::ScrollArea::vertical()
                            .id_salt("update-notes")
                            .max_height((viewport.height() - 320.0).clamp(80.0, 340.0))
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                ui.label(RichText::new(if update.history_complete {
                                    "This update includes all of the following changes:"
                                } else {
                                    "Recorded changes are shown below. Earlier release notes aren’t available."
                                }).color(MUTED));
                                ui.add_space(8.0);
                                let fallback = updates::ChangelogEntry {
                                    version: update.version.clone(),
                                    notes: update.notes.clone(),
                                };
                                let entries = if update.changelog.is_empty() {
                                    std::slice::from_ref(&fallback)
                                } else {
                                    &update.changelog
                                };
                                for (index, entry) in entries.iter().enumerate() {
                                    if index > 0 {
                                        ui.add_space(6.0);
                                        ui.separator();
                                        ui.add_space(6.0);
                                    }
                                    ui.label(bold(&entry.version).size(14.0));
                                    if entry.notes.trim().is_empty() {
                                        ui.label(RichText::new("Release notes aren’t available for this version.").color(MUTED));
                                    }
                                    for line in entry.notes.lines().filter(|line| !line.trim().is_empty()) {
                                        ui.label(RichText::new(line).color(MUTED).size(14.0));
                                    }
                                }
                            });
                    });
                ui.add_space(12.0);
                let in_call = !matches!(self.voice.state.phase, Phase::Idle);
                ui.label(RichText::new(if !update.can_apply {
                    "Download the installer to update this copy manually."
                } else if in_call {
                    "Caper will restart and leave your voice call."
                } else {
                    "Caper will restart to install the update."
                }).color(MUTED).size(13.0));
                if let Some(error) = &self.updates.error {
                    ui.colored_label(ERROR, error);
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if update.can_apply {
                            let label = if in_call {
                                "Restart and leave call"
                            } else {
                                "Restart to update"
                            };
                            if primary_button(ui, label, true).clicked() && self.updates.apply() {
                                context.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                        } else if primary_button(ui, "Download", true).clicked() {
                            context.open_url(egui::OpenUrl::new_tab(updates::DOWNLOAD_URL));
                        }
                        if secondary_button(ui, "Later", true).clicked() {
                            self.updates.dismiss();
                        }
                    });
                });
            });
    }

    /// A signed-in account without a profile finishes it on its own page, as on web.
    fn onboarding(&self) -> bool {
        matches!(self.dialog, Some(Dialog::Profile))
            && self
                .account
                .as_ref()
                .is_some_and(|account| account.username.is_none() || account.display_name.is_none())
    }

    /// Web's first-space page: a signed-in account whose loaded space list is
    /// empty names its first space instead of seeing an empty workspace.
    fn needs_first_space(&self) -> bool {
        self.account.is_some()
            && self.limits.is_some()
            && self.dialog.is_none()
            && self.selected_direct.is_none()
            && !self.navigation_open
            && self.invitations.is_empty()
            && !self.spaces.iter().any(|space| !space.demo)
    }

    fn wordmark(&self, ui: &mut egui::Ui) {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(132.0, 35.0), egui::Sense::hover());
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, "Caper"));
        egui::Image::new(egui::include_image!(
            "../../../web/public/caper-wordmark-letters.svg"
        ))
        .paint_at(ui, rect);
        // Dot slot in the original viewBox (20 17 1042 276): x=924, y=108, size=132.
        let character = egui::Rect::from_min_size(
            rect.min + egui::vec2(rect.width() * 904.0 / 1042.0, rect.height() * 91.0 / 276.0),
            egui::vec2(rect.width() * 132.0 / 1042.0, rect.width() * 132.0 / 1042.0),
        );
        let index = self.daily_icon.as_ref().map_or(0, |icon| icon.index);
        egui::Image::from_bytes(
            format!("bytes://caper-branding-v1/{index}.svg"),
            avatar_images::BRANDING[index],
        )
        .paint_at(ui, character);
    }

    fn first_space_page(&mut self, context: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BLACKOUT))
            .show(context, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.set_max_width(440.0);
                        ui.add_space(100.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            self.wordmark(ui);
                            ui.add_space(58.0);
                            ui.label(black("Name your space").size(40.0));
                            ui.add_space(20.0);
                            ui.label(
                                RichText::new(
                                    "Choose something you will recognize easily. You can always change it later!",
                                )
                                .size(16.0)
                                .color(MUTED),
                            );
                            ui.add_space(24.0);
                            ui.label(bold("Space name").size(14.0));
                            ui.add_space(4.0);
                            let field = ui.add(
                                egui::TextEdit::singleline(&mut self.form_name)
                                    .vertical_align(egui::Align::Center)
                                    .char_limit(64)
                                    .min_size(egui::vec2(0.0, 52.0))
                                    .desired_width(f32::INFINITY),
                            );
                            let allowed = self.can_create_space();
                            if !allowed {
                                ui.add_space(6.0);
                                ui.label(
                                    RichText::new("You have reached your space limit.")
                                        .size(12.8)
                                        .color(MUTED),
                                );
                            }
                            if let Some(error) = &self.error {
                                ui.add_space(16.0);
                                login_error_frame(ui, error);
                            }
                            ui.add_space(24.0);
                            let blocked =
                                self.loading || !allowed || self.form_name.trim().is_empty();
                            let entered = field.lost_focus()
                                && ui.input(|input| input.key_pressed(egui::Key::Enter));
                            let clicked = login_action(
                                ui,
                                if self.loading { "Creating…" } else { "Create space" },
                                blocked,
                            )
                            .clicked();
                            if (clicked || entered) && !blocked {
                                // Web validates on submit and keeps the name on failure.
                                if let Some(error) = space_name_error(&self.form_name) {
                                    self.error = Some(error.into());
                                } else {
                                    self.admin(AdminOperation::CreateSpace {
                                        name: self.form_name.clone(),
                                    });
                                }
                            }
                            ui.add_space(16.0);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Log out").size(13.6).color(MUTED),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    self.logout();
                                }
                                if secondary_button(ui, "Direct messages", true).clicked() {
                                    self.navigation_open = true;
                                }
                            });
                        });
                    });
                });
            });
    }

    fn onboarding_page(&mut self, context: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BLACKOUT))
            .show(context, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.set_max_width(440.0);
                        ui.add_space(100.0);
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            self.wordmark(ui);
                            ui.add_space(58.0);
                            ui.label(
                                bold("ONE LAST THING")
                                    .size(11.0)
                                    .extra_letter_spacing(1.5)
                                    .color(MUTED),
                            );
                            ui.add_space(24.0);
                            ui.label(black("Choose how you show up.").size(40.0));
                            ui.add_space(20.0);
                            ui.label(
                                RichText::new(
                                    "Your username is unique. Your display name is what people see in conversations.",
                                )
                                .size(16.0)
                                .color(MUTED),
                            );
                            ui.add_space(24.0);
                            ui.label(bold("Username").size(14.0));
                            ui.add_space(4.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.username)
                                    .vertical_align(egui::Align::Center)
                                    .char_limit(32)
                                    .min_size(egui::vec2(0.0, 52.0))
                                    .desired_width(f32::INFINITY),
                            );
                            self.username = normalize_username(&self.username);
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("3-32 lowercase letters, numbers, or underscores.")
                                    .size(12.8)
                                    .color(MUTED),
                            );
                            ui.add_space(20.0);
                            ui.label(bold("Display name").size(14.0));
                            ui.add_space(4.0);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.display_name)
                                    .vertical_align(egui::Align::Center)
                                    .char_limit(64)
                                    .min_size(egui::vec2(0.0, 52.0))
                                    .desired_width(f32::INFINITY),
                            );
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new("Shown to other people. It does not need to be unique.")
                                    .size(12.8)
                                    .color(MUTED),
                            );
                            if let Some(error) = &self.error {
                                ui.add_space(16.0);
                                login_error_frame(ui, error);
                            }
                            ui.add_space(24.0);
                            if login_action(
                                ui,
                                if self.loading { "Saving…" } else { "Finish account" },
                                self.loading
                                    || self.username.len() < 3
                                    || self.display_name.trim().is_empty(),
                            )
                            .clicked()
                            {
                                self.loading = true;
                                self.error = None;
                                self.worker.send(Command::Profile {
                                    generation: self.generation,
                                    token: self.token.clone().unwrap_or_default(),
                                    username: self.username.trim().to_ascii_lowercase(),
                                    display_name: self.display_name.trim().into(),
                                });
                            }
                            ui.add_space(16.0);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Log out").size(13.6).color(MUTED),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    self.logout();
                                }
                            });
                        });
                    });
                });
            });
    }

    fn login_page(&mut self, context: &egui::Context) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BLACKOUT)
                    .inner_margin(egui::Margin::symmetric(20, 0)),
            )
            .show(context, |ui| {
                ui.vertical_centered(|ui| {
                    let width = ui.available_width().min(440.0);
                    ui.set_max_width(width);
                    ui.add_space(100.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        // This form controls its own gaps; don't add egui's item spacing too.
                        ui.spacing_mut().item_spacing.y = 0.0;
                        self.wordmark(ui);
                        ui.add_space(58.0);
                        ui.label(black(if self.challenge.is_some() {
                            "Check your email."
                        } else {
                            "Welcome to Caper"
                        }).size(40.0));
                        ui.add_space(4.0);
                        ui.label(
                            RichText::new(if self.challenge.is_some() {
                                format!(
                                    "Enter the six-character code sent to {}. It expires in 10 minutes.",
                                    self.email.trim()
                                )
                            } else {
                                "Use your email to create an account or return to one. We’ll send a code to your email."
                                    .into()
                            })
                            .size(16.0)
                            .color(MUTED),
                        );
                        ui.add_space(20.0);
                        if self.challenge.is_some() {
                            ui.label(bold("Sign-in code").size(14.0));
                            ui.add_space(8.0);
                            let exhausted = self.attempts_remaining == Some(0);
                            let response = ui.add_enabled(
                                !exhausted,
                                egui::TextEdit::singleline(&mut self.code)
                                    .vertical_align(egui::Align::Center)
                                    .char_limit(6)
                                    .min_size(egui::vec2(width, 52.0))
                                    .desired_width(width),
                            );
                            self.code.make_ascii_uppercase();
                            self.code.retain(|character| {
                                "ABCDEFGHJKMNPQRSTWXYZ23456789".contains(character)
                            });
                            if let Some(error) = self.error.as_ref().or(self.warning.as_ref()) {
                                ui.add_space(12.0);
                                login_error_frame(ui, error);
                            }
                            if self.attempts_remaining == Some(1) {
                                ui.add_space(8.0);
                                ui.label(
                                    bold("One attempt left. Check the code carefully.").size(14.0),
                                );
                            }
                            ui.add_space(12.0);
                            if exhausted {
                                if login_action(
                                    ui,
                                    if self.loading { "Sending…" } else { "Email me a new code" },
                                    self.loading,
                                )
                                .clicked()
                                {
                                    self.loading = true;
                                    self.error = None;
                                    self.code.clear();
                                    self.worker.send(Command::RequestCode {
                                        generation: self.generation,
                                        email: self.email.trim().to_owned(),
                                    });
                                }
                            } else if login_action(
                                ui,
                                if self.loading { "Checking…" } else { "Continue" },
                                self.loading || self.code.len() != 6,
                            )
                            .clicked()
                                || (response.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                            {
                                let challenge = self.challenge.clone().unwrap_or_default();
                                self.loading = true;
                                self.error = None;
                                self.worker.send(Command::VerifyCode {
                                    generation: self.generation,
                                    challenge,
                                    code: self.code.clone(),
                                });
                            }
                            ui.add_space(10.0);
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Use a different email")
                                            .size(13.6)
                                            .color(MUTED),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.challenge = None;
                                self.code.clear();
                                self.error = None;
                                self.attempts_remaining = None;
                            }
                        } else {
                            ui.label(bold("Email address").size(14.0));
                            ui.add_space(8.0);
                            let response = ui.add_sized(
                                [width, 52.0],
                                egui::TextEdit::singleline(&mut self.email)
                                    .vertical_align(egui::Align::Center)
                                    .hint_text("you@example.com"),
                            );
                            if let Some(error) = self.error.as_ref().or(self.warning.as_ref()) {
                                ui.add_space(12.0);
                                login_error_frame(ui, error);
                            }
                            ui.add_space(12.0);
                            let submit = ui.with_layout(egui::Layout::top_down(egui::Align::RIGHT), |ui| {
                                let width = ui.painter().layout_no_wrap(
                                    "Email me a code".into(),
                                    egui::FontId::new(16.0, egui::FontFamily::Name("Satoshi Medium".into())),
                                    TEXT,
                                ).size().x + 84.0;
                                ui.allocate_ui_with_layout(
                                    egui::vec2(width, 58.0),
                                    egui::Layout::top_down(egui::Align::LEFT),
                                    |ui| login_action(
                                        ui,
                                        if self.loading { "Sending…" } else { "Email me a code" },
                                        self.loading || !self.email.contains('@'),
                                    ),
                                ).inner
                            }).inner;
                            if submit.clicked()
                                || (response.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter)))
                            {
                                self.loading = true;
                                self.error = None;
                                self.worker.send(Command::RequestCode {
                                    generation: self.generation,
                                    email: self.email.trim().into(),
                                });
                            }
                        }
                    });
                });
            });
    }

    fn shell(&mut self, context: &egui::Context) {
        let narrow = context.viewport_rect().width() <= 760.0;
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(SURFACE))
            .show(context, |ui| {
                if self.dialog.is_some() {
                    ui.disable();
                }
                let content = ui.max_rect();
                if narrow {
                    if self.navigation_open {
                        let rail_rect = egui::Rect::from_min_max(
                            content.min,
                            egui::pos2(content.left() + 59.0, content.bottom()),
                        );
                        let sidebar_rect = egui::Rect::from_min_max(
                            egui::pos2(rail_rect.right(), content.top()),
                            content.max,
                        );
                        ui.scope_builder(egui::UiBuilder::new().max_rect(rail_rect), |ui| {
                            self.rail(ui);
                        });
                        ui.scope_builder(egui::UiBuilder::new().max_rect(sidebar_rect), |ui| {
                            self.sidebar(ui, sidebar_rect.width());
                        });
                    } else {
                        ui.scope_builder(egui::UiBuilder::new().max_rect(content), |ui| {
                            self.conversation(ui, true)
                        });
                        if self.selected_direct.is_none()
                            && self.narrow_members_visible
                            && !self.no_accessible_channels()
                        {
                            let members_rect = egui::Rect::from_min_max(
                                egui::pos2(
                                    (content.right() - 280.0).max(content.left()),
                                    content.top() + 53.0,
                                ),
                                content.max,
                            );
                            ui.scope_builder(egui::UiBuilder::new().max_rect(members_rect), |ui| {
                                self.member_presence(ui)
                            });
                        }
                    }
                } else {
                    let sidebar_width = self.sidebar_width.min(content.width() - 380.0).max(220.0);
                    let rail_rect = egui::Rect::from_min_max(
                        content.min,
                        egui::pos2(content.left() + 59.0, content.bottom()),
                    );
                    let sidebar_rect = egui::Rect::from_min_max(
                        egui::pos2(rail_rect.right(), content.top()),
                        egui::pos2(rail_rect.right() + sidebar_width, content.bottom()),
                    );
                    let separator_rect = egui::Rect::from_min_max(
                        egui::pos2(sidebar_rect.right(), content.top()),
                        egui::pos2(sidebar_rect.right() + 1.0, content.bottom()),
                    );
                    let stage_rect = egui::Rect::from_min_max(
                        egui::pos2(separator_rect.right(), content.top()),
                        content.max,
                    );
                    // Reserve 320px for chat; otherwise show members as a right-side
                    // overlay below the header, never as a second row below chat.
                    let wide_members = stage_rect.width() >= 540.0;
                    let members_visible = self.selected_direct.is_none()
                        && self.members_visible
                        && !self.no_accessible_channels();
                    let (conversation_rect, members_rect) = if members_visible && wide_members {
                        let members = egui::Rect::from_min_max(
                            egui::pos2(stage_rect.right() - 220.0, stage_rect.top()),
                            stage_rect.max,
                        );
                        (
                            egui::Rect::from_min_max(
                                stage_rect.min,
                                egui::pos2(members.left(), stage_rect.bottom()),
                            ),
                            Some(members),
                        )
                    } else if members_visible {
                        let members = egui::Rect::from_min_max(
                            egui::pos2(stage_rect.right() - 220.0, stage_rect.top() + 53.0),
                            stage_rect.max,
                        );
                        (stage_rect, Some(members))
                    } else {
                        (stage_rect, None)
                    };
                    ui.scope_builder(egui::UiBuilder::new().max_rect(rail_rect), |ui| {
                        self.rail(ui);
                    });
                    ui.scope_builder(egui::UiBuilder::new().max_rect(sidebar_rect), |ui| {
                        self.sidebar(ui, sidebar_width)
                    });
                    let separator = ui
                        .interact(
                            separator_rect.expand2(egui::vec2(3.0, 0.0)),
                            egui::Id::new("sidebar-resize"),
                            egui::Sense::click_and_drag(),
                        )
                        .on_hover_text(
                            "Drag to resize. Arrow keys to adjust. Double-click to reset.",
                        );
                    separator.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Slider,
                            true,
                            "Channel sidebar width",
                        )
                    });
                    if separator.dragged() {
                        self.sidebar_width = (sidebar_width + separator.drag_delta().x)
                            .clamp(220.0, (content.width() - 380.0).clamp(220.0, 440.0));
                    }
                    if separator.double_clicked() {
                        self.sidebar_width = 280.0;
                    }
                    if separator.clicked() {
                        separator.request_focus();
                    }
                    if separator.hovered() || separator.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    if separator.has_focus() {
                        ui.memory_mut(|memory| {
                            memory.set_focus_lock_filter(
                                separator.id,
                                egui::EventFilter {
                                    horizontal_arrows: true,
                                    ..Default::default()
                                },
                            )
                        });
                        ui.input_mut(|input| {
                            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft) {
                                self.sidebar_width = sidebar_width - 10.0;
                            }
                            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight) {
                                self.sidebar_width = sidebar_width + 10.0;
                            }
                            if input.consume_key(egui::Modifiers::NONE, egui::Key::Home) {
                                self.sidebar_width = 220.0;
                            }
                            if input.consume_key(egui::Modifiers::NONE, egui::Key::End) {
                                self.sidebar_width = 440.0;
                            }
                        });
                        self.sidebar_width = self
                            .sidebar_width
                            .clamp(220.0, (content.width() - 380.0).clamp(220.0, 440.0));
                    }
                    ui.scope_builder(egui::UiBuilder::new().max_rect(conversation_rect), |ui| {
                        self.conversation(ui, false)
                    });
                    if let Some(members_rect) = members_rect {
                        ui.scope_builder(egui::UiBuilder::new().max_rect(members_rect), |ui| {
                            self.member_presence(ui)
                        });
                    }
                }
            });
    }

    fn rail(&mut self, ui: &mut egui::Ui) {
        egui::Frame::new()
            .fill(BLACKOUT)
            .inner_margin(egui::Margin::symmetric(9, 14))
            .show(ui, |ui| {
                ui.set_width(42.0);
                ui.set_min_height(ui.available_height());
                ui.spacing_mut().item_spacing.y = 0.0;
                let spaces: Vec<_> = self
                    .spaces
                    .iter()
                    .map(|space| (space.id.clone(), space.name.clone(), space.demo))
                    .collect();
                for (id, name, demo) in spaces {
                    let active = self.selected_space.as_deref() == Some(&id);
                    let muted = (!demo && self.account.is_some())
                        .then(|| self.muted_label(&Scope::Space(id.clone())))
                        .flatten();
                    let text = if demo {
                        "C".into()
                    } else {
                        name.chars()
                            .next()
                            .unwrap_or('C')
                            .to_uppercase()
                            .to_string()
                    };
                    let mut letter = if active { TEXT } else { MUTED };
                    if muted.is_some() {
                        letter = muted_color(letter);
                    }
                    let button = egui::Button::new(RichText::new(text).strong().color(letter))
                        .min_size(egui::vec2(40.0, 40.0))
                        .fill(if active {
                            Color32::from_rgb(57, 35, 30)
                        } else {
                            SURFACE
                        })
                        .stroke(Stroke::new(
                            1.0,
                            if active {
                                Color32::from_rgb(128, 81, 67)
                            } else {
                                BORDER
                            },
                        ))
                        .corner_radius(if active { 8 } else { 12 });
                    let response = ui
                        .with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.add(button)
                        })
                        .inner;
                    // A muted space gets a bell-slash badge and says so on hover.
                    let response = match &muted {
                        Some(label) => {
                            let badge = egui::Rect::from_center_size(
                                response.rect.right_bottom() - egui::vec2(3.0, 3.0),
                                egui::vec2(18.0, 18.0),
                            );
                            ui.painter().circle_filled(badge.center(), 9.0, BLACKOUT);
                            paint_icon(ui.painter(), badge.shrink(3.0), NavIcon::BellOff, MUTED);
                            response.on_hover_text(format!("{name}\n{label}"))
                        }
                        None => response.on_hover_text(name),
                    };
                    if active {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(
                                egui::pos2(ui.max_rect().left() - 9.0, response.rect.top() + 8.0),
                                egui::vec2(3.0, 24.0),
                            ),
                            2.0,
                            TERRACOTTA_BRIGHT,
                        );
                    }
                    if response.clicked() {
                        self.select_space(id);
                    } else if response.hovered() || response.has_focus() {
                        self.prefetch(NavigationTarget {
                            space: if demo { None } else { Some(id) },
                            channel: None,
                        });
                    }
                    ui.add_space(10.0);
                }
                for invitation in self.invitations.clone() {
                    let response = ui
                        .add(
                            egui::Button::new(RichText::new("?").strong().color(TERRACOTTA_BRIGHT))
                                .min_size(egui::vec2(40.0, 40.0))
                                .fill(SURFACE)
                                .stroke(Stroke::new(1.0, TERRACOTTA))
                                .corner_radius(12),
                        )
                        .on_hover_text(format!("Invitation to {}", invitation.name));
                    if response.clicked() {
                        self.dialog = Some(Dialog::Invitation(invitation));
                    }
                    ui.add_space(10.0);
                }
                let tooltip = self.create_space_tooltip();
                let add_enabled = self.account.is_none() || self.can_create_space();
                let (rect, add) = ui.allocate_exact_size(
                    egui::vec2(40.0, 40.0),
                    if add_enabled {
                        egui::Sense::click()
                    } else {
                        egui::Sense::hover()
                    },
                );
                let plus = if add_enabled {
                    TERRACOTTA_BRIGHT
                } else {
                    TERRACOTTA_BRIGHT.gamma_multiply(0.45)
                };
                ui.painter()
                    .rect_filled(rect, 12.0, if add.hovered() { RAISED } else { SURFACE });
                let inset = rect.shrink(0.5);
                let corners = [
                    inset.right_top() + egui::vec2(-12.0, 12.0),
                    inset.right_bottom() + egui::vec2(-12.0, -12.0),
                    inset.left_bottom() + egui::vec2(12.0, -12.0),
                    inset.left_top() + egui::vec2(12.0, 12.0),
                ];
                let mut outline = Vec::new();
                for (corner, center) in corners.into_iter().enumerate() {
                    for step in 0..=8 {
                        let angle =
                            (corner as f32 - 1.0 + step as f32 / 8.0) * std::f32::consts::FRAC_PI_2;
                        outline.push(center + egui::vec2(angle.cos(), angle.sin()) * 12.0);
                    }
                }
                outline.push(outline[0]);
                ui.painter().extend(egui::Shape::dashed_line(
                    &outline,
                    Stroke::new(1.0, BORDER),
                    3.0,
                    3.0,
                ));
                add.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, add_enabled, "Create space")
                });
                paint_icon(
                    ui.painter(),
                    egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0)),
                    NavIcon::Plus,
                    plus,
                );
                if add.on_hover_text(tooltip).clicked() {
                    self.dialog = Some(if self.account.is_some() {
                        Dialog::CreateSpace
                    } else {
                        Dialog::SignIn
                    });
                    self.form_name.clear();
                }
            });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, width: f32) {
        // Redraw when a timed mute ends.
        if let Some(wait) = self.notifications.next_expiry(chrono::Utc::now()) {
            ui.ctx().request_repaint_after(wait);
        }
        let space_scope = self
            .detail
            .as_ref()
            .filter(|detail| !detail.space.demo && self.account.is_some())
            .map(|detail| Scope::Space(detail.space.id.clone()));
        egui::Frame::new()
            .fill(SIDEBAR)
            .inner_margin(egui::Margin {
                left: 12,
                right: 12,
                top: 0,
                bottom: 12,
            })
            .show(ui, |ui| {
                ui.set_width(width - 24.0);
                ui.set_height(ui.available_height());
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::TopBottomPanel::bottom("native-account")
                    .show_separator_line(false)
                    .frame(egui::Frame::NONE)
                    .show_inside(ui, |ui| self.account_bar(ui));
                egui::ScrollArea::vertical()
                    .id_salt("sidebar-scroll")
                    .show(ui, |ui| {
                        let header = ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 54.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                let width = ui.available_width()
                                    - if self.navigation_open { 36.0 } else { 0.0 };
                                let (rect, actions) = ui.allocate_exact_size(
                                    egui::vec2(width, 38.0),
                                    if self.owner() || self.can_leave_space() {
                                        egui::Sense::click()
                                    } else {
                                        egui::Sense::hover()
                                    },
                                );
                                if (self.owner() || self.can_leave_space())
                                    && (actions.hovered() || actions.has_focus())
                                {
                                    ui.painter().rect_filled(rect, 8.0, RAISED);
                                }
                                let name = self
                                    .detail
                                    .as_ref()
                                    .map_or("Caper", |detail| if detail.space.demo { "Caper" } else { detail.space.name.as_str() });
                                let title_rect = rect.shrink2(egui::vec2(8.0, 0.0));
                                let muted = space_scope.as_ref().and_then(|scope| self.muted_label(scope));
                                if let Some(label) = &muted {
                                    muted_indicator(
                                        ui,
                                        egui::Rect::from_center_size(
                                            egui::pos2(rect.right() - 38.0, rect.center().y),
                                            egui::vec2(14.0, 14.0),
                                        ),
                                        actions.id.with("muted"),
                                        label,
                                    );
                                }
                                ui.painter()
                                    .with_clip_rect(egui::Rect::from_min_max(
                                        title_rect.min,
                                        egui::pos2(
                                            title_rect.right() - if muted.is_some() { 46.0 } else { 24.0 },
                                            title_rect.bottom(),
                                        ),
                                    ))
                                    .text(
                                        egui::pos2(title_rect.left(), title_rect.center().y),
                                        egui::Align2::LEFT_CENTER,
                                        name,
                                        egui::FontId::new(
                                            15.0,
                                            egui::FontFamily::Name("Satoshi Bold".into()),
                                        ),
                                        TEXT,
                                    );
                                actions.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Button,
                                        self.owner() || self.can_leave_space(),
                                        format!("{name} actions"),
                                    )
                                });
                                if self.owner() || self.can_leave_space() {
                                    paint_icon(
                                        ui.painter(),
                                        egui::Rect::from_center_size(
                                            egui::pos2(rect.right() - 16.0, rect.center().y),
                                            egui::vec2(16.0, 16.0),
                                        ),
                                        NavIcon::Chevron,
                                        MUTED,
                                    );
                                    if actions.clicked() && !egui::Popup::menu(&actions).is_open() {
                                        self.load_notifications();
                                    }
                                    egui::Popup::menu(&actions).width(width).show(|ui| {
                                        if ui.button("Browse channels").clicked() {
                                            self.browse_channels = true;
                                            self.channel_search.clear();
                                            ui.close();
                                        }
                                        if let Some(scope) = &space_scope {
                                            self.notification_items(ui, scope, "space");
                                            ui.separator();
                                        }
                                        if self.can_leave_space() {
                                            if ui
                                                .button(RichText::new("Leave space…").color(ERROR))
                                                .clicked()
                                            {
                                                if let Some(detail) = &self.detail {
                                                    self.dialog = Some(Dialog::LeaveSpace {
                                                        id: detail.space.id.clone(),
                                                        name: detail.space.name.clone(),
                                                    });
                                                }
                                                ui.close();
                                            }
                                            return;
                                        }
                                        if ui
                                            .add(
                                                egui::Button::image_and_text(
                                                    egui::Image::new(egui::include_image!(
                                                        "../resources/icons/settings.svg"
                                                    ))
                                                    .fit_to_exact_size(egui::vec2(16.0, 16.0)),
                                                    "Space settings",
                                                )
                                                .min_size(egui::vec2(width - 16.0, 36.0)),
                                            )
                                            .clicked()
                                        {
                                            self.open_manage_space();
                                            ui.close();
                                        }
                                    });
                                }
                                if self.navigation_open
                                    && drawn_icon_button(ui, NavIcon::Close, "Close navigation")
                                        .clicked()
                                {
                                    self.navigation_open = false;
                                }
                            },
                        );
                        ui.painter().hline(
                            (ui.max_rect().left() - 12.0)..=(ui.max_rect().right() + 12.0),
                            header.response.rect.bottom(),
                            Stroke::new(1.0, BORDER),
                        );
                        if let Some(scope) = &space_scope
                            && self.notifications.error(scope).is_some()
                        {
                            ui.add_space(6.0);
                            self.notification_error(ui, scope);
                        }
                        ui.add_space(12.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(ui.available_width(), 32.0),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                if drawn_icon_button(
                                    ui,
                                    if self.channels_expanded {
                                        NavIcon::Chevron
                                    } else {
                                        NavIcon::ChevronRight
                                    },
                                    "Toggle channels",
                                )
                                .clicked()
                                {
                                    self.channels_expanded = !self.channels_expanded;
                                    self.effects.toggle(self.channels_expanded);
                                }
                                ui.label(bold("Channels").size(12.0).color(MUTED));
                                let count = self
                                    .detail
                                    .as_ref()
                                    .map_or(0, |detail| detail.channels.len());
                                ui.add_space(6.0);
                                ui.label(RichText::new(count.to_string()).size(10.0).color(MUTED));
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
                                        let can_create = self.can_create_channel();
                                        let channel_tooltip = self.create_channel_tooltip();
                                        if self.owner() {
                                            let options = drawn_icon_button(
                                                ui,
                                                NavIcon::More,
                                                "Channel options",
                                            );
                                            egui::Popup::menu(&options)
                                                .align(egui::RectAlign::BOTTOM_END)
                                                .width(190.0)
                                                .show(|ui| {
                                                    if ui
                                                        .add_enabled(can_create, egui::Button::image_and_text(
                                                            egui::Image::new(egui::include_image!(
                                                                "../resources/icons/plus.svg"
                                                            ))
                                                            .fit_to_exact_size(egui::vec2(
                                                                16.0, 16.0,
                                                            )),
                                                            "Create channel",
                                                        ))
                                                        .clicked()
                                                    {
                                                        self.form_name.clear();
                                                        self.form_private = false;
                                                        self.dialog = Some(Dialog::CreateChannel);
                                                        ui.close();
                                                    }
                                                    if ui
                                                        .button(if self.channels_expanded {
                                                            "Collapse channels"
                                                        } else {
                                                            "Expand channels"
                                                        })
                                                        .clicked()
                                                    {
                                                        self.channels_expanded =
                                                            !self.channels_expanded;
                                                        self.effects.toggle(self.channels_expanded);
                                                        ui.close();
                                                    }
                                                });
                                        }
                                        if self.owner()
                                            && ui
                                                .add_enabled_ui(can_create, |ui| {
                                                    drawn_icon_button_with_tooltip(
                                                        ui,
                                                        NavIcon::Plus,
                                                        "Create channel",
                                                        &channel_tooltip,
                                                    )
                                                    .on_disabled_hover_text(&channel_tooltip)
                                                })
                                                .inner
                                                .clicked()
                                        {
                                            self.form_name.clear();
                                            self.form_private = false;
                                            self.dialog = Some(Dialog::CreateChannel);
                                        }
                                    },
                                );
                            },
                        );
                        ui.add_space(4.0);
                        if self.browse_channels {
                            if ui.button("Close Browse").clicked() {
                                self.browse_channels = false;
                                self.channel_search.clear();
                            }
                            ui.add(egui::TextEdit::singleline(&mut self.channel_search).hint_text("Search channels"));
                            let query = self.channel_search.to_lowercase();
                            let previews: Vec<_> = self.detail.as_ref().map_or_else(Vec::new, |detail| detail.channels.iter()
                                .filter(|channel| !channel.joined && channel.name.to_lowercase().contains(&query))
                                .map(|channel| (channel.id.clone(), channel.name.clone())).collect());
                            for (id, name) in previews {
                                ui.horizontal(|ui| {
                                    if ui.button(format!("# {name}")).clicked() { self.select_channel(id.clone(), false); }
                                    if ui.small_button("Join").clicked() && let Some(space) = self.selected_space.clone() {
                                        self.admin(AdminOperation::JoinChannel { space, channel: id.clone() });
                                    }
                                });
                            }
                            let invitations = self.detail.as_ref().map_or_else(Vec::new, |detail| detail.channel_invitations.clone());
                            for invitation in invitations {
                                ui.group(|ui| {
                                    ui.label(bold(format!("Private invitation · #{}", invitation.channel.name)).size(12.0));
                                    ui.label(RichText::new(format!("{} (@{}) invited you. Expires seven days after it was sent.", invitation.inviter.display_name, invitation.inviter.username)).size(11.0).color(MUTED));
                                    ui.label(RichText::new("Messages stay hidden until acceptance. Accepting joins the channel, not its voice call.").size(11.0).color(MUTED));
                                    ui.horizontal(|ui| {
                                        if ui.button("Decline").clicked() && let Some(space) = self.selected_space.clone() {
                                            self.admin(AdminOperation::DeclineChannelInvitation { space, channel: invitation.channel.id.clone() });
                                        }
                                        if ui.button("Accept").clicked() && let Some(space) = self.selected_space.clone() {
                                            self.admin(AdminOperation::AcceptChannelInvitation { space, channel: invitation.channel.id.clone() });
                                        }
                                    });
                                });
                            }
                        }
                        let channels: Vec<_> =
                            self.detail.as_ref().map_or_else(Vec::new, |detail| {
                                detail
                                    .channels
                                    .iter()
                                    .filter(|channel| channel.joined)
                                    .map(|channel| {
                                        (channel.id.clone(), channel.name.clone(), channel.private)
                                    })
                                    .collect()
                            });
                        for (id, name, private) in channels {
                            if !self.channels_expanded {
                                break;
                            }
                            ui.scope(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.spacing_mut().interact_size.y = 28.0;
                                let active = self.selected_channel.as_deref() == Some(&id);
                                let started = if self.voice.state.active_channel() == Some(&id) {
                                    if matches!(self.voice.state.phase, Phase::Joining(_)) {
                                        self.voice_session_starts.get(&id).copied().or(self.voice.session_started_at)
                                    } else {
                                        self.voice.session_started_at
                                    }
                                } else {
                                    self.voice_session_starts.get(&id).copied()
                                }.or_else(|| self.pending_voice_join.as_ref().filter(|(channel, _, _)| channel == &id).map(|(_, _, clicked)| *clicked));
                                let duration = started.map(|started| {
                                    ui.ctx().request_repaint_after(Duration::from_secs(1));
                                    voice_session_duration(started, chrono::Utc::now().timestamp_millis().max(0) as u64)
                                });
                                let space = self
                                    .detail
                                    .as_ref()
                                    .filter(|detail| !detail.space.demo)
                                    .map(|detail| detail.space.id.clone());
                                let scope = space.clone().map(|space| Scope::Channel {
                                    space,
                                    channel: id.clone(),
                                });
                                let muted =
                                    scope.as_ref().and_then(|scope| self.muted_label(scope));
                                // Demo channels have no options.
                                let mut menu = None;
                                if let Some(space) = &space {
                                    menu = Some(RowMenu {
                                        label: format!("Channel options for {name}"),
                                        on_hover: false,
                                        content: Box::new(|ui: &mut egui::Ui| {
                                            self.channel_menu(ui, space, &id, &name, private)
                                        }),
                                    });
                                }
                                let (response, opened) = channel_button(
                                    ui,
                                    egui::vec2(ui.available_width(), 32.0),
                                    &name,
                                    Some(if private { NavIcon::Lock } else { NavIcon::Hash }),
                                    active,
                                    RowExtras {
                                        muted,
                                        duration: duration.as_deref(),
                                        menu,
                                    },
                                );
                                if opened {
                                    self.load_notifications();
                                } else if response.clicked() {
                                    self.select_channel(id.clone(), false);
                                } else if response.hovered() || response.has_focus() {
                                    self.prefetch(NavigationTarget {
                                        space: self.selected_space.clone(),
                                        channel: Some(id.clone()),
                                    });
                                }
                                if let Some(scope) = &scope {
                                    self.notification_error(ui, scope);
                                }
                                ui.push_id(&id, |ui| {
                                    self.channel_voice_summary(ui, &id, &name);
                                    self.channel_voice_roster(ui, &id);
                                });
                            });
                            ui.add_space(3.0);
                        }
                        let active_visible = self.channels_expanded
                            && self.detail.as_ref().is_some_and(|detail| {
                                detail.channels.iter().any(|channel| {
                                    self.voice.state.active_channel() == Some(&channel.id)
                                })
                            });
                        if !active_visible && !self.voice.participants.is_empty() {
                            ui.add_space(14.0);
                            self.voice_roster(ui, self.roster_for_active_call(), true);
                        }
                        self.direct_navigation(ui);
                    });
            });
    }

    fn direct_navigation(&mut self, ui: &mut egui::Ui) {
        let Some(account) = self.account.clone() else {
            return;
        };
        ui.add_space(8.0);
        full_bleed_separator(ui, ui.cursor().top());
        ui.add_space(6.0);
        ui.scope(|ui| {
            ui.spacing_mut().interact_size.y = 28.0;
            ui.horizontal(|ui| {
                let heading_hovered = ui.rect_contains_pointer(ui.max_rect());
                ui.label(bold("Direct messages").size(12.0).color(MUTED));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // Keep the target in the focus order even when its icon is hidden.
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                    let response = ui.interact(
                        rect,
                        egui::Id::new("direct-heading-plus"),
                        egui::Sense::click(),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            ui.is_enabled(),
                            "Start direct message",
                        )
                    });
                    if heading_hovered || response.has_focus() {
                        if response.hovered() || response.has_focus() {
                            ui.painter().rect_filled(rect, 6.0, RAISED);
                        }
                        paint_icon(
                            ui.painter(),
                            rect.shrink(5.0),
                            NavIcon::Plus,
                            if response.hovered() { TEXT } else { MUTED },
                        );
                    }
                    if response.on_hover_text("Start direct message").clicked() {
                        self.member_username.clear();
                        self.error = None;
                        self.dialog = Some(Dialog::StartDirect);
                    }
                });
            });
            self.request_navigation(ui);
            let self_direct = self
                .directs
                .iter()
                .find(|direct| direct.peer.id == account.id)
                .cloned();
            let self_active = self_direct
                .as_ref()
                .is_some_and(|direct| self.selected_direct.as_deref() == Some(&direct.id));
            let self_unread = self_direct.as_ref().is_some_and(|direct| {
                model::sequence(&direct.last_seq).unwrap_or(0)
                    > model::sequence(&direct.read_seq).unwrap_or(0)
            });
            ui.horizontal(|ui| {
                let display_name = account
                    .display_name
                    .as_deref()
                    .or(account.username.as_deref())
                    .unwrap_or("You");
                let name = format!("{display_name} you");
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width() - 18.0, 28.0),
                    egui::Sense::click(),
                );
                response.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::SelectableLabel,
                        ui.is_enabled(),
                        self_active,
                        &name,
                    )
                });
                if self_active || response.hovered() || response.has_focus() {
                    ui.painter().rect_filled(
                        rect,
                        6.0,
                        if self_active {
                            Color32::from_rgba_unmultiplied(182, 77, 50, 40)
                        } else {
                            RAISED
                        },
                    );
                }
                if response.has_focus() {
                    ui.painter().rect_stroke(
                        rect,
                        6.0,
                        Stroke::new(1.0, TERRACOTTA_BRIGHT),
                        egui::StrokeKind::Inside,
                    );
                }
                paint_avatar(
                    ui,
                    egui::Rect::from_center_size(
                        egui::pos2(rect.left() + 17.5, rect.center().y),
                        egui::vec2(20.0, 20.0),
                    ),
                    display_name,
                    account.avatar_id,
                );
                let mut label = egui::text::LayoutJob::default();
                label.append(
                    display_name,
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::new(
                            13.0,
                            egui::FontFamily::Name("Satoshi Medium".into()),
                        ),
                        color: if self_active { TEXT } else { MUTED },
                        ..Default::default()
                    },
                );
                label.append(
                    " you",
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::new(
                            12.0,
                            egui::FontFamily::Name("Satoshi Medium".into()),
                        ),
                        color: MUTED,
                        ..Default::default()
                    },
                );
                let galley = ui.painter().layout_job(label);
                ui.painter().with_clip_rect(rect).galley(
                    egui::pos2(
                        rect.left() + 35.0,
                        rect.center().y - galley.size().y / 2.0 - 1.0,
                    ),
                    galley,
                    TEXT,
                );
                if self_unread {
                    ui.label(RichText::new("●").size(9.0).color(TERRACOTTA_BRIGHT));
                }
                if response.clicked() {
                    self.select_or_create_self_direct();
                }
            });
            ui.add_space(2.0);
            let directs = self.directs.clone();
            for direct in directs {
                // Requests live under "Message requests" and never show unread.
                if direct.peer.id == account.id || direct.status == model::DirectStatus::Incoming {
                    continue;
                }
                let active = self.selected_direct.as_deref() == Some(&direct.id);
                let scope = Scope::Direct(direct.id.clone());
                let muted = self.muted_label(&scope);
                // A muted DM shows no unread dot.
                let unread = muted.is_none()
                    && model::sequence(&direct.last_seq).unwrap_or(0)
                        > model::sequence(&direct.read_seq).unwrap_or(0);
                ui.horizontal(|ui| {
                    let (response, opened) = channel_button(
                        ui,
                        egui::vec2(ui.available_width() - 18.0, 28.0),
                        &direct.peer.display_name,
                        None,
                        active,
                        RowExtras {
                            muted,
                            duration: None,
                            menu: Some(RowMenu {
                                label: format!(
                                    "Conversation options for {}",
                                    direct.peer.display_name
                                ),
                                on_hover: true,
                                content: Box::new(|ui: &mut egui::Ui| {
                                    self.notification_items(ui, &scope, "conversation")
                                }),
                            }),
                        },
                    );
                    paint_avatar(
                        ui,
                        egui::Rect::from_center_size(
                            egui::pos2(response.rect.left() + 17.5, response.rect.center().y),
                            egui::vec2(20.0, 20.0),
                        ),
                        &direct.peer.display_name,
                        direct.peer.avatar_id,
                    );
                    if unread {
                        ui.label(RichText::new("●").size(9.0).color(TERRACOTTA_BRIGHT));
                    }
                    if opened {
                        self.load_notifications();
                    } else if response.clicked() {
                        self.select_direct(direct.clone());
                    }
                });
                self.notification_error(ui, &scope);
                ui.add_space(2.0);
            }
            let action = if self.owner() {
                "Invite people"
            } else {
                "New message"
            };
            let (response, _) = channel_button(
                ui,
                egui::vec2(ui.available_width(), 28.0),
                action,
                Some(NavIcon::Plus),
                false,
                RowExtras::default(),
            );
            if response.clicked() {
                self.open_direct_action();
            }
        });
    }

    /// "⊘ N blocked messages — Show/Hide" for a run by blocked accounts. Returns
    /// whether the run is shown; the choice lives in memory only.
    fn blocked_row(&mut self, ui: &mut egui::Ui, key: &str, count: usize) -> bool {
        let shown = self.revealed_blocked.contains(key);
        egui::Frame::new()
            .inner_margin(egui::Margin::symmetric(18, 6))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // ⊘, painted: the bundled fonts have no glyph for it.
                    let (icon, _) =
                        ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                    let stroke = Stroke::new(1.2, MUTED);
                    ui.painter().circle_stroke(icon.center(), 5.0, stroke);
                    ui.painter().line_segment(
                        [
                            icon.center() + egui::vec2(-3.5, 3.5),
                            icon.center() + egui::vec2(3.5, -3.5),
                        ],
                        stroke,
                    );
                    ui.label(
                        RichText::new(format!("{} —", blocking::label(count)))
                            .size(12.0)
                            .color(MUTED),
                    );
                    let toggle = ui.add(
                        egui::Button::new(
                            RichText::new(if shown { "Hide" } else { "Show" })
                                .size(12.0)
                                .color(TEXT),
                        )
                        .frame(false),
                    );
                    if toggle.clicked() {
                        if shown {
                            self.revealed_blocked.remove(key);
                        } else {
                            self.revealed_blocked.insert(key.to_owned());
                        }
                    }
                });
            });
        self.revealed_blocked.contains(key)
    }

    /// Thread rows (root or replies), collapsing blocked authors' runs.
    fn messages_or_blocked(&mut self, ui: &mut egui::Ui, messages: &[&model::Message]) {
        let me = self.account.as_ref().map(|account| account.id.clone());
        for row in blocking::rows(messages, &self.blocked_ids(), me.as_deref()) {
            let range = match row {
                blocking::Row::Message(index) => index..index + 1,
                blocking::Row::Blocked { range, key } => {
                    if !self.blocked_row(ui, &key, range.len()) {
                        continue;
                    }
                    range
                }
            };
            for message in &messages[range] {
                self.message(ui, message, true);
            }
        }
    }

    /// Replaces the composer for an incoming request: who it is, then Accept,
    /// Decline or Block. Errors stay in the bar.
    fn request_bar(&mut self, ui: &mut egui::Ui, direct: &model::DirectConversation) {
        let plain = egui::TextFormat::simple(egui::FontId::proportional(13.0), MUTED);
        let mut copy = egui::text::LayoutJob::default();
        copy.append(
            &direct.peer.display_name,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::new(13.0, egui::FontFamily::Name("Satoshi Bold".into())),
                color: TEXT,
                ..Default::default()
            },
        );
        copy.append(
            &format!(
                " (@{}) wants to message you. You don't share a space.",
                direct.peer.username
            ),
            0.0,
            plain,
        );
        ui.label(copy);
        ui.add_space(8.0);
        let busy = self.request_busy;
        ui.horizontal(|ui| {
            if primary_button(ui, if busy { "Working…" } else { "Accept" }, !busy).clicked() {
                self.request_busy = true;
                self.request_error = None;
                self.account_op(AccountOperation::Accept(direct.id.clone()));
            }
            if secondary_button(ui, "Decline", !busy).clicked() {
                self.request_busy = true;
                self.request_error = None;
                self.account_op(AccountOperation::Decline(direct.id.clone()));
            }
            if secondary_button(ui, "Block", !busy).clicked() {
                self.confirm_block(Self::peer_account(&direct.peer), Some(direct.id.clone()));
            }
        });
        if let Some(error) = &self.request_error {
            ui.colored_label(ERROR, error);
        }
    }

    fn blocked_composer(&mut self, ui: &mut egui::Ui, direct: &model::DirectConversation) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("You blocked @{}.", direct.peer.username))
                    .size(13.0)
                    .color(MUTED),
            );
            if secondary_button(
                ui,
                if self.block_busy {
                    "Unblocking…"
                } else {
                    "Unblock"
                },
                !self.block_busy,
            )
            .clicked()
            {
                self.unblock(Self::peer_account(&direct.peer));
            }
        });
        if let Some(error) = &self.block_error {
            ui.colored_label(ERROR, error);
        }
    }

    /// "Message requests" with a count of incoming requests, expanding to the
    /// requests themselves. Opening one only reads it.
    fn request_navigation(&mut self, ui: &mut egui::Ui) {
        let requests = self.incoming_requests();
        if requests.is_empty() {
            return;
        }
        let open = self.requests_open
            || self
                .selected_request()
                .is_some_and(|request| requests.iter().any(|item| item.id == request.id));
        ui.horizontal(|ui| {
            let (response, _) = channel_button(
                ui,
                egui::vec2(ui.available_width() - 18.0, 28.0),
                "Message requests",
                Some(if open {
                    NavIcon::Chevron
                } else {
                    NavIcon::ChevronRight
                }),
                false,
                RowExtras::default(),
            );
            ui.label(
                RichText::new(requests.len().to_string())
                    .size(11.0)
                    .color(MUTED),
            );
            if response.clicked() {
                self.requests_open = !open;
            }
        });
        ui.add_space(2.0);
        if !open {
            return;
        }
        for request in requests {
            let active = self.selected_direct.as_deref() == Some(&request.id);
            let name = format!("{} @{}", request.peer.display_name, request.peer.username);
            let (rect, response) = ui.allocate_exact_size(
                egui::vec2(ui.available_width() - 18.0, 28.0),
                egui::Sense::click(),
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::SelectableLabel,
                    ui.is_enabled(),
                    active,
                    format!("Message request from {name}"),
                )
            });
            if active || response.hovered() || response.has_focus() {
                ui.painter().rect_filled(
                    rect,
                    6.0,
                    if active {
                        Color32::from_rgba_unmultiplied(182, 77, 50, 40)
                    } else {
                        RAISED
                    },
                );
            }
            paint_avatar(
                ui,
                egui::Rect::from_center_size(
                    egui::pos2(rect.left() + 29.5, rect.center().y),
                    egui::vec2(20.0, 20.0),
                ),
                &request.peer.display_name,
                request.peer.avatar_id,
            );
            let mut label = egui::text::LayoutJob::default();
            label.append(
                &request.peer.display_name,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::new(
                        13.0,
                        egui::FontFamily::Name("Satoshi Medium".into()),
                    ),
                    color: if active { TEXT } else { MUTED },
                    ..Default::default()
                },
            );
            label.append(
                &format!(" @{}", request.peer.username),
                0.0,
                egui::TextFormat::simple(egui::FontId::proportional(12.0), MUTED),
            );
            let galley = ui.painter().layout_job(label);
            ui.painter().with_clip_rect(rect).galley(
                egui::pos2(rect.left() + 47.0, rect.center().y - galley.size().y / 2.0),
                galley,
                TEXT,
            );
            if response.clicked() {
                self.select_direct(request);
            }
            ui.add_space(2.0);
        }
    }

    fn roster_for_active_call(&self) -> Vec<model::VoiceOccupant> {
        self.voice
            .participants
            .iter()
            .map(|participant| model::VoiceOccupant {
                id: participant.id.clone(),
                avatar_id: participant.avatar_id,
                name: participant.name.clone(),
                muted: participant.muted,
                deafened: participant.deafened,
            })
            .collect()
    }

    fn channel_voice_summary(&mut self, ui: &mut egui::Ui, id: &str, name: &str) {
        let own = self.voice.state.active_channel() == Some(id)
            && !matches!(self.voice.state.phase, Phase::Failed(_));
        let people = if own {
            self.roster_for_active_call()
        } else {
            self.channel_rosters.get(id).cloned().unwrap_or_default()
        };
        let open = self.expanded_rosters.contains(id);
        let connected = own && matches!(self.voice.state.phase, Phase::Connected(_));
        let authorizing = self
            .pending_voice_join
            .as_ref()
            .map(|(channel, _, _)| channel.as_str());
        let connecting = matches!(
            self.voice.state.phase,
            Phase::Joining(_) | Phase::Reconnecting(_)
        );
        let joining_here = authorizing == Some(id) || own && connecting;
        let switching = !matches!(self.voice.state.phase, Phase::Idle | Phase::Failed(_));
        let action = if joining_here {
            "Joining…"
        } else if switching {
            "Switch here"
        } else {
            "Join voice"
        };
        let enabled = authorizing.is_none() && !connecting && self.voice_target(id).is_some();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.add_space(9.0);
            let width = (ui.available_width() - 108.0 - 6.0).max(0.0);
            if !people.is_empty() {
                let (rect, stack) =
                    ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::click());
                let label = format!(
                    "{} in voice in {name}. {} who is in voice.",
                    people.len(),
                    if open { "Hide" } else { "Show" }
                );
                stack.widget_info(|| {
                    egui::WidgetInfo::selected(
                        egui::WidgetType::Button,
                        ui.is_enabled(),
                        open,
                        &label,
                    )
                });
                if stack.hovered() || stack.has_focus() {
                    ui.painter().rect_filled(rect, 8.0, RAISED);
                }
                if stack.has_focus() {
                    ui.painter().rect_stroke(
                        rect,
                        8.0,
                        Stroke::new(1.0, TERRACOTTA_BRIGHT),
                        egui::StrokeKind::Inside,
                    );
                }
                let font = egui::FontId::new(11.0, egui::FontFamily::Name("Satoshi Medium".into()));
                let mut count = format!("{} in voice", people.len());
                let mut text_width = ui
                    .painter()
                    .layout_no_wrap(count.clone(), font.clone(), MUTED)
                    .size()
                    .x;
                if text_width + 22.0 > width {
                    count = people.len().to_string();
                    text_width = ui
                        .painter()
                        .layout_no_wrap(count.clone(), font.clone(), MUTED)
                        .size()
                        .x;
                }
                // Drop faces before sacrificing the readable count at 220px.
                let faces = if width >= text_width + 61.0 {
                    people.len().min(2)
                } else {
                    0
                };
                let now = Instant::now();
                for (index, person) in people.iter().take(faces).enumerate() {
                    let center =
                        egui::pos2(rect.left() + 12.0 + index as f32 * 14.0, rect.center().y);
                    let muted = if person.id == self.voice.self_id {
                        self.voice.state.audio.muted
                    } else {
                        person.muted
                    };
                    let speaking = own && self.voice.speaking(&person.id, muted, now);
                    paint_avatar(
                        ui,
                        egui::Rect::from_center_size(center, egui::vec2(20.0, 20.0)),
                        &person.name,
                        person.avatar_id,
                    );
                    if speaking {
                        // Web: caper border plus a 1px caper ring.
                        ui.painter()
                            .circle_stroke(center, 11.0, Stroke::new(2.5, CAPER));
                    }
                }
                let count_x = rect.left()
                    + if faces == 0 {
                        2.0
                    } else {
                        faces as f32 * 14.0 + 13.0
                    };
                ui.painter().text(
                    egui::pos2(count_x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    count,
                    font,
                    MUTED,
                );
                paint_icon(
                    ui.painter(),
                    egui::Rect::from_center_size(
                        egui::pos2(count_x + text_width + 9.0, rect.center().y),
                        egui::vec2(14.0, 14.0),
                    ),
                    if open {
                        NavIcon::Chevron
                    } else {
                        NavIcon::ChevronRight
                    },
                    MUTED,
                );
                let stack = stack.on_hover_text(label);
                if stack.clicked() {
                    if open {
                        self.expanded_rosters.remove(id);
                    } else {
                        self.expanded_rosters.insert(id.into());
                    }
                }
            } else {
                ui.allocate_exact_size(egui::vec2(width, 28.0), egui::Sense::hover());
            }
            if connected {
                // Keep other rows stationary; disconnect belongs to the dock.
                ui.allocate_exact_size(egui::vec2(108.0, 28.0), egui::Sense::hover());
                return;
            }
            ui.add_enabled_ui(enabled, |ui| {
                let button = voice_join_button(ui, action);
                // Web's 120 px approach radius around Join.
                if !own
                    && ui.is_enabled()
                    && ui.ctx().pointer_hover_pos().is_some_and(|pointer| {
                        button.rect.distance_sq_to_pos(pointer) <= 120.0 * 120.0
                    })
                {
                    self.prepare_voice_join(id);
                }
                let label = if joining_here {
                    format!("Joining voice in #{name}")
                } else if switching {
                    format!("Switch voice to #{name}")
                } else {
                    format!("Join voice in #{name}")
                };
                button.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
                });
                let button = match self.join_unavailable(id).filter(|_| !own) {
                    Some(reason) => button.on_disabled_hover_text(reason),
                    None => button.on_hover_text(label),
                };
                if button.clicked() {
                    self.join_voice_channel(id);
                }
            });
        });
    }

    fn channel_voice_roster(&mut self, ui: &mut egui::Ui, id: &str) {
        if !self.expanded_rosters.contains(id) {
            return;
        }
        let own = self.voice.state.active_channel() == Some(id)
            && !matches!(self.voice.state.phase, Phase::Failed(_));
        let people = if own {
            self.roster_for_active_call()
        } else {
            self.channel_rosters.get(id).cloned().unwrap_or_default()
        };
        if !people.is_empty() {
            self.voice_roster(ui, people, own);
        }
    }

    fn voice_roster(&mut self, ui: &mut egui::Ui, people: Vec<model::VoiceOccupant>, own: bool) {
        let now = Instant::now();
        for participant in people {
            let is_self = own && participant.id == self.voice.self_id;
            let muted = if is_self {
                self.voice.state.audio.muted
            } else {
                participant.muted
            };
            let deafened = if is_self {
                self.voice.state.audio.deafened
            } else {
                participant.deafened
            };
            let mut playback = self.voice.playback(&participant.id);
            let local_muted = own && !is_self && playback.muted;
            ui.push_id(&participant.id, |ui| {
                // Web opens the participant's audio menu on right-click too.
                let menu_id = ui.id().with("audio-menu");
                ui.scope_builder(egui::UiBuilder::new().sense(egui::Sense::click()), |ui| {
                    if own && !is_self && ui.response().secondary_clicked() {
                        egui::Popup::open_id(ui.ctx(), menu_id);
                    }
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 7.0;
                        avatar(
                            ui,
                            &participant.name,
                            participant.avatar_id,
                            28.0,
                            own && self.voice.speaking(&participant.id, muted, now),
                        );
                        ui.allocate_ui_with_layout(
                            egui::vec2(
                                (ui.available_width()
                                    - if own && participant.id != self.voice.self_id {
                                        80.0
                                    } else {
                                        23.0
                                    }
                                    - if muted && deafened { 23.0 } else { 0.0 })
                                .max(0.0),
                                32.0,
                            ),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                ui.vertical(|ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    ui.add(
                                        egui::Label::new(
                                            bold(if is_self {
                                                format!("{} (you)", participant.name)
                                            } else {
                                                participant.name.clone()
                                            })
                                            .size(12.0),
                                        )
                                        .truncate(),
                                    );
                                    if local_muted {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 4.0;
                                            let (rect, _) = ui.allocate_exact_size(
                                                egui::vec2(10.0, 10.0),
                                                egui::Sense::hover(),
                                            );
                                            paint_icon(
                                                ui.painter(),
                                                rect,
                                                NavIcon::VolumeX,
                                                TERRACOTTA_BRIGHT,
                                            );
                                            ui.add(
                                                egui::Label::new(
                                                    RichText::new(format!(
                                                        "You muted {}",
                                                        participant.name
                                                    ))
                                                    .size(10.0)
                                                    .color(TERRACOTTA_BRIGHT),
                                                )
                                                .truncate(),
                                            );
                                        });
                                    }
                                });
                            },
                        );
                        if !muted && !deafened {
                            ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                        }
                        for (active, icon, label) in [
                            (muted, NavIcon::MicOff, "Muted"),
                            (deafened, NavIcon::HeadphoneOff, "Deafened"),
                        ] {
                            if active {
                                let (rect, response) = ui.allocate_exact_size(
                                    egui::vec2(16.0, 16.0),
                                    egui::Sense::hover(),
                                );
                                paint_icon(ui.painter(), rect, icon, MUTED);
                                response.widget_info(|| {
                                    egui::WidgetInfo::labeled(
                                        egui::WidgetType::Image,
                                        ui.is_enabled(),
                                        format!("{}: {label}", participant.name),
                                    )
                                });
                                response.on_hover_text(label);
                            }
                        }
                        if own && participant.id != self.voice.self_id {
                            let options = ui.add(
                                egui::Button::new(RichText::new("Audio").size(10.0).color(MUTED))
                                    .frame(false)
                                    .min_size(egui::vec2(46.0, 28.0)),
                            );
                            options.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Button,
                                    ui.is_enabled(),
                                    format!("Audio controls for {}", participant.name),
                                )
                            });
                            if options.clicked() {
                                self.effects.toggle(!egui::Popup::menu(&options).is_open());
                            }
                            egui::Popup::menu(&options)
                                .id(menu_id)
                                .width(240.0)
                                .show(|ui| {
                                    let label = ui.horizontal(|ui| {
                                        let label = ui.label(bold("User volume"));
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                ui.label(format!("{}%", playback.gain_percent));
                                            },
                                        );
                                        label
                                    });
                                    let volume = ui
                                        .add(
                                            egui::Slider::new(&mut playback.gain_percent, 0..=200)
                                                .show_value(false),
                                        )
                                        .labelled_by(label.inner.id);
                                    let muted = ui.checkbox(&mut playback.muted, "Mute");
                                    if volume.changed() {
                                        self.effects
                                            .slider(f32::from(playback.gain_percent) / 200.0);
                                    }
                                    if muted.changed() {
                                        self.effects.toggle(!playback.muted);
                                    }
                                    if volume.changed() || muted.changed() {
                                        self.voice
                                            .set_participant_playback(&participant.id, playback);
                                    }
                                    ui.label(
                                        RichText::new("Only changes what you hear.")
                                            .size(11.0)
                                            .color(MUTED),
                                    );
                                });
                        }
                    });
                });
            });
        }
    }

    fn account_bar(&mut self, ui: &mut egui::Ui) {
        if let Phase::Joining(context) | Phase::Connected(context) | Phase::Reconnecting(context) =
            &self.voice.state.phase
        {
            let label = format!("{} / {}", context.channel_name, context.space_name);
            let connected = matches!(self.voice.state.phase, Phase::Connected(_));
            let target = NavigationTarget {
                space: self.voice.active_space.clone(),
                channel: self
                    .voice
                    .active_space
                    .as_ref()
                    .map(|_| context.channel_id.clone()),
            };
            ui.add_space(8.0);
            full_bleed_separator(ui, ui.cursor().top());
            ui.add_space(8.0);
            let joining = matches!(self.voice.state.phase, Phase::Joining(_));
            let status_height = ui
                .fonts_mut(|fonts| {
                    fonts.row_height(&egui::FontId::new(
                        12.0,
                        egui::FontFamily::Name("Satoshi Bold".into()),
                    )) + fonts.row_height(&egui::FontId::proportional(11.0))
                        + 2.0
                })
                .max(28.0);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), status_height),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    let (icon, _) =
                        ui.allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
                    // Web: green when connected, amber while connecting or reconnecting.
                    let tone = if connected {
                        Color32::from_rgb(140, 178, 98)
                    } else {
                        Color32::from_rgb(217, 171, 92)
                    };
                    paint_icon(ui.painter(), icon, NavIcon::AudioLines, tone);
                    let status = ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.add(
                            egui::Label::new(
                                bold(if connected {
                                    "Voice connected"
                                } else if joining {
                                    "Connecting…"
                                } else {
                                    "Reconnecting…"
                                })
                                .size(12.0)
                                .color(tone),
                            )
                            .selectable(false),
                        );
                        ui.add(
                            egui::Label::new(RichText::new(&label).size(11.0).color(MUTED))
                                .selectable(false),
                        );
                    });
                    let open = status.response.interact(egui::Sense::click());
                    open.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
                    });
                    if open.clicked() {
                        self.navigate(target);
                    }
                    let hangup = ui
                        .with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            drawn_icon_button_with_tooltip(
                                ui,
                                NavIcon::PhoneOff,
                                if connected {
                                    "Leave voice"
                                } else {
                                    "Cancel joining voice"
                                },
                                if connected { "Disconnect" } else { "Cancel" },
                            )
                        })
                        .inner;
                    if hangup.clicked() {
                        if connected {
                            self.effects.play(Effect::Disconnect);
                        }
                        self.voice.leave();
                    }
                },
            );
            ui.add_space(8.0);
        }
        if let Some(error) = self.voice.error.clone() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::Label::new(RichText::new(error).size(11.0).color(ERROR))
                        .wrap()
                        .selectable(false),
                );
                if drawn_icon_button(ui, NavIcon::Close, "Dismiss voice error").clicked() {
                    self.voice.error = None;
                }
            });
            ui.add_space(8.0);
        }
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 42.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 6.0, RAISED);
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, BORDER),
            egui::StrokeKind::Inside,
        );
        let content = rect.shrink(5.0);
        let profile = egui::Rect::from_min_max(
            content.min,
            egui::pos2(content.right() - 122.0, content.bottom()),
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(profile)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                presence_avatar(
                    ui,
                    &self.identity_name(),
                    self.account.as_ref().and_then(|account| account.avatar_id),
                    30.0,
                    if self.live == "Live" {
                        "online"
                    } else {
                        "unknown"
                    },
                );
                ui.add(egui::Label::new(bold(self.identity_name()).size(12.8)).truncate());
            },
        );
        if ui
            .interact(profile, ui.id().with("profile"), egui::Sense::click())
            .on_hover_text(if self.account.is_some() {
                format!("Edit profile for {}", self.identity_name())
            } else {
                "Sign in to edit your profile".into()
            })
            .clicked()
        {
            self.dialog = Some(if self.account.is_some() {
                Dialog::Profile
            } else {
                Dialog::SignIn
            });
        }
        let controls = egui::Rect::from_min_max(
            egui::pos2(profile.right() + 2.0, content.top()),
            content.max,
        );
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(controls)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                let muted = self.voice.state.audio.muted;
                if audio_icon_button(
                    ui,
                    if muted { NavIcon::MicOff } else { NavIcon::Mic },
                    28.0,
                    if muted {
                        "Unmute microphone"
                    } else {
                        "Mute microphone"
                    },
                    muted,
                )
                .on_hover_text(if muted { "Unmute" } else { "Mute" })
                .clicked()
                {
                    self.voice.command(VoiceOperation::Mute(!muted));
                    self.effects.toggle(muted);
                }
                let input = audio_icon_button(ui, NavIcon::Chevron, 16.0, "Input Options", muted)
                    .on_hover_text("Input Options");
                if input.clicked() {
                    self.effects.toggle(!egui::Popup::menu(&input).is_open());
                    self.voice.refresh_devices();
                }
                egui::Popup::menu(&input)
                    .align(egui::RectAlign::TOP_START)
                    .width(232.0)
                    .show(|ui| self.device_options(ui, true));
                ui.add_space(2.0);
                let deafened = self.voice.state.audio.deafened;
                if audio_icon_button(
                    ui,
                    if deafened {
                        NavIcon::VolumeX
                    } else {
                        NavIcon::Headphones
                    },
                    28.0,
                    if deafened {
                        "Undeafen audio"
                    } else {
                        "Deafen audio"
                    },
                    deafened,
                )
                .on_hover_text(if deafened { "Undeafen" } else { "Deafen" })
                .clicked()
                {
                    self.voice.command(VoiceOperation::Deafen(!deafened));
                    self.effects.toggle(deafened);
                }
                let output =
                    audio_icon_button(ui, NavIcon::Chevron, 16.0, "Output Options", deafened)
                        .on_hover_text("Output Options");
                if output.clicked() {
                    self.effects.toggle(!egui::Popup::menu(&output).is_open());
                    self.voice.refresh_devices();
                }
                egui::Popup::menu(&output)
                    .align(egui::RectAlign::TOP_START)
                    .width(232.0)
                    .show(|ui| self.device_options(ui, false));
                ui.add_space(2.0);
                let settings =
                    audio_icon_button(ui, NavIcon::Settings, 28.0, "User Settings", false)
                        .on_hover_text("User Settings");
                if settings.clicked() {
                    self.effects.toggle(!egui::Popup::menu(&settings).is_open());
                }
                egui::Popup::menu(&settings)
                    .align(egui::RectAlign::TOP_END)
                    .width(232.0)
                    .show(|ui| {
                        if ui.button("Settings…").clicked() {
                            if self.persist_preferences {
                                match startup::enabled() {
                                    Ok(enabled) => self.launch_at_login = enabled,
                                    Err(error) => {
                                        self.startup_error = Some(format!(
                                            "Could not read startup settings: {error}"
                                        ));
                                    }
                                }
                            }
                            self.dialog = Some(Dialog::Settings);
                            if !self.privacy_saving {
                                self.privacy_error = None;
                                self.account_op(AccountOperation::LoadPrivacy);
                            }
                            self.account_op(AccountOperation::LoadBlocks);
                            self.load_notifications();
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Audio test").clicked() {
                            self.voice.refresh_devices();
                            self.dialog = Some(Dialog::Audio);
                            ui.close();
                        }
                        if self.voice.diagnostics.is_some()
                            && ui.button("Connection details").clicked()
                        {
                            self.connection_copy_status = "";
                            self.dialog = Some(Dialog::Connection);
                            ui.close();
                        }
                        if self
                            .account
                            .as_ref()
                            .is_some_and(|account| account.debug_enabled)
                            && ui.button("Audio diagnostics").clicked()
                        {
                            self.dialog = Some(Dialog::Diagnostics);
                            ui.close();
                        }
                        ui.separator();
                        // Web: Log out for accounts, Sign in for guests.
                        if self.account.is_some() {
                            if ui.button("Log out").clicked() {
                                ui.close();
                                self.logout();
                            }
                        } else if ui.button("Sign in").clicked() {
                            self.dialog = Some(Dialog::SignIn);
                            ui.close();
                        }
                    });
            },
        );
    }

    fn device_options(&mut self, ui: &mut egui::Ui, input: bool) {
        if !matches!(self.voice.microphone, MicrophoneState::Idle) {
            ui.label("End the microphone test to change devices.");
            return;
        }
        ui.label(
            bold(if input { "Microphone" } else { "Audio output" })
                .size(12.0)
                .color(MUTED),
        );
        let devices = if input {
            self.voice.inputs.clone()
        } else {
            self.voice.outputs.clone()
        };
        let selected = if input {
            &self.voice.preferences.input
        } else {
            &self.voice.preferences.output
        };
        if ui
            .selectable_label(selected.is_none(), "System default")
            .clicked()
        {
            self.voice.command(if input {
                VoiceOperation::DefaultInput
            } else {
                VoiceOperation::DefaultOutput
            });
        }
        if self.voice.refreshing_devices {
            ui.label("Finding devices…");
        } else if devices.is_empty() {
            ui.label("No devices found. Check system audio settings.");
        }
        if let Some(error) = &self.voice.device_error {
            ui.label(RichText::new(error).color(ERROR));
        }
        if ui
            .add_enabled(
                !self.voice.refreshing_devices,
                egui::Button::new("Refresh devices"),
            )
            .clicked()
        {
            self.voice.refresh_devices();
        }
        for (guid, name) in devices {
            let selected = if input {
                &self.voice.preferences.input
            } else {
                &self.voice.preferences.output
            };
            if ui
                .selectable_label(selected.as_deref() == Some(&guid), name)
                .clicked()
            {
                self.voice.command(if input {
                    VoiceOperation::Input(guid)
                } else {
                    VoiceOperation::Output(guid)
                });
                ui.close();
            }
        }
        ui.separator();
        if input {
            self.input_processing(ui);
        } else {
            self.output_gain(ui);
        }
    }

    fn input_processing(&mut self, ui: &mut egui::Ui) {
        let mut gain = self.voice.preferences.input_percent;
        let mut strength = self.voice.preferences.processing_strength;
        let gain_label = ui.label(bold("Input volume").size(13.0));
        let gain_changed = ui
            .add(egui::Slider::new(&mut gain, 0..=200).suffix("%"))
            .labelled_by(gain_label.id)
            .changed();
        let strength_label = ui.label(bold("Voice processing").size(13.0));
        let strength_changed = ui
            .add(egui::Slider::new(&mut strength, 0..=100).suffix("%"))
            .labelled_by(strength_label.id)
            .changed();
        if gain_changed || strength_changed {
            self.voice.set_input_processing(gain, strength);
            self.effects.slider(if gain_changed {
                f32::from(gain) / 200.0
            } else {
                f32::from(strength) / 100.0
            });
        }
    }

    fn output_gain(&mut self, ui: &mut egui::Ui) {
        let mut gain = self.voice.preferences.master_percent;
        let label = ui
            .horizontal(|ui| {
                let label = ui.label(bold("Output volume").size(13.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("{gain}%")).color(MUTED));
                });
                label
            })
            .inner;
        ui.add_space(6.0);
        let changed = ui
            .scope(|ui| {
                ui.spacing_mut().slider_width = ui.available_width();
                ui.add(
                    egui::Slider::new(&mut gain, 0..=200)
                        .show_value(false)
                        .trailing_fill(true),
                )
                .labelled_by(label.id)
                .changed()
            })
            .inner;
        if changed {
            self.voice.set_master_gain(gain);
            self.effects.slider(f32::from(gain) / 200.0);
        }
    }

    fn audio_test(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("Only you can hear these tests.")
                .size(12.8)
                .color(MUTED),
        );
        ui.add_space(20.0);
        // Web stacks the pickers below a 540px viewport.
        if ui.available_width() >= 500.0 {
            let spacing = ui.spacing().item_spacing.x;
            ui.spacing_mut().item_spacing.x = 24.0;
            ui.columns(2, |columns| {
                for (column, input) in columns.iter_mut().zip([true, false]) {
                    column.spacing_mut().item_spacing.x = spacing;
                    self.audio_test_device(column, input);
                }
            });
            ui.spacing_mut().item_spacing.x = spacing;
        } else {
            self.audio_test_device(ui, true);
            ui.add_space(20.0);
            self.audio_test_device(ui, false);
        }
        if let Some(error) = &self.voice.device_error {
            ui.add_space(8.0);
            ui.label(RichText::new(error).color(ERROR));
        }
        ui.add_space(24.0);
        ui.separator();
        ui.add_space(24.0);
        self.microphone_test(ui);
        if self
            .account
            .as_ref()
            .is_some_and(|account| account.debug_enabled)
        {
            ui.add_space(12.0);
            ui.collapsing("Audio diagnostics", |ui| self.audio_diagnostics(ui));
        }
    }

    fn audio_test_device(&mut self, ui: &mut egui::Ui, input: bool) {
        let (preferred, devices) = if input {
            (&self.voice.preferences.input, &self.voice.inputs)
        } else {
            (&self.voice.preferences.output, &self.voice.outputs)
        };
        let selected = preferred
            .as_ref()
            .map_or("System default", |id| {
                devices
                    .iter()
                    .find(|(guid, _)| guid == id)
                    .map_or("Saved device (unavailable)", |(_, name)| name.as_str())
            })
            .to_owned();
        let kind = if input { "Microphone" } else { "Speaker" };
        let label = ui.label(bold(kind).size(11.5).color(MUTED));
        ui.add_space(8.0);
        let button = ui
            .add_sized(
                [ui.available_width(), 36.0],
                egui::Button::new(RichText::new(selected).size(12.8))
                    .fill(RAISED)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(8),
            )
            .labelled_by(label.id);
        egui::Popup::menu(&button).width(300.0).show(|ui| {
            ui.add_enabled_ui(
                matches!(self.voice.microphone, MicrophoneState::Idle),
                |ui| self.device_options(ui, input),
            );
        });
        ui.add_space(16.0);
        if input {
            let mut gain = self.voice.preferences.input_percent;
            if volume_slider(ui, "Microphone volume", &mut gain) {
                let strength = self.voice.preferences.processing_strength;
                self.voice.set_input_processing(gain, strength);
                self.effects.slider(f32::from(gain) / 200.0);
            }
            return;
        }
        let mut gain = self.voice.preferences.master_percent;
        if volume_slider(ui, "Speaker volume", &mut gain) {
            self.voice.set_master_gain(gain);
            self.effects.slider(f32::from(gain) / 200.0);
        }
        ui.add_space(12.0);
        let testing = self.voice.speaker_testing;
        let speaker = ui
            .add_enabled_ui(self.persist_preferences, |ui| {
                outlined_button(
                    ui,
                    if testing {
                        "Stop speaker test"
                    } else {
                        "Test speakers"
                    },
                    testing,
                )
            })
            .inner;
        if speaker.clicked() {
            if testing {
                self.voice.stop_speaker_test();
            } else {
                self.voice.start_speaker_test();
            }
        }
        if self.voice.speaker_failed {
            ui.add_space(8.0);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.label(
                    RichText::new("Couldn’t play audio. Check your output and try again.")
                        .size(12.8)
                        .color(ERROR),
                )
            });
        }
    }

    fn microphone_test(&mut self, ui: &mut egui::Ui) {
        let state = self.voice.microphone.clone();
        let recording = match state {
            MicrophoneState::Recording(started) => Some(started),
            _ => None,
        };
        ui.horizontal(|ui| {
            ui.label(
                bold(if recording.is_some() {
                    "Recording…"
                } else {
                    "Try your microphone"
                })
                .size(16.0),
            );
            if let Some(started) = recording {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        bold(format!("{:.1}s", started.elapsed().as_secs_f32().min(30.0)))
                            .size(14.0),
                    );
                    let (dot, _) =
                        ui.allocate_exact_size(egui::vec2(20.0, 20.0), egui::Sense::hover());
                    ui.painter().circle_filled(
                        dot.center(),
                        10.0,
                        TERRACOTTA_BRIGHT.gamma_multiply(0.14),
                    );
                    ui.painter()
                        .circle_filled(dot.center(), 5.0, TERRACOTTA_BRIGHT);
                });
            }
        });
        ui.add_space(12.0);
        ui.label(RichText::new("Less noise. Clearer voice.").color(MUTED));
        if !self.persist_preferences {
            ui.add_space(8.0);
            ui.label(
                RichText::new("TEST FIXTURE — no recording or playback.")
                    .size(11.0)
                    .color(MUTED),
            );
        }
        ui.add_space(24.0);
        egui::Frame::new()
            .fill(BLACKOUT)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(8)
            .inner_margin(16)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                let mut strength = self.voice.preferences.processing_strength;
                let label = ui
                    .horizontal(|ui| {
                        let label = ui.label(bold("Voice enhancement").size(13.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(bold(format!("{strength}%")).color(TERRACOTTA_BRIGHT));
                        });
                        label
                    })
                    .inner;
                let changed = ui
                    .add_enabled_ui(recording.is_none(), |ui| {
                        ui.spacing_mut().slider_width = ui.available_width();
                        ui.add(
                            egui::Slider::new(&mut strength, 0..=100)
                                .show_value(false)
                                .trailing_fill(true),
                        )
                        .labelled_by(label.id)
                        .changed()
                    })
                    .inner;
                if changed {
                    let gain = self.voice.preferences.input_percent;
                    self.voice.set_input_processing(gain, strength);
                    self.effects.slider(f32::from(strength) / 100.0);
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Natural").size(10.9).color(MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("Enhanced").size(10.9).color(MUTED));
                    });
                });
            });
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            let (text, busy) = match state {
                MicrophoneState::Recording(_) => ("Stop recording", false),
                MicrophoneState::Preparing | MicrophoneState::Processing(_) => ("Preparing…", true),
                _ => ("Test microphone", false),
            };
            let button = ui.add_enabled(
                self.persist_preferences && !busy,
                egui::Button::new(bold(text).color(Color32::WHITE))
                    .fill(TERRACOTTA)
                    .stroke(Stroke::new(1.0, TERRACOTTA))
                    .corner_radius(8)
                    .min_size(egui::vec2(160.0, 48.0)),
            );
            if button.clicked() {
                if recording.is_some() {
                    self.voice.finish_mic_recording();
                } else {
                    self.voice.start_mic_test();
                }
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 7.0;
                ui.label(bold("Input level").size(11.2).color(MUTED));
                self.voice.sample_input_meter(Instant::now());
                input_meter(ui, &self.voice.input_meter, recording.is_some());
            });
        });
        if recording.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(80));
        }
        if let Some(error) = &self.voice.microphone_error {
            ui.add_space(8.0);
            ui.label(RichText::new(error).color(ERROR));
        }
        let (recorded, playing, processing) = match state {
            MicrophoneState::Ready(recorded) => (recorded, None, false),
            MicrophoneState::Playing { recorded, enhanced } => (recorded, Some(enhanced), false),
            MicrophoneState::Processing(recorded) => (recorded, None, true),
            _ => return,
        };
        ui.add_space(16.0);
        let mut action = None;
        let mut sample = |ui: &mut egui::Ui, enhanced: bool| {
            let latest = enhanced && !processing;
            egui::Frame::new()
                .fill(BLACKOUT)
                .stroke(Stroke::new(
                    1.0,
                    if latest {
                        Color32::from_rgb(83, 99, 63)
                    } else {
                        BORDER
                    },
                ))
                .corner_radius(8)
                .inner_margin(14)
                .show(ui, |ui| {
                    ui.set_min_size(egui::vec2(ui.available_width(), 74.0));
                    let name = if enhanced { "Enhanced" } else { "Natural" };
                    ui.label(bold(name).size(13.0));
                    ui.add_space(10.0);
                    if enhanced && processing {
                        ui.label(RichText::new("Preparing…").size(12.0).color(MUTED));
                        return;
                    }
                    ui.horizontal(|ui| {
                        let this = playing == Some(enhanced);
                        let play = ui.add_enabled(
                            self.persist_preferences && !processing && (playing.is_none() || this),
                            egui::Button::new(if this { "Stop" } else { "Play" })
                                .min_size(egui::vec2(64.0, 30.0)),
                        );
                        play.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                play.enabled(),
                                format!(
                                    "{} {} audio sample",
                                    if this { "Stop" } else { "Play" },
                                    name
                                ),
                            )
                        });
                        if play.clicked() {
                            action = Some((enhanced, this));
                        }
                        ui.label(RichText::new(format!("{:.1}s", recorded.seconds)).color(MUTED));
                    });
                    if recorded.silent {
                        ui.add_space(8.0);
                        // Column layouts justify wrapped labels; web does not.
                        ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                            ui.label(
                                RichText::new(
                                    "No audible signal detected. Check your mic and try again.",
                                )
                                .size(12.0)
                                .color(ERROR),
                            )
                        });
                    }
                });
        };
        if ui.available_width() >= 500.0 {
            let spacing = ui.spacing().item_spacing.x;
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.columns(2, |columns| {
                for (column, enhanced) in columns.iter_mut().zip([false, true]) {
                    column.spacing_mut().item_spacing.x = spacing;
                    sample(column, enhanced);
                }
            });
            ui.spacing_mut().item_spacing.x = spacing;
        } else {
            sample(ui, false);
            ui.add_space(10.0);
            sample(ui, true);
        }
        match action {
            Some((_, true)) => self.voice.stop_mic_playback(),
            Some((enhanced, false)) => self.voice.play_mic_sample(enhanced),
            None => {}
        }
    }

    fn audio_diagnostics(&mut self, ui: &mut egui::Ui) {
        if !self
            .account
            .as_ref()
            .is_some_and(|account| account.debug_enabled)
        {
            return;
        }
        ui.label("Local diagnostics only. No audio, device identifiers, or credentials. Nothing is uploaded.");
        let processing = self.voice.audio_processing_report();
        if processing.is_none() {
            ui.label("No microphone capture started. Open Mic Test or join voice first.");
        }
        let report = serde_json::to_string_pretty(&serde_json::json!({
            "platform": std::env::consts::OS,
            "processing": processing,
        }))
        .expect("numeric diagnostics serialize");
        if ui.button("Copy diagnostics").clicked() {
            ui.ctx().copy_text(report.clone());
            self.diagnostics_copied = true;
        }
        if self.diagnostics_copied {
            ui.label(RichText::new("Copied diagnostics").color(MUTED));
        }
        ui.add(egui::Label::new(RichText::new(report).monospace()).selectable(true));
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    }

    fn connection_details(&mut self, ui: &mut egui::Ui) {
        if !self.persist_preferences {
            ui.label(
                RichText::new("TEST FIXTURE — synthetic statistics, no live connection.")
                    .color(MUTED),
            );
            ui.add_space(12.0);
        }
        let Some((stats, _)) = &self.voice.diagnostics else {
            ui.label(if matches!(self.voice.state.phase, Phase::Idle) {
                "Join voice to see connection details."
            } else {
                "Waiting for connection statistics…"
            });
            return;
        };
        let report = ConnectionReport::new(self.voice.join_times.as_ref(), stats);
        egui::Grid::new("connection-statistics")
            .num_columns(2)
            .spacing([28.0, 12.0])
            .show(ui, |ui| {
                for (label, value) in report.rows() {
                    ui.label(RichText::new(label).color(MUTED));
                    ui.label(value);
                    ui.end_row();
                }
            });
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            if ui.button("Copy connection details").clicked() {
                // Web copies its diagnostics object as indented JSON.
                match serde_json::to_string_pretty(&report) {
                    Ok(json) => {
                        ui.ctx().copy_text(json);
                        self.connection_copy_status = "Copied connection details";
                    }
                    Err(_) => self.connection_copy_status = "Copy failed; try again.",
                }
            }
            ui.label(RichText::new(self.connection_copy_status).color(MUTED));
        });
    }

    fn member_presence(&mut self, ui: &mut egui::Ui) {
        let Some(detail) = &self.detail else { return };
        let member_count = detail.members.len();
        let members = detail.members.clone();
        let demo = detail.space.demo;
        egui::Frame::new()
            .fill(SIDEBAR)
            .show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        ui.spacing_mut().item_spacing.y = 0.0;
        let bounds = ui.max_rect();
        ui.painter().vline(bounds.left(), bounds.y_range(), Stroke::new(1.0, BORDER));
        let (heading, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(),54.0), egui::Sense::hover());
        ui.painter().text(egui::pos2(heading.left()+12.0,heading.center().y), egui::Align2::LEFT_CENTER,"Members", egui::FontId::new(12.0,egui::FontFamily::Name("Satoshi Bold".into())), MUTED);
            if !demo {
                ui.painter().text(egui::pos2(heading.right()-12.0,heading.center().y), egui::Align2::RIGHT_CENTER,member_count.to_string(),egui::FontId::proportional(10.4),MUTED);
            }
        ui.painter().hline(heading.x_range(), heading.bottom(), Stroke::new(1.0, BORDER));
        ui.add_space(8.0);
        if demo {
            ui.label(
                RichText::new("General is open to everyone. People in voice appear in the channel sidebar.")
                    .size(11.0)
                    .color(MUTED),
            );
            return;
        }
        let page_count = member_count.div_ceil(MEMBER_PAGE_SIZE);
        let members: Vec<_> = members
            .iter()
            .skip(self.member_page * MEMBER_PAGE_SIZE)
            .take(MEMBER_PAGE_SIZE)
            .cloned()
            .collect();
        if members.is_empty() {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(RichText::new("No members to show.").size(11.0).color(MUTED));
            });
        }
        for member in members {
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),44.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add_space(16.0);
                let status = self
                    .presence
                    .get(&member.id)
                    .map_or("unknown", String::as_str);
                presence_avatar(ui, &member.display_name, member.avatar_id, 30.0, status);
                ui.label(bold(&member.display_name).size(12.0));
            });
        }
        if page_count > 1 {
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.member_page > 0, egui::Button::new("Previous"))
                    .clicked()
                {
                    self.member_page -= 1;
                    self.connect_gateway();
                }
                ui.label(
                    RichText::new(format!("{} / {page_count}", self.member_page + 1))
                        .size(10.0)
                        .color(MUTED),
                );
                if ui
                    .add_enabled(self.member_page + 1 < page_count, egui::Button::new("Next"))
                    .clicked()
                {
                    self.member_page += 1;
                    self.connect_gateway();
                }
            });
        }
            });
    }

    /// An account space whose channels are all hidden from this member.
    fn no_accessible_channels(&self) -> bool {
        self.selected_direct.is_none()
            && ((self.account.is_some() && self.selected_channel.is_none())
                || self.detail.as_ref().is_some_and(|detail| {
                    !detail.space.demo
                        && !detail.channels.iter().any(|channel| channel.joined)
                        && !detail
                            .channels
                            .iter()
                            .any(|channel| Some(&channel.id) == self.selected_channel.as_ref())
                }))
    }

    /// Web's `.empty-channel` stage.
    fn empty_channels(&mut self, ui: &mut egui::Ui, narrow: bool) {
        egui::Frame::new().fill(CONVERSATION).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            let owner = self.owner();
            let no_spaces = !self.spaces.iter().any(|space| !space.demo);
            let height = if owner { 170.0 } else { 110.0 } + if narrow { 56.0 } else { 0.0 };
            ui.vertical_centered(|ui| {
                ui.add_space(((ui.available_height() - height) / 2.0).max(24.0));
                if narrow && navigation_toggle(ui, NavIcon::Hash, "Browse spaces").clicked() {
                    self.navigation_open = true;
                }
                if narrow {
                    ui.add_space(20.0);
                }
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(32.0, 32.0), egui::Sense::hover());
                paint_icon(ui.painter(), rect, NavIcon::Hash, TERRACOTTA_BRIGHT);
                ui.add_space(14.0);
                ui.label(
                    bold(if no_spaces {
                        "Select a direct message"
                    } else {
                        "No joined channels"
                    })
                    .size(20.0),
                );
                ui.add_space(7.0);
                ui.label(
                    RichText::new(if no_spaces {
                        "Open a conversation from Direct messages."
                    } else if owner {
                        "Create a channel to start a conversation."
                    } else {
                        "Browse public channels or accept a private invitation."
                    })
                    .size(13.0)
                    .color(MUTED),
                );
                if owner {
                    ui.add_space(18.0);
                    if ui
                        .add(
                            egui::Button::new(
                                bold("Create channel").size(12.5).color(Color32::WHITE),
                            )
                            .fill(TERRACOTTA)
                            .stroke(Stroke::new(1.0, TERRACOTTA))
                            .corner_radius(8)
                            .min_size(egui::vec2(0.0, 38.0)),
                        )
                        .clicked()
                    {
                        self.form_name.clear();
                        self.form_private = false;
                        self.error = None;
                        self.dialog = Some(Dialog::CreateChannel);
                    }
                }
            });
        });
    }

    fn conversation(&mut self, ui: &mut egui::Ui, narrow: bool) {
        if self.thread_view.is_some() && narrow {
            self.thread_panel(ui);
        } else {
            if self.thread_view.is_some() {
                egui::SidePanel::right("message-thread")
                    .default_width(340.0)
                    .min_width(300.0)
                    .max_width(480.0)
                    .show_inside(ui, |ui| self.thread_panel(ui));
            }
            self.channel_conversation(ui, narrow);
        }
        // Once per pass, for pills in the channel and in thread replies.
        self.mention_card(ui.ctx());
    }

    fn thread_panel(&mut self, ui: &mut egui::Ui) {
        let Some(thread) = &self.thread_view else {
            return;
        };
        let root = thread.root.clone();
        let loading = thread.loading;
        let has_more = thread.has_more;
        let error = thread.error.clone();
        let broadcast_label = format!("Also send to #{}", self.channel_name());
        ui.set_min_height(ui.available_height());
        ui.horizontal(|ui| {
            ui.heading("Thread");
            if ui.button("Back to channel").clicked() {
                self.thread_view = None;
                self.thread_request += 1;
            }
        });
        ui.label(format!("in #{}", self.channel_name()));
        ui.separator();
        if self.selected_is_joined() {
            egui::TopBottomPanel::bottom("thread-composer").show_inside(ui, |ui| {
                let pending = self.pending.clone().filter(|pending| pending.thread_root_id.as_deref() == Some(&root));
                if let Some(pending) = &pending {
                    ui.label(&pending.text);
                    if let Some(error) = &pending.rejection {
                        ui.colored_label(ERROR, format!("Not sent. {error}"));
                        ui.horizontal(|ui| {
                            if ui.add_enabled(self.thread_drafts.entry(root.clone()).or_default().0.is_empty(), egui::Button::new("Edit")).clicked()
                                && let Some(text) = self.discard_rejected() { self.thread_drafts.entry(root.clone()).or_default().0 = text; }
                            if ui.button("Dismiss").clicked() { self.discard_rejected(); }
                        });
                    } else if !pending.sending && ui.button("Retry send").clicked() { self.send_message_to(Some(root.clone()), pending.broadcast); }
                }
                let blocked = self.pending.is_some();
                if blocked && pending.is_none() { ui.label("Confirm or dismiss the pending message first."); }
                let draft = self.thread_drafts.entry(root.clone()).or_default();
                let output = ui.add_enabled(!loading, egui::TextEdit::multiline(&mut draft.0)
                    .id_salt(("thread-draft", &root)).desired_rows(3).desired_width(f32::INFINITY).char_limit(4000)
                    .hint_text("Reply to thread…").return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter))));
                let enter = output.has_focus() && ui.input(|input| !input.events.iter().any(|event| matches!(event, egui::Event::Ime(_))) && input.events.iter().any(|event| matches!(event, egui::Event::Key { key: egui::Key::Enter, pressed: true, modifiers, .. } if !modifiers.shift)));
                ui.add_enabled(!blocked, egui::Checkbox::new(&mut draft.1, &broadcast_label));
                let draft = self.thread_drafts.get(&root).cloned().unwrap_or_default();
                let send = ui.add_enabled(!blocked && !loading && !draft.0.trim().is_empty(), egui::Button::new("Send reply")).clicked();
                if (enter || send) && !blocked && !loading { self.send_message_to(Some(root.clone()), draft.1); }
            });
        } else {
            ui.label("Join the channel to reply.");
        }
        egui::ScrollArea::vertical()
            .id_salt(("thread-history", &root))
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let rows: Vec<_> = self
                    .timeline
                    .messages()
                    .filter(|message| {
                        message.id == root || message.thread_root_id.as_deref() == Some(&root)
                    })
                    .cloned()
                    .collect();
                if let Some(parent) = rows.iter().find(|message| message.id == root) {
                    self.messages_or_blocked(ui, &[parent]);
                }
                if loading {
                    ui.label("Loading thread…");
                }
                if let Some(error) = error {
                    ui.colored_label(ERROR, error);
                    if ui.button("Retry").clicked() {
                        self.load_thread(false);
                    }
                }
                if has_more
                    && ui
                        .add_enabled(!loading, egui::Button::new("Load older replies"))
                        .clicked()
                {
                    self.load_thread(true);
                }
                let replies: Vec<_> = rows
                    .iter()
                    .filter(|message| message.thread_root_id.is_some())
                    .collect();
                if replies.is_empty() && !loading {
                    ui.label("No replies yet. Start the thread.");
                }
                self.messages_or_blocked(ui, &replies);
            });
        self.emoji_picker(ui.ctx());
    }

    fn channel_conversation(&mut self, ui: &mut egui::Ui, narrow: bool) {
        if self.no_accessible_channels() {
            if self.opening || self.navigation_error.is_some() {
                egui::Frame::new().fill(CONVERSATION).show(ui, |ui| {
                    ui.set_min_size(ui.available_size());
                    self.navigation_state(ui);
                });
            } else {
                self.empty_channels(ui, narrow);
            }
            return;
        }
        egui::Frame::new().fill(CONVERSATION).show(ui, |ui| {
            ui.set_min_width(320.0);
            ui.set_height(ui.available_height());
            ui.spacing_mut().item_spacing.y = 0.0;
            let heading = egui::TopBottomPanel::top("chat-heading")
                .exact_height(54.0)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(CONVERSATION)
                        .inner_margin(egui::Margin::symmetric(18, 8)),
                )
                .show_inside(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 38.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            if narrow && navigation_toggle(ui, NavIcon::Menu, "Browse").clicked() {
                                self.navigation_open = true;
                            }
                            let direct = self.selected_direct.as_ref().and_then(|id| self.directs.iter().find(|item| &item.id == id));
                            ui.label(RichText::new(match direct {
                                Some(item) => item.peer.display_name.clone(),
                                None => format!("# {}", self.channel_name()),
                            }).size(13.76));
                            if self.showing_pins {
                                ui.label(RichText::new("Pinned messages").size(12.0).color(MUTED));
                            }
                            // Web: a failed refresh keeps the conversation and offers Retry in the header.
                            if let Some(error) = self.load_error.clone().filter(|_| self.timeline.messages().next().is_some()) {
                                ui.add(egui::Label::new(RichText::new(error).size(11.2).color(ERROR)).truncate());
                                if ui.small_button("Retry").clicked() {
                                    self.reload_channel();
                                }
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let pin_count = self.mutations.pinned(&self.timeline).len();
                                    if self.showing_pins {
                                        if ui.button("Messages").clicked() { self.showing_pins = false; }
                                    } else if ui.button(format!("Pins ({pin_count})")).clicked() {
                                        self.showing_pins = true;
                                    }
                                    if self.selected_direct.is_none() {
                                        if let (Some(space), Some(channel)) = (self.selected_space.clone(), self.selected_channel.clone())
                                            && let Some(entry) = self.detail.as_ref().and_then(|detail| detail.channels.iter().find(|item| item.id == channel)).cloned()
                                            && !entry.joined
                                            && ui.button("Join channel").clicked()
                                        {
                                            self.admin(AdminOperation::JoinChannel { space, channel });
                                        }
                                        if self
                                            .detail
                                            .as_ref()
                                            .is_some_and(|detail| !detail.members.is_empty())
                                            && users_button(ui, if narrow { self.narrow_members_visible } else { self.members_visible }).clicked()
                                        {
                                            if narrow { self.narrow_members_visible = !self.narrow_members_visible; }
                                            else { self.members_visible = !self.members_visible; }
                                        }
                                    } else if let Some(direct) = self.selected_direct_conversation().cloned()
                                        && self.account.as_ref().is_some_and(|account| account.id != direct.peer.id)
                                    {
                                        // 1:1 DMs only; personal notes have no peer to block.
                                        let blocked = direct.blocked || self.is_blocked(&direct.peer.id);
                                        let account = Self::peer_account(&direct.peer);
                                        if ui.add_enabled(!self.block_busy, egui::Button::new(if blocked { "Unblock" } else { "Block" })).clicked() {
                                            if blocked {
                                                self.unblock(account);
                                            } else {
                                                let request = (direct.status == model::DirectStatus::Incoming).then(|| direct.id.clone());
                                                self.confirm_block(account, request);
                                            }
                                        }
                                    }
                                },
                            );
                        },
                    );
                });
            ui.painter().hline(
                heading.response.rect.x_range(),
                heading.response.rect.bottom(),
                Stroke::new(1.0, BORDER),
            );
            // A persisted bottom-panel height otherwise constrains the scroll
            // viewport to the previous draft's size, even when the text grows.
            let draft_height = ui.fonts_mut(|fonts| fonts.layout(
                self.draft.clone(), egui::FontId::proportional(13.6), TEXT,
                (ui.available_width() - 36.0 - 22.0).max(1.0),
            ).size().y);
            let editor_height = (draft_height + 20.0).clamp(42.0, (ui.ctx().viewport_rect().height() * 0.4).min(320.0));
            let session_row = if self.session_error.is_some() { 24.0 } else { 0.0 };
            let joined = self.selected_is_joined();
            let composer = egui::TopBottomPanel::bottom("composer")
                .min_height(if joined { editor_height + 24.0 + session_row } else { 44.0 })
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(CONVERSATION)
                        .inner_margin(egui::Margin::symmetric(18, 12)),
                )
                .show_inside(ui, |ui| {
                    if self.opening || self.navigation_error.is_some() {
                        ui.disable();
                    }
                    ui.visuals_mut().widgets.inactive.corner_radius = CornerRadius::same(6);
                    ui.visuals_mut().widgets.hovered.corner_radius = CornerRadius::same(6);
                    ui.visuals_mut().widgets.active.corner_radius = CornerRadius::same(6);
                    if let Some(error) = &self.error {
                        ui.colored_label(ERROR, error);
                    }
                    let me = self.account.as_ref().map(|account| account.id.clone());
                    if let Some(direct) = self.selected_direct_conversation().cloned()
                        && Some(&direct.peer.id) != me.as_ref()
                    {
                        // A request is read-only until accepted; a block replaces the composer.
                        if direct.status == model::DirectStatus::Incoming {
                            self.request_bar(ui, &direct);
                            return;
                        }
                        if direct.blocked || self.is_blocked(&direct.peer.id) {
                            self.blocked_composer(ui, &direct);
                            return;
                        }
                        if direct.status == model::DirectStatus::Outgoing {
                            ui.label(RichText::new(format!("Waiting for @{} to accept. They'll see your messages when they do.", direct.peer.username)).size(12.0).color(MUTED));
                            ui.add_space(6.0);
                        }
                    }
                    // Web: the conversation stays; only sending waits on a new session.
                    if let Some(root) = self.pending.as_ref().and_then(|pending| pending.thread_root_id.clone())
                        && ui.button("Pending reply · Open thread").clicked() { self.open_thread(root); }
                    if !joined {
                        ui.label(bold("Preview").size(12.0));
                        let format = egui::TextFormat {
                            font_id: egui::FontId::proportional(12.0),
                            color: MUTED,
                            ..Default::default()
                        };
                        let mut copy = egui::text::LayoutJob::default();
                        copy.append("Join ", 0.0, format.clone());
                        copy.append(&format!("#{}", self.channel_name()), 0.0, egui::TextFormat {
                            font_id: egui::FontId::new(12.0, egui::FontFamily::Name("Satoshi Bold".into())),
                            ..format.clone()
                        });
                        copy.append(" to interact with people here", 0.0, format);
                        ui.label(copy);
                        return;
                    }
                    if let Some(error) = self.session_error.clone() {
                        ui.horizontal(|ui| {
                            ui.colored_label(ERROR, error);
                            if ui.small_button("Retry session").clicked() {
                                self.worker.send(Command::CreateSession {
                                    generation: self.generation,
                                    token: self.token.clone(),
                                    name: self.identity_name(),
                                });
                            }
                        });
                    }
                    let before = self.draft.clone();
                    let placeholder = if self.selected_direct.is_some() {
                        format!("Message {}", self.channel_name())
                    } else {
                        format!("Message #{}", self.channel_name())
                    };
                    let composer_id = egui::Id::new("message-composer");
                    let ime_frame = ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Ime(_))));
                    ui.input(|input| {
                        for event in &input.events {
                            if let egui::Event::Ime(event) = event {
                                self.ime_composing = matches!(event, egui::ImeEvent::Enabled | egui::ImeEvent::Preedit(_));
                            }
                        }
                    });
                    let cursor = egui::TextEdit::load_state(ui.ctx(), composer_id)
                        .and_then(|state| state.cursor.char_range());
                    if let Some((text, caret)) = &self.suggestion_dismissed
                        && (text != &self.draft || cursor.is_none_or(|range| !range.is_empty() || range.primary.index != *caret)) {
                        self.suggestion_dismissed = None;
                    }
                    let active = cursor.filter(|range| range.is_empty())
                        .filter(|_| !self.ime_composing && !ime_frame && ui.memory(|memory| memory.has_focus(composer_id)))
                        .and_then(|range| ComposerToken::at(&self.draft, range.primary.index))
                        .filter(|token| self.suggestion_dismissed.as_ref() != Some(&(self.draft.clone(), token.end())));
                    if active != self.suggestion_token { self.suggestion_selected = 0; self.suggestion_token = active.clone(); }
                    let choices = active.as_ref().map(|token| self.suggestions(token)).unwrap_or_default();
                    let mut chosen = None;
                    if !choices.is_empty() {
                        self.suggestion_selected = self.suggestion_selected.min(choices.len() - 1);
                        ui.input_mut(|input| {
                            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) { self.suggestion_selected = (self.suggestion_selected + 1) % choices.len(); }
                            if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) { self.suggestion_selected = (self.suggestion_selected + choices.len() - 1) % choices.len(); }
                            if input.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || input.consume_key(egui::Modifiers::NONE, egui::Key::Tab) { chosen = Some(choices[self.suggestion_selected].clone()); }
                            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) { self.suggestion_dismissed = active.as_ref().map(|token| (self.draft.clone(), token.end())); }
                        });
                    }
                    let editor = egui::ScrollArea::vertical()
                        .id_salt("composer-scroll")
                        .max_height((ui.ctx().viewport_rect().height() * 0.4).min(320.0))
                        .min_scrolled_height(42.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui|
                        egui::TextEdit::multiline(&mut self.draft)
                            .id(composer_id)
                            .desired_width(f32::INFINITY)
                            .min_size(egui::vec2(0.0, 42.0))
                            .desired_rows(1)
                            .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)))
                            .font(egui::FontId::proportional(13.6))
                            .margin(egui::vec2(11.0, 10.0))
                            .hint_text(
                                RichText::new(placeholder)
                                    .color(Color32::from_rgb(142, 149, 152)),
                            )
                            .background_color(COMPOSER)
                            .char_limit(4_000).show(ui)
                    );
                    let mut output = editor.inner;
                    let response = &output.response;
                    let active = output.cursor_range.filter(|range| range.is_empty())
                        .filter(|_| response.has_focus() && !self.ime_composing && !ime_frame)
                        .and_then(|range| ComposerToken::at(&self.draft, range.primary.index))
                        .filter(|token| self.suggestion_dismissed.as_ref() != Some(&(self.draft.clone(), token.end())));
                    let choices = active.as_ref().map(|token| self.suggestions(token)).unwrap_or_default();
                    // One popup serves `:` emoji and `@` mention rows.
                    if !choices.is_empty() && chosen.is_none() {
                        egui::Area::new(egui::Id::new("composer-suggestions"))
                            .order(egui::Order::Foreground).pivot(egui::Align2::LEFT_BOTTOM)
                            .fixed_pos(response.rect.left_top() - egui::vec2(0.0, 6.0))
                            .show(ui.ctx(), |ui| {
                                egui::Frame::new().fill(COMPOSER).stroke(Stroke::new(1.0, BORDER)).corner_radius(8).inner_margin(4).show(ui, |ui| {
                                    let width = response.rect.width().min(260.0) - 8.0;
                                    ui.set_width(width);
                                    ui.spacing_mut().icon_spacing = 12.0;
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    for (index, choice) in choices.iter().enumerate() {
                                        let fill = if index == self.suggestion_selected { Color32::from_rgb(67, 36, 30) } else { Color32::TRANSPARENT };
                                        let clicked = match choice {
                                            Suggestion::Emoji(entry) => {
                                                let image = self.reaction_textures.image(ui, entry, 24.0);
                                                let label = format!(":{}:", entry.name.replace(' ', "_"));
                                                let button = egui::Button::image_and_text(image, label)
                                                    .min_size(egui::vec2(width, 44.0))
                                                    .truncate()
                                                    .stroke(Stroke::NONE)
                                                    .fill(fill);
                                                ui.add(button).clicked()
                                            }
                                            Suggestion::Mention(candidate) => mention_suggestion(ui, candidate, width, fill).clicked(),
                                        };
                                        if clicked { chosen = Some(choice.clone()); }
                                    }
                                });
                            });
                    }
                    if let Some(choice) = &chosen
                        && let Some(token) = active.as_ref()
                        && let Some((value, caret)) = token.insert(&self.draft, choice) {
                        self.draft = value;
                        output.state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(caret))));
                        output.state.store(ui.ctx(), composer_id);
                        response.request_focus();
                        self.suggestion_token = None;
                    }
                    // egui processes focus traversal before widgets handle keys.
                    // Keep Tab/Escape in the editor while suggestions are open.
                    let lock_suggestions = chosen.is_none() && !choices.is_empty();
                    ui.memory_mut(|memory| memory.set_focus_lock_filter(composer_id, egui::EventFilter {
                        horizontal_arrows: true, vertical_arrows: true,
                        tab: lock_suggestions, escape: lock_suggestions,
                    }));
                    ui.painter().rect_stroke(
                        editor.inner_rect,
                        6.0,
                        Stroke::new(1.0, BORDER),
                        egui::StrokeKind::Inside,
                    );
                    if self.draft != before {
                        self.typing_edited = Instant::now();
                    }
                    let send = response.has_focus() && !self.ime_composing && !ime_frame
                        && ui.input(|input| {
                            // The modifier belongs to the key event, not the end of
                            // the frame (Shift may already have been released).
                            input.events.iter().any(|event| matches!(event,
                                egui::Event::Key { key: egui::Key::Enter, pressed: true, repeat: false, modifiers, .. } if !modifiers.shift
                            ))
                        });
                    if send {
                        self.send_message();
                    }
                    let count = self.draft.chars().count();
                    if count >= 3_000 {
                        ui.add_space(7.0);
                        ui.label(
                            RichText::new(format!("{} / 4,000", grouped(count)))
                                .size(10.24)
                                .color(counter_tone(count)),
                        );
                    }
                });
            ui.painter().hline(
                composer.response.rect.x_range(),
                composer.response.rect.top(),
                Stroke::new(1.0, BORDER),
            );
            egui::TopBottomPanel::bottom("typing")
                .exact_height(20.0)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(CONVERSATION)
                        .inner_margin(egui::Margin::symmetric(18, 2)),
                )
                .show_inside(ui, |ui| {
                    let names: Vec<_> = self
                        .typers
                        .values()
                        .filter(|typer| typer.typing)
                        .map(|typer| typer.author.name.as_str())
                        .collect();
                    if !names.is_empty() {
                        ui.horizontal_centered(|ui| {
                        ui.spacing_mut().item_spacing.x = 7.0;
                        typing_dots(ui);
                        ui.label(
                            RichText::new(if names.len() > 2 {
                                "Several people are typing…".into()
                            } else {
                                format!(
                                    "{} {} typing…",
                                    names.join(" and "),
                                    if names.len() == 1 { "is" } else { "are" }
                                )
                            })
                            .size(11.0)
                            .color(MUTED),
                        );
                        });
                    }
                });
            // Keep loading/retry inside the existing message viewport. Do not
            // touch the retained history's scroll state or paging anchors.
            if self.navigation_state(ui) {
                return;
            }
            let empty = self.timeline.messages().next().is_none() && self.pending.is_none();
            if self.loading && empty {
                message_skeleton(ui);
                return;
            }
            // Web's End key on the message list jumps to the latest message.
            let jump_latest = ui.memory(|memory| memory.focused().is_none())
                && ui.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::End));
            let mut history = egui::ScrollArea::vertical()
                .id_salt("history")
                .stick_to_bottom(true)
                .auto_shrink([false, false]);
            if let Some(offset) = self.history_offset.take() {
                history = history.vertical_scroll_offset(offset);
            } else if jump_latest {
                history = history.vertical_scroll_offset(f32::MAX);
            }
            let history = history.show(ui, |ui| {
                    if self.showing_pins {
                        let pins = self.mutations.pinned(&self.timeline);
                        if pins.is_empty() {
                            chat_state(ui, 2, |ui| { ui.label(RichText::new("No pinned messages.").color(MUTED)); });
                        } else {
                            for message in &pins { self.message(ui, message, false); }
                        }
                        return;
                    }
                    if let Some(error) = self.load_error.clone().filter(|_| empty) {
                        chat_state(ui, 2, |ui| {
                            ui.label(RichText::new(error).color(MUTED));
                            ui.add_space(10.0);
                            ui.spacing_mut().button_padding = egui::vec2(8.0, 5.0);
                            if ui
                                .add(
                                    egui::Button::new(bold("Try again").size(11.52))
                                        .fill(RAISED)
                                        .stroke(Stroke::new(1.0, BORDER))
                                        .corner_radius(8),
                                )
                                .clicked()
                            {
                                self.reload_channel();
                            }
                        });
                        return;
                    }
                    // Web's history header: older-page status above the messages.
                    self.history_header(ui);
                    // `message` needs mutable access to the app for reaction
                    // controls, so iterating through `self.timeline` directly
                    // would borrow `self` twice. Move it out only while drawing
                    // rows instead of deep-cloning every loaded message (including
                    // content and reactions) on every frame. Restore it before
                    // pending/empty state and paging can inspect the timeline.
                    let timeline = std::mem::take(&mut self.timeline);
                    let mut last_date = None;
                    let messages: Vec<_> = timeline.messages().filter(|message| message.is_channel_message() && !self.thread_only_rows.contains(&message.id)).collect();
                    let me = self.account.as_ref().map(|account| account.id.clone());
                    for row in blocking::rows(&messages, &self.blocked_ids(), me.as_deref()) {
                        let (range, blocked) = match row {
                            blocking::Row::Message(index) => (index..index + 1, None),
                            blocking::Row::Blocked { range, key } => (range, Some(key)),
                        };
                        if let Some(key) = blocked {
                            if let Some(date) = display_date(&messages[range.start].created_at)
                                && take_date_divider(&mut last_date, &date.key)
                            {
                                date_divider(ui, &date.label);
                            }
                            if !self.blocked_row(ui, &key, range.len()) {
                                continue;
                            }
                        }
                        for message in &messages[range] {
                            if let Some(date) = display_date(&message.created_at)
                                && take_date_divider(&mut last_date, &date.key)
                            {
                                date_divider(ui, &date.label);
                            }
                            self.message(ui, message, false);
                        }
                    }
                    self.timeline = timeline;
                    if let Some(pending) = self.pending.clone().filter(|pending| pending.thread_root_id.is_none()) {
                        if let Some(pending_date) = display_date(&pending.created_at)
                            && take_date_divider(&mut last_date, &pending_date.key)
                        {
                            date_divider(ui, &pending_date.label);
                        }
                        let author = self.session.as_ref().map_or_else(
                            || self.identity_name(),
                            |session| session.author.name.clone(),
                        );
                        let avatar_id = self.session.as_ref().and_then(|session| session.author.avatar_id);
                        message_row(ui, &author, avatar_id, "Now", false, |ui| {
                            ui.label(RichText::new(&pending.text).size(14.0).color(MUTED));
                        });
                        if let Some(rejection) = &pending.rejection {
                            egui::Frame::new().inner_margin(egui::Margin { left: 62, right: 18, top: 0, bottom: 8 }).show(ui, |ui| {
                                ui.colored_label(ERROR, format!("Not sent. {rejection}"));
                                ui.horizontal(|ui| {
                                    let button = |label| egui::Button::new(RichText::new(label).size(12.0)).fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0, BORDER)).corner_radius(8).min_size(egui::vec2(60.0, 32.0));
                                    if ui.add_enabled(self.draft.is_empty(), button("Edit"))
                                        .on_disabled_hover_text("Clear your current draft to edit this message.").clicked()
                                        && let Some(text) = self.discard_rejected() {
                                        self.draft = text;
                                        ui.memory_mut(|memory| memory.request_focus(egui::Id::new("message-composer")));
                                    }
                                    if ui.add(button("Dismiss")).clicked() { self.discard_rejected(); }
                                });
                            });
                        } else if !pending.sending {
                            ui.horizontal(|ui| {
                                ui.colored_label(ERROR, "Not confirmed yet.");
                                if ui.button("Retry send").clicked() {
                                    self.send_message();
                                }
                            });
                        }
                    }
                    if self.timeline.messages().next().is_none()
                        && self.pending.is_none()
                        && !self.loading
                    {
                        ui.centered_and_justified(|ui| {
                            ui.vertical_centered(|ui| {
                                ui.label("No messages yet.");
                                ui.label(
                                    RichText::new(if self.selected_direct.is_some() {
                                        "Only you and this person can read this conversation.".to_owned()
                                    } else {
                                        format!("Start the conversation in #{}.", self.channel_name())
                                    })
                                    .color(MUTED),
                                );
                            });
                        });
                    }
                });
            self.emoji_picker(ui.ctx());
            self.after_history(ui, &history, heading.response.rect);
        });
    }

    fn navigation_state(&mut self, ui: &mut egui::Ui) -> bool {
        if self.opening {
            message_skeleton(ui);
            return true;
        }
        let Some(error) = self.navigation_error.clone() else {
            return false;
        };
        chat_state(ui, 2, |ui| {
            ui.set_max_width((ui.available_width() - 36.0).min(440.0));
            ui.label(RichText::new(error).color(ERROR));
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Retry opening").clicked()
                    && let Some(target) = self.navigation_target.clone()
                {
                    self.navigate(target);
                }
                if ui.button("Dismiss").clicked() {
                    self.navigation_error = None;
                    self.navigation_target = None;
                }
            });
        });
        true
    }

    fn history_header(&mut self, ui: &mut egui::Ui) {
        let font = egui::FontId::proportional(11.52);
        let width = |ui: &egui::Ui, text: &str| {
            ui.fonts_mut(|fonts| {
                fonts
                    .layout_no_wrap(text.into(), font.clone(), MUTED)
                    .size()
                    .x
            })
        };
        let button = |text: &str| {
            egui::Button::new(bold(text).size(11.52).color(MUTED))
                .fill(Color32::TRANSPARENT)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(8)
        };
        ui.allocate_ui_with_layout(
            egui::vec2(ui.available_width(), 44.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_height(44.0);
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.spacing_mut().button_padding = egui::vec2(10.0, 6.0);
                if self.older_error.is_some() {
                    let label = "Couldn’t load older messages.";
                    let total = width(ui, label) + 8.0 + width(ui, "Retry") + 20.0;
                    ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                    ui.label(RichText::new(label).font(font.clone()).color(MUTED));
                    if ui.add(button("Retry")).clicked() {
                        self.load_older();
                    }
                } else if self.has_more {
                    let label = if self.loading_older {
                        "Loading…"
                    } else {
                        "Load older messages"
                    };
                    ui.add_space(((ui.available_width() - width(ui, label) - 20.0) / 2.0).max(0.0));
                    if ui.add_enabled(!self.loading_older, button(label)).clicked() {
                        self.load_older();
                    }
                } else {
                    let label = "Beginning of conversation";
                    ui.add_space(((ui.available_width() - width(ui, label)) / 2.0).max(0.0));
                    ui.label(RichText::new(label).font(font.clone()).color(MUTED));
                }
            },
        );
    }

    fn after_history(
        &mut self,
        ui: &mut egui::Ui,
        history: &egui::scroll_area::ScrollAreaOutput<()>,
        heading: egui::Rect,
    ) {
        let height = history.content_size.y;
        let viewport = history.inner_rect.height();
        let offset = history.state.offset.y;
        if let Some(previous) = self.older_anchor.take() {
            // Keep the reader's place after older messages arrive above.
            self.history_offset = Some(offset + (height - previous).max(0.0));
            ui.ctx().request_repaint();
        }
        self.history_height = height;
        if self.timeline.messages().next().is_some()
            && (height <= viewport + 1.0 || offset >= height - viewport - 2.0)
        {
            self.older_armed = true;
        }
        // Web loads the previous page when the list reaches its start.
        if self.older_armed
            && self.history_offset.is_none()
            && offset <= 1.0
            && self.has_more
            && !self.loading_older
            && self.older_error.is_none()
        {
            self.load_older();
        }
        // Web shows the connection state under the header after a second.
        if self.live != "Live" && self.selected_channel.is_some() {
            let waited = self.live_changed.elapsed();
            if waited >= Duration::from_secs(1) {
                let text = if self.load_error.is_some() || self.live == "Offline" {
                    "Offline"
                } else {
                    "Connecting…"
                };
                let galley = ui.painter().layout_no_wrap(
                    text.into(),
                    egui::FontId::new(11.2, egui::FontFamily::Name("Satoshi Bold".into())),
                    MUTED,
                );
                let anchor = egui::pos2(heading.right() - 18.0, heading.bottom() + 8.0);
                let rect = egui::Rect::from_min_size(
                    anchor - egui::vec2(galley.size().x + 12.0, 0.0),
                    galley.size() + egui::vec2(12.0, 6.0),
                );
                ui.painter().rect_filled(rect, 4.0, CONVERSATION);
                ui.painter()
                    .galley(rect.min + egui::vec2(6.0, 3.0), galley, MUTED);
            } else {
                ui.ctx()
                    .request_repaint_after(Duration::from_secs(1) - waited);
            }
        }
    }

    fn message(&mut self, ui: &mut egui::Ui, message: &model::Message, in_thread: bool) {
        let projected = self.mutations.project(message);
        let message = projected.as_ref();
        let time = if self.showing_pins {
            DateTime::parse_from_rfc3339(&message.created_at).map_or_else(
                |_| message.created_at.clone(),
                |date| {
                    date.with_timezone(&Local)
                        .format("%b %-d, %Y · %-I:%M %p")
                        .to_string()
                },
            )
        } else {
            display_time(&message.created_at)
        };
        let time = if message.revision > 1 {
            format!("{time} (edited)")
        } else {
            time
        };
        let mentioned = mentions::mentions_me(
            &message.content.mentions,
            &message.author.id,
            self.account.as_ref().map(|account| account.id.as_str()),
        );
        // Reserved beneath the row so the tint can be sized after drawing.
        let tint = ui.painter().add(egui::Shape::Noop);
        let pinned = message.pin.as_ref();
        let shown = egui::Frame::new()
            .fill(
                if !in_thread
                    && self
                        .thread_view
                        .as_ref()
                        .is_some_and(|thread| thread.root == message.id)
                {
                    Color32::from_rgba_unmultiplied(228, 199, 106, 26)
                } else if pinned.is_some() {
                    Color32::from_rgba_unmultiplied(228, 199, 106, 15)
                } else {
                    Color32::TRANSPARENT
                },
            )
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                if let Some(pin) = pinned {
                    egui::Frame::new()
                        .inner_margin(egui::Margin {
                            left: 62,
                            right: 74,
                            top: 6,
                            bottom: 0,
                        })
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(format!("Pinned by {}", pin.author.name))
                                    .size(11.0)
                                    .color(Color32::from_rgb(228, 199, 106)),
                            );
                        });
                }
                let (timestamp, pill) = message_row(
                    ui,
                    &message.author.name,
                    message.author.avatar_id,
                    &time,
                    message.author.is_guest,
                    |ui| {
                        message_body(
                            ui,
                            &message.id,
                            &message.content.text,
                            &message.content.mentions,
                            |entry| {
                                let name = self.mention_profile(entry).map_or_else(
                                    || {
                                        format!(
                                            "@{}",
                                            entry.username.as_deref().unwrap_or_default()
                                        )
                                    },
                                    |profile| profile.title(),
                                );
                                format!("Open profile for {name}")
                            },
                        )
                    },
                );
                if message.forward.is_none() && message.revision > 1 {
                    let marker = ui.interact(
                        timestamp,
                        ui.id().with(("edit-history", &message.id)),
                        egui::Sense::click(),
                    );
                    marker.widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::WidgetType::Button,
                            true,
                            "View edit history",
                        )
                    });
                    if marker.on_hover_text("View edit history").clicked() {
                        self.open_edit_history(message);
                    }
                }
                pill
            });
        let message_rect = shown.response.rect;
        if let Some(pill) = shown.inner {
            // Opening another pill replaces the card.
            self.mention_card = Some(MentionCard {
                pill,
                opening: None,
                error: None,
                fresh: true,
            });
        }
        if let Some(token) = self.token.clone() {
            // Hover only: sensing clicks here, above the row, would swallow
            // clicks inside it (the edited marker). Right-clicks still open it.
            let row = ui.interact(
                message_rect,
                ui.id().with((&message.id, "forward-context")),
                egui::Sense::hover(),
            );
            let opened = row.contains_pointer() && ui.input(|i| i.pointer.secondary_clicked());
            egui::Popup::menu(&row)
                .open_memory(opened.then_some(egui::SetOpenCommand::Bool(true)))
                .at_pointer_fixed()
                .show(|ui| {
                    if ui.button("Forward message").clicked() {
                        self.forwarding.picker(
                            &self.worker,
                            self.generation,
                            token,
                            message.clone(),
                        );
                        ui.close();
                    }
                });
        }
        if let Some(forward) = &message.forward {
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 62,
                    right: 18,
                    top: 4,
                    bottom: 8,
                })
                .show(ui, |ui| {
                    egui::Frame::new()
                        .stroke(Stroke::new(1.0, BORDER))
                        .corner_radius(8)
                        .inner_margin(12)
                        .show(ui, |ui| {
                            ui.label(RichText::new("Forwarded · live").size(11.0).color(MUTED));
                            if let Some(original) = &forward.message {
                                forwarding::original(ui, original, &mut self.reaction_textures);
                                let count = original
                                    .thread
                                    .as_ref()
                                    .map(|summary| {
                                        format!(
                                            "{} {} · ",
                                            summary.reply_count,
                                            if summary.reply_count == 1 {
                                                "reply"
                                            } else {
                                                "replies"
                                            }
                                        )
                                    })
                                    .unwrap_or_default();
                                if ui.button(format!("{count}View conversation")).clicked()
                                    && let Some(token) = self.token.clone()
                                {
                                    self.forwarding.conversation(
                                        &self.worker,
                                        self.generation,
                                        token,
                                        message.clone(),
                                    );
                                }
                            } else {
                                ui.label("Original conversation unavailable.");
                            }
                        });
                });
        }
        let author = self
            .session
            .as_ref()
            .map(|session| session.author.id.clone());
        let reactions = projected_reactions(
            &message.reactions,
            &message.id,
            author.as_deref(),
            &self.pending_reactions,
        );
        let can_react = self.selected_is_joined() && self.session.is_some();
        if can_react {
            let action_width = if in_thread { 50.0 } else { 76.0 };
            let actions_rect = egui::Rect::from_min_size(
                egui::pos2(
                    message_rect.right() - action_width - 18.0,
                    message_rect.top() + 4.0,
                ),
                egui::vec2(action_width, 24.0),
            );
            // Overlay controls must not move the timeline cursor back into the message.
            let mut actions_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("message-actions", &message.id))
                    .max_rect(actions_rect),
            );
            actions_ui.spacing_mut().item_spacing.x = 2.0;
            actions_ui.horizontal(|ui| {
                let reply = if in_thread {
                    None
                } else {
                    let rect = ui.allocate_space(egui::vec2(24.0, 24.0)).1;
                    Some(ui.interact(
                        rect,
                        ui.id().with((&message.id, "thread-action")),
                        egui::Sense::click(),
                    ))
                };
                let emoji_rect = ui.allocate_space(egui::vec2(24.0, 24.0)).1;
                let emoji = ui.interact(
                    emoji_rect,
                    ui.id().with((&message.id, "reaction-action")),
                    egui::Sense::click(),
                );
                let more_rect = ui.allocate_space(egui::vec2(24.0, 24.0)).1;
                let more = ui.interact(
                    more_rect,
                    ui.id().with((&message.id, "more-action")),
                    egui::Sense::click(),
                );
                let visible = ui.rect_contains_pointer(message_rect)
                    || emoji.has_focus()
                    || more.has_focus()
                    || reply.as_ref().is_some_and(|reply| reply.has_focus());
                if visible {
                    if let Some(reply) = &reply {
                        paint_icon(ui.painter(), reply.rect.shrink(5.0), NavIcon::Speech, MUTED);
                    }
                    if let Some(entry) = emoji::find("🙂") {
                        self.reaction_textures
                            .image(ui, entry, 14.0)
                            .paint_at(ui, emoji_rect.shrink(5.0));
                    }
                    paint_icon(ui.painter(), more_rect.shrink(5.0), NavIcon::More, MUTED);
                }
                if let Some(reply) = reply {
                    reply.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Reply in thread")
                    });
                    if reply.on_hover_text("Reply in thread").clicked() {
                        self.open_thread(
                            message
                                .thread_root_id
                                .clone()
                                .unwrap_or_else(|| message.id.clone()),
                        );
                    }
                }
                emoji.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Add reaction")
                });
                more.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Message actions")
                });
                if emoji.on_hover_text("Add reaction").clicked() {
                    self.reaction_picker = Some(message.id.clone());
                    self.reaction_search.clear();
                    self.reaction_search_focus = true;
                }
                let active = message.pin.is_some();
                egui::Popup::menu(&more).show(|ui| {
                    if ui.button("Forward message").clicked()
                        && let Some(token) = self.token.clone()
                    {
                        self.forwarding.picker(
                            &self.worker,
                            self.generation,
                            token,
                            message.clone(),
                        );
                        ui.close();
                    }
                    if self.can_edit(message) && ui.button("Edit message").clicked() {
                        self.open_editor(message);
                        ui.close();
                    }
                    if message.forward.is_none()
                        && message.revision > 1
                        && ui.button("View edit history").clicked()
                    {
                        self.open_edit_history(message);
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            !self.pending_pins.contains(&message.id),
                            egui::Button::new(if active { "Unpin" } else { "Pin" }),
                        )
                        .clicked()
                    {
                        self.set_pin(message, !active);
                        ui.close();
                    }
                    if let Some(account) = self.blockable_author(&message.author) {
                        let name = if account.username.is_empty() {
                            account.display_name.clone()
                        } else {
                            format!("@{}", account.username)
                        };
                        if self.is_blocked(&account.id) {
                            if ui.button(format!("Unblock {name}")).clicked() {
                                self.unblock(account);
                                ui.close();
                            }
                        } else if ui.button(format!("Block {name}")).clicked() {
                            self.confirm_block(account, None);
                            ui.close();
                        }
                    }
                });
            });
        }
        let controls = egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 62,
                right: 18,
                top: 2,
                bottom: 5,
            })
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for reaction in &reactions {
                        let owned = author
                            .as_ref()
                            .is_some_and(|id| reaction.author_ids.iter().any(|entry| entry == id));
                        let Some(entry) = emoji::find(&reaction.emoji) else {
                            continue;
                        };
                        let image = self.reaction_textures.image(ui, entry, 18.0);
                        let label = format!(
                            "{}, {} {}{}",
                            entry.name,
                            reaction.author_ids.len(),
                            if reaction.author_ids.len() == 1 {
                                "reaction"
                            } else {
                                "reactions"
                            },
                            if owned { ", including you" } else { "" }
                        );
                        let response = ui.add_enabled(
                            can_react,
                            egui::Button::image_and_text(
                                image,
                                RichText::new(reaction.author_ids.len().to_string()).size(11.0),
                            )
                            .fill(if owned {
                                Color32::from_rgb(57, 35, 30)
                            } else {
                                RAISED
                            })
                            .stroke(Stroke::new(
                                1.0,
                                if owned { TERRACOTTA_BRIGHT } else { BORDER },
                            ))
                            .corner_radius(8),
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::WidgetType::Button,
                                can_react,
                                label.clone(),
                            )
                        });
                        self.reactor_tooltip(&response, message, reaction, entry);
                        if response.clicked() {
                            self.set_reaction(&message.id, &reaction.emoji, !owned);
                        }
                    }
                    if self.showing_pins
                        && can_react
                        && ui
                            .add_enabled(
                                !self.pending_pins.contains(&message.id),
                                egui::Button::new("Unpin").small(),
                            )
                            .clicked()
                    {
                        self.set_pin(message, false);
                    }
                    if !in_thread
                        && self.selected_request().is_none()
                        && ui.small_button("Reply in thread").clicked()
                    {
                        self.open_thread(
                            message
                                .thread_root_id
                                .clone()
                                .unwrap_or_else(|| message.id.clone()),
                        );
                    }
                });
                if let Some((active, error)) = self.pin_errors.get(&message.id).cloned() {
                    ui.horizontal(|ui| {
                        ui.colored_label(ERROR, error);
                        if can_react && ui.small_button("Retry").clicked() {
                            self.set_pin(message, active);
                        }
                        if ui.small_button("Dismiss").clicked() {
                            self.pin_errors.remove(&message.id);
                        }
                    });
                }
                if !in_thread
                    && message.thread_root_id.is_none()
                    && let Some(summary) = &message.thread
                {
                    ui.horizontal(|ui| {
                        for author in &summary.participants {
                            avatar(ui, &author.name, author.avatar_id, 24.0, false);
                        }
                        if ui
                            .small_button(format!(
                                "{} {} · View thread",
                                summary.reply_count,
                                if summary.reply_count == 1 {
                                    "reply"
                                } else {
                                    "replies"
                                }
                            ))
                            .clicked()
                        {
                            self.open_thread(message.id.clone());
                        }
                    });
                }
                if let Some(error) = self.reaction_errors.get(&message.id).cloned() {
                    ui.horizontal(|ui| {
                        ui.colored_label(ERROR, error);
                        if can_react
                            && ui.small_button("Retry").clicked()
                            && let Some(((message, emoji), pending)) = self
                                .pending_reactions
                                .iter()
                                .find(|((id, _), pending)| id == &message.id && !pending.visible)
                                .map(|(key, value)| (key.clone(), value.clone()))
                        {
                            self.set_reaction(&message, &emoji, pending.desired);
                        }
                        if ui.small_button("Dismiss").clicked() {
                            self.reaction_errors.remove(&message.id);
                            self.pending_reactions
                                .retain(|(id, _), pending| id != &message.id || pending.visible);
                        }
                    });
                }
            });
        if mentioned {
            // Messages that mention you: an 8% terracotta row with a 2px edge.
            let rect = egui::Rect::from_x_y_ranges(
                ui.max_rect().x_range(),
                message_rect.top()..=controls.response.rect.bottom(),
            );
            ui.painter().set(
                tint,
                egui::Shape::rect_filled(rect, 0.0, TERRACOTTA.gamma_multiply(0.08)),
            );
            ui.painter().rect_filled(
                egui::Rect::from_min_size(rect.min, egui::vec2(2.0, rect.height())),
                0.0,
                TERRACOTTA,
            );
        }
    }

    /// Hovering (or keyboard-focusing) a reaction chip shows who reacted,
    /// Discord-style: the emoji large, then the shared summary. Clicking
    /// still toggles the reaction and hides the card, as for every tooltip.
    fn reactor_tooltip(
        &mut self,
        chip: &egui::Response,
        message: &model::Message,
        reaction: &model::Reaction,
        entry: &'static emoji::Entry,
    ) {
        const BELOW: [egui::RectAlign; 1] = [egui::RectAlign::BOTTOM_START];
        let mut tooltip = egui::Tooltip::for_widget(chip);
        tooltip.popup = tooltip
            .popup
            .open(chip.has_focus() || egui::Tooltip::should_show_tooltip(chip, false))
            .align(egui::RectAlign::TOP_START)
            .align_alternatives(&BELOW)
            .width(REACTOR_TOOLTIP_WIDTH)
            .frame(
                egui::Frame::popup(&chip.ctx.style())
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(8)
                    .inner_margin(10),
            );
        tooltip.show(|ui| {
            // Only runs while the card is showing, so names load on demand.
            self.load_reactors(message);
            let summary = self.reactor_summary(message, reaction);
            ui.set_max_width(REACTOR_TOOLTIP_WIDTH - 20.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let image = self
                    .reaction_textures
                    .image(ui, entry, REACTOR_TOOLTIP_EMOJI);
                ui.add(image);
                ui.add(egui::Label::new(RichText::new(summary).size(13.0).color(TEXT)).wrap());
            });
        });
    }

    /// The signed-in person, as reaction snapshots name them.
    fn reactor_self_id(&self) -> Option<&str> {
        self.session
            .as_ref()
            .map(|session| session.author.id.as_str())
            .or_else(|| self.account.as_ref().map(|account| account.id.as_str()))
    }

    /// Starts reading who reacted to `message` unless its current reaction
    /// revision is cached or already loading.
    fn load_reactors(&mut self, message: &model::Message) {
        let revision = message.reaction_seq.clone().unwrap_or_else(|| "0".into());
        if self
            .reactors
            .get(&message.id)
            .is_some_and(|cached| !cached.wants_request(&revision))
        {
            return;
        }
        if self.reactors.len() >= REACTOR_CACHE_LIMIT && !self.reactors.contains_key(&message.id) {
            self.reactors.clear();
        }
        if !self.persist_preferences {
            // Fixtures never contact Caper: name the labelled fixture members.
            let members = self
                .detail
                .as_ref()
                .map_or(&[][..], |detail| detail.members.as_slice());
            let reactions = message
                .reactions
                .iter()
                .map(|reaction| model::ReactorGroup {
                    emoji: reaction.emoji.clone(),
                    authors: reaction
                        .author_ids
                        .iter()
                        .map(|id| {
                            let member = members.iter().find(|member| member.id == *id);
                            model::Reactor {
                                id: id.clone(),
                                username: member.map(|member| member.username.clone()),
                                display_name: member.map(|member| member.display_name.clone()),
                                avatar_id: member.and_then(|member| member.avatar_id),
                            }
                        })
                        .collect(),
                })
                .collect();
            self.reactors.insert(
                message.id.clone(),
                ReactorCache {
                    revision,
                    state: ReactorState::Loaded(reactions),
                },
            );
            return;
        }
        self.reactors.insert(
            message.id.clone(),
            ReactorCache {
                revision: revision.clone(),
                state: ReactorState::Loading(Instant::now()),
            },
        );
        self.worker.send(Command::LoadReactors {
            generation: self.generation,
            token: self.token.clone(),
            channel: message.channel_id.clone(),
            message: message.id.clone(),
            revision,
        });
    }

    /// Names once they load for this exact set of people (a pending toggle
    /// of your own changes the set); the snapshot's count until then.
    fn reactor_summary(&self, message: &model::Message, reaction: &model::Reaction) -> String {
        let self_id = self.reactor_self_id();
        let name = emoji::name(&reaction.emoji);
        let revision = message.reaction_seq.as_deref().unwrap_or("0");
        let loaded = self
            .reactors
            .get(&message.id)
            .filter(|cached| cached.revision == revision)
            .and_then(|cached| match &cached.state {
                ReactorState::Loaded(groups) => {
                    groups.iter().find(|group| group.emoji == reaction.emoji)
                }
                _ => None,
            })
            .filter(|group| {
                group.authors.len() == reaction.author_ids.len()
                    && group
                        .authors
                        .iter()
                        .all(|author| reaction.author_ids.contains(&author.id))
            });
        match loaded {
            Some(group) => model::reactor_summary(&group.authors, self_id, name, &reaction.emoji),
            None => model::reactor_fallback(&reaction.author_ids, self_id, name, &reaction.emoji),
        }
    }

    fn set_reaction(&mut self, message: &str, emoji: &str, active: bool) {
        if !self.selected_is_joined() {
            return;
        }
        if self.session.is_none() || self.selected_channel.is_none() {
            return;
        }
        let key = (message.to_owned(), emoji.to_owned());
        self.reaction_errors.remove(message);
        if let Some(pending) = self.pending_reactions.get_mut(&key) {
            pending.desired = active;
            pending.visible = true;
            pending.superseded = pending.sent;
        } else {
            self.pending_reactions.insert(
                key,
                PendingReaction {
                    desired: active,
                    sent: false,
                    visible: true,
                    superseded: false,
                },
            );
        }
        self.send_next_reaction(message);
    }

    fn set_pin(&mut self, target: &model::Message, active: bool) {
        let message = target.id.as_str();
        if !self.selected_is_joined() || self.pending_pins.contains(message) {
            return;
        }
        let (Some(session), Some(channel)) = (&self.session, &self.selected_channel) else {
            return;
        };
        if target.channel_id != *channel {
            return;
        }
        let pin = active.then(|| model::Pin {
            author: session.author.clone(),
            created_at: chrono::Utc::now().to_rfc3339(),
        });
        self.mutations
            .pins
            .insert(message.to_owned(), (target.clone(), pin));
        self.pin_errors.remove(message);
        self.pending_pins.insert(message.to_owned());
        self.worker.send(Command::Pin {
            generation: self.generation,
            token: self.token.clone(),
            chat_token: session.token.clone(),
            channel: channel.clone(),
            message: message.to_owned(),
            active,
        });
    }

    fn send_next_reaction(&mut self, message: &str) {
        if self
            .pending_reactions
            .iter()
            .any(|((id, _), pending)| id == message && pending.sent)
        {
            return;
        }
        let Some((key, active)) = self.pending_reactions.iter().find_map(|(key, pending)| {
            (key.0 == message && pending.visible && !pending.sent)
                .then(|| (key.clone(), pending.desired))
        }) else {
            return;
        };
        let (Some(session), Some(channel)) = (&self.session, &self.selected_channel) else {
            return;
        };
        let chat_token = session.token.clone();
        let channel = channel.clone();
        let pending = self.pending_reactions.get_mut(&key).unwrap();
        pending.sent = true;
        pending.superseded = false;
        self.worker.send(Command::React {
            generation: self.generation,
            token: self.token.clone(),
            chat_token,
            channel,
            message: key.0,
            emoji: key.1,
            active,
        });
    }

    fn emoji_picker(&mut self, context: &egui::Context) {
        let Some(message) = self.reaction_picker.clone() else {
            return;
        };
        if !self.selected_is_joined() || self.session.is_none() {
            self.reaction_picker = None;
            return;
        }
        let mut open = true;
        let mut close_requested = context.input(|input| input.key_pressed(egui::Key::Escape));
        let mut selected = None;
        egui::Window::new("Add reaction")
            .id(egui::Id::new("emoji-picker"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .show(context, |ui| {
                let search_id = egui::Id::new("emoji-picker-search");
                let _search = ui.add(
                    egui::TextEdit::singleline(&mut self.reaction_search)
                        .id(search_id)
                        .hint_text("Search emoji…")
                        .desired_width(f32::INFINITY),
                );
                if self.reaction_search_focus {
                    ui.memory_mut(|memory| memory.request_focus(search_id));
                    self.reaction_search_focus = false;
                }
                ui.add_space(6.0);
                let query = self.reaction_search.trim().to_lowercase();
                let choices: Vec<_> = emoji::catalog()
                    .iter()
                    .filter(|entry| {
                        entry.selectable
                            && (query.is_empty()
                                || entry.name.to_lowercase().contains(&query)
                                || entry.keywords.to_lowercase().contains(&query)
                                || entry.category.to_lowercase().contains(&query))
                    })
                    .collect();
                if choices.is_empty() {
                    ui.label(RichText::new("No emoji found").color(MUTED));
                }
                egui::ScrollArea::vertical().max_height(330.0).show_rows(
                    ui,
                    38.0,
                    choices.len().div_ceil(8),
                    |ui, rows| {
                        for row in rows {
                            ui.horizontal(|ui| {
                                for entry in choices.iter().skip(row * 8).take(8) {
                                    let image = self.reaction_textures.image(ui, entry, 30.0);
                                    let response = ui
                                        .add(egui::Button::image(image).frame(false))
                                        .on_hover_text(&entry.name);
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::WidgetType::Button,
                                            true,
                                            &entry.name,
                                        )
                                    });
                                    if response.clicked() {
                                        selected = Some(entry.emoji.clone());
                                    }
                                }
                            });
                        }
                    },
                );
            });
        if let Some(emoji) = selected {
            self.set_reaction(&message, &emoji, true);
            close_requested = true;
        }
        if !open || close_requested {
            self.reaction_picker = None;
        }
    }

    fn channel_name(&self) -> &str {
        if let Some(id) = &self.selected_direct {
            return self
                .directs
                .iter()
                .find(|conversation| conversation.id == *id)
                .map_or("Direct message", |conversation| {
                    conversation.peer.display_name.as_str()
                });
        }
        let id = self.selected_channel.as_deref();
        self.detail
            .as_ref()
            .and_then(|detail| {
                detail
                    .channels
                    .iter()
                    .find(|channel| Some(channel.id.as_str()) == id)
            })
            .map_or("general", |channel| channel.name.as_str())
    }

    fn suggestions(&self, token: &ComposerToken) -> Vec<Suggestion> {
        match token {
            ComposerToken::Emoji(token) => emoji::suggestions(&token.query)
                .into_iter()
                .map(Suggestion::Emoji)
                .collect(),
            ComposerToken::Mention(token) => {
                let (people, specials) = self.mention_people();
                mentions::suggestions(&people, specials, &token.query)
                    .into_iter()
                    .map(Suggestion::Mention)
                    .collect()
            }
        }
    }

    /// Who `@` can suggest, never yourself, and whether `everyone`/`here` apply.
    /// Space channels use the members already loaded by
    /// `GET /api/spaces/{space}` (none yet leaves only the specials). Any DM,
    /// self-notes included, uses `GET /api/people`; until that loads, or if it
    /// never does, a DM offers its other participant (self-notes: nobody).
    fn mention_people(&self) -> (Vec<mentions::Person>, bool) {
        let me = self.account.as_ref().map(|account| account.id.as_str());
        if let Some(id) = &self.selected_direct {
            let known = self.people.as_deref().unwrap_or_default();
            // The peer also covers a DM opened after the list was fetched.
            let peer = self
                .directs
                .iter()
                .find(|direct| &direct.id == id)
                .map(|direct| &direct.peer)
                .filter(|peer| !known.iter().any(|person| person.id == peer.id))
                .map(|peer| model::Person {
                    id: peer.id.clone(),
                    username: peer.username.clone(),
                    display_name: peer.display_name.clone(),
                    avatar_id: None,
                });
            let people = known
                .iter()
                .chain(peer.as_ref())
                .filter(|person| Some(person.id.as_str()) != me)
                .map(|person| mentions::Person {
                    username: person.username.clone(),
                    display_name: person.display_name.clone(),
                    avatar_id: person.avatar_id,
                })
                .collect();
            return (people, false);
        }
        let people = self
            .detail
            .as_ref()
            .filter(|detail| self.selected_space.as_ref() == Some(&detail.space.id))
            .map_or_else(Vec::new, |detail| {
                detail
                    .members
                    .iter()
                    .filter(|member| Some(member.id.as_str()) != me)
                    .map(|member| mentions::Person {
                        username: member.username.clone(),
                        display_name: member.display_name.clone(),
                        avatar_id: member.avatar_id,
                    })
                    .collect()
            });
        (people, true)
    }

    /// A person pill's card data from what is already loaded.
    fn mention_profile(&self, entry: &model::Mention) -> Option<mentions::Profile> {
        mentions::profile(
            entry,
            self.account.as_ref().map(|account| account.id.as_str()),
            self.detail
                .as_ref()
                .map_or(&[][..], |detail| detail.members.as_slice()),
            self.people.as_deref().unwrap_or_default(),
            &self.directs,
        )
    }

    /// The card for a clicked person pill: below the pill, or above it when
    /// there is no room. Escape, a click outside, or opening the DM closes it.
    fn mention_card(&mut self, context: &egui::Context) {
        let Some(card) = self.mention_card.clone() else {
            return;
        };
        let Some(profile) = self.mention_profile(&card.pill.entry) else {
            self.mention_card = None;
            return;
        };
        let id = egui::Id::new("mention-card");
        let height = context
            .memory(|memory| memory.area_rect(id))
            .map_or(190.0, |rect| rect.height());
        // Follow the pill while the history scrolls.
        let anchor = context
            .read_response(card.pill.id)
            .map_or(card.pill.rect, |pill| pill.rect);
        let (pivot, position) =
            if anchor.bottom() + 6.0 + height <= context.content_rect().bottom() - 8.0 {
                (
                    egui::Align2::LEFT_TOP,
                    anchor.left_bottom() + egui::vec2(0.0, 6.0),
                )
            } else {
                (
                    egui::Align2::LEFT_BOTTOM,
                    anchor.left_top() - egui::vec2(0.0, 6.0),
                )
            };
        let mut message = false;
        let shown = egui::Area::new(id)
            .order(egui::Order::Foreground)
            .pivot(pivot)
            .fixed_pos(position)
            .constrain(true)
            .show(context, |ui| {
                egui::Frame::new()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(8)
                    .inner_margin(16)
                    .show(ui, |ui| {
                        // 280px including the padding and the border.
                        ui.set_width(246.0);
                        ui.spacing_mut().item_spacing.y = 12.0;
                        let initial = profile.display_name.as_ref().unwrap_or(&profile.username);
                        avatar(ui, initial, profile.avatar_id, 48.0, false);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.label(bold(profile.title()).size(15.0).color(TEXT));
                            if profile.display_name.is_some() {
                                ui.label(
                                    RichText::new(format!("@{}", profile.username))
                                        .size(12.0)
                                        .color(MUTED),
                                );
                            }
                        });
                        if profile.me {
                            ui.label(RichText::new("You").size(12.0).color(MUTED));
                            return;
                        }
                        let opening = card.opening.is_some();
                        // Full width with a centered label.
                        let button = ui
                            .vertical_centered_justified(|ui| {
                                ui.add_enabled(
                                    !opening,
                                    egui::Button::new(
                                        bold(if opening { "Opening…" } else { "Message" })
                                            .size(12.0),
                                    )
                                    .fill(TERRACOTTA)
                                    .stroke(Stroke::new(1.0, TERRACOTTA))
                                    .corner_radius(7)
                                    .min_size(egui::vec2(0.0, 36.0)),
                                )
                            })
                            .inner;
                        if card.fresh {
                            button.request_focus();
                        }
                        message = button.clicked();
                        if let Some(error) = &card.error {
                            ui.label(RichText::new(error).size(12.0).color(ERROR));
                        }
                    });
            });
        let escape =
            context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if escape || (!card.fresh && shown.response.clicked_elsewhere()) {
            self.mention_card = None;
            if escape {
                context.memory_mut(|memory| memory.request_focus(card.pill.id));
            }
            return;
        }
        if let Some(open) = &mut self.mention_card {
            open.fresh = false;
        }
        if message {
            self.message_from_card(&card.pill.entry, &profile.username);
        }
    }

    /// **Message** opens the existing DM with this person, or creates one by
    /// username through the same request as the Start conversation dialog.
    fn message_from_card(&mut self, entry: &model::Mention, username: &str) {
        if let Some(direct) = self
            .directs
            .iter()
            .find(|direct| entry.id.as_deref() == Some(direct.peer.id.as_str()))
            .cloned()
        {
            self.select_direct(direct);
            return;
        }
        let Some(token) = self.token.clone() else {
            return;
        };
        self.navigation += 1;
        self.opening = false;
        self.navigation_target = None;
        if let Some(card) = &mut self.mention_card {
            card.opening = Some(self.navigation);
            card.error = None;
        }
        self.worker.send(Command::CreateDirect {
            generation: self.generation,
            navigation: self.navigation,
            token,
            username: username.to_owned(),
        });
    }

    /// Web's `canCreateSpace`: needs the server's limits.
    fn can_create_space(&self) -> bool {
        let (Some(limits), Some(account)) = (&self.limits, &self.account) else {
            return false;
        };
        let owned = self
            .spaces
            .iter()
            .filter(|space| space.owner_id == account.id && !space.demo)
            .count();
        owned < limits.owned_spaces
            && self.spaces.iter().filter(|space| !space.demo).count() < limits.total_spaces
    }

    fn can_create_channel(&self) -> bool {
        self.limits
            .as_ref()
            .zip(self.detail.as_ref())
            .is_some_and(|(limits, detail)| detail.channels.len() < limits.channels_per_space)
    }

    fn create_space_tooltip(&self) -> String {
        if self.account.is_none() {
            "Sign in to create a space".into()
        } else if self.can_create_space() {
            "Create space".into()
        } else {
            let (owned, total) = self.limits.as_ref().map_or((20, 100), |limits| {
                (limits.owned_spaces, limits.total_spaces)
            });
            format!("Space limit reached ({owned} owned, {total} total)")
        }
    }

    fn create_channel_tooltip(&self) -> String {
        if self.can_create_channel() {
            "Create channel".into()
        } else {
            format!(
                "Channel limit reached ({})",
                self.limits
                    .as_ref()
                    .map_or(100, |limits| limits.channels_per_space)
            )
        }
    }

    fn owner(&self) -> bool {
        self.account
            .as_ref()
            .zip(self.detail.as_ref())
            .is_some_and(|(account, detail)| {
                account.id == detail.space.owner_id && !detail.space.demo
            })
    }

    fn can_leave_space(&self) -> bool {
        self.account
            .as_ref()
            .zip(self.detail.as_ref())
            .is_some_and(|(account, detail)| {
                !detail.space.demo && account.id != detail.space.owner_id
            })
    }

    fn dialogs(&mut self, context: &egui::Context, dialog_was_open: bool) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        if matches!(dialog, Dialog::SignIn) {
            return;
        }
        let mut leave_title = None;
        let title: &str = match &dialog {
            Dialog::SignIn => {
                if self.challenge.is_some() {
                    "Check your email"
                } else {
                    "Sign in to Caper"
                }
            }
            Dialog::Profile => "Edit profile",
            Dialog::Settings => "Settings",
            Dialog::Audio => "Audio test",
            Dialog::Connection => "Connection details",
            Dialog::Diagnostics => "Audio diagnostics",
            Dialog::CreateSpace => "Create a space",
            Dialog::ManageSpace => "Manage space",
            Dialog::Invitation(_) => "You’re invited!",
            Dialog::LeaveSpace { name, .. } => leave_title.get_or_insert(format!("Leave {name}?")),
            Dialog::LeaveChannel { name, .. } => {
                leave_title.get_or_insert(format!("Leave #{name}?"))
            }
            Dialog::ConfirmDelete { channel, .. } => {
                if channel.is_some() {
                    "Delete channel"
                } else {
                    "Delete space"
                }
            }
            Dialog::CreateChannel => "Create a channel",
            Dialog::ManageChannel(_) => "Overview",
            Dialog::StartDirect => "Start a direct message",
            Dialog::Block { account, .. } => {
                leave_title.get_or_insert(format!("Block {}?", account.display_name))
            }
        };
        context
            .layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("dialog-backdrop"),
            ))
            .rect_filled(context.viewport_rect(), 0.0, Color32::from_black_alpha(190));
        let wide = matches!(dialog, Dialog::ManageSpace | Dialog::ManageChannel(_));
        let settings = matches!(dialog, Dialog::Settings);
        let width: f32 = if matches!(dialog, Dialog::Audio) {
            720.0
        } else if wide {
            600.0
        } else if settings {
            500.0
        } else {
            440.0
        };
        let dialog_height: f32 = if matches!(dialog, Dialog::ManageSpace) {
            658.0
        } else if matches!(dialog, Dialog::ManageChannel(_)) {
            618.0
        } else if matches!(dialog, Dialog::Audio) {
            700.0
        } else if matches!(dialog, Dialog::Connection | Dialog::Diagnostics) {
            520.0
        } else if matches!(dialog, Dialog::Profile) {
            460.0
        } else if settings {
            560.0
        } else if matches!(
            dialog,
            Dialog::ConfirmDelete { .. } | Dialog::LeaveSpace { .. } | Dialog::Block { .. }
        ) {
            300.0
        } else {
            420.0
        };
        let available = context.viewport_rect().size() - egui::vec2(32.0, 32.0);
        let return_to = match &dialog {
            Dialog::ConfirmDelete { channel, .. } => Some(
                channel
                    .clone()
                    .map_or(Dialog::ManageSpace, Dialog::ManageChannel),
            ),
            _ => None,
        };
        let mut close = context.input(|input| input.key_pressed(egui::Key::Escape))
            && !egui::Popup::is_any_open(context);
        let dismiss_on_backdrop = matches!(
            dialog,
            Dialog::Settings | Dialog::ManageSpace | Dialog::ManageChannel(_) | Dialog::StartDirect
        );
        let modal = egui::Area::new(egui::Id::new("caper-dialog"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                // Every modal keeps a viewport-bounded shell so asynchronous
                // content changes scroll rather than resize or recenter it.
                ui.set_height(dialog_height.min(available.y));
                egui::Frame::new()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(8)
                    .show(ui, |ui| {
                        ui.set_width(width.min(available.x));
                        ui.set_height(dialog_height.min(available.y));
                        let dialog_top = ui.min_rect().top();
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(22, 18))
                            .show(ui, |ui| {
                                ui.horizontal_top(|ui| {
                                    ui.vertical(|ui| {
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 12.0;
                                            if matches!(dialog, Dialog::Invitation(_)) {
                                                ui.add(egui::Image::from_bytes("bytes://invitation-envelope.png", include_bytes!("../../../web/public/images/invitation/1f4e8.png"))
                                                    .fit_to_exact_size(egui::vec2(32.0, 32.0)));
                                            }
                                            ui.label(bold(title).size(19.0));
                                        });
                                        if matches!(dialog, Dialog::ManageSpace) {
                                            ui.add_space(5.0);
                                            ui.label(
                                                RichText::new("Only the owner can change this space and its membership.")
                                                    .size(12.0)
                                                    .color(MUTED),
                                            );
                                        }
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::TOP),
                                        |ui| {
                                            if drawn_icon_button(
                                                ui,
                                                NavIcon::Close,
                                                &format!("Close {title}"),
                                            )
                                            .clicked()
                                            {
                                                close = true;
                                            }
                                        },
                                    );
                                });
                            });
                        ui.separator();
                        let save_bar = match &dialog {
                            Dialog::ManageChannel(id) => {
                                Some((id.clone(), self.channel_dirty(id)))
                            }
                            _ => None,
                        };
                        egui::ScrollArea::vertical()
                            .id_salt(("modal-content", title))
                            .max_height((dialog_height.min(available.y) - if save_bar.is_some() { 160.0 } else { 92.0 }).max(1.0))
                            .show(ui, |ui| {
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(22, 20))
                                    .show(ui, |ui| {
                                        ui.set_width((width.min(available.x) - 44.0).max(1.0));
                                        match dialog {
                                            Dialog::SignIn => unreachable!("sign-in is rendered as a full page"),
                                            Dialog::Profile => self.profile_dialog(ui),
                                            Dialog::Settings => self.settings_dialog(ui),
                                            Dialog::Audio => self.audio_test(ui),
                                            Dialog::Connection => self.connection_details(ui),
                                            Dialog::Diagnostics => self.audio_diagnostics(ui),
                                            Dialog::CreateSpace => self.space_dialog(ui, false),
                                            Dialog::ManageSpace => self.space_dialog(ui, true),
                                            Dialog::Invitation(invitation) => {
                                                ui.heading(format!("Join {}?", invitation.name));
                                                if let Some(inviter) = &invitation.inviter {
                                                    ui.label(format!("{} (@{}) invited you.", inviter.display_name, inviter.username));
                                                }
                                                ui.add_space(16.0);
                                                ui.horizontal(|ui| {
                                                    if ui.add_enabled(!self.loading, egui::Button::new("Decline")).clicked() {
                                                        self.admin(AdminOperation::DeclineInvitation { space: invitation.id.clone() });
                                                    }
                                                    if primary_button(ui, if self.loading { "Accepting…" } else { "Accept invitation" }, !self.loading).clicked() {
                                                        self.admin(AdminOperation::AcceptInvitation { space: invitation.id });
                                                    }
                                                });
                                            }
                                            Dialog::ConfirmDelete { space, channel, name } => {
                                                let kind = if channel.is_some() { "channel" } else { "space" };
                                                let display = if channel.is_some() { format!("#{name}") } else { name };
                                                ui.label(format!("Delete {display} for everyone? {} This cannot be undone.", if channel.is_some() { "This channel and its messages will disappear from the space." } else { "All its channels and their messages will disappear from the space." }));
                                                ui.add_space(16.0);
                                                ui.horizontal(|ui| {
                                                    if ui.add_enabled(!self.loading, egui::Button::new("Cancel")).clicked() { close = true; }
                                                    let delete = ui.add_enabled(!self.loading, egui::Button::new(RichText::new(if self.loading { "Deleting…".into() } else { format!("Delete {kind}") }).color(ERROR)));
                                                    if delete.clicked() && !delete.double_clicked() {
                                                        self.admin(match channel { Some(channel) => AdminOperation::DeleteChannel { space, channel }, None => AdminOperation::DeleteSpace { space } });
                                                    }
                                                });
                                            }
                                            Dialog::LeaveSpace { id, .. } => {
                                                ui.label("You will lose access to its channels and conversations. An owner can add you again later.");
                                                ui.add_space(16.0);
                                                ui.horizontal(|ui| {
                                                    if ui.add_enabled(!self.loading, egui::Button::new("Cancel")).clicked() {
                                                        self.dialog = None;
                                                    }
                                                    if ui.add_enabled(!self.loading, egui::Button::new(RichText::new(if self.loading { "Leaving…" } else { "Leave space" }).color(ERROR))).clicked()
                                                        && let Some(account) = &self.account {
                                                        let member = account.id.clone();
                                                        self.voice.revoke_space(&id);
                                                        self.admin(AdminOperation::LeaveSpace { space: id, member });
                                                    }
                                                });
                                            }
                                            Dialog::LeaveChannel { space, channel, .. } => {
                                                let private_loss = !self.owner() && self.detail.as_ref().is_some_and(|detail| detail.channels.iter().any(|item| item.id == channel && item.private));
                                                ui.label(if private_loss { "You’ll lose access and need another invitation to return. You’ll disconnect from this channel’s voice call." } else { "It will leave your sidebar. You can preview and rejoin from Browse channels. You’ll disconnect from this channel’s voice call." });
                                                let (cancel, leave) = dialog_actions(ui, "Leave channel", !self.loading);
                                                if cancel { self.dialog = None; }
                                                if leave { self.admin(AdminOperation::LeaveChannel { space, channel }); }
                                            }
                                            Dialog::CreateChannel => self.channel_dialog(ui, None),
                                            Dialog::ManageChannel(id) => self.channel_dialog(ui, Some(id)),
                                            Dialog::Block { account, request } => {
                                                ui.label("You won't see their messages unless you choose to, and they can't send you DMs or requests.");
                                                ui.add_space(16.0);
                                                ui.horizontal(|ui| {
                                                    if ui.add_enabled(!self.block_busy, egui::Button::new("Cancel")).clicked() { close = true; }
                                                    let block = ui.add_enabled(!self.block_busy, egui::Button::new(RichText::new(if self.block_busy { "Blocking…" } else { "Block" }).color(ERROR)));
                                                    if block.clicked() {
                                                        self.block_busy = true;
                                                        self.block_error = None;
                                                        self.account_op(AccountOperation::SetBlock { account, blocked: true, request });
                                                    }
                                                });
                                                if let Some(error) = &self.block_error {
                                                    ui.colored_label(ERROR, error);
                                                }
                                            }
                                            Dialog::StartDirect => {
                                                ui.label("Enter an exact username.");
                                                ui.add_space(12.0);
                                                ui.add(egui::TextEdit::singleline(&mut self.member_username).hint_text("Username"));
                                                ui.add_space(16.0);
                                                if ui.add_enabled(!self.loading && !self.member_username.trim().is_empty(), egui::Button::new(if self.loading { "Starting…" } else { "Start conversation" })).clicked()
                                                    && let Some(token) = self.token.clone() {
                                                    self.loading = true;
                                                    self.error = None;
                                                    self.worker.send(Command::CreateDirect { generation: self.generation, navigation: self.navigation, token, username: self.member_username.trim().to_owned() });
                                                }
                                            }
                                        }
                                        // Local preferences have their own startup errors;
                                        // account/network notices do not belong in Settings.
                                        if !settings {
                                            notices(ui, &self.error, &self.warning);
                                        }
                                    });
                            });
                        if let Some((id, dirty)) = save_bar {
                            // Web's bar is sticky at the dialog's bottom edge.
                            let filler = dialog_top + dialog_height.min(available.y) - 66.0 - ui.cursor().top();
                            if filler > 0.0 {
                                ui.add_space(filler);
                            }
                            if dirty {
                                self.channel_save_bar(ui, &id);
                            } else {
                                // Match channel_save_bar's 36px row and 14px vertical margins
                                // without creating visible or accessible widgets.
                                ui.allocate_space(egui::vec2(ui.available_width(), 64.0));
                            }
                        }
                    });
            });
        if dialog_was_open && dismiss_on_backdrop && !egui::Popup::is_any_open(context) {
            // A fast click can deliver press and release in the opening frame.
            // Neither event may dismiss the dialog that click just opened.
            close |= context.input(|input| {
                input.pointer.any_pressed()
                    && input
                        .pointer
                        .interact_pos()
                        .is_some_and(|position| !modal.response.rect.contains(position))
            });
        }
        if close && !(self.loading && return_to.is_some()) {
            self.dialog = return_to;
            self.error = None;
        }
    }

    fn settings_dialog(&mut self, ui: &mut egui::Ui) {
        ui.label(bold("Startup").size(12.0).color(MUTED));
        ui.add_space(6.0);
        let mut requested = self.launch_at_login;
        if ui
            .add_enabled_ui(self.persist_preferences, |ui| {
                settings_switch(
                    ui,
                    &mut requested,
                    "Launch at login",
                    "Open Caper when you sign in to your computer.",
                )
            })
            .inner
            .changed()
        {
            match startup::set_enabled(requested) {
                Ok(()) => {
                    self.launch_at_login = requested;
                    self.startup_error = None;
                }
                Err(error) => {
                    self.startup_error =
                        Some(format!("Could not change startup settings: {error}"));
                }
            }
        }
        if let Some(error) = &self.startup_error {
            ui.colored_label(ERROR, error);
            if ui.button("Dismiss startup error").clicked() {
                self.startup_error = None;
            }
        }
        ui.add_space(18.0);
        ui.separator();
        ui.add_space(18.0);
        ui.label(bold("Sounds").size(12.0).color(MUTED));
        ui.add_space(6.0);
        if settings_switch(
            ui,
            &mut self.sound_effects,
            "Caper sound effects",
            "Play sounds for messages and app interactions.",
        )
        .changed()
        {
            self.effects = Effects::new(self.sound_effects && self.persist_preferences);
            if self.sound_effects {
                self.effects.play(Effect::ToggleOn);
            }
        }
        ui.add_space(18.0);
        ui.separator();
        ui.add_space(18.0);
        ui.label(bold("Updates").size(12.0).color(MUTED));
        ui.add_space(6.0);
        let status = self.updates.status();
        let checking = status == updates::Status::Checking;
        let label = if checking {
            "Checking…"
        } else {
            "Check for updates"
        };
        if secondary_button(ui, label, self.updates.can_check() && !checking).clicked() {
            self.updates.check_now();
        }
        match &status {
            updates::Status::UpToDate => {
                ui.label(RichText::new("Caper is up to date.").color(MUTED));
            }
            updates::Status::Available => {
                ui.label(
                    RichText::new(if self.updates.available().is_some() {
                        "An update is available above the window."
                    } else {
                        "An update is available. Check again to show it."
                    })
                    .color(MUTED),
                );
            }
            updates::Status::Failed(error) => {
                ui.colored_label(ERROR, format!("Could not check for updates: {error}"));
            }
            updates::Status::Idle | updates::Status::Checking => {}
        }
        ui.label(
            RichText::new(
                if !self.updates.can_check() && status == updates::Status::Idle {
                    "Update checks are available in packaged release builds."
                } else {
                    "Checks automatically every minute."
                },
            )
            .size(12.0)
            .color(MUTED),
        );
        if self.account.is_some() {
            ui.add_space(18.0);
            ui.separator();
            ui.add_space(18.0);
            self.notification_settings(ui);
            ui.add_space(18.0);
            ui.separator();
            ui.add_space(18.0);
            self.privacy_settings(ui);
            ui.add_space(18.0);
            ui.separator();
            ui.add_space(18.0);
            self.blocked_settings(ui);
        }
    }

    /// "Notify me about", saved as soon as it changes and reverted with an
    /// inline error if the save fails. Desktop has no phone setting.
    fn notification_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(bold("Notifications").size(12.0).color(MUTED));
        ui.add_space(6.0);
        ui.label(RichText::new("Notify me about").size(13.0).color(TEXT));
        let current = self.notifications.account_level();
        let mut chosen = None;
        ui.add_enabled_ui(
            current.is_some() && !self.notifications.saving(&Scope::Account),
            |ui| {
                for level in [
                    NotificationLevel::All,
                    NotificationLevel::Mentions,
                    NotificationLevel::Nothing,
                ] {
                    if ui
                        .radio(
                            current == Some(level),
                            notifications::account_level_label(level),
                        )
                        .clicked()
                    {
                        chosen = Some(level);
                    }
                }
            },
        );
        if let Some(level) = chosen {
            self.change_notifications(Scope::Account, Change::Level(Some(level)));
        }
        if current.is_none() {
            match self.notifications.load_error().map(str::to_owned) {
                None => {
                    ui.label(RichText::new("Loading…").size(12.0).color(MUTED));
                }
                Some(error) => {
                    ui.horizontal(|ui| {
                        ui.colored_label(ERROR, error);
                        if ui.small_button("Retry").clicked() {
                            self.load_notifications();
                        }
                    });
                }
            }
        }
        if let Some(error) = self.notifications.error(&Scope::Account) {
            ui.colored_label(ERROR, error.to_owned());
        }
    }

    /// "Who can start a DM with you", saved as soon as it changes and reverted
    /// with an inline error if the save fails.
    fn privacy_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(bold("Who can start a DM with you").size(12.0).color(MUTED));
        ui.add_space(6.0);
        let options = [
            (
                "anyone",
                "Anyone",
                Some("People outside your spaces send a message request first."),
            ),
            ("spaces", "People in my spaces", None),
            (
                "nobody",
                "No one new",
                Some("Conversations you already have stay open."),
            ),
        ];
        let loaded = self.privacy.is_some();
        let mut chosen = None;
        ui.add_enabled_ui(loaded && !self.privacy_saving, |ui| {
            for (value, label, detail) in options {
                if ui
                    .radio(self.privacy.as_deref() == Some(value), label)
                    .clicked()
                {
                    chosen = Some(value);
                }
                if let Some(detail) = detail {
                    ui.label(RichText::new(detail).size(12.0).color(MUTED));
                }
            }
        });
        if let Some(value) = chosen {
            self.save_privacy(value);
        }
        if !loaded && self.privacy_error.is_none() {
            ui.label(RichText::new("Loading…").size(12.0).color(MUTED));
        }
        if let Some(error) = self.privacy_error.clone() {
            ui.horizontal(|ui| {
                ui.colored_label(ERROR, error);
                if !loaded && ui.small_button("Retry").clicked() {
                    self.privacy_error = None;
                    self.account_op(AccountOperation::LoadPrivacy);
                }
            });
        }
    }

    fn blocked_settings(&mut self, ui: &mut egui::Ui) {
        ui.label(bold("Blocked accounts").size(12.0).color(MUTED));
        ui.add_space(6.0);
        match self.blocks.clone() {
            None => {
                ui.label(RichText::new("Loading…").size(12.0).color(MUTED));
            }
            Some(blocks) if blocks.is_empty() => {
                ui.label(
                    RichText::new("You haven't blocked anyone.")
                        .size(12.0)
                        .color(MUTED),
                );
            }
            Some(blocks) => {
                for account in blocks {
                    ui.horizontal(|ui| {
                        avatar(ui, &account.display_name, account.avatar_id, 24.0, false);
                        ui.label(&account.display_name);
                        if !account.username.is_empty() {
                            ui.label(
                                RichText::new(format!("@{}", account.username))
                                    .size(12.0)
                                    .color(MUTED),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if secondary_button(ui, "Unblock", !self.block_busy).clicked() {
                                self.unblock(account.clone());
                            }
                        });
                    });
                }
            }
        }
        if let Some(error) = &self.block_error {
            ui.colored_label(ERROR, error);
        }
    }

    fn profile_dialog(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new(
                "Your username is unique. Your display name is what people see in conversations.",
            )
            .color(MUTED),
        );
        ui.add_space(10.0);
        ui.label("Username");
        ui.add_sized(
            [ui.available_width(), 42.0],
            egui::TextEdit::singleline(&mut self.username)
                .vertical_align(egui::Align::Center)
                .char_limit(32),
        );
        self.username = normalize_username(&self.username);
        ui.label(
            RichText::new("3–32 lowercase letters, numbers, or underscores.")
                .small()
                .color(MUTED),
        );
        ui.label("Display name");
        ui.add_sized(
            [ui.available_width(), 42.0],
            egui::TextEdit::singleline(&mut self.display_name)
                .vertical_align(egui::Align::Center)
                .char_limit(64),
        );
        ui.label(
            RichText::new("Shown to other people. It does not need to be unique.")
                .small()
                .color(MUTED),
        );
        if primary(
            ui,
            if self.loading {
                "Saving…"
            } else {
                "Save profile"
            },
            self.loading || self.username.len() < 3 || self.display_name.trim().is_empty(),
        )
        .clicked()
        {
            self.loading = true;
            self.worker.send(Command::Profile {
                generation: self.generation,
                token: self.token.clone().unwrap_or_default(),
                username: self.username.trim().to_ascii_lowercase(),
                display_name: self.display_name.trim().into(),
            });
        }
        if self.account.is_some() && ui.button("Log out").clicked() {
            self.logout();
        }
    }

    fn space_dialog(&mut self, ui: &mut egui::Ui, manage: bool) {
        name_field(ui, "Space name", &mut self.form_name, "Studio", None);
        if manage {
            let unchanged = self
                .detail
                .as_ref()
                .is_some_and(|detail| self.form_name.trim() == detail.space.name);
            let mut save = false;
            ui.add_space(20.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    save = secondary_button(
                        ui,
                        if self.loading {
                            "Saving…"
                        } else {
                            "Save name"
                        },
                        !self.loading && !unchanged,
                    )
                    .clicked();
                });
            });
            if save && let Some(space) = self.selected_space.clone() {
                self.admin(AdminOperation::UpdateSpace {
                    space,
                    name: self.form_name.clone(),
                });
            }
        } else {
            let (cancel, submit) = dialog_actions(
                ui,
                if self.loading {
                    "Saving…"
                } else {
                    "Create space"
                },
                !self.loading,
            );
            if cancel {
                self.dialog = None;
                self.error = None;
            } else if submit {
                // Web validates on submit and says why (spaces/client.ts).
                if let Some(error) = space_name_error(&self.form_name) {
                    self.error = Some(error.into());
                } else {
                    self.admin(AdminOperation::CreateSpace {
                        name: self.form_name.clone(),
                    });
                }
            }
        }
        if manage {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(bold("Members").size(14.0));
                egui::Frame::new()
                    .fill(RAISED)
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(7, 2))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(self.managed_members.len().to_string())
                                .size(10.0)
                                .color(MUTED),
                        );
                    });
            });
            self.members_dialog(ui, None);
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.label(bold("Pending invitations").size(14.0));
                ui.label(RichText::new(self.managed_invitations.len().to_string()).color(MUTED));
            });
            for invitation in self.managed_invitations.clone() {
                ui.horizontal(|ui| {
                    ui.label(format!("@{}", invitation.username));
                    if ui
                        .add_enabled(!self.loading, egui::Button::new("Cancel"))
                        .clicked()
                        && let Some(space) = self.selected_space.clone()
                    {
                        self.admin(AdminOperation::CancelInvitation {
                            space,
                            user: invitation.id,
                        });
                    }
                });
            }
            ui.add_space(14.0);
            ui.separator();
            ui.label(RichText::new("Delete space").size(14.0));
            ui.label(
                RichText::new("Delete this space and all its channels for every member.")
                    .size(12.0)
                    .color(MUTED),
            );
            if destructive(ui, "Delete space").clicked()
                && let Some(space) = self.selected_space.clone()
            {
                self.effects.play(Effect::Warning);
                self.dialog = Some(Dialog::ConfirmDelete {
                    space,
                    channel: None,
                    name: self
                        .detail
                        .as_ref()
                        .map_or("this space", |detail| detail.space.name.as_str())
                        .into(),
                });
            }
        }
    }

    /// Web's channel Overview: private channels load their member grants.
    fn open_manage_channel(&mut self, id: &str, name: &str, private: bool) {
        self.form_name = name.into();
        self.form_private = private;
        self.managed_channel = Some(id.into());
        self.managed_members.clear();
        self.managed_invitations.clear();
        self.member_username.clear();
        self.member_error = None;
        self.error = None;
        if private
            && let (Some(token), Some(space)) = (self.token.clone(), self.selected_space.clone())
        {
            self.worker.send(Command::Admin {
                generation: self.generation,
                token,
                operation: AdminOperation::LoadMembers {
                    space,
                    channel: Some(id.into()),
                },
            });
        }
        self.dialog = Some(Dialog::ManageChannel(id.into()));
    }

    /// The managed channel's saved name and privacy, for web's dirty check.
    fn saved_channel(&self, channel: &str) -> Option<(String, bool)> {
        self.detail
            .as_ref()?
            .channels
            .iter()
            .find(|entry| entry.id == channel)
            .map(|entry| (entry.name.clone(), entry.private))
    }

    fn channel_dirty(&self, channel: &str) -> bool {
        self.saved_channel(channel)
            .is_some_and(|(name, private)| self.form_name != name || self.form_private != private)
    }

    fn channel_dialog(&mut self, ui: &mut egui::Ui, channel: Option<String>) {
        name_field(
            ui,
            "Channel name",
            &mut self.form_name,
            "project-updates",
            channel.is_none().then_some(if self.form_private {
                NavIcon::Lock
            } else {
                NavIcon::Hash
            }),
        );
        self.form_name = normalize_channel(&self.form_name);
        if channel.is_none() {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Channels are where conversations happen around a topic. Use a name that is easy to find and understand.")
                    .size(12.0)
                    .color(MUTED),
            );
        }
        ui.add_space(14.0);
        if ui
            .checkbox(&mut self.form_private, "Private channel")
            .changed()
        {
            self.effects.toggle(self.form_private);
        }
        ui.label(
            RichText::new(if self.form_private {
                "Only you and the people you add can view or join.".to_owned()
            } else {
                format!(
                    "Anyone in {} can view or join this channel.",
                    self.detail
                        .as_ref()
                        .map_or("this space", |detail| detail.space.name.as_str())
                )
            })
            .size(11.0)
            .color(MUTED),
        );
        let Some(channel) = channel else {
            let (cancel, submit) = dialog_actions(
                ui,
                if self.loading {
                    "Saving…"
                } else {
                    "Create channel"
                },
                !self.loading,
            );
            if cancel {
                self.dialog = None;
                self.error = None;
            } else if submit
                && let Some(error) = channel_name_error(self.form_name.trim_end_matches('-'))
            {
                self.error = Some(error.into());
            } else if submit && let Some(space) = self.selected_space.clone() {
                self.admin(AdminOperation::CreateChannel {
                    space,
                    name: self.form_name.trim_end_matches('-').to_owned(),
                    private: self.form_private,
                });
            }
            return;
        };
        // Members follow the saved privacy, as on web.
        if self
            .saved_channel(&channel)
            .is_some_and(|(_, private)| private)
        {
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(RichText::new("Members").size(14.0));
                ui.label(
                    RichText::new(self.managed_members.len().to_string())
                        .size(11.0)
                        .color(MUTED),
                );
            });
            self.members_dialog(ui, Some(channel.clone()));
            ui.add_space(10.0);
            ui.label(
                RichText::new(format!(
                    "Pending invitations · {}",
                    self.managed_invitations.len()
                ))
                .size(12.0)
                .color(MUTED),
            );
            for invitation in self.managed_invitations.clone() {
                ui.horizontal(|ui| {
                    ui.label(format!("@{}", invitation.username));
                    if ui.button("Cancel").clicked()
                        && let Some(space) = self.selected_space.clone()
                    {
                        self.admin(AdminOperation::RemoveMember {
                            space,
                            channel: Some(channel.clone()),
                            member: invitation.id,
                        });
                    }
                });
            }
        }
        ui.separator();
        ui.label(RichText::new("Delete channel").size(14.0));
        ui.label(
            RichText::new("Delete this channel for everyone in the space.")
                .size(12.0)
                .color(MUTED),
        );
        if destructive(ui, "Delete channel").clicked()
            && let Some(space) = self.selected_space.clone()
        {
            self.effects.play(Effect::Warning);
            let name = self
                .saved_channel(&channel)
                .map_or_else(|| "this channel".to_owned(), |(name, _)| name);
            self.dialog = Some(Dialog::ConfirmDelete {
                space,
                channel: Some(channel),
                name,
            });
        }
    }

    /// Web's sticky footer while the channel overview has unsaved changes.
    fn channel_save_bar(&mut self, ui: &mut egui::Ui, channel: &str) {
        let bar = egui::Frame::new()
            .fill(BLACKOUT)
            .inner_margin(egui::Margin::symmetric(22, 14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.allocate_ui_with_layout(
                    egui::vec2(ui.available_width(), 36.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new("You have unsaved changes.").size(12.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let save = primary_button(
                                ui,
                                if self.loading {
                                    "Saving…"
                                } else {
                                    "Save changes"
                                },
                                !self.loading,
                            );
                            let reset = secondary_button(ui, "Reset", !self.loading);
                            if reset.clicked() {
                                if let Some((name, private)) = self.saved_channel(channel) {
                                    self.form_name = name;
                                    self.form_private = private;
                                }
                                self.error = None;
                            } else if save.clicked()
                                && let Some(space) = self.selected_space.clone()
                            {
                                self.admin(AdminOperation::UpdateChannel {
                                    space,
                                    channel: channel.to_owned(),
                                    name: self.form_name.trim_end_matches('-').to_owned(),
                                    private: self.form_private,
                                });
                            }
                        });
                    },
                );
            });
        ui.painter().hline(
            bar.response.rect.x_range(),
            bar.response.rect.top(),
            Stroke::new(1.0, BORDER),
        );
    }

    fn members_dialog(&mut self, ui: &mut egui::Ui, channel: Option<String>) {
        ui.horizontal(|ui| {
            ui.add_sized(
                [(ui.available_width() - 80.0).max(1.0), 38.0],
                egui::TextEdit::singleline(&mut self.member_username)
                    .vertical_align(egui::Align::Center)
                    .char_limit(32)
                    .hint_text(RichText::new("Exact username").color(MUTED.gamma_multiply(0.65))),
            );
            self.member_username = normalize_username(&self.member_username);
            if ui
                .add_enabled(
                    !self.loading,
                    egui::Button::new(
                        bold(if channel.is_some() { "Add" } else { "Invite" }).size(12.0),
                    )
                    .min_size(egui::vec2(64.0, 38.0)),
                )
                .clicked()
            {
                if self.member_username.len() < 3 {
                    self.member_error =
                        Some("Use 3–32 lowercase letters, numbers, or underscores.");
                } else if let Some(space) = self.selected_space.clone() {
                    self.member_error = None;
                    self.admin(AdminOperation::AddMember {
                        space,
                        channel: channel.clone(),
                        username: self.member_username.trim().into(),
                    });
                }
            }
        });
        if let Some(error) = self.member_error {
            ui.label(RichText::new(error).size(12.0).color(ERROR));
        }
        ui.separator();
        let members = self.managed_members.clone();
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                for member in members {
                    ui.horizontal(|ui| {
                        avatar(ui, &member.display_name, member.avatar_id, 30.0, false);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&member.display_name).strong());
                            ui.label(
                                RichText::new(format!(
                                    "@{}{}",
                                    member.username,
                                    if member.owner { " · Owner" } else { "" }
                                ))
                                .size(10.0)
                                .color(MUTED),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !member.owner
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Remove").size(11.0).color(MUTED),
                                        )
                                        .fill(Color32::TRANSPARENT)
                                        .stroke(Stroke::new(1.0, BORDER))
                                        .corner_radius(7)
                                        .min_size(egui::vec2(64.0, 32.0)),
                                    )
                                    .clicked()
                                && let Some(space) = self.selected_space.clone()
                            {
                                self.admin(AdminOperation::RemoveMember {
                                    space,
                                    channel: channel.clone(),
                                    member: member.id.clone(),
                                });
                            }
                        });
                    });
                }
            });
    }
}

/// Web's connection diagnostics, limited to what desktop measures. Field names
/// match web's copied JSON.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    join: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ice_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    roster_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    checks: Option<String>,
    received_bytes: u64,
    sent_bytes: u64,
    receive_bitrate: f64,
    send_bitrate: f64,
    packets_lost: i64,
    max_jitter_ms: f64,
    round_trip_ms: f64,
    route: &'static str,
}

impl ConnectionReport {
    fn new(times: Option<&voice::JoinTimes>, stats: &media::Diagnostics) -> Self {
        Self {
            join: times.map(|times| format!("Joined in {:.0} ms", times.joined_ms)),
            session_ms: times.map(|times| times.session_ms),
            transport_ms: times.map(|times| times.transport_ms),
            ice_ms: times.and_then(|times| times.ice_ms),
            roster_ms: times.map(|times| times.roster_ms),
            checks: stats.checks.clone(),
            received_bytes: stats.received_bytes,
            sent_bytes: stats.sent_bytes,
            receive_bitrate: stats.receive_bitrate,
            send_bitrate: stats.send_bitrate,
            packets_lost: stats.packets_lost,
            max_jitter_ms: stats.max_jitter_ms,
            round_trip_ms: stats.round_trip_ms,
            route: match stats.route {
                "relay" | "direct" => stats.route,
                _ => "unknown",
            },
        }
    }

    fn rows(&self) -> Vec<(&'static str, String)> {
        let mut rows = Vec::new();
        if let (Some(join), Some(session), Some(transport), Some(roster)) = (
            &self.join,
            self.session_ms,
            self.transport_ms,
            self.roster_ms,
        ) {
            rows.push(("Joined", join.clone()));
            rows.push(("Session + publish", format!("{session:.0} ms")));
            rows.push((
                "Transport + state",
                match self.ice_ms {
                    Some(ice) => format!("{transport:.0} ms (ICE {ice:.0} ms)"),
                    None => format!("{transport:.0} ms"),
                },
            ));
            rows.push((
                "Connectivity checks",
                self.checks
                    .clone()
                    .unwrap_or_else(|| "Not observed yet".into()),
            ));
            rows.push(("Roster", format!("{roster:.0} ms")));
        }
        rows.extend([
            (
                "Received",
                format!("{:.2} MB", self.received_bytes as f64 / 1e6),
            ),
            (
                "Live receive",
                format!("{:.0} kbps", self.receive_bitrate / 1_000.0),
            ),
            ("Sent", format!("{:.2} MB", self.sent_bytes as f64 / 1e6)),
            (
                "Live send",
                format!("{:.0} kbps", self.send_bitrate / 1_000.0),
            ),
            ("Packets lost", self.packets_lost.to_string()),
            ("Max jitter", format!("{:.0} ms", self.max_jitter_ms)),
            ("RTT", format!("{:.0} ms", self.round_trip_ms)),
            (
                "Route",
                match self.route {
                    "relay" => "TURN relay",
                    "direct" => "Direct",
                    _ => "Not observed yet",
                }
                .into(),
            ),
        ]);
        rows
    }
}

fn login_error_frame(ui: &mut egui::Ui, error: &str) {
    egui::Frame::new()
        .stroke(Stroke::new(1.0, TERRACOTTA))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(12, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width().min(414.0));
            ui.label(RichText::new(error).size(13.0).color(ERROR));
        });
}

fn drawn_icon_button(ui: &mut egui::Ui, icon: NavIcon, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 6.0, RAISED);
    }
    paint_icon(
        ui.painter(),
        rect.shrink(5.0),
        icon,
        if response.hovered() { TEXT } else { MUTED },
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response.on_hover_text(label)
}

fn settings_switch(
    ui: &mut egui::Ui,
    value: &mut bool,
    label: &str,
    description: &str,
) -> egui::Response {
    let row = ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 58.0).max(1.0));
            ui.label(RichText::new(label).size(14.0));
            ui.label(RichText::new(description).size(12.0).color(MUTED));
        });
        ui.allocate_exact_size(egui::vec2(42.0, 24.0), egui::Sense::hover())
            .0
    });
    let rect = row.inner;
    // The whole labelled row activates the switch, including with keyboard or AX.
    let mut response = ui.interact(row.response.rect, ui.id().with(label), egui::Sense::click());
    if response.clicked() {
        response.request_focus();
        *value = !*value;
        response.mark_changed();
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *value, label)
    });
    let fill = if *value { TERRACOTTA } else { BORDER };
    ui.painter().rect_filled(rect, 12.0, fill);
    if response.hovered() || response.has_focus() {
        ui.painter().rect_stroke(
            rect.expand(3.0),
            15.0,
            Stroke::new(1.0, TERRACOTTA_BRIGHT),
            egui::StrokeKind::Outside,
        );
    }
    let x = if *value {
        rect.right() - 12.0
    } else {
        rect.left() + 12.0
    };
    ui.painter()
        .circle_filled(egui::pos2(x, rect.center().y), 9.0, TEXT);
    response
}

fn drawn_icon_button_with_tooltip(
    ui: &mut egui::Ui,
    icon: NavIcon,
    label: &str,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(rect, 6.0, RAISED);
    }
    paint_icon(
        ui.painter(),
        rect.shrink(5.0),
        icon,
        if response.hovered() { TEXT } else { MUTED },
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response.on_hover_text(tooltip)
}

fn audio_icon_button(
    ui: &mut egui::Ui,
    icon: NavIcon,
    width: f32,
    label: &str,
    active: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
    let hover = response.hovered() || response.has_focus();
    if active || hover {
        ui.painter().rect_filled(
            rect,
            4.0,
            if active {
                Color32::from_rgba_unmultiplied(212, 67, 85, if hover { 75 } else { 36 })
            } else {
                COMPOSER
            },
        );
    }
    let color = if active {
        Color32::from_rgb(237, 82, 101)
    } else if hover {
        TEXT
    } else {
        MUTED
    };
    let size = if width < 20.0 { 12.0 } else { 19.0 };
    paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), egui::vec2(size, size)),
        icon,
        color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response
}

/// Web's spaces/client.ts validation copy.
fn space_name_error(name: &str) -> Option<&'static str> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        Some("Enter a space name.")
    } else if trimmed.chars().count() > 80 {
        Some("Space names can be at most 80 characters.")
    } else if trimmed.chars().any(char::is_control) {
        Some("Space names cannot contain control characters.")
    } else {
        None
    }
}

fn channel_name_error(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        Some("Enter a channel name.")
    } else if name.chars().count() > 80 {
        Some("Channel names can be at most 80 characters.")
    } else if !name
        .split('-')
        .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_lowercase()))
    {
        Some("Use lowercase letters separated by single dashes.")
    } else {
        None
    }
}

fn voice_join_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(108.0, 28.0), egui::Sense::click());
    let hover = response.hovered() || response.has_focus();
    if hover {
        ui.painter().rect_filled(rect, 8.0, RAISED);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            8.0,
            Stroke::new(1.0, TERRACOTTA_BRIGHT),
            egui::StrokeKind::Inside,
        );
    }
    paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 15.0, rect.center().y),
            egui::vec2(14.0, 14.0),
        ),
        NavIcon::Speech,
        MUTED,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 28.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::new(11.0, egui::FontFamily::Name("Satoshi Medium".into())),
        MUTED,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), label)
    });
    response
}

fn paint_icon(painter: &egui::Painter, rect: egui::Rect, icon: NavIcon, color: Color32) {
    // These are the web client's Lucide vectors, not approximate glyphs.
    let source = match icon {
        NavIcon::Chevron => egui::include_image!("../resources/icons/chevron-down.svg"),
        NavIcon::ChevronRight => egui::include_image!("../resources/icons/chevron-right.svg"),
        NavIcon::Close => egui::include_image!("../resources/icons/x.svg"),
        NavIcon::More => egui::include_image!("../resources/icons/ellipsis.svg"),
        NavIcon::Plus => egui::include_image!("../resources/icons/plus.svg"),
        NavIcon::Settings => egui::include_image!("../resources/icons/settings.svg"),
        NavIcon::Hash => egui::include_image!("../resources/icons/hash.svg"),
        NavIcon::Lock => egui::include_image!("../resources/icons/lock.svg"),
        NavIcon::Users => egui::include_image!("../resources/icons/users.svg"),
        NavIcon::Speech => egui::include_image!("../resources/icons/speech.svg"),
        NavIcon::Mic => egui::include_image!("../resources/icons/mic.svg"),
        NavIcon::MicOff => egui::include_image!("../resources/icons/mic-off.svg"),
        NavIcon::Headphones => egui::include_image!("../resources/icons/headphones.svg"),
        NavIcon::VolumeX => egui::include_image!("../resources/icons/volume-x.svg"),
        NavIcon::Menu => egui::include_image!("../resources/icons/menu.svg"),
        NavIcon::PhoneOff => egui::include_image!("../resources/icons/phone-off.svg"),
        NavIcon::HeadphoneOff => egui::include_image!("../resources/icons/headphone-off.svg"),
        NavIcon::AudioLines => egui::include_image!("../resources/icons/audio-lines.svg"),
        NavIcon::BellOff => egui::include_image!("../resources/icons/bell-off.svg"),
    };
    let image = egui::Image::new(source).tint(color);
    if let Ok(egui::load::TexturePoll::Ready { texture }) =
        image.load_for_size(painter.ctx(), rect.size())
    {
        painter.image(
            texture.id,
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            color,
        );
    }
}

fn voice_session_duration(started_at: u64, now: u64) -> String {
    let seconds = now.saturating_sub(started_at) / 1_000;
    let tail = format!("{:02}:{:02}", seconds / 60 % 60, seconds % 60);
    if seconds < 3_600 {
        tail
    } else {
        format!("{}:{tail}", seconds / 3_600)
    }
}

/// A sidebar row's options (⋯) menu.
struct RowMenu<'a> {
    /// Accessible name and tooltip, like "Channel options for general".
    label: String,
    /// Shown only while the row is hovered or the button focused or open, as
    /// on web's DM rows. Its space stays reserved.
    on_hover: bool,
    content: Box<dyn FnOnce(&mut egui::Ui) + 'a>,
}

/// What a sidebar row shows besides its name and icon.
#[derive(Default)]
struct RowExtras<'a> {
    /// Muted rows are dimmed with a bell-slash; this is its tooltip, like
    /// "Muted until 5:00 PM".
    muted: Option<String>,
    duration: Option<&'a str>,
    menu: Option<RowMenu<'a>>,
}

/// Dims a muted row's name and icon.
fn muted_color(color: Color32) -> Color32 {
    color.gamma_multiply(0.5)
}

/// Paints the bell-slash centered in `rect`, with `label` as its tooltip and
/// accessible name.
fn muted_indicator(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id, label: &str) {
    let indicator = ui.interact(rect, id, egui::Sense::hover());
    indicator.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, label));
    paint_icon(ui.painter(), rect, NavIcon::BellOff, MUTED);
    indicator.on_hover_text(label);
}

/// Draws a channel or DM row. Returns the row's response and whether its
/// options menu opened this frame.
fn channel_button(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    name: &str,
    icon: Option<NavIcon>,
    active: bool,
    extras: RowExtras<'_>,
) -> (egui::Response, bool) {
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            active,
            name,
        )
    });
    if active || response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            6.0,
            if active {
                Color32::from_rgba_unmultiplied(182, 77, 50, 40)
            } else {
                RAISED
            },
        );
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0, TERRACOTTA_BRIGHT),
            egui::StrokeKind::Inside,
        );
    }
    let dim = |color| {
        if extras.muted.is_some() {
            muted_color(color)
        } else {
            color
        }
    };
    let color = dim(if active { TEXT } else { MUTED });
    if let Some(icon) = icon {
        paint_icon(
            ui.painter(),
            egui::Rect::from_center_size(
                egui::pos2(rect.left() + 17.5, rect.center().y),
                egui::vec2(17.0, 17.0),
            ),
            icon,
            if active {
                dim(TERRACOTTA_BRIGHT)
            } else {
                color
            },
        );
    }
    let timer_font = egui::FontId::monospace(11.0);
    let timer_right = rect.right() - if extras.menu.is_some() { 34.0 } else { 6.0 };
    let timer_width = extras.duration.map_or(0.0, |text| {
        let galley =
            ui.painter()
                .layout_no_wrap(text.into(), timer_font.clone(), VOICE_SESSION_GREEN);
        let width = galley.size().x;
        let timer_rect = egui::Rect::from_min_size(
            egui::pos2(timer_right - width, rect.center().y - galley.size().y / 2.0),
            galley.size(),
        );
        let timer = ui.interact(
            timer_rect,
            response.id.with("duration"),
            egui::Sense::hover(),
        );
        timer.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Label,
                true,
                format!("Voice session duration: {text}"),
            )
        });
        timer.on_hover_text("Voice session duration");
        ui.painter()
            .galley(timer_rect.min, galley, VOICE_SESSION_GREEN);
        width + 9.0
    });
    let mut name_right = timer_right - timer_width;
    if let Some(label) = &extras.muted {
        let bell = egui::Rect::from_center_size(
            egui::pos2(name_right - 9.0, rect.center().y),
            egui::vec2(14.0, 14.0),
        );
        muted_indicator(ui, bell, response.id.with("muted"), label);
        name_right -= 20.0;
    }
    ui.painter()
        .with_clip_rect(egui::Rect::from_min_max(
            egui::pos2(rect.left() + 34.0, rect.top()),
            egui::pos2(name_right, rect.bottom()),
        ))
        .text(
            egui::pos2(rect.left() + 35.0, rect.center().y - 1.0),
            egui::Align2::LEFT_CENTER,
            name,
            egui::FontId::new(13.0, egui::FontFamily::Name("Satoshi Medium".into())),
            color,
        );
    let mut opened = false;
    if let Some(menu) = extras.menu {
        let row_hovered = ui.rect_contains_pointer(rect);
        let rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 18.0, rect.center().y),
            egui::vec2(32.0, 32.0).min(egui::vec2(32.0, rect.height())),
        );
        let settings = ui.interact(rect, response.id.with("settings"), egui::Sense::click());
        let popup = egui::Popup::menu(&settings);
        let open = popup.is_open();
        opened = settings.clicked() && !open;
        if !menu.on_hover || row_hovered || open || settings.has_focus() {
            if settings.hovered() || settings.has_focus() {
                ui.painter().rect_filled(rect, 8.0, SURFACE);
            }
            if settings.has_focus() {
                ui.painter().rect_stroke(
                    rect,
                    8.0,
                    Stroke::new(1.0, TERRACOTTA_BRIGHT),
                    egui::StrokeKind::Inside,
                );
            }
            paint_icon(
                ui.painter(),
                egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0)),
                NavIcon::More,
                if settings.hovered() || settings.has_focus() {
                    TEXT
                } else {
                    MUTED
                },
            );
        }
        settings.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &menu.label)
        });
        popup
            .align(egui::RectAlign::BOTTOM_END)
            .width(200.0)
            .show(menu.content);
        settings.on_hover_text(menu.label);
    }
    (response, opened)
}

fn users_button(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(32.0, 36.0), egui::Sense::click());
    if active || response.hovered() {
        ui.painter().rect_filled(rect, 7.0, RAISED);
    }
    let color = if response.hovered() || active {
        TEXT
    } else {
        MUTED
    };
    paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(rect.center(), egui::vec2(20.0, 20.0)),
        NavIcon::Users,
        color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            if active {
                "Hide member list"
            } else {
                "Show member list"
            },
        )
    });
    response.on_hover_text(if active {
        "Hide member list"
    } else {
        "Show member list"
    })
}

fn message_row<R>(
    ui: &mut egui::Ui,
    author: &str,
    avatar_id: Option<i32>,
    time: &str,
    guest: bool,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> (egui::Rect, R) {
    // The timestamp's rect (the edit-history target) and the body's result.
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(18, 10))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 4.0);
            ui.horizontal_top(|ui| {
                avatar(ui, author, avatar_id, 34.0, false);
                ui.vertical(|ui| {
                    let timestamp = ui
                        .horizontal_wrapped(|ui| {
                            ui.label(bold(author).size(13.0));
                            if guest {
                                ui.label(RichText::new("GUEST").size(9.0).color(MUTED));
                            }
                            ui.label(RichText::new(time).size(10.0).color(MUTED)).rect
                        })
                        .inner;
                    (timestamp, body(ui))
                })
                .inner
            })
            .inner
        })
        .inner
}

/// Message text with resolved mentions drawn as pills: one weight bolder,
/// primary text color and a terracotta background (24%, or 32% while hovered
/// or focused) with 2px side padding. egui `LayoutJob` backgrounds are plain
/// rectangles, so pills have square corners rather than the 4px corners other
/// clients draw.
///
/// The text stays one selectable label, so wrapping and drag-selection are
/// unchanged. Person pills get click targets on top of it from the laid-out
/// glyphs, one per wrapped row; only the first takes keyboard focus. The
/// hover/focus state of the last pass picks each pill's fill.
fn message_body(
    ui: &mut egui::Ui,
    message: &str,
    text: &str,
    mentions: &[model::Mention],
    describe: impl Fn(&model::Mention) -> String,
) -> Option<PillClick> {
    let pills = mentions::highlights(text, mentions);
    if pills.is_empty() {
        ui.label(RichText::new(text).size(14.0).color(MESSAGE_TEXT));
        return None;
    }
    let ids: Vec<_> = (0..pills.len())
        .map(|index| ui.make_persistent_id(("mention-pill", message, index)))
        .collect();
    let hot: Vec<bool> = ids
        .iter()
        .map(|id| ui.data(|data| data.get_temp::<bool>(*id)).unwrap_or(false))
        .collect();
    let plain = egui::TextFormat::simple(egui::FontId::proportional(14.0), MESSAGE_TEXT);
    let pill = |hot: bool| egui::TextFormat {
        font_id: egui::FontId::new(14.0, egui::FontFamily::Name("Satoshi Medium".into())),
        color: TEXT,
        background: TERRACOTTA.gamma_multiply(if hot { 0.32 } else { 0.24 }),
        // Grow the background into the 2px gaps left on either side.
        expand_bg: 2.0,
        ..Default::default()
    };
    let mut job = egui::text::LayoutJob::default();
    let mut written = 0;
    for ((range, _), hot) in pills.iter().zip(&hot) {
        if range.start > written {
            let gap = if written == 0 { 0.0 } else { 2.0 };
            job.append(&text[written..range.start], gap, plain.clone());
        }
        job.append(&text[range.clone()], 2.0, pill(*hot));
        written = range.end;
    }
    if written < text.len() {
        job.append(&text[written..], 2.0, plain);
    }
    let (position, galley, response) = egui::Label::new(job).layout_in_ui(ui);
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Label, ui.is_enabled(), galley.text())
    });
    if ui.is_rect_visible(response.rect) {
        egui::text_selection::LabelSelectionState::label_text_selection(
            ui,
            &response,
            position,
            galley.clone(),
            ui.visuals().text_color(),
            Stroke::NONE,
        );
    }
    let mut clicked = None;
    for (((range, entry), id), was_hot) in pills.iter().zip(&ids).zip(&hot) {
        if entry.kind != "user" {
            continue;
        }
        let characters = text[..range.start].chars().count()..text[..range.end].chars().count();
        let mut is_hot = false;
        for (row, rect) in pill_rects(&galley, characters).into_iter().enumerate() {
            let rect = rect.translate(position.to_vec2()).expand(2.0);
            let (piece, sense) = if row == 0 {
                (*id, egui::Sense::click())
            } else {
                (id.with(row), egui::Sense::CLICK)
            };
            let target = ui
                .interact(rect, piece, sense)
                .on_hover_cursor(egui::CursorIcon::PointingHand);
            if row == 0 {
                target.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, describe(entry))
                });
            }
            is_hot |= target.hovered() || target.has_focus();
            if target.clicked() {
                clicked = Some(PillClick {
                    id: *id,
                    rect,
                    entry: (*entry).clone(),
                });
            }
        }
        if is_hot != *was_hot {
            ui.data_mut(|data| data.insert_temp(*id, is_hot));
            ui.ctx().request_repaint();
        }
    }
    clicked
}

/// Galley-relative bounds of the characters in `range`, one rect per row.
fn pill_rects(galley: &egui::Galley, range: std::ops::Range<usize>) -> Vec<egui::Rect> {
    let mut rects = Vec::new();
    let mut offset = 0;
    for placed in &galley.rows {
        let mut rect = egui::Rect::NOTHING;
        for (index, glyph) in placed.row.glyphs.iter().enumerate() {
            if range.contains(&(offset + index)) {
                rect = rect.union(glyph.logical_rect().translate(placed.pos.to_vec2()));
            }
        }
        if rect.is_positive() {
            rects.push(rect);
        }
        offset += placed.row.char_count_including_newline();
    }
    rects
}

/// One `@` row in the shared suggestion popup, laid out like an emoji row:
/// a 24px avatar slot, the 12px icon gap, then the name and a muted detail.
fn mention_suggestion(
    ui: &mut egui::Ui,
    candidate: &mentions::Candidate,
    width: f32,
    fill: Color32,
) -> egui::Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let (title, detail) = match candidate {
        mentions::Candidate::Person(person) => {
            (person.display_name.clone(), format!("@{}", person.username))
        }
        mentions::Candidate::Everyone => ("@everyone".into(), "Everyone in this channel".into()),
        mentions::Candidate::Here => ("@here".into(), "Everyone online in this channel".into()),
    };
    let mut text = egui::text::LayoutJob::default();
    text.append(
        &title,
        0.0,
        egui::TextFormat {
            valign: egui::Align::Center,
            ..egui::TextFormat::simple(font, TEXT)
        },
    );
    // Smaller secondary text keeps the specials' descriptions inside 260px.
    text.append(
        &format!(" {detail}"),
        4.0,
        egui::TextFormat {
            valign: egui::Align::Center,
            ..egui::TextFormat::simple(egui::FontId::proportional(11.0), MUTED)
        },
    );
    let slot = egui::Id::new("mention-suggestion-avatar").with(candidate.name());
    let row = egui::Button::new((egui::Atom::custom(slot, egui::vec2(24.0, 24.0)), text))
        .min_size(egui::vec2(width, 44.0))
        .truncate()
        .stroke(Stroke::NONE)
        .fill(fill)
        .atom_ui(ui);
    if let Some(rect) = row.rect(slot) {
        match candidate {
            mentions::Candidate::Person(person) => {
                paint_avatar(ui, rect, &person.display_name, person.avatar_id);
            }
            mentions::Candidate::Everyone | mentions::Candidate::Here => {
                ui.painter()
                    .circle_filled(rect.center(), rect.width() / 2.0, RAISED);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "@",
                    egui::FontId::proportional(rect.width() * 0.5),
                    MUTED,
                );
            }
        }
    }
    row.response
}

fn date_divider(ui: &mut egui::Ui, label: &str) {
    ui.horizontal(|ui| {
        ui.add_space(18.0);
        let available = (ui.available_width() - 36.0).max(0.0);
        let text = RichText::new(label).size(10.0).color(MUTED);
        let label_width = ui.fonts_mut(|fonts| {
            fonts
                .layout_no_wrap(label.into(), egui::FontId::proportional(10.0), MUTED)
                .size()
                .x
        });
        let rule_width = ((available - label_width - 20.0) / 2.0).max(0.0);
        let rule = |ui: &mut egui::Ui| {
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(rule_width, 1.0), egui::Sense::hover());
            ui.painter().line_segment(
                [rect.left_center(), rect.right_center()],
                Stroke::new(1.0, BORDER),
            );
        };
        rule(ui);
        ui.add_space(10.0);
        ui.label(text);
        ui.add_space(10.0);
        rule(ui);
    });
}

fn caper_avatar_index(avatar_id: Option<i32>) -> Option<u16> {
    avatar_id.and_then(|index| u16::try_from(index).ok().filter(|index| *index < 800))
}

fn paint_avatar(ui: &egui::Ui, rect: egui::Rect, name: &str, avatar_id: Option<i32>) {
    if let Some(index) = caper_avatar_index(avatar_id).map(usize::from) {
        egui::Image::from_bytes(
            format!("bytes://caper-avatars-v3/{index}.svg"),
            avatar_images::SVG[index],
        )
        .paint_at(ui, rect);
    } else {
        ui.painter()
            .circle_filled(rect.center(), rect.width() / 2.0, RAISED);
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            name.chars()
                .next()
                .unwrap_or('C')
                .to_uppercase()
                .to_string(),
            egui::FontId::proportional(rect.width() * 0.38),
            TEXT,
        );
    }
}

fn avatar(
    ui: &mut egui::Ui,
    name: &str,
    avatar_id: Option<i32>,
    size: f32,
    speaking: bool,
) -> egui::Rect {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    paint_avatar(ui, rect, name, avatar_id);
    if speaking {
        // Web: caper border with a soft 3px caper halo.
        ui.painter().circle_stroke(
            rect.center(),
            size / 2.0 + 1.5,
            Stroke::new(3.0, CAPER.gamma_multiply(0.2)),
        );
        ui.painter()
            .circle_stroke(rect.center(), size / 2.0 - 1.0, Stroke::new(2.0, CAPER));
    }
    rect
}

fn presence_avatar(ui: &mut egui::Ui, name: &str, avatar_id: Option<i32>, size: f32, status: &str) {
    let rect = avatar(ui, name, avatar_id, size, false);
    let dot = egui::pos2(rect.right() - 2.0, rect.bottom() - 3.0);
    ui.painter().circle_filled(dot, 5.0, SIDEBAR);
    ui.painter().circle_filled(
        dot,
        3.5,
        match status {
            "online" => CAPER,
            "idle" => Color32::from_rgb(205, 158, 82),
            _ => BORDER,
        },
    );
}

#[cfg(test)]
mod avatar_tests {
    use super::caper_avatar_index;
    #[test]
    fn persisted_indices_and_fallback() {
        for index in [0, 31, 32, 255, 256, 799] {
            assert_eq!(caper_avatar_index(Some(index)), Some(index as u16));
        }
        assert_eq!(caper_avatar_index(None), None);
        assert_eq!(caper_avatar_index(Some(-1)), None);
        assert_eq!(caper_avatar_index(Some(800)), None);
    }
}

fn normalize_username(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
        .take(32)
        .collect()
}

fn normalize_channel(value: &str) -> String {
    let mut output = String::new();
    for character in value.to_lowercase().chars() {
        if character.is_ascii_lowercase() {
            output.push(character);
        } else if (character.is_whitespace() || character == '-')
            && !output.is_empty()
            && !output.ends_with('-')
        {
            output.push('-');
        }
        if output.chars().count() >= 80 {
            break;
        }
    }
    output
}

fn member_page_ids(detail: &SpaceDetail, page: usize) -> Vec<String> {
    detail
        .members
        .iter()
        .skip(page * MEMBER_PAGE_SIZE)
        .take(MEMBER_PAGE_SIZE)
        .map(|member| member.id.clone())
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
struct DisplayDate {
    key: String,
    label: String,
}

fn timestamp_parts<Tz: TimeZone>(timestamp: &str, timezone: &Tz) -> Option<(DisplayDate, String)>
where
    Tz::Offset: std::fmt::Display,
{
    let parsed = DateTime::parse_from_rfc3339(timestamp).ok()?;
    let local = parsed.with_timezone(timezone);
    Some((
        DisplayDate {
            key: local.format("%Y-%m-%d").to_string(),
            label: local.format("%A, %B %-d, %Y").to_string(),
        },
        local.format("%-I:%M %p").to_string(),
    ))
}

fn display_date(timestamp: &str) -> Option<DisplayDate> {
    timestamp_parts(timestamp, &Local).map(|parts| parts.0)
}

fn take_date_divider(previous: &mut Option<String>, date: &str) -> bool {
    if previous.as_deref() == Some(date) {
        return false;
    }
    *previous = Some(date.to_owned());
    true
}

fn display_time(timestamp: &str) -> String {
    timestamp_parts(timestamp, &Local).map_or_else(|| timestamp.to_owned(), |parts| parts.1)
}

fn configure(context: &egui::Context) {
    egui_extras::install_image_loaders(context);
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        (
            "Satoshi Regular",
            include_bytes!("../../../../shared/fonts/cache/Satoshi-Regular.otf").as_slice(),
        ),
        (
            "Satoshi Medium",
            include_bytes!("../../../../shared/fonts/cache/Satoshi-Medium.otf").as_slice(),
        ),
        (
            "Satoshi Bold",
            include_bytes!("../../../../shared/fonts/cache/Satoshi-Bold.otf").as_slice(),
        ),
        (
            "Satoshi Black",
            include_bytes!("../../../../shared/fonts/cache/Satoshi-Black.otf").as_slice(),
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
        fonts
            .families
            .insert(egui::FontFamily::Name(name.into()), vec![name.to_owned()]);
    }
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "Satoshi Regular".into());
    context.set_fonts(fonts);

    let mut style = (*context.style()).clone();
    style.visuals.dark_mode = true;
    style.visuals.panel_fill = BLACKOUT;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = COMPOSER;
    style.visuals.widgets.inactive.bg_fill = RAISED;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    style.visuals.widgets.inactive.corner_radius = CornerRadius::same(7);
    style.visuals.widgets.hovered.bg_fill = COMPOSER;
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, TERRACOTTA_BRIGHT);
    style.visuals.widgets.hovered.corner_radius = CornerRadius::same(7);
    style.visuals.widgets.active.bg_fill = Color32::from_rgb(57, 35, 30);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, TERRACOTTA_BRIGHT);
    style.visuals.widgets.active.corner_radius = CornerRadius::same(7);
    style.visuals.widgets.open.bg_fill = COMPOSER;
    style.visuals.widgets.open.bg_stroke = Stroke::new(1.0, TERRACOTTA_BRIGHT);
    style.visuals.widgets.open.corner_radius = CornerRadius::same(7);
    style.visuals.selection.bg_fill = TERRACOTTA;
    style.visuals.override_text_color = Some(TEXT);
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(10.0, 7.0);
    style.spacing.interact_size.y = 36.0;
    style.visuals.window_corner_radius = CornerRadius::same(8);
    context.set_style(style);
}

fn bold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(egui::FontFamily::Name("Satoshi Bold".into()))
}

fn black(text: impl Into<String>) -> RichText {
    RichText::new(text).family(egui::FontFamily::Name("Satoshi Black".into()))
}

fn login_action(ui: &mut egui::Ui, text: &str, disabled: bool) -> egui::Response {
    let sense = if disabled {
        egui::Sense::hover()
    } else {
        egui::Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 58.0), sense);
    let fill = if disabled {
        Color32::from_rgb(91, 48, 39)
    } else if response.hovered() {
        TERRACOTTA_BRIGHT
    } else {
        TERRACOTTA
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    ui.painter().text(
        egui::pos2(rect.left() + 21.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        egui::FontId::new(16.0, egui::FontFamily::Name("Satoshi Medium".into())),
        if disabled { MUTED } else { TEXT },
    );
    let center = egui::pos2(rect.right() - 22.0, rect.center().y);
    let stroke = Stroke::new(1.5, if disabled { MUTED } else { TEXT });
    ui.painter().line_segment(
        [
            egui::pos2(center.x - 9.0, center.y),
            egui::pos2(center.x + 7.0, center.y),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x + 2.0, center.y - 5.0),
            egui::pos2(center.x + 7.0, center.y),
        ],
        stroke,
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x + 2.0, center.y + 5.0),
            egui::pos2(center.x + 7.0, center.y),
        ],
        stroke,
    );
    response
}

/// Web's labelled 0–200% volume control: name and value above the slider.
fn volume_slider(ui: &mut egui::Ui, name: &str, value: &mut u16) -> bool {
    let label = ui
        .horizontal(|ui| {
            let label = ui.label(bold(name).size(12.8).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(bold(format!("{value}%")).size(12.8).color(MUTED));
            });
            label
        })
        .inner;
    ui.add_space(5.0);
    ui.scope(|ui| {
        ui.spacing_mut().slider_width = ui.available_width();
        ui.add(
            egui::Slider::new(value, 0..=200)
                .show_value(false)
                .trailing_fill(true),
        )
        .labelled_by(label.id)
        .changed()
    })
    .inner
}

/// Web's `.voice-button`: terracotta outline on a faint terracotta fill.
fn outlined_button(ui: &mut egui::Ui, text: &str, pressed: bool) -> egui::Response {
    ui.spacing_mut().button_padding = egui::vec2(12.0, 7.0);
    // Web's flex button fills its column with its label at the start.
    let button = ui
        .with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
            ui.add(
                egui::Button::new(
                    bold(text)
                        .size(12.5)
                        .color(Color32::from_rgb(227, 153, 133)),
                )
                .fill(Color32::from_rgba_unmultiplied(182, 77, 50, 36))
                .stroke(Stroke::new(1.0, Color32::from_rgb(137, 70, 53)))
                .corner_radius(8)
                .min_size(egui::vec2(0.0, 36.0)),
            )
        })
        .inner;
    button.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, button.enabled(), pressed, text)
    });
    button
}

/// Web's dotted 40-bar input meter, faded in from the left.
fn input_meter(ui: &mut egui::Ui, levels: &std::collections::VecDeque<f32>, active: bool) {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 48.0), egui::Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Other,
            true,
            if active {
                "Received microphone level"
            } else {
                "Microphone level inactive"
            },
        )
    });
    let count = levels.len().max(1);
    let step = rect.width() / count as f32;
    let lit = Color32::from_rgb(145, 171, 120);
    for (index, level) in levels.iter().enumerate() {
        let x = rect.left() + step * (index as f32 + 0.5);
        let fade = ((x - rect.left()) / (rect.width() * 0.65)).min(1.0);
        let height = 3.0 + (level * 8.0).round() * 4.0;
        let dots = (height / 4.0).ceil() as usize;
        let color = if active { lit } else { BORDER }.gamma_multiply(fade);
        for dot in 0..dots {
            let offset = (dot as f32 - (dots as f32 - 1.0) / 2.0) * 4.0;
            ui.painter()
                .circle_filled(egui::pos2(x, rect.center().y + offset), 1.4, color);
        }
    }
}

fn primary(ui: &mut egui::Ui, text: &str, disabled: bool) -> egui::Response {
    ui.add_enabled(
        !disabled,
        egui::Button::new(text)
            .fill(TERRACOTTA)
            .min_size(egui::vec2(ui.available_width(), 44.0))
            .corner_radius(8),
    )
}

/// Web's `.space-field` input; channel creation shows its # or lock icon inside.
fn name_field(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut String,
    placeholder: &str,
    icon: Option<NavIcon>,
) {
    ui.label(label);
    let response = ui.add_sized(
        [ui.available_width(), 42.0],
        egui::TextEdit::singleline(value)
            .vertical_align(egui::Align::Center)
            .char_limit(80)
            .hint_text(RichText::new(placeholder).color(MUTED.gamma_multiply(0.65)))
            .margin(egui::Margin {
                left: if icon.is_some() { 37 } else { 8 },
                right: 8,
                top: 4,
                bottom: 4,
            }),
    );
    if let Some(icon) = icon {
        paint_icon(
            ui.painter(),
            egui::Rect::from_center_size(
                egui::pos2(response.rect.left() + 20.0, response.rect.center().y),
                egui::vec2(18.0, 18.0),
            ),
            icon,
            MUTED,
        );
    }
}

/// Web's `.secondary` dialog button.
fn secondary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    ui.spacing_mut().button_padding.x = 12.0;
    ui.add_enabled(
        enabled,
        egui::Button::new(bold(text).size(12.0))
            .fill(RAISED)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(7)
            .min_size(egui::vec2(0.0, 36.0)),
    )
}

/// Web's `.primary` dialog button.
fn primary_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    ui.spacing_mut().button_padding.x = 12.0;
    ui.add_enabled(
        enabled,
        egui::Button::new(bold(text).size(12.0))
            .fill(TERRACOTTA)
            .stroke(Stroke::new(1.0, TERRACOTTA))
            .corner_radius(7)
            .min_size(egui::vec2(0.0, 36.0)),
    )
}

/// Web's `SubmitRow`: Cancel, then the primary action, aligned to the end.
/// Returns (cancel, submit).
fn dialog_actions(ui: &mut egui::Ui, label: &str, enabled: bool) -> (bool, bool) {
    ui.add_space(20.0);
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let submit = primary_button(ui, label, enabled).clicked();
            let cancel = secondary_button(ui, "Cancel", true).clicked();
            (cancel, submit)
        })
        .inner
    })
    .inner
}

fn message_skeleton(ui: &mut egui::Ui) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(), "Loading messages")
    });
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let width = (rect.width() - 80.0).max(0.0);
    for (row, fraction) in [0.72, 0.92, 0.56, 0.81, 0.64].into_iter().enumerate() {
        let top = rect.top() + 28.0 + row as f32 * 84.0;
        if top + 54.0 > rect.bottom() {
            break;
        }
        painter.circle_filled(egui::pos2(rect.left() + 35.0, top + 17.0), 17.0, BORDER);
        for (offset, length, height) in [
            (0.0, width.min(96.0), 12.0),
            (24.0, width.min(520.0) * fraction, 10.0),
            (42.0, width.min(340.0) * fraction, 10.0),
        ] {
            painter.rect_filled(
                egui::Rect::from_min_size(
                    egui::pos2(rect.left() + 62.0, top + offset),
                    egui::vec2(length, height),
                ),
                4.0,
                BORDER,
            );
        }
    }
}

/// Web's `.chat-state`: a 120 px block at the top of the list, content centered.
fn chat_state(ui: &mut egui::Ui, lines: u8, content: impl FnOnce(&mut egui::Ui)) {
    let height = if lines > 1 { 58.0 } else { 18.0 };
    ui.allocate_ui_with_layout(
        egui::vec2(ui.available_width(), 120.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.set_min_height(120.0);
            ui.add_space((120.0 - height) / 2.0);
            content(ui);
        },
    );
}

/// Web's composer counter tones.
fn counter_tone(count: usize) -> Color32 {
    match count {
        3_900.. => Color32::from_rgb(0xff, 0x82, 0x7c),
        3_750.. => Color32::from_rgb(0xed, 0xa3, 0x61),
        3_500.. => Color32::from_rgb(0xe4, 0xc7, 0x6a),
        _ => MUTED,
    }
}

/// `toLocaleString()` for the counter: 3500 → "3,500".
fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Web's three typing dots: 4 px, 3 px apart, bouncing 3 px every 1.2 s.
fn typing_dots(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 16.0), egui::Sense::hover());
    let time = ui.input(|input| input.time);
    for index in 0..3 {
        let phase = ((time - index as f64 * 0.15).rem_euclid(1.2) / 1.2) as f32;
        // Keyframes: rest at 0%/60%/100%, peak at 30%, eased in and out.
        let lift = if phase < 0.6 {
            let t = if phase < 0.3 {
                phase / 0.3
            } else {
                (0.6 - phase) / 0.3
            };
            t * t * (3.0 - 2.0 * t)
        } else {
            0.0
        };
        let center = egui::pos2(
            rect.left() + 2.0 + index as f32 * 7.0,
            rect.center().y - 3.0 * lift,
        );
        ui.painter()
            .circle_filled(center, 2.0, MUTED.gamma_multiply(0.45 + 0.55 * lift));
    }
    ui.ctx().request_repaint();
}

/// Web's narrow `.navigation-toggle`: a bordered icon + label button.
fn navigation_toggle(ui: &mut egui::Ui, icon: NavIcon, text: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(
        text.into(),
        egui::FontId::new(11.2, egui::FontFamily::Name("Satoshi Bold".into())),
        MUTED,
    );
    let size = egui::vec2(9.0 + 15.0 + 6.0 + galley.size().x + 9.0, 34.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let hover = response.hovered() || response.has_focus();
    if hover {
        ui.painter().rect_filled(rect, 6.0, RAISED);
    }
    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, BORDER),
        egui::StrokeKind::Inside,
    );
    let color = if hover { TEXT } else { MUTED };
    paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 16.5, rect.center().y),
            egui::vec2(15.0, 15.0),
        ),
        icon,
        color,
    );
    ui.painter().galley_with_override_text_color(
        egui::pos2(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response
        .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text));
    response
}

fn destructive(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::from_rgb(255, 128, 149)))
            .stroke(Stroke::new(1.0, Color32::from_rgb(185, 54, 77)))
            .fill(Color32::TRANSPARENT)
            .min_size(egui::vec2(112.0, 36.0)),
    )
}

fn full_bleed_separator(ui: &egui::Ui, y: f32) {
    // Panels clip to their padded content; extend the clip as well as the line.
    let mut painter = ui.painter().clone();
    painter.set_clip_rect(ui.clip_rect().expand2(egui::vec2(12.0, 0.5)));
    painter.hline(
        (ui.max_rect().left() - 12.0)..=(ui.max_rect().right() + 12.0),
        y,
        Stroke::new(1.0, BORDER),
    );
}

fn notices(ui: &mut egui::Ui, error: &Option<String>, warning: &Option<String>) {
    if let Some(error) = error {
        ui.add_space(8.0);
        ui.colored_label(ERROR, error);
    }
    if let Some(warning) = warning {
        ui.add_space(8.0);
        ui.colored_label(Color32::from_rgb(232, 189, 113), warning);
    }
}

fn endpoint(args: &[String], fixture: Option<&str>) -> Result<String, String> {
    let explicit = args
        .windows(2)
        .find(|pair| pair[0] == "--api-url")
        .map(|pair| pair[1].clone())
        .or_else(|| std::env::var("CAPER_API_URL").ok());
    let endpoint = explicit.unwrap_or_else(|| {
        if fixture.is_some() {
            "http://127.0.0.1:3001".into()
        } else {
            "https://caper.chat".into()
        }
    });
    if fixture.is_some() {
        let parsed = url::Url::parse(&endpoint)
            .map_err(|_| "Fixture API URL must be a valid loopback URL.".to_owned())?;
        let loopback = parsed
            .host_str()
            .and_then(|host| host.parse::<std::net::IpAddr>().ok())
            .is_some_and(|host| host.is_loopback())
            || parsed.host_str() == Some("localhost");
        if !loopback {
            return Err("--fixture refuses non-loopback API endpoints.".into());
        }
    }
    Ok(endpoint)
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let fixture = args
        .windows(2)
        .find(|pair| pair[0] == "--fixture")
        .map(|pair| pair[1].clone());
    let endpoint = endpoint(&args, fixture.as_deref()).unwrap_or_else(|message| {
        eprintln!("Caper could not start: {message}");
        std::process::exit(2)
    });
    let api = api::Api::new(&endpoint).unwrap_or_else(|message| {
        eprintln!("Caper could not start: {message}");
        std::process::exit(2)
    });
    let viewport_size = if fixture.as_deref().is_some_and(|name| {
        matches!(
            name,
            "parity-narrow"
                | "parity-browse"
                | "parity-opening-narrow"
                | "parity-channel-preview-narrow"
                | "parity-channel-directory-narrow"
                | "parity-voice-rosters-narrow"
                | "parity-invitation-narrow"
                | "parity-update-narrow"
                | "parity-update-download"
        )
    }) {
        [390.0, 844.0]
    } else {
        [1440.0, 900.0]
    };
    let icon = image::load_from_memory(include_bytes!("../resources/caper-icon.png"))
        .expect("bundled Caper icon is valid PNG")
        .to_rgba8();
    let (width, height) = icon.dimensions();
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(viewport_size)
        .with_min_inner_size([320.0, 560.0])
        .with_icon(egui::IconData {
            rgba: icon.into_raw(),
            width,
            height,
        });
    // Match caper.desktop so Wayland can resolve the packaged icon.
    #[cfg(target_os = "linux")]
    let viewport = viewport.with_app_id("caper");
    eframe::run_native(
        "Caper",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Wgpu,
            persist_window: fixture.is_none(),
            viewport,
            ..Default::default()
        },
        Box::new(move |creation| {
            let mut app = CaperApp::new(&creation.egui_ctx, api, fixture.as_deref());
            if let Some(storage) = creation.storage {
                app.restore_preferences(storage);
            }
            if fixture.is_none() {
                app.daily_icon = Some(daily_icon::DailyIcon::load(creation.storage));
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        AccountOperation, CaperApp, ComposerToken, ConnectionReport, Dialog, GatewayEvent,
        NavigationTarget, PendingReaction, PendingSend, Phase, SelfDirectTarget, Suggestion,
        TERRACOTTA, TEXT, endpoint, media, member_page_ids, normalize_channel,
        permanent_send_rejection, pill_rects, projected_reactions, take_date_divider,
        timestamp_parts, voice,
    };
    use crate::{mentions, navigation};
    use std::time::{Duration, Instant};

    #[test]
    fn all_wordmark_characters_render_without_the_avatar_tile() {
        for (index, svg) in crate::avatar_images::BRANDING.iter().enumerate() {
            let image = egui_extras::image::load_svg_bytes(svg, &Default::default())
                .expect("bundled branding character is valid SVG");
            assert_eq!(image.size, [256, 256]);
            assert_eq!(image.pixels[128 * 256 + 252].a(), 0, "tile in {index}");
            assert!(
                image.pixels.iter().any(|pixel| pixel.a() == 255),
                "blank {index}"
            );
        }
    }

    use crate::model::{
        self, Account, Author, ChatSession, Content, History, HistoryPlace, Member, Message, Space,
        SpaceDetail, Spaces,
    };

    #[test]
    fn pending_reactions_project_own_membership_over_latest_authoritative_counts() {
        let mut pending = std::collections::BTreeMap::new();
        pending.insert(
            ("message".into(), "👍".into()),
            PendingReaction {
                desired: false,
                sent: true,
                visible: true,
                superseded: false,
            },
        );
        pending.insert(
            ("message".into(), "🎉".into()),
            PendingReaction {
                desired: true,
                sent: false,
                visible: true,
                superseded: false,
            },
        );
        let authoritative = vec![
            model::Reaction {
                emoji: "👍".into(),
                author_ids: vec!["me".into(), "other".into(), "gateway".into()],
            },
            model::Reaction {
                emoji: "🎉".into(),
                author_ids: vec!["other".into()],
            },
        ];

        let projected = projected_reactions(&authoritative, "message", Some("me"), &pending);

        assert_eq!(
            authoritative[0].author_ids.len(),
            3,
            "source stays authoritative"
        );
        assert_eq!(projected[0].author_ids, ["other", "gateway"]);
        assert_eq!(projected[1].author_ids, ["other", "me"]);

        let only_me = vec![model::Reaction {
            emoji: "👍".into(),
            author_ids: vec!["me".into()],
        }];
        assert!(
            projected_reactions(&only_me, "message", Some("me"), &pending)
                .iter()
                .all(|reaction| reaction.emoji != "👍")
        );
    }

    #[test]
    fn failed_reaction_overlay_rolls_back_without_hiding_other_authors() {
        let pending = std::collections::BTreeMap::from([(
            ("message".into(), "👍".into()),
            PendingReaction {
                desired: false,
                sent: false,
                visible: false,
                superseded: false,
            },
        )]);
        let authoritative = vec![model::Reaction {
            emoji: "👍".into(),
            author_ids: vec!["me".into(), "other".into()],
        }];
        assert_eq!(
            projected_reactions(&authoritative, "message", Some("me"), &pending),
            authoritative
        );
    }

    #[test]
    fn rapid_reaction_failure_retries_latest_intent_and_keeps_other_emoji_moving() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-reactions"),
        );
        let (events, receiver) = std::sync::mpsc::channel();
        app.worker.events = receiver;
        let message = app.timeline.messages().nth(1).unwrap().clone();
        let channel = app.selected_channel.clone().unwrap();
        let key = (message.id.clone(), "👀".to_owned());
        app.set_reaction(&message.id, "👀", true);
        app.set_reaction(&message.id, "👀", false);
        app.set_reaction(&message.id, "👀", true);
        assert!(app.pending_reactions[&key].superseded);
        let fail = || crate::worker::Event::Reacted {
            generation: app.generation,
            channel: channel.clone(),
            message: message.id.clone(),
            emoji: "👀".into(),
            active: true,
            result: Err(crate::worker::SendFailure {
                status: Some(503),
                message: "failed".into(),
                code: None,
            }),
        };
        events.send(fail()).unwrap();
        app.receive();
        assert!(
            app.pending_reactions[&key].sent,
            "latest intent automatically follows superseded failure"
        );
        assert!(app.pending_reactions[&key].visible);
        assert!(!app.pending_reactions[&key].superseded);
        assert!(!app.reaction_errors.contains_key(&message.id));

        app.set_reaction(&message.id, "🎉", true);
        events
            .send(crate::worker::Event::Reacted {
                generation: app.generation,
                channel: channel.clone(),
                message: message.id.clone(),
                emoji: "👀".into(),
                active: true,
                result: Err(crate::worker::SendFailure {
                    status: Some(503),
                    message: "failed".into(),
                    code: None,
                }),
            })
            .unwrap();
        app.receive();
        assert!(
            !app.pending_reactions[&key].visible,
            "current failure rolls back"
        );
        assert!(
            app.pending_reactions[&(message.id.clone(), "🎉".into())].sent,
            "failure cannot block another emoji"
        );
        assert!(app.reaction_errors.contains_key(&message.id));
        events
            .send(crate::worker::Event::Reacted {
                generation: app.generation,
                channel,
                message: message.id.clone(),
                emoji: "🎉".into(),
                active: true,
                result: Ok(model::ReactionUpdate {
                    kind: "message.reactions".into(),
                    schema_version: 1,
                    channel_id: message.channel_id.clone(),
                    message_id: message.id.clone(),
                    seq: "5".into(),
                    reactions: message.reactions.clone(),
                }),
            })
            .unwrap();
        app.receive();
        assert!(
            app.reaction_errors.contains_key(&message.id),
            "another emoji's success cannot hide the failure"
        );
        let output = render(&mut app, &context, vec![]);
        assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Text(text) if text.galley.job.text.contains("Saving reaction"))));
    }

    use crate::worker::LoadError;
    use chrono::FixedOffset;
    use eframe::egui;

    fn render(
        app: &mut CaperApp,
        context: &egui::Context,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 900.0),
                )),
                events,
                ..Default::default()
            },
            |context| app.page(context),
        )
    }

    #[test]
    fn rendering_history_restores_messages_reactions_and_cursor_after_each_frame() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-reactions"),
        );
        let mut messages: Vec<_> = app.timeline.messages().cloned().collect();
        messages[1].content.text = "Wrapped history must remain authoritative. ".repeat(12)
            + "\nA second line has different geometry.";
        let cursor = app.timeline.cursor();
        app.timeline.reset(messages.clone(), &cursor).unwrap();
        for width in [1440.0, 840.0, 390.0, 1440.0] {
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.0),
                    )),
                    ..Default::default()
                },
                |context| app.page(context),
            );
            assert_eq!(app.timeline.cursor(), cursor);
            assert_eq!(
                app.timeline.messages().cloned().collect::<Vec<_>>(),
                messages
            );
        }
    }

    #[test]
    fn pinned_message_has_gold_attribution_above_only_two_compact_hover_actions() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.session = Some(session());
        let mut message = app.timeline.messages().next().unwrap().clone();
        message.author.name = "Pinned original author".into();
        message.content.text = "Pinned original text".into();
        let mut pinner = message.author.clone();
        pinner.name = "Fixture Pinner".into();
        message.pin = Some(crate::model::Pin {
            author: pinner,
            created_at: message.created_at.clone(),
        });
        message.pin_seq = Some("7".into());
        app.timeline.reset(vec![message.clone()], "7").unwrap();
        render(&mut app, &context, vec![]);
        render(&mut app, &context, vec![]);
        context.enable_accesskit();
        let output = render(&mut app, &context, vec![]);
        assert!(
            text_position(&output, "Pinned by Fixture Pinner").y
                < text_position(&output, "Pinned original author").y
        );
        assert!(
            text_position(&output, "Pinned original author").y
                < text_position(&output, "Pinned original text").y
        );
        let gold = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "Pinned by Fixture Pinner" => {
                    Some(&text.galley.job)
                }
                _ => None,
            })
            .unwrap();
        assert!(
            gold.sections
                .iter()
                .all(|section| section.format.color == egui::Color32::from_rgb(228, 199, 106))
        );
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(rect) if rect.fill == egui::Color32::from_rgba_unmultiplied(228, 199, 106, 15) && rect.rect.width() > 700.0
        )), "a short pinned message must still highlight the full conversation row");
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        let bounds = |label| {
            nodes
                .iter()
                .find_map(|(_, node)| {
                    (node.label() == Some(label))
                        .then(|| node.bounds())
                        .flatten()
                })
                .unwrap()
        };
        let emoji = bounds("Add reaction");
        let more = bounds("Message actions");
        for button in [emoji, more] {
            assert_eq!((button.width(), button.height()), (24.0, 24.0));
        }
        assert_eq!(more.x0 - emoji.x1, 2.0);
        assert!(
            !nodes
                .iter()
                .any(|(_, node)| matches!(node.label(), Some("Pin" | "Unpin")))
        );
        let action_rect = egui::Rect::from_min_max(
            egui::pos2(emoji.x0 as f32, emoji.y0 as f32),
            egui::pos2(more.x1 as f32, more.y1 as f32),
        );
        let painted = |output: &egui::FullOutput| {
            output.shapes.iter().any(|shape| match &shape.shape {
                egui::Shape::Rect(rect) => {
                    rect.brush.is_some() && action_rect.contains_rect(rect.rect)
                }
                egui::Shape::Mesh(mesh) => action_rect.contains_rect(mesh.calc_bounds()),
                _ => false,
            })
        };
        assert!(
            !painted(&output),
            "actions must not paint without hover or focus"
        );
        render(
            &mut app,
            &context,
            vec![egui::Event::PointerMoved(text_position(
                &output,
                "Pinned original text",
            ))],
        );
        let started = std::time::Instant::now();
        while context.has_pending_images() {
            assert!(started.elapsed() < std::time::Duration::from_secs(10));
            std::thread::sleep(std::time::Duration::from_millis(10));
            render(&mut app, &context, vec![]);
        }
        assert!(
            painted(&render(&mut app, &context, vec![])),
            "hover reveals compact actions"
        );
        click(
            &mut app,
            &context,
            action_rect.right_center() - egui::vec2(12.0, 0.0),
        );
        let opened = render(&mut app, &context, vec![]);
        assert!(opened.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Unpin")));
        egui::Popup::close_all(&context);
        app.showing_pins = true;
        app.timeline.reset_pins(vec![message]).unwrap();
        let pins = render(&mut app, &context, vec![]);
        assert!(pins.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Unpin")), "pin list retains direct Unpin");
    }

    #[test]
    fn update_download_button_opens_the_platform_installer() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-update-download"),
        );
        let mut frame = |events| {
            context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |context| app.update_notice(context),
            )
        };
        // Let the window and its nested scroll area finish egui's sizing passes.
        for _ in 0..3 {
            frame(vec![]);
        }
        let output = frame(vec![]);
        let pos = text_position(&output, "Download");
        for pressed in [true, false] {
            let output = frame(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]);
            let opened = output.platform_output.commands.iter().find_map(|command| {
                if let egui::OutputCommand::OpenUrl(url) = command {
                    Some(url.url.as_str())
                } else {
                    None
                }
            });
            let expected = if cfg!(windows) {
                "https://github.com/joswayski/caper/releases/download/native-latest/Caper-Windows-x64-Setup.exe"
            } else {
                "https://github.com/joswayski/caper/releases/download/native-latest/Caper-Linux-x64.deb"
            };
            assert_eq!(opened, (!pressed).then_some(expected));
        }
    }

    #[test]
    fn update_actions_stay_visible_before_and_after_scrolling_at_small_sizes() {
        for size in [
            egui::vec2(320.0, 560.0),
            egui::vec2(390.0, 600.0),
            egui::vec2(960.0, 540.0),
            egui::vec2(1440.0, 900.0),
        ] {
            for fixture in [
                "parity-update",
                "parity-update-download",
                "parity-update-error",
                "parity-update-incomplete",
            ] {
                let context = egui::Context::default();
                let mut app = CaperApp::new(
                    &context,
                    crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                    Some(fixture),
                );
                let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
                let mut time = 0.0;
                let mut frame = |events| {
                    time += 1.0 / 60.0;
                    context.run(
                        egui::RawInput {
                            screen_rect: Some(screen),
                            time: Some(time),
                            events,
                            ..Default::default()
                        },
                        |context| app.update_notice(context),
                    )
                };
                let visible = |output: &egui::FullOutput, label: &str| {
                    output.shapes.iter().any(|shape| {
                        if let egui::Shape::Text(text) = &shape.shape {
                            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                            text.galley.job.text == label
                                && screen.contains_rect(rect)
                                && shape.clip_rect.contains_rect(rect)
                        } else {
                            false
                        }
                    })
                };
                for _ in 0..3 {
                    frame(vec![]);
                }
                let initial = frame(vec![]);
                let action = if fixture == "parity-update-download" {
                    "Download"
                } else {
                    "Restart to update"
                };
                assert!(
                    visible(&initial, action),
                    "{fixture} {size:?}: action clipped before scrolling"
                );
                assert!(
                    visible(&initial, "Later"),
                    "{fixture} {size:?}: Later clipped"
                );
                let action_pos = text_position(&initial, action);
                let notes_pos = text_position(&initial, "0.1.42");
                frame(vec![
                    egui::Event::PointerMoved(notes_pos),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -4000.0),
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
                for _ in 0..60 {
                    frame(vec![]);
                }
                let scrolled = frame(vec![]);
                assert!(
                    visible(&scrolled, "0.1.36"),
                    "{fixture} {size:?}: skipped version cannot be reached"
                );
                assert!(visible(&scrolled, action));
                assert_eq!(
                    text_position(&scrolled, action),
                    action_pos,
                    "scroll moved the action footer"
                );
                let later = text_position(&scrolled, "Later");
                for pressed in [true, false] {
                    frame(vec![
                        egui::Event::PointerMoved(later),
                        egui::Event::PointerButton {
                            pos: later,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]);
                }
                assert!(
                    app.updates.available().is_none(),
                    "Later must still work after scrolling"
                );
            }
        }
    }

    #[test]
    fn message_dates_and_times_share_the_local_calendar_boundary() {
        let pacific = FixedOffset::west_opt(8 * 60 * 60).unwrap();
        let (before, before_time) = timestamp_parts("2026-01-02T07:59:00Z", &pacific).unwrap();
        let (after, after_time) = timestamp_parts("2026-01-02T08:00:00Z", &pacific).unwrap();

        assert_eq!(before.key, "2026-01-01");
        assert_eq!(before.label, "Thursday, January 1, 2026");
        assert_eq!(before_time, "11:59 PM");
        assert_eq!(after.key, "2026-01-02");
        assert_eq!(after.label, "Friday, January 2, 2026");
        assert_eq!(after_time, "12:00 AM");
    }

    #[test]
    fn date_dividers_cover_first_and_day_changes_without_pending_duplicates() {
        let mut previous = None;
        assert!(take_date_divider(&mut previous, "2026-09-28"));
        assert!(!take_date_divider(&mut previous, "2026-09-28"));
        assert!(take_date_divider(&mut previous, "2026-09-29"));
        // A pending message on the latest loaded message's day reuses its divider.
        assert!(!take_date_divider(&mut previous, "2026-09-29"));
        // A pending-only conversation still receives the first divider.
        assert!(take_date_divider(&mut None, "2026-09-29"));
    }

    fn click(app: &mut CaperApp, context: &egui::Context, pos: egui::Pos2) {
        for pressed in [true, false] {
            render(
                app,
                context,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
    }

    /// A loopback API answering `"METHOD /path"` routes (anything else is a
    /// 404) that records each request line and body.
    fn account_server(
        routes: Vec<(&'static str, u16, &'static str)>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.local_addr().unwrap());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for stream in server.incoming() {
                let Ok(stream) = stream else { break };
                stream.set_read_timeout(Some(Duration::from_secs(2))).ok();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).is_err() || line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0; length];
                let _ = reader.read_exact(&mut body);
                let line = request.trim_end().trim_end_matches(" HTTP/1.1").to_owned();
                log.lock().unwrap().push(
                    format!("{line} {}", String::from_utf8_lossy(&body))
                        .trim_end()
                        .to_owned(),
                );
                let (status, reply) = routes
                    .iter()
                    .find(|(route, _, _)| *route == line)
                    .map_or((404, r#"{"error":"Not found"}"#), |(_, status, body)| {
                        (*status, *body)
                    });
                let _ = write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
            }
        });
        (base, seen)
    }

    fn receive_until(app: &mut CaperApp, done: impl Fn(&CaperApp) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(app) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            app.receive();
        }
        assert!(done(app), "timed out waiting for the worker");
    }

    fn last_text_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.pos + egui::vec2(4.0, 4.0))
                }
                _ => None,
            })
            .next_back()
            .unwrap_or_else(|| panic!("missing {label}"))
    }

    const REQUEST_BAR: &str =
        "TEST FIXTURE Jordan (@jordan) wants to message you. You don't share a space.";

    #[test]
    fn message_requests_are_read_only_uncounted_and_accept_by_id() {
        let (base, seen) = account_server(vec![(
            "POST /api/dms/dm0000000003/accept",
            200,
            r#"{"id":"dm0000000003","peer":{"id":"stranger0001","username":"jordan","displayName":"TEST FIXTURE Jordan","avatarId":412},"lastSeq":"1","readSeq":"0","status":"accepted","blocked":false}"#,
        )]);
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new(&base).unwrap(),
            Some("parity-requests"),
        );
        app.token = Some("account-token".into());
        app.directs[1].last_seq = "9".into();
        app.directs[1].read_seq = "0".into();
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let labels = texts(&output);
        assert!(labels.contains(&"Message requests"), "{labels:?}");
        assert!(
            labels.contains(&"TEST FIXTURE Jordan @jordan"),
            "{labels:?}"
        );
        assert!(labels.contains(&REQUEST_BAR), "{labels:?}");
        for hidden in ["●", "Message TEST FIXTURE Jordan", "Reply in thread"] {
            assert!(!labels.contains(&hidden), "{hidden}: {labels:?}");
        }
        assert!(!app.selected_is_joined(), "a request stays read-only");

        click(&mut app, &context, text_position(&output, "Accept"));
        receive_until(&mut app, |app| {
            app.directs.iter().any(|direct| {
                direct.id == "dm0000000003" && direct.status == model::DirectStatus::Accepted
            })
        });
        let output = render(&mut app, &context, vec![]);
        let labels = texts(&output);
        assert!(
            labels.contains(&"Message TEST FIXTURE Jordan"),
            "{labels:?}"
        );
        assert!(!labels.contains(&REQUEST_BAR) && !labels.contains(&"Message requests"));
        let seen = seen.lock().unwrap().clone();
        assert!(
            seen.contains(&"POST /api/dms/dm0000000003/accept".to_owned()),
            "{seen:?}"
        );
        assert!(
            !seen.iter().any(|line| line.starts_with("POST /api/dms ")),
            "opening a request never creates or accepts it by username: {seen:?}"
        );
    }

    #[test]
    fn declining_or_blocking_a_request_drops_it_and_moves_on() {
        let (base, seen) = account_server(vec![
            ("POST /api/dms/dm0000000003/decline", 204, ""),
            ("PUT /api/blocks/stranger0003", 204, ""),
        ]);
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new(&base).unwrap(),
            Some("parity-requests"),
        );
        app.token = Some("account-token".into());
        app.directs.push(model::DirectConversation {
            id: "dm0000000005".into(),
            peer: model::DirectPeer {
                id: "stranger0003".into(),
                username: "kim".into(),
                display_name: "TEST FIXTURE Kim".into(),
                avatar_id: None,
            },
            last_seq: "1".into(),
            read_seq: "0".into(),
            status: model::DirectStatus::Incoming,
            blocked: false,
        });
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        click(&mut app, &context, text_position(&output, "Decline"));
        receive_until(&mut app, |app| {
            !app.directs.iter().any(|direct| direct.id == "dm0000000003")
        });
        assert_eq!(
            app.selected_direct.as_deref(),
            Some("dm0000000005"),
            "the next request opens"
        );

        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        click(&mut app, &context, text_position(&output, "Block"));
        assert!(matches!(app.dialog, Some(Dialog::Block { .. })));
        let output = render(&mut app, &context, vec![]);
        assert!(texts(&output).contains(&"Block TEST FIXTURE Kim?"));
        click(&mut app, &context, last_text_position(&output, "Block"));
        receive_until(&mut app, |app| app.is_blocked("stranger0003"));
        assert!(app.dialog.is_none());
        assert!(app.incoming_requests().is_empty());
        assert!(app.selected_direct.is_none(), "no requests left");
        let seen = seen.lock().unwrap().clone();
        assert!(seen.contains(&"POST /api/dms/dm0000000003/decline".to_owned()));
        assert!(seen.contains(&"PUT /api/blocks/stranger0003".to_owned()));
    }

    #[test]
    fn blocked_authors_collapse_into_runs_that_show_and_hide_everywhere() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-blocked"),
        );
        let maya = "The same conversation should feel familiar on every platform.";
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let labels = texts(&output);
        assert!(labels.contains(&"2 blocked messages —"), "{labels:?}");
        assert!(!labels.contains(&maya));
        click(&mut app, &context, text_position(&output, "Show"));
        let output = render(&mut app, &context, vec![]);
        assert!(texts(&output).contains(&maya) && texts(&output).contains(&"Hide"));
        click(&mut app, &context, text_position(&output, "Hide"));
        let output = render(&mut app, &context, vec![]);
        assert!(!texts(&output).contains(&maya));

        // The thread panel's root collapses too.
        let root = app.timeline.messages().nth(1).unwrap().id.clone();
        app.open_thread(root);
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        assert!(
            texts(&output).contains(&"1 blocked message —"),
            "{:?}",
            texts(&output)
        );

        // Typing from a blocked account never shows.
        let channel = app.selected_channel.clone().unwrap();
        app.gateway(GatewayEvent::Typing {
            generation: app.generation,
            channel,
            author: crate::model::Author {
                id: "fixture-maya".into(),
                avatar_id: None,
                name: "Maya".into(),
                is_guest: false,
            },
            typing: true,
            revision: "1".into(),
        });
        assert!(app.typers.is_empty());

        // Message actions offer Block/Unblock only for other signed-in accounts.
        let messages: Vec<_> = app.timeline.messages().cloned().collect();
        assert!(
            app.blockable_author(&messages[0].author).is_none(),
            "yourself"
        );
        let blocked = app.blockable_author(&messages[1].author).unwrap();
        assert_eq!(
            (blocked.id.as_str(), blocked.username.as_str()),
            ("fixture-maya", "maya")
        );
        let mut guest = messages[3].author.clone();
        guest.is_guest = true;
        assert!(app.blockable_author(&guest).is_none(), "guests");
    }

    #[test]
    fn blocked_dms_replace_the_composer_and_unblock_updates_everywhere() {
        let (base, seen) = account_server(vec![("DELETE /api/blocks/fixture-maya", 204, "")]);
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new(&base).unwrap(),
            Some("parity-blocked-dm"),
        );
        app.token = Some("account-token".into());
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let labels = texts(&output);
        assert!(labels.contains(&"You blocked @maya."), "{labels:?}");
        assert!(!labels.contains(&"Message TEST FIXTURE Maya"));
        click(&mut app, &context, last_text_position(&output, "Unblock"));
        receive_until(&mut app, |app| !app.is_blocked("fixture-maya"));
        assert!(!app.directs[0].blocked);
        let output = render(&mut app, &context, vec![]);
        assert!(texts(&output).contains(&"Message TEST FIXTURE Maya"));
        assert!(
            seen.lock()
                .unwrap()
                .contains(&"DELETE /api/blocks/fixture-maya".to_owned())
        );

        // A send refused because of a block is a rejection, not an expired session.
        app.pending = Some(PendingSend::prepare(None, "hello"));
        app.sent(Err(crate::worker::SendFailure {
            status: Some(403),
            message: "You blocked this person. Unblock them to message them.".into(),
            code: Some("dm_blocked".into()),
        }));
        assert!(app.selected_channel.is_some());
        assert_eq!(
            app.pending.as_ref().unwrap().rejection.as_deref(),
            Some("You blocked this person. Unblock them to message them.")
        );
    }

    #[test]
    fn dm_privacy_saves_on_change_and_reverts_on_failure() {
        let (base, seen) = account_server(vec![
            (
                "GET /api/account/privacy",
                200,
                r#"{"directMessages":"anyone"}"#,
            ),
            (
                "PUT /api/account/privacy",
                200,
                r#"{"directMessages":"spaces"}"#,
            ),
            ("GET /api/blocks", 200, r#"{"blocks":[]}"#),
        ]);
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new(&base).unwrap(),
            Some("parity-settings"),
        );
        app.token = Some("account-token".into());
        app.privacy = None;
        app.account_op(AccountOperation::LoadPrivacy);
        receive_until(&mut app, |app| app.privacy.is_some());
        assert_eq!(app.privacy.as_deref(), Some("anyone"));
        render(&mut app, &context, vec![]);
        let output = scroll_modal_to_bottom(&mut app, &context);
        let labels = texts(&output);
        assert!(
            labels.contains(&"Who can start a DM with you"),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"You haven't blocked anyone."),
            "{labels:?}"
        );
        click(
            &mut app,
            &context,
            text_position(&output, "People in my spaces"),
        );
        assert_eq!(
            app.privacy.as_deref(),
            Some("spaces"),
            "applied immediately"
        );
        receive_until(&mut app, |app| !app.privacy_saving);
        assert_eq!(app.privacy.as_deref(), Some("spaces"));
        assert!(
            seen.lock()
                .unwrap()
                .contains(&r#"PUT /api/account/privacy {"directMessages":"spaces"}"#.to_owned())
        );

        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-settings"),
        );
        app.token = Some("account-token".into());
        render(&mut app, &context, vec![]);
        let output = scroll_modal_to_bottom(&mut app, &context);
        click(&mut app, &context, text_position(&output, "No one new"));
        assert_eq!(app.privacy.as_deref(), Some("nobody"));
        receive_until(&mut app, |app| !app.privacy_saving);
        assert_eq!(app.privacy.as_deref(), Some("anyone"), "reverted");
        let error = app.privacy_error.clone().unwrap();
        assert!(error.starts_with("Could not save:"), "{error}");
        let output = render(&mut app, &context, vec![]);
        assert!(texts(&output).contains(&error.as_str()));
    }

    const NOTIFICATION_SETTINGS: &str =
        r#"{"level":"mentions","mobile":"whenInactive","overrides":[]}"#;

    fn notification_settings(value: serde_json::Value) -> crate::notifications::Notifications {
        crate::notifications::Notifications::with_settings(serde_json::from_value(value).unwrap())
    }

    /// A signed-in `parity-desktop` or `parity-direct` app with notification
    /// settings loaded, talking to `base`.
    fn notification_app(
        context: &egui::Context,
        base: &str,
        fixture: &str,
        overrides: serde_json::Value,
    ) -> CaperApp {
        context.enable_accesskit();
        let mut app = CaperApp::new(context, crate::api::Api::new(base).unwrap(), Some(fixture));
        app.token = Some("account-token".into());
        app.notifications = notification_settings(serde_json::json!({
            "level": "mentions", "mobile": "whenInactive", "overrides": overrides
        }));
        app
    }

    /// Renders until the bundled vectors have loaded.
    fn settle(app: &mut CaperApp, context: &egui::Context) -> egui::FullOutput {
        render(app, context, vec![]);
        let started = Instant::now();
        while context.has_pending_images() {
            assert!(started.elapsed() < Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(10));
            render(app, context, vec![]);
        }
        render(app, context, vec![]);
        render(app, context, vec![])
    }

    fn node_bounds(output: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
        output
            .platform_output
            .accesskit_update
            .as_ref()?
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
            .map(|bounds| {
                egui::Rect::from_min_max(
                    egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                    egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
                )
            })
    }

    fn node_center(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        node_bounds(output, label)
            .unwrap_or_else(|| panic!("missing {label}"))
            .center()
    }

    /// Hovers a submenu button so its submenu opens.
    fn open_submenu(
        app: &mut CaperApp,
        context: &egui::Context,
        output: &egui::FullOutput,
        label: &str,
    ) -> egui::FullOutput {
        let position = text_position(output, label);
        render(app, context, vec![egui::Event::PointerMoved(position)]);
        render(app, context, vec![])
    }

    fn text_color(output: &egui::FullOutput, label: &str) -> egui::Color32 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.galley.job.sections[0].format.color)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {label}"))
    }

    fn menu_labels(output: &egui::FullOutput) -> Vec<String> {
        texts(output).into_iter().map(str::to_owned).collect()
    }

    #[test]
    fn notification_settings_load_after_sign_in_and_not_for_a_previous_account() {
        let (base, seen) = account_server(vec![
            (
                "GET /api/notifications/settings",
                200,
                NOTIFICATION_SETTINGS,
            ),
            ("GET /api/dms", 200, r#"{"conversations":[]}"#),
            ("GET /api/blocks", 200, r#"{"blocks":[]}"#),
        ]);
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new(&base).unwrap(),
            Some("signed-out"),
        );
        app.establish("account-token".into(), account(true), spaces());
        receive_until(&mut app, |app| app.notifications.ready());
        assert_eq!(
            app.notifications.account_level(),
            Some(model::NotificationLevel::Mentions)
        );

        app.load_notifications();
        app.reset_account_state();
        let loads = || {
            seen.lock()
                .unwrap()
                .iter()
                .filter(|line| *line == "GET /api/notifications/settings")
                .count()
        };
        let started = Instant::now();
        while loads() < 2 && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(loads(), 2);
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(10));
            app.receive();
        }
        assert!(
            !app.notifications.ready(),
            "an answer for the signed-out account is ignored"
        );
    }

    #[test]
    fn settings_notify_me_about_saves_at_once_and_reverts_on_failure() {
        let (base, seen) = account_server(vec![
            (
                "GET /api/notifications/settings",
                200,
                NOTIFICATION_SETTINGS,
            ),
            (
                "PUT /api/notifications/settings",
                200,
                r#"{"level":"nothing","mobile":"whenInactive","overrides":[]}"#,
            ),
        ]);
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new(&base).unwrap(),
            Some("parity-settings"),
        );
        app.token = Some("account-token".into());
        app.notifications = crate::notifications::Notifications::default();
        render(&mut app, &context, vec![]);
        let output = scroll_modal_to_bottom(&mut app, &context);
        assert!(texts(&output).contains(&"Loading…"), "{:?}", texts(&output));
        app.load_notifications();
        receive_until(&mut app, |app| app.notifications.ready());
        render(&mut app, &context, vec![]);
        scroll_modal_to_bottom(&mut app, &context);
        // Up a little, to the Notifications heading.
        let bounds =
            context.memory(|memory| memory.area_rect(egui::Id::new("caper-dialog")).unwrap());
        render(
            &mut app,
            &context,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 160.0),
                modifiers: egui::Modifiers::NONE,
            }],
        );
        for _ in 0..30 {
            render(&mut app, &context, vec![]);
        }
        let output = render(
            &mut app,
            &context,
            vec![egui::Event::PointerMoved(bounds.center())],
        );
        let labels = texts(&output);
        for label in [
            "Notifications",
            "Notify me about",
            "All messages",
            "Only @mentions and DMs",
            "Nothing",
        ] {
            assert!(labels.contains(&label), "{label}: {labels:?}");
        }
        assert!(
            !labels.iter().any(|label| label.contains("phone")),
            "desktop has no phone setting: {labels:?}"
        );
        click(&mut app, &context, text_position(&output, "Nothing"));
        assert_eq!(
            app.notifications.account_level(),
            Some(model::NotificationLevel::Nothing),
            "applied immediately"
        );
        receive_until(&mut app, |app| {
            !app.notifications
                .saving(&crate::notifications::Scope::Account)
        });
        assert!(
            seen.lock()
                .unwrap()
                .contains(&r#"PUT /api/notifications/settings {"level":"nothing"}"#.to_owned())
        );

        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-settings"),
        );
        app.token = Some("account-token".into());
        app.notifications = notification_settings(serde_json::json!({
            "level": "all", "mobile": "whenInactive", "overrides": []
        }));
        render(&mut app, &context, vec![]);
        let output = scroll_modal_to_bottom(&mut app, &context);
        click(
            &mut app,
            &context,
            text_position(&output, "Only @mentions and DMs"),
        );
        assert_eq!(
            app.notifications.account_level(),
            Some(model::NotificationLevel::Mentions)
        );
        receive_until(&mut app, |app| {
            !app.notifications
                .saving(&crate::notifications::Scope::Account)
        });
        assert_eq!(
            app.notifications.account_level(),
            Some(model::NotificationLevel::All),
            "reverted"
        );
        let error = app
            .notifications
            .error(&crate::notifications::Scope::Account)
            .unwrap()
            .to_owned();
        assert!(error.starts_with("Could not save:"), "{error}");
        let output = render(&mut app, &context, vec![]);
        assert!(texts(&output).contains(&error.as_str()));
    }

    #[test]
    fn space_menu_sets_the_level_and_names_the_account_default() {
        let (base, seen) = account_server(vec![
            (
                "GET /api/notifications/settings",
                200,
                NOTIFICATION_SETTINGS,
            ),
            (
                "PUT /api/spaces/space0000001/notifications",
                200,
                r#"{"spaceId":"space0000001","level":"nothing","mutedUntil":null}"#,
            ),
        ]);
        let context = egui::Context::default();
        let mut app = notification_app(&context, &base, "parity-desktop", serde_json::json!([]));
        let output = settle(&mut app, &context);
        click(
            &mut app,
            &context,
            node_center(&output, "Fixture Studio actions"),
        );
        let opened = render(&mut app, &context, vec![]);
        let labels = menu_labels(&opened);
        for label in [
            "Browse channels",
            "Notifications",
            "Mute space",
            "Space settings",
        ] {
            assert!(
                labels.iter().any(|item| item == label),
                "{label}: {labels:?}"
            );
        }
        let submenu = open_submenu(&mut app, &context, &opened, "Notifications");
        let labels = menu_labels(&submenu);
        for label in [
            "Default (Only @mentions)",
            "All messages",
            "Only @mentions",
            "Nothing",
        ] {
            assert!(
                labels.iter().any(|item| item == label),
                "{label}: {labels:?}"
            );
        }
        click(&mut app, &context, last_text_position(&submenu, "Nothing"));
        let space = crate::notifications::Scope::Space("space0000001".into());
        assert_eq!(
            app.notifications.level(&space),
            Some(model::NotificationLevel::Nothing),
            "applied immediately"
        );
        receive_until(&mut app, |app| !app.notifications.saving(&space));
        assert_eq!(
            app.notifications.level(&space),
            Some(model::NotificationLevel::Nothing)
        );
        let seen = seen.lock().unwrap();
        assert!(
            seen.contains(
                &r#"PUT /api/spaces/space0000001/notifications {"level":"nothing"}"#.to_owned()
            ),
            "{seen:?}"
        );
        assert!(
            seen.contains(&"GET /api/notifications/settings".to_owned()),
            "opening the menu refreshes the settings: {seen:?}"
        );
    }

    #[test]
    fn channel_menu_mutes_dims_the_row_and_unmutes() {
        let (base, seen) = account_server(vec![
            (
                "GET /api/notifications/settings",
                200,
                NOTIFICATION_SETTINGS,
            ),
            (
                "PUT /api/spaces/space0000001/channels/chan00000002/notifications",
                200,
                r#"{"spaceId":"space0000001","channelId":"chan00000002","level":null,"mutedUntil":"forever"}"#,
            ),
        ]);
        let context = egui::Context::default();
        let mut app = notification_app(&context, &base, "parity-desktop", serde_json::json!([]));
        let output = settle(&mut app, &context);
        let normal = text_color(&output, "design");
        let active = text_color(&output, "general");
        assert!(node_bounds(&output, "Muted").is_none());
        click(
            &mut app,
            &context,
            node_center(&output, "Channel options for design"),
        );
        let opened = render(&mut app, &context, vec![]);
        let labels = menu_labels(&opened);
        for label in [
            "Notifications",
            "Mute channel",
            "Channel settings",
            "Leave channel",
        ] {
            assert!(
                labels.iter().any(|item| item == label),
                "{label}: {labels:?}"
            );
        }
        let submenu = open_submenu(&mut app, &context, &opened, "Mute channel");
        let labels = menu_labels(&submenu);
        for (label, _) in crate::notifications::MUTE_PRESETS {
            assert!(
                labels.iter().any(|item| item == label),
                "{label}: {labels:?}"
            );
        }
        click(
            &mut app,
            &context,
            text_position(&submenu, "Until I turn it back on"),
        );
        let channel = crate::notifications::Scope::Channel {
            space: "space0000001".into(),
            channel: "chan00000002".into(),
        };
        assert!(
            app.notifications.muted(&channel, chrono::Utc::now()),
            "applied immediately"
        );
        receive_until(&mut app, |app| !app.notifications.saving(&channel));
        assert!(seen.lock().unwrap().contains(
            &r#"PUT /api/spaces/space0000001/channels/chan00000002/notifications {"mutedUntil":"forever"}"#
                .to_owned()
        ));
        let output = settle(&mut app, &context);
        assert_ne!(text_color(&output, "design"), normal, "dimmed");
        assert_eq!(text_color(&output, "general"), active);
        let bell = node_bounds(&output, "Muted").expect("bell-slash indicator");
        let row = node_bounds(&output, "design").unwrap();
        assert!(row.contains_rect(bell), "{bell:?} in {row:?}");

        click(
            &mut app,
            &context,
            node_center(&output, "Channel options for design"),
        );
        let opened = render(&mut app, &context, vec![]);
        let labels = menu_labels(&opened);
        assert!(
            labels.iter().any(|item| item == "Unmute channel"),
            "{labels:?}"
        );
        assert!(labels.iter().any(|item| item == "Muted"), "{labels:?}");
        assert!(
            !labels.iter().any(|item| item == "Mute channel"),
            "{labels:?}"
        );
        click(&mut app, &context, text_position(&opened, "Unmute channel"));
        assert!(!app.notifications.muted(&channel, chrono::Utc::now()));
        receive_until(&mut app, |app| !app.notifications.saving(&channel));
        assert!(seen.lock().unwrap().contains(
            &r#"PUT /api/spaces/space0000001/channels/chan00000002/notifications {"mutedUntil":null}"#
                .to_owned()
        ));
    }

    #[test]
    fn muted_space_dims_its_channels_and_keeps_their_mute_choices() {
        let context = egui::Context::default();
        let in_an_hour = crate::notifications::mute_value(Some(60), chrono::Utc::now());
        let mut app = notification_app(
            &context,
            "http://127.0.0.1:9",
            "parity-desktop",
            serde_json::json!([
                {"spaceId": "space0000001", "level": null, "mutedUntil": in_an_hour}
            ]),
        );
        let output = settle(&mut app, &context);
        let space_label = crate::notifications::mute_label(
            crate::notifications::mute(Some(&in_an_hour), chrono::Utc::now()).unwrap(),
            &chrono::Local::now(),
        );
        assert!(space_label.starts_with("Muted until "), "{space_label}");
        assert!(
            node_bounds(&output, &space_label).is_some(),
            "the space header shows the bell-slash"
        );
        assert!(node_bounds(&output, "Muted with the space").is_some());
        for channel in ["general", "design", "planning"] {
            assert!(
                text_color(&output, channel).a() < 255,
                "{channel} is dimmed"
            );
        }
        assert_ne!(text_color(&output, "F"), text_color(&output, "C"), "rail");
        click(
            &mut app,
            &context,
            node_center(&output, "Fixture Studio actions"),
        );
        let opened = render(&mut app, &context, vec![]);
        let labels = menu_labels(&opened);
        assert!(
            labels.iter().any(|item| item == "Unmute space"),
            "{labels:?}"
        );
        assert!(labels.contains(&space_label), "{labels:?}");
        click(&mut app, &context, egui::pos2(1200.0, 450.0));
        let output = render(&mut app, &context, vec![]);
        click(
            &mut app,
            &context,
            node_center(&output, "Channel options for design"),
        );
        let opened = render(&mut app, &context, vec![]);
        let labels = menu_labels(&opened);
        assert!(
            labels.iter().any(|item| item == "Muted with the space"),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|item| item == "Mute channel"),
            "{labels:?}"
        );
    }

    #[test]
    fn dm_rows_get_a_hover_menu_and_muted_dms_hide_unread() {
        let (base, seen) = account_server(vec![
            (
                "GET /api/notifications/settings",
                200,
                NOTIFICATION_SETTINGS,
            ),
            (
                "PUT /api/dms/dm0000000001/notifications",
                200,
                r#"{"conversationId":"dm0000000001","level":"nothing","mutedUntil":null}"#,
            ),
        ]);
        let context = egui::Context::default();
        let mut app = notification_app(&context, &base, "parity-direct", serde_json::json!([]));
        app.directs[0].last_seq = "4".into();
        let output = settle(&mut app, &context);
        assert!(texts(&output).contains(&"●"), "unread before muting");
        let options = node_bounds(&output, "Conversation options for TEST FIXTURE Maya")
            .expect("DM options button");
        let painted = |output: &egui::FullOutput| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Mesh(mesh) if options.contains_rect(mesh.calc_bounds()))
            })
        };
        assert!(!painted(&output), "hidden until the row is hovered");
        assert!(
            node_bounds(&output, "Conversation options for Fixture Owner").is_none()
                && !output
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .any(|(_, node)| {
                        node.label().is_some_and(|label| {
                            label.starts_with("Conversation options for Fixture")
                        })
                    }),
            "personal notes have no options"
        );
        render(
            &mut app,
            &context,
            vec![egui::Event::PointerMoved(node_center(
                &output,
                "TEST FIXTURE Maya",
            ))],
        );
        let hovered = settle(&mut app, &context);
        assert!(painted(&hovered), "shown on hover");

        click(&mut app, &context, options.center());
        let opened = render(&mut app, &context, vec![]);
        let labels = menu_labels(&opened);
        assert!(
            labels.iter().any(|item| item == "Turn off notifications"),
            "{labels:?}"
        );
        assert!(
            labels.iter().any(|item| item == "Mute conversation"),
            "{labels:?}"
        );
        click(
            &mut app,
            &context,
            text_position(&opened, "Turn off notifications"),
        );
        let direct = crate::notifications::Scope::Direct("dm0000000001".into());
        receive_until(&mut app, |app| !app.notifications.saving(&direct));
        assert!(seen.lock().unwrap().contains(
            &r#"PUT /api/dms/dm0000000001/notifications {"level":"nothing"}"#.to_owned()
        ));
        let output = settle(&mut app, &context);
        assert!(
            texts(&output).contains(&"●"),
            "notifications off still shows unread"
        );
        click(&mut app, &context, options.center());
        let opened = render(&mut app, &context, vec![]);
        assert!(
            menu_labels(&opened)
                .iter()
                .any(|item| item == "Turn on notifications")
        );
        let submenu = open_submenu(&mut app, &context, &opened, "Mute conversation");
        click(&mut app, &context, text_position(&submenu, "For 1 hour"));
        let mute = app.notifications.mute(&direct, chrono::Utc::now());
        let Some(crate::notifications::Mute::Until(until)) = mute else {
            panic!("expected a timed mute: {mute:?}");
        };
        let minutes = (until - chrono::Utc::now()).num_minutes();
        assert!((58..=60).contains(&minutes), "{minutes}");
        let output = settle(&mut app, &context);
        assert!(
            !texts(&output).contains(&"●"),
            "a muted DM shows no unread dot"
        );
        assert_ne!(
            text_color(&output, "TEST FIXTURE Maya"),
            text_color(&output, "Invite people"),
            "dimmed"
        );
        let label = crate::notifications::mute_label(
            crate::notifications::Mute::Until(until),
            &chrono::Local::now(),
        );
        let bell = node_bounds(&output, &label).expect("the bell names when the mute ends");
        assert!(
            node_bounds(&output, "TEST FIXTURE Maya")
                .unwrap()
                .contains_rect(bell)
        );
        receive_until(&mut app, |app| !app.notifications.saving(&direct));
        let seen = seen.lock().unwrap();
        let sent = seen
            .iter()
            .find(|line| {
                line.starts_with("PUT /api/dms/dm0000000001/notifications {\"mutedUntil\"")
            })
            .expect("mute sent");
        assert!(sent.ends_with(r#"Z"}"#), "UTC timestamp: {sent}");
    }

    #[test]
    fn failed_mute_reverts_with_an_inline_error_under_the_row() {
        let context = egui::Context::default();
        let mut app = notification_app(
            &context,
            "http://127.0.0.1:9",
            "parity-desktop",
            serde_json::json!([]),
        );
        let output = settle(&mut app, &context);
        click(
            &mut app,
            &context,
            node_center(&output, "Channel options for design"),
        );
        let opened = render(&mut app, &context, vec![]);
        let submenu = open_submenu(&mut app, &context, &opened, "Mute channel");
        click(&mut app, &context, text_position(&submenu, "For 8 hours"));
        let channel = crate::notifications::Scope::Channel {
            space: "space0000001".into(),
            channel: "chan00000002".into(),
        };
        assert!(app.notifications.muted(&channel, chrono::Utc::now()));
        receive_until(&mut app, |app| !app.notifications.saving(&channel));
        assert!(
            !app.notifications.muted(&channel, chrono::Utc::now()),
            "reverted"
        );
        let error = app.notifications.error(&channel).unwrap().to_owned();
        assert!(error.starts_with("Could not save:"), "{error}");
        let output = settle(&mut app, &context);
        let row = node_bounds(&output, "design").unwrap();
        let shown = text_position(&output, &error);
        assert!(shown.y > row.bottom(), "under the row: {shown:?} {row:?}");
        assert!(node_bounds(&output, "Muted").is_none());
    }

    #[test]
    fn muted_fixture_previews_dimmed_rows_without_requests() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-muted"),
        );
        assert!(app.token.is_none(), "a static preview");
        let output = settle(&mut app, &context);
        assert!(node_bounds(&output, "Muted").is_some());
        assert!(text_color(&output, "design") != text_color(&output, "general"));
        let bells: Vec<_> = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .filter_map(|(_, node)| node.label().filter(|label| label.starts_with("Muted")))
            .collect();
        assert!(
            bells.iter().any(|label| label.starts_with("Muted until ")),
            "{bells:?}"
        );
        let dots = texts(&output).iter().filter(|text| **text == "●").count();
        assert_eq!(dots, 1, "only Alex (notifications off) is dotted");
    }

    fn scroll_modal_to_bottom(app: &mut CaperApp, context: &egui::Context) -> egui::FullOutput {
        let bounds =
            context.memory(|memory| memory.area_rect(egui::Id::new("caper-dialog")).unwrap());
        render(
            app,
            context,
            vec![
                egui::Event::PointerMoved(bounds.center()),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -2000.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        for _ in 0..30 {
            render(app, context, vec![]);
        }
        render(app, context, vec![])
    }

    #[test]
    fn modal_shells_do_not_recenter_for_errors_or_pending_content() {
        for dialog in [
            Dialog::Profile,
            Dialog::Audio,
            Dialog::Connection,
            Dialog::Diagnostics,
            Dialog::CreateSpace,
            Dialog::ManageSpace,
            Dialog::CreateChannel,
            Dialog::ManageChannel("chan00000003".into()),
            Dialog::ConfirmDelete {
                space: "space0000001".into(),
                channel: Some("chan00000003".into()),
                name: "planning".into(),
            },
            Dialog::LeaveSpace {
                id: "space0000001".into(),
                name: "Fixture Studio".into(),
            },
        ] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-channel"),
            );
            app.dialog = Some(dialog);
            for _ in 0..3 {
                render(&mut app, &context, vec![]);
            }
            let bounds = || {
                context.memory(|memory| memory.area_rect(egui::Id::new("caper-dialog")).unwrap())
            };
            let before = bounds();
            app.error = Some("Test-only asynchronous error content. ".repeat(100));
            for _ in 0..3 {
                render(&mut app, &context, vec![]);
            }
            assert_eq!(bounds(), before, "error resized or recentered modal");
            app.error = None;
            app.loading = true;
            app.form_private = !app.form_private;
            for _ in 0..3 {
                render(&mut app, &context, vec![]);
            }
            assert_eq!(
                bounds(),
                before,
                "pending content resized or recentered modal"
            );
        }
    }

    #[test]
    fn settings_backdrops_dismiss_without_inside_clicks_dismissing() {
        for fixture in ["parity-admin", "parity-channel"] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some(fixture),
            );
            for _ in 0..3 {
                render(&mut app, &context, vec![]);
            }
            let bounds =
                context.memory(|memory| memory.area_rect(egui::Id::new("caper-dialog")).unwrap());
            click(
                &mut app,
                &context,
                bounds.left_top() + egui::vec2(10.0, 10.0),
            );
            assert!(app.dialog.is_some(), "inside click dismissed settings");
            click(&mut app, &context, egui::pos2(5.0, 5.0));
            assert!(
                app.dialog.is_none(),
                "outside click did not dismiss settings"
            );
            assert!(!app.loading);
        }

        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.dialog = Some(Dialog::StartDirect);
        for _ in 0..3 {
            render(&mut app, &context, vec![]);
        }
        let bounds =
            context.memory(|memory| memory.area_rect(egui::Id::new("caper-dialog")).unwrap());
        click(
            &mut app,
            &context,
            bounds.left_top() + egui::vec2(10.0, 10.0),
        );
        assert!(
            matches!(app.dialog, Some(Dialog::StartDirect)),
            "inside click dismissed direct-message dialog"
        );
        click(&mut app, &context, egui::pos2(5.0, 5.0));
        assert!(
            app.dialog.is_none(),
            "outside click did not dismiss direct-message dialog"
        );
    }

    #[test]
    fn spectator_rosters_collapse_without_navigating_or_exposing_playback_controls() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-voice-rosters"),
        );
        app.token = Some("fixture-only".into());
        let selected = app.selected_channel.clone();
        let (target, space) = app.voice_target("chan00000002").unwrap();
        assert_eq!(target.channel_name, "design");
        assert_eq!(target.channel_id, "chan00000002");
        assert_eq!(space.as_deref(), Some("space0000001"));
        assert_eq!(app.selected_channel, selected);
        assert!(app.voice_target("not-in-space").is_none());
        render(&mut app, &context, vec![]);
        // SVG decoding is asynchronous on native; wait for the real avatar rather
        // than inspecting its loading spinner in the first two frames.
        let started = std::time::Instant::now();
        while context.has_pending_images() {
            assert!(started.elapsed() < std::time::Duration::from_secs(30));
            std::thread::sleep(std::time::Duration::from_millis(10));
            render(&mut app, &context, vec![]);
        }
        let output = render(&mut app, &context, vec![]);
        let sidebar_text = |output: &egui::FullOutput, label: &str| {
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.pos.x < 340.0 && text.galley.job.text == label => {
                    Some(text.pos)
                }
                _ => None,
            })
        };
        assert!(
            sidebar_text(&output, "Maya").is_none(),
            "rosters start collapsed"
        );
        let count = sidebar_text(&output, "2 in voice").expect("voice count is visible");
        // The saved avatar is an image now, not the old clickable "M" initial.
        let stack = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if rect.brush.as_ref().is_some_and(|brush| {
                        brush.uv == egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0))
                    }) && rect.rect.width() == 20.0
                        && rect.rect.center().x < 340.0
                        && (rect.rect.center().y - count.y).abs() < 14.0 =>
                {
                    Some(rect.rect.center())
                }
                _ => None,
            })
            .expect("Maya's saved avatar is rendered in the voice stack");
        click(&mut app, &context, stack);
        assert!(app.expanded_rosters.contains("chan00000002"));
        assert!(sidebar_text(&render(&mut app, &context, vec![]), "Maya").is_some());
        assert_eq!(app.selected_channel, selected);
        click(&mut app, &context, stack);
        let reopened = render(&mut app, &context, vec![]);
        assert!(sidebar_text(&reopened, "Maya").is_none());
        assert_eq!(app.selected_channel, selected);
        assert!(matches!(app.voice.state.phase, Phase::Idle));
    }

    #[test]
    fn denied_voice_switch_preserves_call_and_leave_fences_late_permission() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-voice-rosters"),
        );
        app.token = Some("fixture-only".into());
        let (original, _) = app.voice_target("chan00000001").unwrap();
        app.voice.state.phase = Phase::Connected(original.clone());
        let chat = app.selected_channel.clone();
        app.voice_join_request = 7;
        app.pending_voice_join = Some(("chan00000002".into(), 0, 12_345));
        app.accept_voice_target(
            7,
            0,
            "space0000001",
            "chan00000002",
            Err(LoadError {
                message: "Denied target".into(),
                access_denied: true,
                space_access_denied: false,
            }),
        );
        assert_eq!(app.voice.state.phase, Phase::Connected(original.clone()));
        assert_eq!(app.selected_channel, chat);
        assert!(app.voice_target("chan00000002").is_none());
        app.accept_voice_target(6, 0, "space0000001", "chan00000003", Ok(()));
        assert_eq!(app.voice.state.phase, Phase::Connected(original));
        app.voice.leave();
        app.accept_voice_target(7, 0, "space0000001", "chan00000003", Ok(()));
        assert_eq!(app.voice.state.phase, Phase::Idle);
        app.voice.state.phase = Phase::Failed(app.voice_target("chan00000003").unwrap().0);
        app.join_voice_channel("chan00000003");
        assert_eq!(app.voice_join_request, 8, "A failed join must be retryable");
        assert_eq!(app.selected_channel, chat);
    }

    #[test]
    fn roster_epochs_and_revocation_keep_late_private_occupancy_out_of_chat() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-voice-rosters"),
        );
        let people = app.channel_rosters["chan00000002"].clone();
        app.token = Some("fixture-only".into());
        let (target, _) = app.voice_target("chan00000002").unwrap();
        app.voice.state.phase = Phase::Connected(target);
        let selected = app.selected_channel.clone();
        app.roster_generation = 42;
        app.gateway(GatewayEvent::VoiceUnavailable {
            generation: 41,
            channel: "chan00000002".into(),
            revoked: true,
        });
        assert!(app.channel_rosters.contains_key("chan00000002"));
        app.gateway(GatewayEvent::VoiceUnavailable {
            generation: 42,
            channel: "chan00000002".into(),
            revoked: false,
        });
        assert!(!app.channel_rosters.contains_key("chan00000002"));
        assert!(
            matches!(app.voice.state.phase, Phase::Connected(_)),
            "A spectator service failure is not call revocation"
        );
        app.gateway(GatewayEvent::VoiceUnavailable {
            generation: 42,
            channel: "chan00000002".into(),
            revoked: true,
        });
        assert!(app.voice.state.active_channel().is_none());
        assert!(app.voice_target("chan00000002").is_none());
        assert_eq!(app.selected_channel, selected);
        app.clear_channel_state();
        app.gateway(GatewayEvent::VoiceRoster {
            generation: 42,
            channel: "chan00000002".into(),
            participants: people.clone(),
            session_started_at: Some(1_000),
        });
        assert!(app.channel_rosters.is_empty());
        assert!(app.voice_session_starts.is_empty());
        app.selected_channel = selected;
        app.gateway(GatewayEvent::VoiceRoster {
            generation: 43,
            channel: "chan00000003".into(),
            participants: people,
            session_started_at: Some(5_000),
        });
        assert!(app.channel_rosters.contains_key("chan00000003"));
        assert_eq!(app.voice_session_starts["chan00000003"], 5_000);
        app.gateway(GatewayEvent::VoiceReset { generation: 42 });
        assert!(!app.channel_rosters.is_empty());
        app.gateway(GatewayEvent::VoiceReset { generation: 43 });
        assert!(app.channel_rosters.is_empty());
        assert!(app.voice_session_starts.is_empty());
    }

    #[test]
    fn voice_roster_names_align_and_participant_menu_changes_only_that_listener() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-voice-connected"),
        );
        app.expanded_rosters.insert("chan00000001".into());
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let position = |output: &egui::FullOutput, label: &str, sidebar: bool| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if text.galley.job.text == label && (!sidebar || text.pos.x < 340.0) =>
                    {
                        Some(text.pos)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing {label}"))
        };
        for label in ["Fixture Owner (you)", "Maya", "Alex"] {
            assert_eq!(position(&output, label, true).x, 106.0);
        }
        assert_eq!(
            output.shapes.iter().filter(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Audio")
            }).count(),
            2,
            "Only remote participants have Audio buttons"
        );
        assert!(!output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "User volume")
        }));
        app.voice.set_participant_playback(
            "fixture-maya",
            crate::media::TrackPlayback {
                gain_percent: 170,
                muted: false,
            },
        );
        click(
            &mut app,
            &context,
            position(&output, "Audio", true) + egui::vec2(4.0, 4.0),
        );
        let menu = render(&mut app, &context, vec![]);
        position(&menu, "User volume", false);
        position(&menu, "170%", false);
        let mute = position(&menu, "Mute", false) + egui::vec2(4.0, 4.0);
        click(&mut app, &context, mute);
        assert!(app.voice.playback("fixture-maya").muted);
        assert!(!app.voice.playback("fixture-alex").muted);
        assert!(
            !app.voice.state.audio.muted,
            "local listener control must not mute our microphone"
        );
        let muted = render(&mut app, &context, vec![]);
        position(&muted, "You muted Maya", true);
        app.voice.set_participant_playback(
            "fixture-maya",
            crate::media::TrackPlayback {
                gain_percent: 170,
                muted: false,
            },
        );
        let restored = render(&mut app, &context, vec![]);
        assert!(!restored.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.starts_with("You muted "))
        }));
    }

    #[test]
    fn own_roster_icons_use_local_intent_instead_of_stale_snapshot() {
        for (snapshot, muted, deafened) in [
            (true, false, false),
            (false, true, false),
            (false, true, true),
        ] {
            let context = egui::Context::default();
            context.enable_accesskit();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-voice-connected"),
            );
            app.expanded_rosters.insert("chan00000001".into());
            for participant in &mut app.voice.participants {
                participant.muted = snapshot;
                participant.deafened = snapshot;
            }
            app.voice.state.audio.set_muted(muted);
            app.voice.state.audio.set_deafened(deafened);
            // Inspect the initial full tree, not subsequent AccessKit deltas.
            let labels = render(&mut app, &context, vec![])
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .into_iter()
                .filter_map(|(_, node)| node.label().map(str::to_owned))
                .collect::<Vec<_>>();
            assert_eq!(labels.contains(&"Fixture Owner: Muted".into()), muted);
            assert_eq!(labels.contains(&"Fixture Owner: Deafened".into()), deafened);
            assert_eq!(labels.contains(&"Maya: Muted".into()), snapshot);
            assert_eq!(labels.contains(&"Maya: Deafened".into()), snapshot);
        }
    }

    #[test]
    fn call_dock_opens_exact_voice_target_without_replacing_call_or_denied_chat() {
        for space in [Some("space0000001"), Some("other-space"), None] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-voice-connected"),
            );
            let channel = if space.is_some() {
                "chan00000002"
            } else {
                "general"
            };
            app.voice.active_space = space.map(str::to_owned);
            app.voice.state.phase = Phase::Connected(crate::state::CallContext {
                channel_id: channel.into(),
                channel_name: "design".into(),
                space_name: "Voice Space".into(),
            });
            let call = app.voice.state.phase.clone();
            let call_generation = app.voice.state.generation;
            let original_chat = app.selected_channel.clone();
            app.draft = "Unsent text".into();
            render(&mut app, &context, vec![]);
            let output = render(&mut app, &context, vec![]);
            let dock = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "design / Voice Space" => {
                        Some(text.pos + egui::vec2(4.0, 4.0))
                    }
                    _ => None,
                })
                .unwrap();
            click(&mut app, &context, dock);
            let target = app
                .navigation_target
                .as_ref()
                .expect("dock must request navigation, not connection details");
            assert_eq!(target.space.as_deref(), space);
            assert_eq!(target.channel.as_deref(), space.map(|_| "chan00000002"));
            assert!(app.dialog.is_none());
            assert_eq!(app.selected_channel, original_chat);
            app.accept_navigation(
                app.generation,
                app.navigation,
                Err(LoadError {
                    message: "Channel denied".into(),
                    access_denied: true,
                    space_access_denied: false,
                }),
            );
            assert_eq!(app.selected_channel, original_chat);
            assert_eq!(app.draft, "Unsent text");
            assert_eq!(app.voice.state.phase, call);
            click(&mut app, &context, dock);
            let detail = app.detail.as_ref().unwrap().clone();
            let prepared = || {
                let mut detail = detail.clone();
                if let Some(space) = space {
                    detail.space.id = space.into();
                }
                let mut history = history(channel);
                history.space.id = space.unwrap_or("general").into();
                crate::worker::PreparedNavigation {
                    detail: space.map(|_| detail),
                    conversation: Some((history, Ok(session()))),
                }
            };
            app.accept_navigation(app.generation + 1, app.navigation, Ok(prepared()));
            assert_eq!(
                app.selected_channel, original_chat,
                "another account epoch must not commit"
            );
            app.accept_navigation(app.generation, app.navigation, Ok(prepared()));
            assert_eq!(app.selected_channel.as_deref(), Some(channel));
            assert_eq!(app.voice.state.phase, call);
            assert_eq!(
                app.voice.state.generation, call_generation,
                "opening chat must not rejoin voice"
            );
        }
    }

    #[test]
    fn leave_space_is_nonowner_only_and_clears_private_conversation() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        assert!(app.owner());
        assert!(!app.can_leave_space());
        app.account.as_mut().unwrap().id = "fixture-maya".into();
        assert!(!app.owner());
        assert!(app.can_leave_space());
        let space = app.selected_space.clone().unwrap();
        // Exercise leaving the last account space, with no demo fallback.
        app.spaces.retain(|entry| entry.id == space);
        assert!(!app.needs_first_space());
        app.draft = "private draft".into();
        app.pending = Some(PendingSend::prepare(None, "private draft"));
        app.pending_reactions.insert(
            ("message".into(), "👍".into()),
            PendingReaction {
                desired: true,
                sent: true,
                visible: true,
                superseded: false,
            },
        );
        app.reaction_errors
            .insert("message".into(), "failed".into());
        assert!(app.timeline.messages().next().is_some());
        app.select_channel("next".into(), false);
        let stale_generation = app.generation;
        let stale_navigation = app.navigation;
        app.admin_result(crate::worker::AdminResult::SpaceLeft(space.clone()));
        assert!(!app.spaces.iter().any(|entry| entry.id == space));
        assert!(app.detail.is_none());
        assert!(app.selected_space.is_none());
        assert!(app.selected_channel.is_none());
        assert!(app.session.is_none());
        assert!(app.timeline.messages().next().is_none());
        assert!(app.draft.is_empty());
        assert!(app.pending.is_none());
        assert!(app.pending_reactions.is_empty());
        assert!(app.reaction_errors.is_empty());
        app.accept_navigation(
            stale_generation,
            app.navigation,
            Ok(crate::worker::PreparedNavigation {
                detail: None,
                conversation: Some((history("old-private"), Ok(session()))),
            }),
        );
        assert!(
            app.selected_channel.is_none(),
            "late old generation ignored"
        );
        assert!(app.timeline.messages().next().is_none());
        assert!(!app.can_leave_space());
        app.accept_navigation(
            stale_generation,
            stale_navigation,
            Ok(crate::worker::PreparedNavigation {
                detail: None,
                conversation: Some((history("next"), Ok(session()))),
            }),
        );
        assert!(app.selected_channel.is_none());
        assert!(app.timeline.messages().next().is_none());
        // Like web, an account with no spaces is asked to name its first one.
        assert!(app.needs_first_space());
    }

    #[test]
    fn leaving_space_clears_private_state_before_opening_remaining_space() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.token = Some("fixture-token".into());
        app.spaces.retain(|entry| !entry.demo);
        let mut remaining = app.spaces[0].clone();
        remaining.id = "remaining-space".into();
        app.spaces.push(remaining);
        let space = app.selected_space.clone().unwrap();
        let generation = app.generation;
        app.session = Some(session());
        app.draft = "private draft".into();
        app.admin_result(crate::worker::AdminResult::SpaceLeft(space));

        assert!(app.generation > generation);
        assert!(app.selected_space.is_none());
        assert!(app.selected_channel.is_none());
        assert!(app.session.is_none());
        assert!(app.timeline.messages().next().is_none());
        assert!(app.draft.is_empty());
        assert_eq!(
            app.navigation_target
                .as_ref()
                .and_then(|target| target.space.as_deref()),
            Some("remaining-space")
        );
    }

    #[test]
    fn leaving_last_space_preserves_global_direct_conversation() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-direct"),
        );
        let space = app.selected_space.clone().unwrap();
        app.spaces.retain(|entry| entry.id == space);
        app.draft = "global draft".into();
        app.admin_result(crate::worker::AdminResult::SpaceLeft(space));
        assert!(app.spaces.is_empty());
        assert!(app.selected_space.is_none());
        assert_eq!(app.selected_direct.as_deref(), Some("dm0000000001"));
        assert_eq!(app.selected_channel.as_deref(), Some("dm0000000001"));
        assert_eq!(app.timeline.messages().count(), 2);
        assert_eq!(app.draft, "global draft");
        assert!(
            !app.needs_first_space(),
            "first-space page cannot hide an open global DM"
        );
    }

    #[test]
    fn invitations_do_not_install_members_or_private_conversation_before_consent() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-invitation"),
        );
        let invitation = app.invitations[0].clone();
        let consent = app.dialog.take();
        assert!(
            !app.needs_first_space(),
            "pending invitations must remain reachable before creating a space"
        );
        app.dialog = consent;
        assert!(app.spaces.is_empty());
        assert!(app.detail.is_none());
        assert!(app.timeline.messages().next().is_none());
        assert!(app.selected_channel.is_none());
        // egui sizes newly opened modal areas on their first frame.
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(labels.contains(&"Decline"), "{labels:?}");
        assert!(labels.contains(&"Accept invitation"), "{labels:?}");
        assert!(labels.contains(&"You’re invited!"), "{labels:?}");
        assert!(
            labels.contains(&"Join TEST FIXTURE · Invited Studio?"),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"TEST FIXTURE host (@fixture_host) invited you."),
            "{labels:?}"
        );
        assert!(
            !labels.iter().any(|label| label.contains("expire")
                || label.contains("starter")
                || label.contains("Accept to load")),
            "{labels:?}"
        );
        app.admin_result(crate::worker::AdminResult::InvitationDeclined(
            invitation.id,
        ));
        assert!(app.invitations.is_empty());
        assert!(app.spaces.is_empty());
        assert!(app.detail.is_none());
        assert!(
            app.needs_first_space(),
            "declining the last invitation restores first-space setup"
        );

        let mut owner = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-admin"),
        );
        let count = owner.detail.as_ref().unwrap().members.len();
        owner.admin_result(crate::worker::AdminResult::InvitationCreated(Member {
            id: "invited00001".into(),
            avatar_id: None,
            username: "fixture_invitee".into(),
            display_name: "TEST FIXTURE invitee".into(),
            owner: false,
        }));
        assert_eq!(owner.detail.as_ref().unwrap().members.len(), count);
        assert_eq!(owner.managed_invitations.len(), 1);
        owner.admin_result(crate::worker::AdminResult::InvitationCancelled(
            "invited00001".into(),
        ));
        assert!(owner.managed_invitations.is_empty());
    }

    #[test]
    fn public_preview_is_not_empty_even_without_joined_channels_and_has_no_composer() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-channel-preview"),
        );
        for channel in &mut app.detail.as_mut().unwrap().channels {
            channel.joined = false;
        }
        assert!(!app.selected_is_joined());
        assert!(!app.no_accessible_channels());
        assert!(app.session.is_none());
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(labels.contains(&"Join channel"), "{labels:?}");
        assert!(
            !labels.contains(&"Leave"),
            "preview header must only offer Join: {labels:?}"
        );
        assert!(labels.contains(&"Preview"), "{labels:?}");
        assert!(
            labels.contains(&"Join #design to interact with people here"),
            "{labels:?}"
        );
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Text(text) if text.galley.job.text == "Join #design to interact with people here"
                && text.galley.job.sections.iter().any(|section|
                    &text.galley.job.text[section.byte_range.clone()] == "#design"
                        && section.format.font_id.family == egui::FontFamily::Name("Satoshi Bold".into())
                )
        )), "preview channel name must be bold");
        assert!(
            !labels.contains(&"Send") && !labels.contains(&"Retry session"),
            "{labels:?}"
        );
        assert!(
            !labels.iter().any(|label| label.starts_with("Message #")),
            "preview must not render a composer: {labels:?}"
        );
        assert!(!app.loading, "rendering a preview must not send a mutation");
    }

    #[test]
    fn unjoined_preview_shows_reactions_without_reaction_controls_or_mutations() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-reactions"),
        );
        let channel = app.selected_channel.clone().unwrap();
        app.detail
            .as_mut()
            .unwrap()
            .channels
            .iter_mut()
            .find(|entry| entry.id == channel)
            .unwrap()
            .joined = false;
        let message = app.timeline.messages().nth(1).unwrap().id.clone();
        let key = (message.clone(), "👍".to_owned());
        app.pending_reactions.insert(
            key.clone(),
            PendingReaction {
                desired: true,
                sent: false,
                visible: true,
                superseded: false,
            },
        );
        app.reaction_errors
            .insert(message.clone(), "Try again".into());
        app.reaction_picker = Some(message.clone());

        app.set_reaction(&message, "👍", false);
        assert!(app.pending_reactions[&key].desired);
        let output = render(&mut app, &context, vec![]);
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            labels.contains(&"2"),
            "reaction chip count must remain visible: {labels:?}"
        );
        assert!(!labels.contains(&"Add reaction"), "{labels:?}");
        assert!(!labels.contains(&"Retry"), "{labels:?}");
        assert!(app.reaction_picker.is_none());
    }

    fn texts(output: &egui::FullOutput) -> Vec<&str> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect()
    }

    /// The reaction chip count drawn just below a message's text.
    fn chip_position(output: &egui::FullOutput, message: &str, count: &str) -> egui::Pos2 {
        let below = text_position(output, message).y;
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == count && text.pos.y > below => {
                    Some(text.pos + egui::vec2(3.0, 4.0))
                }
                _ => None,
            })
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .unwrap_or_else(|| panic!("missing {count} chip under {message}"))
    }

    /// Tooltips wait for the pointer to rest, so these frames carry time.
    fn timed_frame(
        app: &mut CaperApp,
        context: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 900.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |context| app.page(context),
        )
    }

    #[test]
    fn read_only_previews_still_show_who_reacted() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-reactions"),
        );
        let channel = app.selected_channel.clone().unwrap();
        app.detail
            .as_mut()
            .unwrap()
            .channels
            .iter_mut()
            .find(|entry| entry.id == channel)
            .unwrap()
            .joined = false;
        let message = app.timeline.messages().nth(1).unwrap().clone();
        timed_frame(&mut app, &context, 0.0, vec![]);
        let output = timed_frame(&mut app, &context, 0.1, vec![]);
        let chip = chip_position(&output, &message.content.text, "2");
        timed_frame(
            &mut app,
            &context,
            0.2,
            vec![egui::Event::PointerMoved(chip)],
        );
        let mut output = timed_frame(&mut app, &context, 1.0, vec![]);
        for step in 1..4 {
            output = timed_frame(&mut app, &context, 1.0 + f64::from(step) * 0.05, vec![]);
        }
        assert!(
            texts(&output).contains(&"You and Maya reacted with :thumbs-up:"),
            "disabled chips keep their hover card: {:?}",
            texts(&output)
        );
        click(&mut app, &context, chip);
        assert!(app.pending_reactions.is_empty(), "previews cannot react");
    }

    #[test]
    fn hovering_a_reaction_chip_names_who_reacted_and_clicking_still_toggles() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-reactions"),
        );
        let message = app.timeline.messages().nth(1).unwrap().clone();
        let frame = |app: &mut CaperApp, time: f64, events: Vec<egui::Event>| {
            timed_frame(app, &context, time, events)
        };
        frame(&mut app, 0.0, vec![]);
        let output = frame(&mut app, 0.1, vec![]);
        let chip = chip_position(&output, &message.content.text, "2");
        let summary = "You and Maya reacted with :thumbs-up:";
        let hovered = frame(&mut app, 0.2, vec![egui::Event::PointerMoved(chip)]);
        assert!(
            !texts(&hovered).contains(&summary),
            "the card waits for the tooltip delay"
        );
        assert!(
            app.reactors.is_empty(),
            "nothing loads before the card shows"
        );
        let mut output = frame(&mut app, 1.0, vec![]);
        for step in 1..4 {
            output = frame(&mut app, 1.0 + f64::from(step) * 0.05, vec![]);
        }
        assert!(
            texts(&output).contains(&summary),
            "fixture names replace the count: {:?}",
            texts(&output)
        );
        let card = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == summary => Some(text.pos),
                _ => None,
            })
            .unwrap();
        assert!(card.y < chip.y, "the card opens above the chip");
        assert!(
            matches!(
                app.reactors[&message.id].state,
                super::ReactorState::Loaded(_)
            ),
            "fixtures answer locally instead of contacting Caper"
        );

        click(&mut app, &context, chip);
        let key = (message.id.clone(), "👍".to_owned());
        assert!(
            app.pending_reactions
                .get(&key)
                .is_some_and(|pending| !pending.desired),
            "clicking removes your reaction exactly as before"
        );
    }

    #[test]
    fn reactor_names_load_once_per_reaction_revision_and_stale_answers_are_ignored() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-reactions"),
        );
        // Exercise the networked path: fixtures otherwise answer locally.
        app.persist_preferences = true;
        let (events, receiver) = std::sync::mpsc::channel();
        app.worker.events = receiver;
        let message = app.timeline.messages().nth(1).unwrap().clone();
        let thumbs = message.reactions[0].clone();
        assert_eq!(thumbs.emoji, "👍");

        app.load_reactors(&message);
        assert!(matches!(
            app.reactors[&message.id].state,
            super::ReactorState::Loading(_)
        ));
        assert_eq!(
            app.reactor_summary(&message, &thumbs),
            "2 people reacted with :thumbs-up:",
            "the snapshot count shows until names arrive"
        );
        let generation = app.generation;
        let answer = |revision: &str, maya: &str| crate::worker::Event::Reactors {
            generation,
            channel: message.channel_id.clone(),
            message: message.id.clone(),
            revision: revision.into(),
            result: Ok(model::Reactors {
                message_id: message.id.clone(),
                reaction_seq: revision.into(),
                reactions: vec![model::ReactorGroup {
                    emoji: "👍".into(),
                    authors: vec![
                        model::Reactor {
                            id: "fixture-maya".into(),
                            username: Some("maya".into()),
                            display_name: Some(maya.into()),
                            avatar_id: Some(15),
                        },
                        model::Reactor {
                            id: "fixture-owner".into(),
                            username: Some("fixture_owner".into()),
                            display_name: Some("Fixture Owner".into()),
                            avatar_id: Some(0),
                        },
                    ],
                }],
            }),
        };
        events.send(answer("3", "Stale Maya")).unwrap();
        app.receive();
        assert!(
            matches!(
                app.reactors[&message.id].state,
                super::ReactorState::Loading(_)
            ),
            "an answer for another revision is ignored"
        );
        events.send(answer("4", "Maya B")).unwrap();
        app.receive();
        assert_eq!(
            app.reactor_summary(&message, &thumbs),
            "You and Maya B reacted with :thumbs-up:"
        );
        let heart = &message.reactions[1];
        assert_eq!(
            app.reactor_summary(&message, heart),
            "1 person reacted with :red-heart:",
            "emoji missing from the list keep the snapshot count"
        );

        // Cached for this revision: hovering again sends nothing new.
        app.load_reactors(&message);
        assert!(matches!(
            app.reactors[&message.id].state,
            super::ReactorState::Loaded(_)
        ));
        // Your own pending toggle changes the people, so the count shows.
        let mut toggled = thumbs.clone();
        toggled.author_ids.retain(|id| id != "fixture-owner");
        assert_eq!(
            app.reactor_summary(&message, &toggled),
            "1 person reacted with :thumbs-up:"
        );
        // A new reaction revision reloads.
        let mut changed = message.clone();
        changed.reaction_seq = Some("5".into());
        app.load_reactors(&changed);
        assert_eq!(app.reactors[&message.id].revision, "5");
        assert!(matches!(
            app.reactors[&message.id].state,
            super::ReactorState::Loading(_)
        ));
        events
            .send(crate::worker::Event::Reactors {
                generation: app.generation,
                channel: message.channel_id.clone(),
                message: message.id.clone(),
                revision: "5".into(),
                result: Err("Could not reach Caper.".into()),
            })
            .unwrap();
        app.receive();
        assert!(matches!(
            app.reactors[&message.id].state,
            super::ReactorState::Failed(_)
        ));
        assert_eq!(
            app.reactor_summary(&changed, &thumbs),
            "2 people reacted with :thumbs-up:",
            "failures keep the snapshot count"
        );
        app.load_reactors(&changed);
        assert!(
            matches!(
                app.reactors[&message.id].state,
                super::ReactorState::Failed(_)
            ),
            "failures are not retried on every frame"
        );
    }

    #[test]
    fn first_space_page_can_open_global_direct_navigation() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-direct-no-spaces"),
        );
        assert!(app.needs_first_space());
        assert_eq!(app.directs.len(), 1);
        app.navigation_open = true;
        assert!(!app.needs_first_space());
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"Direct messages"));
        assert!(texts.contains(&"Select a direct message"));
        assert!(!texts.contains(&"# general"));
        app.navigation_open = false;
        assert!(app.needs_first_space());
    }

    #[test]
    fn direct_conversation_chrome_does_not_use_channel_labels() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-direct"),
        );
        app.timeline.reset(vec![], "0").unwrap();
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"Message TEST FIXTURE Maya"));
        assert!(texts.contains(&"Only you and this person can read this conversation."));
        assert!(!texts.contains(&"Message #general"));
        assert!(!texts.contains(&"Members"));
    }

    #[test]
    fn acknowledged_channel_leave_hides_participation_before_navigation_refresh() {
        for private in [false, true] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-desktop"),
            );
            app.account.as_mut().unwrap().id = "fixture-maya".into();
            let channel = app
                .detail
                .as_ref()
                .unwrap()
                .channels
                .iter()
                .find(|item| item.private == private)
                .unwrap()
                .id
                .clone();
            app.selected_channel = Some(channel.clone());
            app.draft = "old conversation draft".into();
            assert!(app.timeline.messages().next().is_some());
            app.admin_result(crate::worker::AdminResult::ChannelLeft(channel.clone()));
            assert!(
                !app.detail
                    .as_ref()
                    .unwrap()
                    .channels
                    .iter()
                    .any(|item| item.id == channel && item.joined)
            );
            assert_eq!(
                app.detail
                    .as_ref()
                    .unwrap()
                    .channels
                    .iter()
                    .any(|item| item.id == channel),
                !private
            );
            assert!(app.session.is_none());
            assert!(app.timeline.messages().next().is_none());
            assert!(app.draft.is_empty());
            assert!(
                app.selected_channel.is_none(),
                "unavailable refresh cannot restore the left conversation"
            );
        }
    }

    #[test]
    fn deletion_requires_confirmation_and_cancel_preserves_editor_and_data() {
        for (fixture, label) in [
            ("parity-admin", "Delete space"),
            ("parity-channel", "Delete channel"),
        ] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some(fixture),
            );
            let button = |output: &egui::FullOutput, label: &str| {
                output
                    .shapes
                    .iter()
                    .rev()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == label => {
                            Some(text.pos + egui::vec2(4.0, 4.0))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("missing {label}"))
            };
            render(&mut app, &context, vec![]);
            render(&mut app, &context, vec![]);
            let output = scroll_modal_to_bottom(&mut app, &context);
            click(&mut app, &context, button(&output, label));
            assert!(matches!(app.dialog, Some(Dialog::ConfirmDelete { .. })));
            assert!(!app.loading, "opening confirmation must not send DELETE");
            render(&mut app, &context, vec![]);
            let output = render(&mut app, &context, vec![]);
            click(&mut app, &context, button(&output, "Cancel"));
            assert!(matches!(
                app.dialog,
                Some(Dialog::ManageSpace | Dialog::ManageChannel(_))
            ));
            assert!(!app.loading);
            assert_eq!(app.detail.as_ref().unwrap().channels.len(), 3);
        }
    }

    #[test]
    fn narrow_resize_does_not_cover_conversation_with_desktop_members() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-voice-connected"),
        );
        render(&mut app, &context, vec![]);
        assert!(app.members_visible);
        let narrow = |app: &mut CaperApp| {
            context.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(390.0, 844.0),
                    )),
                    ..Default::default()
                },
                |context| app.shell(context),
            )
        };
        let has_members = |output: &egui::FullOutput| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Members")
            })
        };
        narrow(&mut app);
        assert!(!has_members(&narrow(&mut app)));
        app.narrow_members_visible = true;
        assert!(has_members(&narrow(&mut app)));
        assert!(
            app.members_visible,
            "narrow toggle must preserve wide preference"
        );
    }

    #[test]
    fn microphone_errors_wrap_within_the_dock_and_audio_dialog() {
        for fixture in ["parity-voice-error", "parity-audio-error"] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some(fixture),
            );
            app.sidebar_width = 220.0;
            for _ in 0..3 {
                render(&mut app, &context, vec![]);
            }
            let output = if fixture == "parity-audio-error" {
                scroll_modal_to_bottom(&mut app, &context)
            } else {
                render(&mut app, &context, vec![])
            };
            let shape = output
                .shapes
                .iter()
                .find(|shape| matches!(
                    &shape.shape,
                    egui::Shape::Text(text) if text.galley.job.text == crate::media::mic_test::CAPTURE_START_ERROR
                ))
                .unwrap_or_else(|| {
                    panic!("{fixture} microphone failure must be visible");
                });
            let egui::Shape::Text(text) = &shape.shape else {
                unreachable!();
            };
            let bounds = egui::Rect::from_min_size(text.pos, text.galley.size());
            assert!(
                shape.clip_rect.contains_rect(bounds),
                "{fixture} error is clipped"
            );
            if fixture == "parity-voice-error" {
                assert!(text.galley.rows.len() > 1, "dock error must wrap");
                assert!(bounds.right() <= 59.0 + app.sidebar_width);
            }
        }
    }

    #[test]
    fn escape_closes_device_popup_before_audio_dialog() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-audio"),
        );
        app.dialog = Some(Dialog::Audio);
        render(&mut app, &context, vec![]);
        click(&mut app, &context, egui::pos2(216.0, 867.0));
        assert!(
            !app.voice.state.audio.muted,
            "dialog must block background audio controls"
        );
        let output = render(&mut app, &context, vec![]);
        let device = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "System default" => {
                    Some(text.pos + egui::vec2(4.0, 4.0))
                }
                _ => None,
            })
            .expect("device button");
        click(&mut app, &context, device);
        assert!(egui::Popup::is_any_open(&context));
        let escape = |pressed| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        render(&mut app, &context, vec![escape(true)]);
        assert!(!egui::Popup::is_any_open(&context));
        assert!(matches!(app.dialog, Some(Dialog::Audio)));
        render(&mut app, &context, vec![escape(false)]);
        render(&mut app, &context, vec![escape(true)]);
        assert!(app.dialog.is_none());
    }

    #[test]
    fn processing_diagnostics_require_debug_account_without_hiding_connection_stats() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-audio-debug"),
        );
        let contains = |output: &egui::FullOutput, label: &str| {
            output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains(label))
        })
        };
        render(&mut app, &context, vec![]);
        assert!(contains(
            &render(&mut app, &context, vec![]),
            "Copy diagnostics"
        ));
        app.account.as_mut().unwrap().debug_enabled = false;
        assert!(!contains(
            &render(&mut app, &context, vec![]),
            "Copy diagnostics"
        ));
        app.dialog = Some(Dialog::Audio);
        render(&mut app, &context, vec![]);
        let audio = render(&mut app, &context, vec![]);
        let scrolled_audio = scroll_modal_to_bottom(&mut app, &context);
        for copy in [
            "Only you can hear these tests.",
            "Microphone volume",
            "Speaker volume",
            "Test speakers",
            "Try your microphone",
            "Less noise. Clearer voice.",
            "Voice enhancement",
            "Natural",
            "Enhanced",
            "Test microphone",
            "Input level",
        ] {
            assert!(
                contains(&audio, copy) || contains(&scrolled_audio, copy),
                "Missing web copy: {copy}"
            );
        }
        for removed in [
            "contour",
            "noise suppression",
            "Mic test",
            "Microphone test",
            "preferences are saved",
        ] {
            assert!(
                !contains(&audio, removed) && !contains(&scrolled_audio, removed),
                "Unexpected explanatory copy: {removed}"
            );
        }
        app.dialog = Some(Dialog::Connection);
        assert!(contains(
            &render(&mut app, &context, vec![]),
            "Join voice to see connection details."
        ));
    }

    #[test]
    fn management_follows_web_limits_validation_and_private_channel_flow() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        assert!(app.can_create_space() && app.can_create_channel());
        app.limits = Some(crate::model::SpaceLimits {
            owned_spaces: 1,
            total_spaces: 100,
            channels_per_space: 3,
        });
        assert!(!app.can_create_space());
        assert_eq!(
            app.create_space_tooltip(),
            "Space limit reached (1 owned, 100 total)"
        );
        assert!(!app.can_create_channel());
        assert_eq!(app.create_channel_tooltip(), "Channel limit reached (3)");

        app.dialog = Some(Dialog::ManageChannel("chan00000003".into()));
        app.form_name = "planning".into();
        app.form_private = true;
        assert!(!app.channel_dirty("chan00000003"));
        app.form_name = "planning-notes".into();
        assert!(app.channel_dirty("chan00000003"));

        app.admin_result(crate::worker::AdminResult::ChannelCreated(
            crate::model::Channel {
                id: "chan00000009".into(),
                space_id: "space0000001".into(),
                name: "secret".into(),
                private: true,
                joined: true,
            },
        ));
        assert!(matches!(&app.dialog, Some(Dialog::ManageChannel(id)) if id == "chan00000009"));
        assert_eq!(app.form_name, "secret");
        assert!(app.form_private);
    }

    #[test]
    fn chat_counter_and_history_follow_web() {
        use super::{counter_tone, grouped};
        assert_eq!(grouped(3_000), "3,000");
        assert_eq!(grouped(999), "999");
        assert_eq!(counter_tone(3_499), super::MUTED);
        assert_eq!(
            counter_tone(3_500),
            egui::Color32::from_rgb(0xe4, 0xc7, 0x6a)
        );
        assert_eq!(
            counter_tone(3_750),
            egui::Color32::from_rgb(0xed, 0xa3, 0x61)
        );
        assert_eq!(
            counter_tone(3_900),
            egui::Color32::from_rgb(0xff, 0x82, 0x7c)
        );

        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.has_more = true;
        // The whole history fits, so its start is visible: web loads older.
        render(&mut app, &context, vec![]);
        render(&mut app, &context, vec![]);
        assert!(app.loading_older);
        // A failed older page waits for Retry instead of looping.
        app.loading_older = false;
        app.older_error = Some("History unavailable".into());
        render(&mut app, &context, vec![]);
        assert!(!app.loading_older);
        let contains = |output: &egui::FullOutput, label: &str| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains(label))
            })
        };
        assert!(contains(
            &render(&mut app, &context, vec![]),
            "Couldn’t load older messages."
        ));
        app.timeline = crate::model::Timeline::default();
        app.load_error = Some("Could not reach Caper.".into());
        let output = render(&mut app, &context, vec![]);
        assert!(contains(&output, "Could not reach Caper.") && contains(&output, "Try again"));
    }

    #[test]
    fn join_waits_for_each_targets_own_voice_availability() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-voice-checking"),
        );
        app.token = Some("fixture-token".into());
        assert_eq!(
            app.join_unavailable("chan00000001"),
            Some("Checking voice availability…")
        );
        assert!(app.voice_target("chan00000002").is_none());
        app.media_availability.insert("chan00000001".into(), false);
        assert_eq!(
            app.join_unavailable("chan00000001"),
            Some("Joining is not available at this time.")
        );
        assert!(app.voice_target("chan00000002").is_none());
        app.media_availability.insert("chan00000001".into(), true);
        assert!(app.join_unavailable("chan00000001").is_none());
        assert!(
            app.voice_target("chan00000002").is_none(),
            "selected availability cannot enable another channel"
        );
        app.media_availability.insert("chan00000002".into(), true);
        app.media_availability.insert("chan00000001".into(), false);
        assert!(app.voice_target("chan00000002").is_some());
        assert!(app.voice_target("chan00000001").is_none());
        // General's demo service has its own status.
        app.detail.as_mut().unwrap().space.demo = true;
        assert_eq!(app.media_root("chan00000002"), ("general".into(), None));
    }

    #[test]
    fn channel_actions_keep_exact_bounds_across_occupancy_and_join_states() {
        for sidebar in [220.0, 280.0] {
            let mut reference = Vec::new();
            for state in [
                "empty",
                "occupied",
                "authorizing",
                "joining",
                "connected",
                "switching",
            ] {
                let context = egui::Context::default();
                context.enable_accesskit();
                let mut app = CaperApp::new(
                    &context,
                    crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                    Some("parity-desktop"),
                );
                app.sidebar_width = sidebar;
                app.token = Some("fixture-only".into());
                let target = app.voice_target("chan00000001").unwrap().0;
                if state == "occupied" {
                    app.channel_rosters.insert(
                        "chan00000002".into(),
                        vec![crate::model::VoiceOccupant {
                            id: "spectator".into(),
                            avatar_id: Some(15),
                            name: "Maya".into(),
                            muted: false,
                            deafened: false,
                        }],
                    );
                }
                let labels = match state {
                    "authorizing" => {
                        app.pending_voice_join = Some(("chan00000001".into(), 0, 12_345));
                        [
                            "Joining voice in #general",
                            "Join voice in #design",
                            "Join voice in #planning",
                        ]
                    }
                    "joining" => {
                        app.voice.state.phase = Phase::Joining(target);
                        [
                            "Joining voice in #general",
                            "Switch voice to #design",
                            "Switch voice to #planning",
                        ]
                    }
                    "connected" | "switching" => {
                        app.voice.state.phase = Phase::Connected(target);
                        if state == "switching" {
                            app.pending_voice_join = Some(("chan00000002".into(), 0, 12_345));
                        }
                        [
                            "Leave voice in #general",
                            if state == "switching" {
                                "Joining voice in #design"
                            } else {
                                "Switch voice to #design"
                            },
                            "Switch voice to #planning",
                        ]
                    }
                    _ => [
                        "Join voice in #general",
                        "Join voice in #design",
                        "Join voice in #planning",
                    ],
                };
                let output = render(&mut app, &context, vec![]);
                let nodes = output.platform_output.accesskit_update.as_ref().unwrap();
                let start = usize::from(matches!(state, "connected" | "switching"));
                if start == 1 {
                    assert!(
                        !nodes
                            .nodes
                            .iter()
                            .any(|(_, node)| node.label() == Some("Leave voice in #general"))
                    );
                    assert_eq!(
                        nodes
                            .nodes
                            .iter()
                            .filter(|(_, node)| node.label() == Some("Leave voice"))
                            .count(),
                        1,
                        "disconnect is only in the dock"
                    );
                }
                let actions: Vec<_> = labels[start..]
                    .iter()
                    .map(|label| {
                        let node = nodes
                            .nodes
                            .iter()
                            .find(|(_, node)| node.label() == Some(label))
                            .unwrap_or_else(|| panic!("missing {label}"));
                        let bounds = node.1.bounds().unwrap();
                        assert_eq!(bounds.x1 - bounds.x0, 108.0);
                        assert_eq!(bounds.y1 - bounds.y0, 28.0);
                        assert!(bounds.x1 <= 60.0 + f64::from(sidebar));
                        let name = label.rsplit('#').next().unwrap();
                        let name_bounds = nodes
                            .nodes
                            .iter()
                            .find(|(_, node)| node.label() == Some(name))
                            .unwrap()
                            .1
                            .bounds()
                            .unwrap();
                        assert_eq!(name_bounds.y1 - name_bounds.y0, 32.0);
                        assert_eq!(
                            bounds.y0, name_bounds.y1,
                            "voice sits directly under its name"
                        );
                        let disabled = matches!(state, "authorizing" | "joining" | "switching");
                        assert_eq!(node.1.is_disabled(), disabled, "{state}: {label}");
                        bounds
                    })
                    .collect();
                if !reference.is_empty() {
                    assert_eq!(
                        actions.as_slice(),
                        &reference[start..],
                        "{state} moved an action at {sidebar}px"
                    );
                } else {
                    reference = actions;
                }
                assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains("No one in voice") || text.galley.job.text == "0 in voice")));
            }
        }
    }

    #[test]
    fn channel_options_leave_without_navigation_and_only_offer_owner_settings() {
        let mut count_position = None;
        for owner in [true, false] {
            let context = egui::Context::default();
            context.enable_accesskit();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-desktop"),
            );
            if !owner {
                app.account.as_mut().unwrap().id = "fixture-member".into();
            }
            let output = render(&mut app, &context, vec![]);
            let count = text_position(&output, "3");
            if let Some(position) = count_position {
                assert_eq!(count, position, "owner controls moved the section count");
            } else {
                count_position = Some(count);
            }
            let nodes = output.platform_output.accesskit_update.as_ref().unwrap();
            let menu = nodes
                .nodes
                .iter()
                .find(|(_, node)| node.label() == Some("Channel options for design"));
            assert!(
                menu.is_some(),
                "joined channels need options for every member"
            );
            if let Some((_, menu)) = menu {
                let bounds = menu.bounds().unwrap();
                assert_eq!(bounds.x1 - bounds.x0, 32.0);
                let selected = app.selected_channel.clone();
                let pos = egui::pos2(
                    ((bounds.x0 + bounds.x1) / 2.0) as f32,
                    ((bounds.y0 + bounds.y1) / 2.0) as f32,
                );
                click(&mut app, &context, pos);
                let opened = render(&mut app, &context, vec![]);
                assert_eq!(app.selected_channel, selected);
                assert!(
                    app.navigation_target.is_none(),
                    "options must not start channel navigation"
                );
                assert!(matches!(app.voice.state.phase, Phase::Idle));
                if owner {
                    click(
                        &mut app,
                        &context,
                        text_position(&opened, "Channel settings"),
                    );
                    assert!(
                        matches!(app.dialog, Some(Dialog::ManageChannel(ref id)) if id == "chan00000002")
                    );
                } else {
                    assert!(
                        !opened.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Channel settings")),
                        "nonowners must not see channel settings"
                    );
                    click(&mut app, &context, text_position(&opened, "Leave channel"));
                    assert!(matches!(
                        app.dialog,
                        Some(Dialog::LeaveChannel {
                            ref channel,
                            ref name,
                            ..
                        }) if channel == "chan00000002" && name == "design"
                    ));
                }
                assert_eq!(app.selected_channel, selected);
            }
        }
    }

    #[test]
    fn channel_name_icon_and_padding_all_select_the_channel() {
        for sidebar in [220.0, 337.0] {
            for owner in [true, false] {
                for offset in [5.0, 17.5, 39.0, sidebar - 70.0] {
                    let context = egui::Context::default();
                    let mut app = CaperApp::new(
                        &context,
                        crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                        Some("parity-desktop"),
                    );
                    app.sidebar_width = sidebar;
                    if !owner {
                        app.account.as_mut().unwrap().id = "fixture-member".into();
                    }
                    render(&mut app, &context, vec![]);
                    render(&mut app, &context, vec![]);
                    context.enable_accesskit();
                    let output = render(&mut app, &context, vec![]);
                    let bounds = output
                        .platform_output
                        .accesskit_update
                        .unwrap()
                        .nodes
                        .into_iter()
                        .find(|(_, node)| node.label() == Some("design"))
                        .unwrap()
                        .1
                        .bounds()
                        .unwrap();
                    let pos = egui::pos2(
                        bounds.x0 as f32 + offset,
                        ((bounds.y0 + bounds.y1) / 2.0) as f32,
                    );
                    click(&mut app, &context, pos);
                    assert_eq!(
                        app.navigation_target
                            .as_ref()
                            .and_then(|target| target.channel.as_deref()),
                        Some("chan00000002"),
                        "missed channel click at {pos:?}, sidebar {sidebar}, owner {owner}"
                    );
                    assert!(app.dialog.is_none());
                }
            }
        }
    }

    #[test]
    fn clickable_controls_use_hand_but_resize_and_text_keep_their_cursors() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.token = Some("fixture-only".into());
        app.session = Some(session());
        render(&mut app, &context, vec![]);
        render(&mut app, &context, vec![]);
        context.enable_accesskit();
        let output = render(&mut app, &context, vec![]);
        let nodes = output.platform_output.accesskit_update.unwrap().nodes;
        for label in [
            "C",
            "F",
            "design",
            "Channel options for design",
            "Join voice in #design",
            "Create space",
            "Mute microphone",
            "User Settings",
        ] {
            let bounds = nodes
                .iter()
                .find(|(_, node)| node.label() == Some(label))
                .unwrap_or_else(|| panic!("missing {label}"))
                .1
                .bounds()
                .unwrap();
            let pos = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            );
            let output = render(&mut app, &context, vec![egui::Event::PointerMoved(pos)]);
            assert_eq!(
                output.platform_output.cursor_icon,
                egui::CursorIcon::PointingHand,
                "{label}"
            );
        }
        let resize = egui::pos2(340.0, 400.0);
        let output = render(&mut app, &context, vec![egui::Event::PointerMoved(resize)]);
        assert_eq!(
            output.platform_output.cursor_icon,
            egui::CursorIcon::ResizeHorizontal
        );
        assert!(
            !output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::LineSegment { points, stroke }
                    if points[0].x == points[1].x && (points[1].y - points[0].y).abs() > 400.0
                        && stroke.color == super::TERRACOTTA
            )),
            "resizing must not paint an orange line"
        );
        render(
            &mut app,
            &context,
            vec![egui::Event::PointerButton {
                pos: resize,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        let dragged = resize + egui::vec2(67.0, 0.0);
        let output = render(&mut app, &context, vec![egui::Event::PointerMoved(dragged)]);
        assert_eq!(app.sidebar_width, 347.0);
        assert_eq!(
            output.platform_output.cursor_icon,
            egui::CursorIcon::ResizeHorizontal
        );
        assert!(
            !output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::LineSegment { points, stroke }
                    if points[0].x == points[1].x && (points[1].y - points[0].y).abs() > 400.0
                        && stroke.color == super::TERRACOTTA
            )),
            "dragging must not paint an orange line"
        );
        render(
            &mut app,
            &context,
            vec![egui::Event::PointerButton {
                pos: dragged,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        app.sidebar_width = 280.0;
        render(&mut app, &context, vec![]);
        let output = render(
            &mut app,
            &context,
            vec![egui::Event::PointerMoved(egui::pos2(440.0, 867.0))],
        );
        assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::Text);
        app.media_availability.insert("chan00000002".into(), false);
        render(&mut app, &context, vec![]);
        let bounds = nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Join voice in #design"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let output = render(
            &mut app,
            &context,
            vec![egui::Event::PointerMoved(egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            ))],
        );
        assert_eq!(
            output.platform_output.cursor_icon,
            egui::CursorIcon::Default,
            "disabled buttons must not advertise a click"
        );
    }

    #[test]
    fn channel_status_reset_fences_the_previous_access_epoch() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.persist_preferences = true;
        app.refresh_media_status();
        assert_eq!(app.media_status_roots.len(), 3);
        app.refresh_media_status();
        assert_eq!(app.media_status_roots.len(), 3);
        let previous = app.navigation_cache_generation;
        app.invalidate_navigation_cache();
        assert!(app.media_status_roots.is_empty());
        assert!(app.media_availability.is_empty());
        let (events, receiver) = std::sync::mpsc::channel();
        app.worker.events = receiver;
        events
            .send(crate::worker::Event::MediaStatus {
                generation: previous,
                root: "chan00000001".into(),
                enabled: true,
            })
            .unwrap();
        events
            .send(crate::worker::Event::MediaStatus {
                generation: app.navigation_cache_generation,
                root: "chan00000002".into(),
                enabled: false,
            })
            .unwrap();
        app.receive();
        assert!(!app.media_availability.contains_key("chan00000001"));
        assert_eq!(app.media_availability.get("chan00000002"), Some(&false));
    }

    #[test]
    fn pending_authorization_blocks_duplicate_and_competing_joins() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.token = Some("fixture-only".into());
        let original = app.voice_target("chan00000001").unwrap().0;
        app.voice.state.phase = Phase::Connected(original.clone());
        app.join_voice_channel("chan00000002");
        let request = app.voice_join_request;
        app.join_voice_channel("chan00000002");
        app.join_voice_channel("chan00000003");
        assert_eq!(app.voice_join_request, request);
        assert_eq!(
            app.pending_voice_join
                .as_ref()
                .map(|(channel, generation, _)| (channel.as_str(), *generation)),
            Some(("chan00000002", 0))
        );
        assert_eq!(app.voice.state.phase, Phase::Connected(original.clone()));
        app.accept_voice_target(
            request,
            0,
            "space0000001",
            "chan00000002",
            Err(LoadError {
                message: "Temporary failure".into(),
                access_denied: false,
                space_access_denied: false,
            }),
        );
        assert!(app.pending_voice_join.is_none());
        assert_eq!(app.voice.state.phase, Phase::Connected(original));
        app.join_voice_channel("chan00000003");
        assert_eq!(
            app.voice_join_request,
            request + 1,
            "failed checks allow retry"
        );
    }

    #[test]
    fn connection_details_use_web_labels_formats_and_json_names() {
        let stats = media::Diagnostics {
            received_bytes: 1_234_567,
            sent_bytes: 2_000_000,
            receive_bitrate: 31_600.0,
            send_bitrate: 40_400.0,
            packets_lost: 3,
            max_jitter_ms: 12.4,
            round_trip_ms: 51.6,
            route: "direct",
            checks: None,
        };
        let without_join = ConnectionReport::new(None, &stats);
        let rows = without_join.rows();
        assert_eq!(rows[0], ("Received", "1.23 MB".into()));
        assert!(rows.contains(&("Live receive", "32 kbps".into())));
        assert!(rows.contains(&("Live send", "40 kbps".into())));
        assert!(rows.contains(&("RTT", "52 ms".into())));
        assert!(rows.contains(&("Route", "Direct".into())));
        let times = voice::JoinTimes {
            joined_ms: 900.4,
            session_ms: 300.0,
            transport_ms: 200.0,
            ice_ms: None,
            roster_ms: 50.0,
        };
        let report = ConnectionReport::new(Some(&times), &stats);
        let rows = report.rows();
        assert_eq!(rows[0], ("Joined", "Joined in 900 ms".into()));
        assert!(rows.contains(&("Transport + state", "200 ms".into())));
        assert!(rows.contains(&("Connectivity checks", "Not observed yet".into())));
        let json = serde_json::to_value(&report).unwrap();
        for field in [
            "join",
            "sessionMs",
            "transportMs",
            "rosterMs",
            "receivedBytes",
            "sentBytes",
            "receiveBitrate",
            "sendBitrate",
            "packetsLost",
            "maxJitterMs",
            "roundTripMs",
            "route",
        ] {
            assert!(json.get(field).is_some(), "missing {field}");
        }
        assert!(json.get("iceMs").is_none() && json.get("checks").is_none());
    }

    #[test]
    fn compact_sidebar_keeps_join_inside_and_members_at_the_right() {
        for (viewport, sidebar) in [(840.0, 220.0), (840.0, 440.0), (1000.0, 280.0)] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-voice-rosters"),
            );
            app.sidebar_width = sidebar;
            let mut frame = || {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(viewport, 800.0),
                        )),
                        ..Default::default()
                    },
                    |context| app.shell(context),
                )
            };
            frame();
            let output = frame();
            let texts: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text),
                    _ => None,
                })
                .collect();
            let general = texts
                .iter()
                .find(|text| text.galley.job.text == "general")
                .unwrap();
            let joins: Vec<_> = texts
                .iter()
                .filter(|text| text.galley.job.text == "Join voice")
                .collect();
            assert_eq!(joins.len(), 3, "every channel keeps its own voice action");
            assert!(
                joins[0].pos.y > general.pos.y + general.galley.size().y,
                "even empty channels put Join below the channel name"
            );
            for join in joins {
                assert!(
                    join.pos.x + join.galley.size().x < 60.0 + sidebar,
                    "Join text must fit inside the sidebar"
                );
            }
            let members = texts
                .iter()
                .find(|text| text.galley.job.text == "Members")
                .unwrap();
            assert!(
                members.pos.x >= viewport - 220.0 && members.pos.y < 100.0,
                "members must be at the right, never below chat"
            );
        }
    }

    #[test]
    fn native_chrome_matches_web_header_channel_and_composer_geometry() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text),
                _ => None,
            })
            .collect();
        let text = |label: &str| {
            *texts
                .iter()
                .find(|text| text.galley.job.text == label)
                .unwrap_or_else(|| panic!("missing {label}"))
        };
        assert!(
            !texts.iter().any(|text| text.galley.job.text == "Join voice"
                && text.pos.x > 340.0
                && text.pos.y < 54.0),
            "web joins voice from the channel list, not the chat header"
        );
        assert!(
            !texts.iter().any(|text| text.galley.job.text == "Leave"
                && text.pos.x > 340.0
                && text.pos.y < 54.0),
            "channel leave belongs in channel options, not the chat header"
        );
        // Independent values from the current web CSS: 54px header, 44px history.
        for (label, center) in [
            ("Fixture Studio", 27.0),
            ("# general", 27.0),
            ("Members", 27.0),
            ("Beginning of conversation", 76.0),
        ] {
            let shape = text(label);
            assert!(
                (shape.pos.y + shape.galley.size().y / 2.0 - center).abs() < 1.0,
                "{label} is not centered"
            );
        }
        for label in ["general", "design", "planning"] {
            assert_eq!(
                text(label).pos.x,
                106.0,
                "public/private labels must share one offset"
            );
        }
        let joins: Vec<_> = texts
            .iter()
            .filter(|text| text.galley.job.text == "Join voice")
            .collect();
        assert_eq!(joins.len(), 3);
        for (index, name) in ["general", "design", "planning"].iter().enumerate() {
            assert!(joins[index].pos.y > text(name).pos.y + text(name).galley.size().y);
            if let Some(next) = ["general", "design", "planning"].get(index + 1) {
                assert!(joins[index].pos.y + joins[index].galley.size().y < text(next).pos.y);
            }
        }
        assert!(!texts.iter().any(|text| text.galley.job.text == "Live"));
        let dividers: Vec<_> = output.shapes.iter().filter(|shape| matches!(&shape.shape,
            egui::Shape::LineSegment { points, stroke } if stroke.width > 0.0 && points[0].x == 340.0 && points[1].x == 1220.0 && points[0].y > 800.0
        )).collect();
        assert_eq!(
            dividers.len(),
            1,
            "only one divider above the composer: {dividers:?}"
        );
    }

    #[test]
    fn space_and_channel_names_use_webs_validation_copy() {
        assert_eq!(crate::space_name_error("  "), Some("Enter a space name."));
        assert_eq!(crate::space_name_error("Studio"), None);
        assert_eq!(crate::channel_name_error(""), Some("Enter a channel name."));
        assert_eq!(
            crate::channel_name_error("a--b"),
            Some("Use lowercase letters separated by single dashes.")
        );
        assert_eq!(crate::channel_name_error("project-updates"), None);
    }

    #[test]
    fn voice_session_duration_clamps_and_crosses_minute_and_hour_boundaries() {
        for (elapsed, expected) in [
            (999, "00:00"),
            (60_000, "01:00"),
            (3_599_999, "59:59"),
            (3_600_000, "1:00:00"),
            (7_384_000, "2:03:04"),
        ] {
            assert_eq!(
                crate::voice_session_duration(12_345, 12_345 + elapsed),
                expected
            );
        }
        assert_eq!(crate::voice_session_duration(12_345, 12_000), "00:00");
        let old: crate::media::Snapshot = serde_json::from_str(r#"{"participants":[]}"#).unwrap();
        assert_eq!(old.session_started_at, None);
        let current: crate::media::Snapshot =
            serde_json::from_str(r#"{"participants":[],"sessionStartedAt":12345}"#).unwrap();
        assert_eq!(current.session_started_at, Some(12_345));
    }

    #[test]
    fn session_failure_keeps_history_and_offers_retry_session() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        let channel = app.selected_channel.clone().unwrap();
        let history = crate::model::History {
            space: crate::model::HistoryPlace {
                id: "space0000001".into(),
                name: "Fixture Studio".into(),
            },
            channel: crate::model::HistoryPlace {
                id: channel.clone(),
                name: "general".into(),
            },
            messages: app.timeline.messages().cloned().collect(),
            pinned_messages: Vec::new(),
            cursor: "0".into(),
            has_more: false,
        };
        app.accept_channel(history, Err("Chat is unavailable.".into()), false, &channel);
        let output = render(&mut app, &context, vec![]);
        let contains = |output: &egui::FullOutput, label: &str| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains(label))
            })
        };
        assert!(app.session.is_none());
        assert!(contains(&output, "Chat is unavailable.") && contains(&output, "Retry session"));
        assert!(contains(
            &output,
            "TEST FIXTURE — local sample data, not a live conversation."
        ));
    }

    #[test]
    fn failed_refresh_keeps_messages_and_offers_retry_in_the_header() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        render(&mut app, &context, vec![]);
        app.load_error = Some("Could not reach Caper.".into());
        let output = render(&mut app, &context, vec![]);
        let contains = |output: &egui::FullOutput, label: &str| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains(label))
            })
        };
        assert!(contains(&output, "Could not reach Caper.") && contains(&output, "Retry"));
        assert!(
            !contains(&output, "Try again"),
            "Try again is only for a failed first load"
        );
        assert!(contains(
            &output,
            "TEST FIXTURE — local sample data, not a live conversation."
        ));
    }

    #[test]
    fn approaching_join_prepares_signed_in_voice_at_most_every_four_seconds() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.persist_preferences = true;
        app.token = Some("account-token".into());
        let root = app.media_root("chan00000001").0;
        app.media_availability.insert(root, true);
        // Isolate General's approach radius from the newly permanent actions.
        app.media_availability.insert("chan00000002".into(), false);
        app.media_availability.insert("chan00000003".into(), false);
        render(&mut app, &context, vec![]);
        render(&mut app, &context, vec![]);
        let join = text_position(&render(&mut app, &context, vec![]), "Join voice");
        // Outside the 120 px radius nothing is prepared.
        render(
            &mut app,
            &context,
            vec![egui::Event::PointerMoved(join + egui::vec2(0.0, 200.0))],
        );
        assert!(app.prepared_voice.is_empty());
        render(
            &mut app,
            &context,
            vec![egui::Event::PointerMoved(join + egui::vec2(0.0, 90.0))],
        );
        let first = *app
            .prepared_voice
            .values()
            .next()
            .expect("approach prepares the join");
        render(&mut app, &context, vec![egui::Event::PointerMoved(join)]);
        assert_eq!(app.prepared_voice.len(), 1);
        assert_eq!(
            *app.prepared_voice.values().next().unwrap(),
            first,
            "reissued within 4 s"
        );
    }

    #[test]
    fn emoji_composer_inserts_at_caret_without_sending_and_dismisses() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.session = Some(session());
        render(&mut app, &context, vec![]);
        let id = egui::Id::new("message-composer");
        context.memory_mut(|memory| memory.request_focus(id));
        render(
            &mut app,
            &context,
            vec![egui::Event::Text(":rocket".into())],
        );
        render(&mut app, &context, vec![]);
        let key = |key| {
            [true, false]
                .map(|pressed| egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                })
                .to_vec()
        };
        render(&mut app, &context, key(egui::Key::Enter));
        assert_eq!(app.draft, "🚀");
        assert!(app.pending.is_none(), "Accepting emoji must not send");

        app.draft = "👩‍💻 hi :rocket suffix".into();
        let mut state = egui::TextEdit::load_state(&context, id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new("👩‍💻 hi :rocket".chars().count()),
            )));
        state.store(&context, id);
        render(&mut app, &context, vec![]);
        render(&mut app, &context, key(egui::Key::Tab));
        assert_eq!(app.draft, "👩‍💻 hi 🚀 suffix");
        assert_eq!(
            egui::TextEdit::load_state(&context, id)
                .unwrap()
                .cursor
                .char_range()
                .unwrap()
                .primary
                .index,
            "👩‍💻 hi 🚀".chars().count()
        );
        assert!(app.pending.is_none());

        app.draft = ":".into();
        let mut state = egui::TextEdit::load_state(&context, id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(1),
            )));
        state.store(&context, id);
        render(&mut app, &context, vec![]);
        render(&mut app, &context, key(egui::Key::ArrowDown));
        render(&mut app, &context, key(egui::Key::Tab));
        assert_eq!(app.draft, "😀");
        render(
            &mut app,
            &context,
            vec![egui::Event::Text(" :thumbs_up".into())],
        );
        render(&mut app, &context, key(egui::Key::Escape));
        assert_eq!(app.draft, "😀 :thumbs_up");
        assert!(
            context.memory(|memory| memory.has_focus(id)),
            "Escape must retain composer focus"
        );
        assert!(
            app.suggestion_dismissed.is_some(),
            "Escape must dismiss the current token"
        );
        render(&mut app, &context, key(egui::Key::Enter));
        assert_eq!(app.pending.as_ref().unwrap().text, "😀 :thumbs_up");
    }

    fn press(key: egui::Key) -> Vec<egui::Event> {
        [true, false]
            .map(|pressed| egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            })
            .to_vec()
    }

    fn place_caret(context: &egui::Context, draft: &str, app: &mut CaperApp) {
        let id = egui::Id::new("message-composer");
        app.draft = draft.replace('|', "");
        let mut state = egui::TextEdit::load_state(context, id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(draft.chars().take_while(|c| *c != '|').count()),
            )));
        state.store(context, id);
    }

    fn caret(context: &egui::Context) -> usize {
        egui::TextEdit::load_state(context, egui::Id::new("message-composer"))
            .unwrap()
            .cursor
            .char_range()
            .unwrap()
            .primary
            .index
    }

    fn text_shapes(output: &egui::FullOutput) -> Vec<String> {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(text) => Some(text.galley.job.text.clone()),
                _ => None,
            })
            .collect()
    }

    fn shows(output: &egui::FullOutput, text: &str) -> bool {
        text_shapes(output).iter().any(|label| label == text)
    }

    #[test]
    fn mention_composer_suggests_members_and_specials_and_inserts_without_sending() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.session = Some(session());
        render(&mut app, &context, vec![]);
        let id = egui::Id::new("message-composer");
        context.memory_mut(|memory| memory.request_focus(id));
        render(&mut app, &context, vec![egui::Event::Text("@".into())]);
        let output = render(&mut app, &context, vec![]);
        let labels = text_shapes(&output);
        for row in [
            "Alex @alex",
            "Maya @maya",
            "@everyone Everyone in this channel",
            "@here Everyone online in this channel",
        ] {
            assert!(labels.iter().any(|label| label == row), "{row}: {labels:?}");
        }
        assert!(
            !labels.iter().any(|label| label.contains("@fixture_owner")),
            "never suggest yourself: {labels:?}"
        );
        render(&mut app, &context, vec![egui::Event::Text("ma".into())]);
        render(&mut app, &context, vec![]);
        render(&mut app, &context, press(egui::Key::Enter));
        assert_eq!(app.draft, "@maya ");
        assert_eq!(caret(&context), 6);
        assert!(app.pending.is_none(), "Accepting a mention must not send");

        place_caret(&context, "👩‍💻 hi @AL| suffix", &mut app);
        render(&mut app, &context, vec![]);
        render(&mut app, &context, press(egui::Key::Tab));
        assert_eq!(app.draft, "👩‍💻 hi @alex  suffix");
        assert_eq!(caret(&context), "👩‍💻 hi @alex ".chars().count());
        assert!(
            context.memory(|memory| memory.has_focus(id)),
            "Tab must stay in the composer"
        );

        place_caret(&context, "(@|", &mut app);
        render(&mut app, &context, vec![]);
        render(&mut app, &context, press(egui::Key::ArrowDown));
        render(&mut app, &context, press(egui::Key::ArrowDown));
        render(&mut app, &context, press(egui::Key::Tab));
        assert_eq!(app.draft, "(@everyone ");

        place_caret(&context, "mail bob@ma|", &mut app);
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        assert!(
            !shows(&output, "Maya @maya"),
            "an email address is not a mention"
        );

        // Selected text and active IME composition never open suggestions.
        app.draft = "hi @ma".into();
        let mut state = egui::TextEdit::load_state(&context, id).unwrap();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(3),
                egui::text::CCursor::new(6),
            )));
        state.store(&context, id);
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        assert!(!shows(&output, "Maya @maya"), "selection");
        place_caret(&context, "hi @ma|", &mut app);
        let ime = |event| vec![egui::Event::Ime(event)];
        render(&mut app, &context, ime(egui::ImeEvent::Enabled));
        let output = render(&mut app, &context, vec![]);
        assert!(!shows(&output, "Maya @maya"), "IME composition");
        render(&mut app, &context, ime(egui::ImeEvent::Disabled));
        let output = render(&mut app, &context, vec![]);
        assert!(shows(&output, "Maya @maya"));
        render(&mut app, &context, press(egui::Key::Escape));
        assert!(app.suggestion_dismissed.is_some());
        assert!(context.memory(|memory| memory.has_focus(id)));
        render(&mut app, &context, press(egui::Key::Enter));
        assert_eq!(app.pending.as_ref().unwrap().text, "hi @ma");
    }

    #[test]
    fn mention_candidates_follow_the_conversation() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-direct"),
        );
        let names = |app: &CaperApp, query: &str| {
            app.suggestions(&ComposerToken::Mention(mentions::Token {
                start: 0,
                end: query.len() + 1,
                query: query.into(),
            }))
            .into_iter()
            .map(|choice| match choice {
                Suggestion::Mention(candidate) => candidate.name().to_owned(),
                Suggestion::Emoji(entry) => entry.emoji.clone(),
            })
            .collect::<Vec<_>>()
        };
        let (events, receiver) = std::sync::mpsc::channel();
        app.worker.events = receiver;
        // The fixture previews `GET /api/people`: space members and DM peers.
        assert_eq!(names(&app, ""), ["alex", "maya"]);
        assert!(names(&app, "every").is_empty(), "no specials in DMs");
        app.people = None;
        assert_eq!(names(&app, ""), ["maya"], "the peer until people load");

        let person = |id: &str, username: &str| model::Person {
            id: id.into(),
            username: username.into(),
            display_name: username.to_uppercase(),
            avatar_id: None,
        };
        let loaded = |generation, result| crate::worker::Event::PeopleLoaded { generation, result };
        events
            .send(loaded(
                app.generation,
                Ok(vec![
                    person("fixture-maya", "maya"),
                    person("fixture-sam", "sam"),
                ]),
            ))
            .unwrap();
        app.receive();
        assert_eq!(names(&app, ""), ["maya", "sam"], "people beyond the peer");
        assert_eq!(names(&app, "SA"), ["sam"]);
        events
            .send(loaded(app.generation - 1, Ok(Vec::new())))
            .unwrap();
        events
            .send(loaded(app.generation, Err("offline".into())))
            .unwrap();
        app.receive();
        assert_eq!(
            names(&app, ""),
            ["maya", "sam"],
            "stale or failed refreshes keep the list"
        );
        app.directs[0].peer = model::DirectPeer {
            id: "fixture-new".into(),
            username: "newbie".into(),
            display_name: "Newbie".into(),
            avatar_id: None,
        };
        assert_eq!(
            names(&app, ""),
            ["maya", "newbie", "sam"],
            "a DM newer than the list still offers its peer"
        );
        app.directs[0].peer.id = app.account.as_ref().unwrap().id.clone();
        assert_eq!(names(&app, ""), ["maya", "sam"], "self-notes uses people");
        app.people = None;
        assert!(
            names(&app, "").is_empty(),
            "self-notes suggests nobody before people load"
        );

        app.selected_direct = None;
        app.selected_channel = Some("chan00000001".into());
        assert_eq!(names(&app, ""), ["alex", "maya", "everyone", "here"]);
        app.detail = None;
        assert_eq!(
            names(&app, ""),
            ["everyone", "here"],
            "only specials until members load"
        );
    }

    const ALEX_MESSAGE: &str =
        "Keep the space rail and audio controls in their usual places, @alex. @nobody stays plain.";
    const OWNER_MESSAGE: &str =
        "@fixture_owner, the same conversation should feel familiar on every platform.";
    const EVERYONE_MESSAGE: &str =
        "Agreed, @everyone. Let’s check the narrow layout and the management dialogs too.";

    /// Screen center of `token`'s pill in the rendered message `text`.
    fn pill_center(output: &egui::FullOutput, text: &str, token: &str) -> egui::Pos2 {
        pill_rect(output, text, token).center()
    }

    fn pill_rect(output: &egui::FullOutput, text: &str, token: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(shape) if shape.galley.job.text == text => {
                    let start = text.find(token)?;
                    let range =
                        text[..start].chars().count()..text[..start + token.len()].chars().count();
                    pill_rects(&shape.galley, range)
                        .first()
                        .map(|rect| rect.translate(shape.pos.to_vec2()))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{token} is not drawn"))
    }

    fn text_center(output: &egui::FullOutput, text: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(shape) if shape.galley.job.text == text => {
                    Some(shape.pos + shape.galley.rect.center().to_vec2())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("{text} is not drawn"))
    }

    fn click_at(app: &mut CaperApp, context: &egui::Context, pos: egui::Pos2) -> egui::FullOutput {
        let mut output = None;
        for pressed in [true, false] {
            output = Some(render(
                app,
                context,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            ));
        }
        output.unwrap()
    }

    fn pill_fill(output: &egui::FullOutput, text: &str, token: &str) -> Option<egui::Color32> {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(shape) if shape.galley.job.text == text => shape
                .galley
                .job
                .sections
                .iter()
                .find(|section| &text[section.byte_range.clone()] == token)
                .map(|section| section.format.background),
            _ => None,
        })
    }

    fn card_entry(app: &CaperApp) -> Option<&str> {
        app.mention_card
            .as_ref()
            .and_then(|card| card.pill.entry.id.as_deref())
    }

    #[test]
    fn person_pills_open_profile_cards_that_close_on_escape_or_outside_click() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-mentions"),
        );
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let alex = pill_center(&output, ALEX_MESSAGE, "@alex");
        let unhovered = TERRACOTTA.gamma_multiply(0.24);
        assert_eq!(pill_fill(&output, ALEX_MESSAGE, "@alex"), Some(unhovered));

        let hovered = render(&mut app, &context, vec![egui::Event::PointerMoved(alex)]);
        assert_eq!(
            hovered.platform_output.cursor_icon,
            egui::CursorIcon::PointingHand
        );
        let hovered = render(&mut app, &context, vec![]);
        assert_eq!(
            pill_fill(&hovered, ALEX_MESSAGE, "@alex"),
            Some(TERRACOTTA.gamma_multiply(0.32))
        );
        assert_eq!(
            pill_fill(&hovered, ALEX_MESSAGE, "@nobody"),
            None,
            "unresolved names stay plain"
        );

        click_at(&mut app, &context, alex);
        assert_eq!(card_entry(&app), Some("fixture-alex"));
        // New egui areas spend their first pass measuring, invisibly.
        let output = render(&mut app, &context, vec![]);
        for label in ["Alex", "@alex", "Message"] {
            assert!(shows(&output, label), "{label}: {:?}", text_shapes(&output));
        }
        let output = render(&mut app, &context, vec![]);
        assert!(
            shows(&output, "Message"),
            "the opening click is not outside"
        );
        let pill = app.mention_card.as_ref().unwrap().pill.id;

        render(&mut app, &context, press(egui::Key::Escape));
        assert!(app.mention_card.is_none());
        render(&mut app, &context, vec![]);
        assert!(
            context.memory(|memory| memory.has_focus(pill)),
            "focus returns to the pill"
        );
        render(&mut app, &context, press(egui::Key::Enter));
        assert_eq!(
            card_entry(&app),
            Some("fixture-alex"),
            "Enter on a focused pill opens its card"
        );
        assert!(app.pending.is_none());

        let output = render(&mut app, &context, vec![]);
        let owner = pill_center(&output, OWNER_MESSAGE, "@fixture_owner");
        click_at(&mut app, &context, owner);
        assert_eq!(
            card_entry(&app),
            Some("fixture-owner"),
            "another pill replaces the card"
        );
        let output = render(&mut app, &context, vec![]);
        assert!(shows(&output, "Fixture Owner") && shows(&output, "You"));
        assert!(!shows(&output, "Message"), "no Message for yourself");

        let outside = text_center(&output, "Beginning of conversation");
        click_at(&mut app, &context, outside);
        assert!(
            app.mention_card.is_none(),
            "a click outside closes the card"
        );

        let output = render(&mut app, &context, vec![]);
        let everyone = pill_center(&output, EVERYONE_MESSAGE, "@everyone");
        click_at(&mut app, &context, everyone);
        assert!(
            app.mention_card.is_none(),
            "@everyone stays non-interactive"
        );
    }

    #[test]
    fn mention_card_opens_below_its_pill_or_above_it_near_the_bottom() {
        let card_beside_pill = |height: f32, text: &str, token: &str| {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-mentions"),
            );
            // The last message sits just above the composer.
            let mut messages: Vec<_> = app.timeline.messages().cloned().collect();
            messages[3].content.text = "Thanks @maya".into();
            messages[3].content.mentions = vec![model::Mention {
                kind: "user".into(),
                id: Some("fixture-maya".into()),
                username: Some("maya".into()),
            }];
            app.timeline.reset(messages, "4").unwrap();
            let frame = |app: &mut CaperApp, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1440.0, height),
                        )),
                        events,
                        ..Default::default()
                    },
                    |context| app.page(context),
                )
            };
            frame(&mut app, vec![]);
            let output = frame(&mut app, vec![]);
            let pill = pill_rect(&output, text, token);
            for pressed in [true, false] {
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pill.center()),
                        egui::Event::PointerButton {
                            pos: pill.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            frame(&mut app, vec![]);
            frame(&mut app, vec![]);
            let card = context
                .memory(|memory| memory.area_rect(egui::Id::new("mention-card")))
                .unwrap();
            (pill, card)
        };
        let (pill, card) = card_beside_pill(900.0, ALEX_MESSAGE, "@alex");
        assert!(card.top() >= pill.bottom(), "{pill:?} {card:?}");
        assert!(card.width() <= 280.0 + 0.5, "{card:?}");
        let (pill, card) = card_beside_pill(560.0, "Thanks @maya", "@maya");
        assert!(card.bottom() <= pill.top(), "{pill:?} {card:?}");
    }

    #[test]
    fn mention_card_message_opens_an_existing_dm_or_shows_a_creation_error() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-mentions"),
        );
        app.token = Some("account-token".into());
        let existing = model::DirectConversation {
            id: "dm0000000009".into(),
            peer: model::DirectPeer {
                id: "fixture-alex".into(),
                username: "alex".into(),
                display_name: "Alex".into(),
                avatar_id: None,
            },
            last_seq: "0".into(),
            read_seq: "0".into(),
            status: model::DirectStatus::Accepted,
            blocked: false,
        };
        app.directs.push(existing.clone());
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        click_at(
            &mut app,
            &context,
            pill_center(&output, ALEX_MESSAGE, "@alex"),
        );
        // New egui areas spend their first pass measuring, invisibly.
        let output = render(&mut app, &context, vec![]);
        click_at(&mut app, &context, text_center(&output, "Message"));
        assert_eq!(app.selected_direct.as_deref(), Some("dm0000000009"));
        assert!(app.mention_card.is_none(), "opening the DM closes the card");

        // Without a DM, Message creates one by username; this API is unreachable.
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-mentions"),
        );
        app.token = Some("account-token".into());
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        click_at(
            &mut app,
            &context,
            pill_center(&output, ALEX_MESSAGE, "@alex"),
        );
        // New egui areas spend their first pass measuring, invisibly.
        let output = render(&mut app, &context, vec![]);
        click_at(&mut app, &context, text_center(&output, "Message"));
        assert_eq!(
            app.mention_card.as_ref().unwrap().opening,
            Some(app.navigation)
        );
        assert!(shows(&render(&mut app, &context, vec![]), "Opening…"));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.mention_card.as_ref().unwrap().opening.is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            app.receive();
        }
        let card = app
            .mention_card
            .as_ref()
            .expect("a failure keeps the card open");
        let error = card
            .error
            .clone()
            .expect("the API error is shown in the card");
        assert!(app.error.is_none(), "not duplicated outside the card");
        assert!(app.selected_direct.is_none());
        let output = render(&mut app, &context, vec![]);
        assert!(shows(&output, &error) && shows(&output, "Message"));
    }

    #[test]
    fn mention_card_message_creates_the_dm_by_username_and_navigates() {
        use std::io::{BufRead, BufReader, Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let api =
            crate::api::Api::new(&format!("http://{}", server.local_addr().unwrap())).unwrap();
        let requests = std::thread::spawn(move || {
            loop {
                let (stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                let (mut length, mut bearer) = (0, false);
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let lower = line.to_ascii_lowercase();
                    if let Some(value) = lower.strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                    bearer |= lower == "authorization: bearer account-token\r\n";
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                if request != "POST /api/dms HTTP/1.1\r\n" {
                    write!(reader.get_mut(), "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
                    continue;
                }
                let direct = r#"{"id":"dm0000000002","peer":{"id":"fixture-alex","username":"alex","displayName":"Alex"},"lastSeq":"0","readSeq":"0"}"#;
                write!(reader.get_mut(), "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{direct}", direct.len()).unwrap();
                return (
                    bearer,
                    serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                );
            }
        });
        let context = egui::Context::default();
        let mut app = CaperApp::new(&context, api, Some("parity-mentions"));
        app.token = Some("account-token".into());
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        click_at(
            &mut app,
            &context,
            pill_center(&output, ALEX_MESSAGE, "@alex"),
        );
        // New egui areas spend their first pass measuring, invisibly.
        let output = render(&mut app, &context, vec![]);
        click_at(&mut app, &context, text_center(&output, "Message"));
        let (bearer, body) = requests.join().unwrap();
        assert!(bearer);
        assert_eq!(body, serde_json::json!({"username": "alex"}));
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.selected_direct.is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            app.receive();
        }
        assert_eq!(app.selected_direct.as_deref(), Some("dm0000000002"));
        assert!(app.directs.iter().any(|direct| direct.id == "dm0000000002"));
        assert!(app.mention_card.is_none());
    }

    #[test]
    fn thread_reply_pills_open_the_profile_card() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-mentions"),
        );
        let mut messages: Vec<_> = app.timeline.messages().cloned().collect();
        let root = messages[0].id.clone();
        messages[3].thread_root_id = Some(root.clone());
        messages[3].content.text = "Replying to @alex".into();
        messages[3].content.mentions = vec![model::Mention {
            kind: "user".into(),
            id: Some("fixture-alex".into()),
            username: Some("alex".into()),
        }];
        app.timeline.reset(messages, "4").unwrap();
        app.open_thread(root);
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        // The thread panel is on the right; take the rightmost copy of the reply.
        let reply = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::epaint::Shape::Text(shape)
                    if shape.galley.job.text == "Replying to @alex" =>
                {
                    let start = "Replying to ".chars().count();
                    pill_rects(&shape.galley, start..start + "@alex".len())
                        .first()
                        .map(|rect| rect.translate(shape.pos.to_vec2()).center())
                }
                _ => None,
            })
            .max_by(|left, right| left.x.total_cmp(&right.x))
            .expect("the thread reply is drawn");
        click_at(&mut app, &context, reply);
        assert_eq!(card_entry(&app), Some("fixture-alex"));
        let output = render(&mut app, &context, vec![]);
        assert!(shows(&output, "Alex") && shows(&output, "Message"));
    }

    #[test]
    fn mention_pills_render_in_thread_replies_and_follow_edits() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-mentions"),
        );
        let user = |id: &str, username: &str| model::Mention {
            kind: "user".into(),
            id: Some(id.into()),
            username: Some(username.into()),
        };
        let mut messages: Vec<_> = app.timeline.messages().cloned().collect();
        let root = messages[0].id.clone();
        messages[3].thread_root_id = Some(root.clone());
        messages[3].content.text = "Replying to @alex".into();
        messages[3].content.mentions = vec![user("fixture-alex", "alex")];
        app.timeline.reset(messages, "4").unwrap();
        app.open_thread(root);
        let pill = |output: &egui::FullOutput, text: &str, token: &str| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape,
                egui::epaint::Shape::Text(shape) if shape.galley.job.text == text
                    && shape.galley.job.sections.iter().any(|section|
                        &text[section.byte_range.clone()] == token
                            && section.format.background == TERRACOTTA.gamma_multiply(0.24)))
            })
        };
        let tints = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter(|shape| {
                    matches!(&shape.shape,
                    egui::epaint::Shape::Rect(rect) if rect.fill == TERRACOTTA.gamma_multiply(0.08))
                })
                .count()
        };
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        assert!(
            pill(&output, "Replying to @alex", "@alex"),
            "thread reply pill"
        );
        let before = tints(&output);

        // An edit carries the server's re-resolved mentions.
        let mut edited = app.timeline.messages().nth(2).unwrap().clone();
        edited.content.text = "Now asking @fixture_owner instead".into();
        edited.content.mentions = vec![user("fixture-owner", "fixture_owner")];
        edited.revision = 2;
        edited.edited_at = Some("2026-10-06T09:44:00Z".into());
        edited.edit_seq = Some("5".into());
        app.timeline
            .apply_edit(model::EditUpdate {
                kind: "message.edited".into(),
                schema_version: 1,
                channel_id: edited.channel_id.clone(),
                seq: "5".into(),
                message: edited,
            })
            .unwrap();
        let output = render(&mut app, &context, vec![]);
        assert!(pill(
            &output,
            "Now asking @fixture_owner instead",
            "@fixture_owner"
        ));
        assert_eq!(tints(&output), before + 1, "the edit now mentions you");
    }

    #[test]
    fn mention_pills_and_mentions_me_rows_render_from_server_entries() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        let entry = |kind: &str, id: Option<&str>, username: Option<&str>| model::Mention {
            kind: kind.into(),
            id: id.map(Into::into),
            username: username.map(Into::into),
        };
        let mut messages: Vec<_> = app.timeline.messages().cloned().collect();
        messages[0].content.text = "@everyone from me".into();
        messages[0].content.mentions = vec![entry("everyone", None, None)];
        messages[1].content.text = "hi @Fixture_Owner and @nobody".into();
        messages[1].content.mentions =
            vec![entry("user", Some("fixture-owner"), Some("fixture_owner"))];
        messages[2].content.text = "ping @alex @here".into();
        messages[2].content.mentions = vec![entry("user", Some("fixture-alex"), Some("alex"))];
        app.timeline.reset(messages, "4").unwrap();
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let pill = |text: &str, token: &str| {
            output.shapes.iter().any(|shape| {
                matches!(&shape.shape,
                egui::epaint::Shape::Text(galley) if galley.galley.job.text == text
                    && galley.galley.job.sections.iter().any(|section|
                        &text[section.byte_range.clone()] == token
                            && section.format.background == TERRACOTTA.gamma_multiply(0.24)
                            && section.format.color == TEXT
                            && section.format.font_id.family
                                == egui::FontFamily::Name("Satoshi Medium".into())))
            })
        };
        assert!(pill("@everyone from me", "@everyone"));
        assert!(pill("hi @Fixture_Owner and @nobody", "@Fixture_Owner"));
        assert!(!pill("hi @Fixture_Owner and @nobody", "@nobody"));
        assert!(pill("ping @alex @here", "@alex"));
        assert!(!pill("ping @alex @here", "@here"), "no here entry");
        let tints = output
            .shapes
            .iter()
            .filter(|shape| {
                matches!(&shape.shape,
                egui::epaint::Shape::Rect(rect) if rect.fill == TERRACOTTA.gamma_multiply(0.08))
            })
            .count();
        assert_eq!(
            tints, 1,
            "only the message naming me is tinted, not my own @everyone"
        );
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::epaint::Shape::Rect(rect) if rect.fill == TERRACOTTA && rect.rect.width() == 2.0)));
    }

    #[test]
    fn composer_grows_and_shift_enter_does_not_send_after_shift_is_released() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.session = Some(session());
        app.draft = "first".into();
        let editor_height = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect)
                        if rect.rect.left() == 358.0
                            && rect.fill == egui::Color32::TRANSPARENT
                            && rect.stroke.width == 1.0 =>
                    {
                        Some(rect.rect.height())
                    }
                    _ => None,
                })
                .expect("composer outline")
        };
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        assert_eq!(editor_height(&output), 42.0);
        let id = egui::Id::new("message-composer");
        context.memory_mut(|memory| memory.request_focus(id));
        let enter = |pressed, modifiers| egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        // Both Shift+Enter and the release can arrive in one frame. RawInput's
        // final modifiers are NONE; only the Enter event records the held Shift.
        render(
            &mut app,
            &context,
            vec![
                enter(true, egui::Modifiers::SHIFT),
                enter(false, egui::Modifiers::NONE),
                egui::Event::Text("second".into()),
            ],
        );
        assert_eq!(app.draft, "first\nsecond");
        assert!(
            app.pending.is_none(),
            "Shift+Enter must never submit a draft"
        );
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        assert!(
            editor_height(&output) >= 56.0,
            "both lines must fit: {}",
            editor_height(&output)
        );
        render(&mut app, &context, vec![enter(true, egui::Modifiers::NONE)]);
        assert_eq!(app.pending.as_ref().unwrap().text, "first\nsecond");
        assert!(app.error.is_none());
        app.draft = "line\n".repeat(40);
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        assert!(
            editor_height(&output) <= 320.0,
            "long drafts must not consume the conversation"
        );
    }

    #[test]
    fn startup_control_is_accessible_but_cannot_change_os_registration_in_fixtures() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        render(&mut app, &context, vec![]);
        render(&mut app, &context, vec![]);
        click(&mut app, &context, egui::pos2(307.0, 867.0));
        let menu = render(&mut app, &context, vec![]);
        assert!(
            !menu.shapes.iter().any(|shape| matches!(
                &shape.shape,
                egui::Shape::Text(text) if text.galley.job.text == "Launch at login"
                    || text.galley.job.text == "Caper sound effects"
            )),
            "infrequent preferences must not appear in the quick menu"
        );
        let pos = text_position(&menu, "Settings…");
        render(
            &mut app,
            &context,
            [true, false]
                .into_iter()
                .map(|pressed| egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                })
                .collect(),
        );
        assert!(matches!(app.dialog, Some(Dialog::Settings)));
        context.enable_accesskit();
        let output = render(&mut app, &context, vec![]);
        let node = output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some("Launch at login"))
            .expect("startup option has an accessible label");
        assert!(
            node.1.is_disabled(),
            "preview must not alter real startup settings"
        );
        click(
            &mut app,
            &context,
            text_position(&output, "Launch at login"),
        );
        assert!(!app.launch_at_login);
        assert!(app.startup_error.is_none());
    }

    #[test]
    fn startup_failures_remain_visible_until_dismissed() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.startup_error = Some("TEST FIXTURE — startup registration denied".into());
        app.warning = Some("TEST FIXTURE — unrelated account warning".into());
        render(&mut app, &context, vec![]);
        render(&mut app, &context, vec![]);
        click(&mut app, &context, egui::pos2(307.0, 867.0));
        let menu = render(&mut app, &context, vec![]);
        click(&mut app, &context, text_position(&menu, "Settings…"));
        let output = render(&mut app, &context, vec![]);
        text_position(&output, "TEST FIXTURE — startup registration denied");
        assert!(!output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Text(text) if text.galley.job.text == "TEST FIXTURE — unrelated account warning"
        )), "account notices must not leak into local preferences");
        click(&mut app, &context, egui::pos2(5.0, 5.0));
        assert!(app.dialog.is_none());
        assert!(
            app.startup_error.is_some(),
            "closing settings must not lose an error"
        );
        click(&mut app, &context, egui::pos2(307.0, 867.0));
        let menu = render(&mut app, &context, vec![]);
        click(&mut app, &context, text_position(&menu, "Settings…"));
        let output = render(&mut app, &context, vec![]);
        click(
            &mut app,
            &context,
            text_position(&output, "Dismiss startup error"),
        );
        assert!(app.startup_error.is_none());
        assert!(!app.launch_at_login);
    }

    #[test]
    fn audio_controls_and_voice_dock_act_on_current_intent() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        render(&mut app, &context, vec![]);
        // Panels settle their persisted height on the second frame, as in eframe.
        render(&mut app, &context, vec![]);
        click(&mut app, &context, egui::pos2(216.0, 867.0));
        assert!(app.voice.state.audio.muted);
        click(&mut app, &context, egui::pos2(216.0, 867.0));
        assert!(!app.voice.state.audio.muted);
        click(&mut app, &context, egui::pos2(262.0, 867.0));
        assert!(app.voice.state.audio.muted && app.voice.state.audio.deafened);
        click(&mut app, &context, egui::pos2(262.0, 867.0));
        assert!(!app.voice.state.audio.muted && !app.voice.state.audio.deafened);
        click(&mut app, &context, egui::pos2(307.0, 867.0));
        let menu = render(&mut app, &context, vec![]);
        click(&mut app, &context, text_position(&menu, "Settings…"));
        let output = render(&mut app, &context, vec![]);
        assert!(app.sound_effects);
        click(
            &mut app,
            &context,
            text_position(&output, "Caper sound effects"),
        );
        assert!(!app.sound_effects);
        assert!(
            matches!(app.dialog, Some(Dialog::Settings)),
            "switches must not dismiss settings"
        );
        let output = render(&mut app, &context, vec![]);
        click(
            &mut app,
            &context,
            text_position(&output, "Caper sound effects"),
        );
        assert!(app.sound_effects);
        let output = render(&mut app, &context, vec![]);
        let pos = text_position(&output, "Caper sound effects");
        let id = context.memory(|memory| memory.focused().unwrap());
        render(
            &mut app,
            &context,
            vec![egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(
            !app.sound_effects,
            "focused switch must support keyboard activation"
        );
        assert_eq!(context.memory(|memory| memory.focused()), Some(id));
        click(&mut app, &context, pos);
        assert!(app.sound_effects);
        for (fixture, label) in [
            ("parity-voice-joining", "Cancel joining voice"),
            ("parity-voice-connected", "Leave voice"),
        ] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some(fixture),
            );
            // Panels settle on the second frame; AccessKit's first update is
            // the full tree, so enable it for that frame.
            render(&mut app, &context, vec![]);
            context.enable_accesskit();
            // The dock's hang-up; web has no chat-header Leave.
            let bounds = render(&mut app, &context, vec![])
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .into_iter()
                .find_map(|(_, node)| {
                    (node.label() == Some(label))
                        .then(|| node.bounds())
                        .flatten()
                })
                .unwrap_or_else(|| panic!("missing {label}"));
            let center = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            );
            click(&mut app, &context, center);
            assert!(
                matches!(app.voice.state.phase, crate::state::Phase::Idle),
                "Cancel/Leave must end the current call"
            );
        }
    }

    fn account(profile: bool) -> Account {
        Account {
            id: "account".into(),
            avatar_id: None,
            username: profile.then(|| "member".into()),
            display_name: profile.then(|| "Member".into()),
            debug_enabled: false,
        }
    }

    fn spaces() -> Spaces {
        Spaces {
            spaces: Vec::new(),
            invitations: Vec::new(),
            limits: None,
        }
    }

    fn history(channel: &str) -> History {
        History {
            messages: Vec::new(),
            pinned_messages: Vec::new(),
            cursor: "0".into(),
            has_more: false,
            space: HistoryPlace {
                id: "space".into(),
                name: "Space".into(),
            },
            channel: HistoryPlace {
                id: channel.into(),
                name: channel.into(),
            },
        }
    }

    fn session() -> ChatSession {
        ChatSession {
            token: "chat-token".into(),
            author: Author {
                id: "account".into(),
                avatar_id: None,
                name: "Member".into(),
                is_guest: false,
            },
        }
    }

    #[test]
    fn unknown_send_retry_preserves_id_and_original_text() {
        let mut first = PendingSend::prepare(None, "first payload");
        first.created_at = "2026-09-28T23:59:00Z".into();
        let retry = PendingSend::prepare(Some(&first), "edited payload");
        assert_eq!(retry.id, first.id);
        assert_eq!(retry.text, "first payload");
        assert_eq!(retry.created_at, "2026-09-28T23:59:00Z");
        assert!(retry.sending);
    }

    #[test]
    fn definitive_rejection_unlocks_editing_but_transport_failure_preserves_retry() {
        assert!(permanent_send_rejection(Some(400)));
        assert!(permanent_send_rejection(Some(422)));
        assert!(!permanent_send_rejection(None));
        assert!(!permanent_send_rejection(Some(503)));
    }

    #[test]
    fn sidebar_always_lists_self_first_without_empty_copy() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.directs.clear();
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let texts: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"Fixture Owner you"), "{texts:?}");
        assert!(texts.contains(&"Invite people"), "{texts:?}");
        assert!(!texts.contains(&"No direct messages yet"));
        let nodes = output.platform_output.accesskit_update.as_ref().unwrap();
        assert!(nodes.nodes.iter().any(|(_, node)| {
            node.label()
                .is_some_and(|label| label == "Fixture Owner you")
        }));
    }

    #[test]
    fn direct_section_follows_channels_and_browse_lives_in_every_member_space_menu() {
        for owner in [true, false] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-desktop"),
            );
            if !owner {
                app.account.as_mut().unwrap().id = "fixture-member".into();
            }
            app.voice.participants.clear();
            app.channel_rosters.clear();
            render(&mut app, &context, vec![]);
            render(&mut app, &context, vec![]);
            context.enable_accesskit();
            let output = render(&mut app, &context, vec![]);
            let nodes = &output
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes;
            let bounds = |label: &str| {
                nodes
                    .iter()
                    .find_map(|(_, node)| {
                        (node.label() == Some(label))
                            .then(|| node.bounds())
                            .flatten()
                    })
                    .unwrap_or_else(|| panic!("missing {label}"))
            };
            let last_channel = bounds("Join voice in #planning");
            let direct_y = text_position(&output, "Direct messages").y;
            let gap = direct_y - last_channel.y1 as f32;
            assert!(
                (0.0..40.0).contains(&gap),
                "DM heading should directly follow the last channel, not be above it or bottom-anchored: {gap}"
            );
            let direct_action = if owner {
                "Invite people"
            } else {
                "New message"
            };
            assert!(bounds("Mute microphone").y0 > bounds(direct_action).y1);
            assert!(!output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Browse channels")));
            let name = format!("{} actions", app.detail.as_ref().unwrap().space.name);
            let menu = bounds(&name);
            click(
                &mut app,
                &context,
                egui::pos2(
                    ((menu.x0 + menu.x1) / 2.0) as f32,
                    ((menu.y0 + menu.y1) / 2.0) as f32,
                ),
            );
            let opened = render(&mut app, &context, vec![]);
            click(
                &mut app,
                &context,
                text_position(&opened, "Browse channels"),
            );
            assert!(
                app.browse_channels,
                "Browse must be available to owners and members"
            );
        }
    }

    #[test]
    fn direct_rows_are_compact_without_shrinking_channels_or_the_dock() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-direct"),
        );
        app.directs[0].peer.avatar_id = Some(31);
        let mut second = app.directs[0].clone();
        second.id = "second-direct".into();
        second.peer.id = "second-peer".into();
        second.peer.display_name = "Second fixture peer".into();
        second.peer.avatar_id = Some(799);
        second.last_seq = "7".into();
        app.directs.push(second);
        render(&mut app, &context, vec![]);
        let started = std::time::Instant::now();
        while context.has_pending_images() {
            assert!(started.elapsed() < std::time::Duration::from_secs(30));
            std::thread::sleep(std::time::Duration::from_millis(10));
            render(&mut app, &context, vec![]);
        }
        render(&mut app, &context, vec![]);
        context.enable_accesskit();
        let output = render(&mut app, &context, vec![]);
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .unwrap()
            .nodes;
        let bounds = |label: &str| {
            nodes
                .iter()
                .find_map(|(_, node)| {
                    (node.label() == Some(label))
                        .then(|| node.bounds())
                        .flatten()
                })
                .unwrap_or_else(|| panic!("missing bounds for {label}"))
        };
        let self_row = bounds("Fixture Owner you");
        let peer = bounds("TEST FIXTURE Maya");
        let second_peer = bounds("Second fixture peer");
        for row in [self_row, peer, second_peer, bounds("Invite people")] {
            assert_eq!(row.height(), 28.0);
        }
        assert_eq!(peer.y0 - self_row.y1, 2.0);
        assert_eq!(second_peer.y0 - peer.y1, 2.0);
        assert_eq!(bounds("general").height(), 32.0);
        let images: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.brush.is_some() => Some(rect.rect),
                _ => None,
            })
            .collect();
        for row in [self_row, peer, second_peer] {
            assert!(
                images.iter().any(|rect| (rect.width() - 20.0).abs() < 0.01
                    && (rect.height() - 20.0).abs() < 0.01
                    // Images snap to whole pixels, so a centre drawn at
                    // +17.5 lands up to half a pixel away.
                    && (rect.center().x as f64 - (row.x0 + 17.5)).abs() <= 0.5
                    && (rect.center().y as f64 - (row.y0 + row.y1) / 2.0).abs() <= 0.5),
                "Each DM uses saved avatar artwork, aligned with self-notes in {row:?}: {images:?}"
            );
        }
        let microphone = bounds("Mute microphone");
        assert!(
            images.iter().any(|rect| (rect.width() - 30.0).abs() < 0.01
                && (rect.height() - 30.0).abs() < 0.01
                && rect.left() as f64 >= self_row.x0
                && (rect.center().y as f64 - (microphone.y0 + microphone.y1) / 2.0).abs() < 0.5),
            "the account dock keeps its larger avatar"
        );
    }

    #[test]
    fn direct_heading_plus_is_hidden_until_heading_hover_or_keyboard_focus() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        // Bottom panels need two layout passes before reading the prior widget bounds.
        render(&mut app, &context, vec![]);
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let id = egui::Id::new("direct-heading-plus");
        let button = context
            .read_response(id)
            .expect("hidden button remains focusable")
            .rect;
        let painted = |output: &egui::FullOutput| {
            output.shapes.iter().any(|shape|
            matches!(&shape.shape, egui::Shape::Mesh(mesh) if button.contains_rect(mesh.calc_bounds())))
        };
        assert!(!painted(&output), "plus must not paint before hover");
        let heading = text_position(&output, "Direct messages");
        render(&mut app, &context, vec![egui::Event::PointerMoved(heading)]);
        let hovered = render(&mut app, &context, vec![]);
        assert!(
            painted(&hovered),
            "hovering the title {heading:?}, not only the plus {button:?}, reveals it"
        );
        let outside = render(
            &mut app,
            &context,
            vec![egui::Event::PointerMoved(egui::pos2(700.0, 400.0))],
        );
        assert!(!painted(&outside), "plus hides after leaving the heading");
        context.memory_mut(|memory| memory.request_focus(id));
        assert!(
            painted(&render(&mut app, &context, vec![])),
            "keyboard focus reveals the plus"
        );
        render(
            &mut app,
            &context,
            vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(matches!(app.dialog, Some(Dialog::StartDirect)));
    }

    #[test]
    fn self_direct_uses_own_username_then_reuses_peer_id_without_duplicates() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.directs.clear();
        assert_eq!(
            app.self_direct_target(),
            Some(SelfDirectTarget::Create("fixture_owner".into()))
        );
        let account = app.account.as_ref().unwrap().clone();
        let self_direct = model::DirectConversation {
            id: "self-notes".into(),
            peer: model::DirectPeer {
                id: account.id,
                username: account.username.unwrap(),
                display_name: account.display_name.unwrap(),
                avatar_id: None,
            },
            last_seq: "3".into(),
            read_seq: "2".into(),
            status: model::DirectStatus::Accepted,
            blocked: false,
        };
        app.directs = vec![self_direct.clone()];
        assert_eq!(
            app.self_direct_target(),
            Some(SelfDirectTarget::Existing(self_direct.clone()))
        );
        app.select_or_create_self_direct();
        assert_eq!(app.selected_direct.as_deref(), Some("self-notes"));
        assert_eq!(app.directs, vec![self_direct]);
    }

    #[test]
    fn late_self_creation_does_not_override_clicking_the_displayed_channel() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        let (events, receiver) = std::sync::mpsc::channel();
        app.worker.events = receiver;
        app.token = Some("fixture-owner-token".into());
        let selected = app.selected_channel.clone().unwrap();
        app.draft = "keep this channel draft".into();
        app.select_or_create_self_direct();
        assert!(app.loading);
        let creation = app.navigation;
        app.select_channel(selected.clone(), false);
        assert!(app.navigation > creation);
        let account = app.account.as_ref().unwrap();
        let notes = model::DirectConversation {
            id: "self-notes".into(),
            peer: model::DirectPeer {
                id: account.id.clone(),
                username: account.username.clone().unwrap(),
                display_name: account.display_name.clone().unwrap(),
                avatar_id: None,
            },
            last_seq: "0".into(),
            read_seq: "0".into(),
            status: model::DirectStatus::Accepted,
            blocked: false,
        };
        events
            .send(crate::worker::Event::DirectCreated {
                generation: app.generation,
                navigation: creation,
                result: Ok(notes.clone()),
            })
            .unwrap();
        app.receive();
        assert_eq!(app.selected_channel.as_deref(), Some(selected.as_str()));
        assert!(app.selected_direct.is_none());
        assert_eq!(app.draft, "keep this channel draft");
        assert!(!app.loading);
        assert_eq!(app.directs, vec![notes]);
    }

    #[test]
    fn lower_sidebar_action_routes_owner_and_member_or_no_spaces() {
        let context = egui::Context::default();
        let api = || crate::api::Api::new("http://127.0.0.1:9").unwrap();
        let mut owner = CaperApp::new(&context, api(), Some("parity-desktop"));
        let expected_members = owner.detail.as_ref().unwrap().members.len();
        owner.open_direct_action();
        assert!(matches!(owner.dialog, Some(Dialog::ManageSpace)));
        assert_eq!(owner.managed_members.len(), expected_members);

        let mut member = CaperApp::new(&context, api(), Some("parity-desktop"));
        member.account.as_mut().unwrap().id = "fixture-maya".into();
        member.open_direct_action();
        assert!(matches!(member.dialog, Some(Dialog::StartDirect)));

        let mut no_spaces = CaperApp::new(&context, api(), Some("parity-direct-no-spaces"));
        no_spaces.open_direct_action();
        assert!(matches!(no_spaces.dialog, Some(Dialog::StartDirect)));
    }

    #[test]
    fn lower_sidebar_separators_span_the_full_sidebar_width() {
        for width in [220.0, 337.0] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-voice-connected"),
            );
            app.sidebar_width = width;
            render(&mut app, &context, vec![]);
            let output = render(&mut app, &context, vec![]);
            // The rail occupies 59px, followed by the channel sidebar and a 1px divider.
            let dividers: Vec<_> = output.shapes.iter().filter(|shape| matches!(
                &shape.shape,
                egui::Shape::LineSegment { points, stroke }
                    if stroke.color == super::BORDER && points[0].x == 59.0 && points[1].x == 59.0 + width
            )).collect();
            assert!(
                dividers.len() >= 3,
                "header, DM, and voice separators must span {width}px; found {dividers:?}"
            );
            assert!(
                dividers.iter().all(|shape| shape.clip_rect.left() <= 59.0
                    && shape.clip_rect.right() >= 59.0 + width),
                "full-width lines must not be clipped to the padded content: {dividers:?}"
            );
        }
    }

    #[test]
    fn voice_dock_has_balanced_padding_and_centered_icons() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-voice-connected"),
        );
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        let text_rect = |label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == label => {
                        Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                    }
                    _ => None,
                })
                .unwrap()
        };
        let status = text_rect("Voice connected").union(text_rect("general / Fixture Studio"));
        let icons: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh) => {
                    let rect = mesh.calc_bounds();
                    (rect.left() >= 71.0
                        && rect.right() <= 327.0
                        && rect.center().y >= status.top()
                        && rect.center().y <= status.bottom())
                    .then_some(rect)
                }
                _ => None,
            })
            .collect();
        assert_eq!(icons.len(), 2, "audio and disconnect icons: {icons:?}");
        for icon in &icons {
            assert!(
                (icon.center().y - status.center().y).abs() <= 1.0,
                "icon {icon:?} not centered with {status:?}"
            );
        }
        let divider = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::LineSegment { points, stroke }
                    if stroke.color == super::BORDER
                        && points[0].x == 59.0
                        && points[0].y < status.top() =>
                {
                    Some(points[0].y)
                }
                _ => None,
            })
            .max_by(f32::total_cmp)
            .unwrap();
        let account_top = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if rect.rect.left() == 71.0
                        && rect.rect.top() > status.bottom()
                        && rect.rect.height() == 42.0 =>
                {
                    Some(rect.rect.top())
                }
                _ => None,
            })
            .unwrap();
        assert!(
            (status.top() - divider - (account_top - status.bottom())).abs() <= 1.0,
            "unequal voice padding: top {}, bottom {}",
            status.top() - divider,
            account_top - status.bottom()
        );
    }

    fn text_position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == label => {
                    Some(text.pos + egui::vec2(4.0, 4.0))
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing {label}"))
    }

    #[test]
    fn rejected_message_requires_explicit_edit_or_dismiss_and_protects_new_draft() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.session = Some(session());
        app.draft = "rejected original".into();
        app.send_message();
        assert!(app.draft.is_empty());
        let original_id = app.pending.as_ref().unwrap().id.clone();
        app.sent(Err(crate::worker::SendFailure {
            status: Some(422),
            message: "Rejected fixture".into(),
            code: None,
        }));
        app.draft = "next unsent draft".into();
        app.send_message();
        assert_eq!(app.pending.as_ref().unwrap().id, original_id);
        assert!(
            !app.pending.as_ref().unwrap().sending,
            "rejected IDs cannot be retried"
        );
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        click(&mut app, &context, text_position(&output, "Edit"));
        assert_eq!(
            app.draft, "next unsent draft",
            "Edit must not overwrite new text"
        );
        assert!(app.pending.is_some());
        click(&mut app, &context, text_position(&output, "Dismiss"));
        assert!(app.pending.is_none());
        assert_eq!(app.draft, "next unsent draft");

        app.draft = "rejected original".into();
        app.send_message();
        let second_id = app.pending.as_ref().unwrap().id.clone();
        app.sent(Err(crate::worker::SendFailure {
            status: Some(400),
            message: "Rejected again".into(),
            code: None,
        }));
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        click(&mut app, &context, text_position(&output, "Edit"));
        assert_eq!(app.draft, "rejected original");
        assert!(app.pending.is_none());
        assert!(context.memory(|memory| memory.has_focus(egui::Id::new("message-composer"))));
        app.send_message();
        assert_ne!(app.pending.as_ref().unwrap().id, second_id);
    }

    #[test]
    fn both_confirmation_paths_preserve_the_next_draft() {
        for via_gateway in [false, true] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-desktop"),
            );
            app.session = Some(session());
            app.draft = "sent original".into();
            app.send_message();
            assert!(app.draft.is_empty());
            app.draft = "different new draft".into();
            let mut message = app.timeline.messages().last().unwrap().clone();
            message.seq = (message.seq.parse::<u64>().unwrap() + 1).to_string();
            message.id = "confirmation".into();
            message.client_message_id = app.pending.as_ref().unwrap().id.clone();
            message.content.text = "sent original".into();
            message.author = app.session.as_ref().unwrap().author.clone();
            if via_gateway {
                app.gateway(crate::gateway::GatewayEvent::Message {
                    generation: app.generation,
                    channel: message.channel_id.clone(),
                    message: Box::new(message),
                });
            } else {
                app.sent(Ok(message));
            }
            assert!(app.pending.is_none());
            assert_eq!(app.draft, "different new draft");
        }
    }

    #[test]
    fn profile_normalizes_username_and_blocks_incomplete_submission() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-profile"),
        );
        app.username = "A. B_1@".into();
        render(&mut app, &context, vec![]);
        assert_eq!(app.username, "ab_1");
        for (username, name) in [("ab", "Valid name"), ("abc", "   ")] {
            app.username = username.into();
            app.display_name = name.into();
            let output = render(&mut app, &context, vec![]);
            click(&mut app, &context, text_position(&output, "Save profile"));
            assert!(!app.loading, "incomplete profile must not be submitted");
        }
        assert_eq!(
            super::normalize_username(&"Z_2".repeat(12)),
            "z_2z_2z_2z_2z_2z_2z_2z_2z_2z_2z_"
        );
        app.username = "abc".into();
        app.display_name = "Valid name".into();
        let output = render(&mut app, &context, vec![]);
        click(&mut app, &context, text_position(&output, "Save profile"));
        assert!(app.loading);
    }

    #[test]
    fn sidebar_keyboard_and_persistence_preserve_width_without_fixture_leaks() {
        #[derive(Default)]
        struct Storage(std::collections::BTreeMap<String, String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn flush(&mut self) {}
        }
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        render(&mut app, &context, vec![]);
        context.memory_mut(|memory| memory.request_focus(egui::Id::new("sidebar-resize")));
        render(&mut app, &context, vec![]);
        render(&mut app, &context, vec![]);
        for (key, width) in [
            (egui::Key::ArrowRight, 290.0),
            (egui::Key::End, 440.0),
            (egui::Key::ArrowRight, 440.0),
            (egui::Key::Home, 220.0),
            (egui::Key::ArrowLeft, 220.0),
        ] {
            render(
                &mut app,
                &context,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert_eq!(app.sidebar_width, width);
            render(
                &mut app,
                &context,
                vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: false,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        }
        let mut storage = Storage::default();
        eframe::App::save(&mut app, &mut storage);
        assert!(
            storage.0.is_empty(),
            "fixtures must not overwrite real preferences"
        );
        app.persist_preferences = true;
        app.sidebar_width = 337.0;
        app.sound_effects = false;
        eframe::App::save(&mut app, &mut storage);
        app.sidebar_width = 280.0;
        app.sound_effects = true;
        app.restore_preferences(&storage);
        assert_eq!(app.sidebar_width, 337.0);
        assert!(!app.sound_effects);
        app.sound_effects = true;
        eframe::App::save(&mut app, &mut storage);
        app.sound_effects = false;
        app.restore_preferences(&storage);
        assert!(app.sound_effects);
        for invalid in ["NaN", "inf", "219", "441"] {
            app.sidebar_width = 280.0;
            storage.0.insert("sidebar-width-v1".into(), invalid.into());
            app.restore_preferences(&storage);
            assert_eq!(app.sidebar_width, 280.0);
        }
    }

    #[test]
    fn channel_names_match_web_normalization() {
        assert_eq!(normalize_channel("  Product---Launch! "), "product-launch-");
        assert_eq!(normalize_channel("RUST  DESKTOP"), "rust-desktop");
    }

    #[test]
    fn fixture_mode_cannot_target_a_remote_api() {
        let no_override = vec!["caper-desktop".into()];
        assert_eq!(
            endpoint(&no_override, Some("parity")).unwrap(),
            "http://127.0.0.1:3001"
        );
        assert_eq!(endpoint(&no_override, None).unwrap(), "https://caper.chat");

        let loopback = vec![
            "caper-desktop".into(),
            "--api-url".into(),
            "http://localhost:3001".into(),
        ];
        assert!(endpoint(&loopback, Some("parity")).is_ok());

        let remote = vec![
            "caper-desktop".into(),
            "--api-url".into(),
            "https://caper.chat".into(),
        ];
        assert_eq!(
            endpoint(&remote, Some("login")).unwrap_err(),
            "--fixture refuses non-loopback API endpoints."
        );
    }

    #[test]
    fn signed_out_startup_and_restore_failure_show_login_without_workspace() {
        for result in [Ok(None), Err("Session restoration unavailable.".into())] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("signed-out"),
            );
            let (sender, events) = std::sync::mpsc::channel();
            app.worker.events = events;
            app.loading = true;
            render(&mut app, &context, vec![]);
            let loading = render(&mut app, &context, vec![]);
            text_position(&loading, "Welcome to Caper");
            text_position(&loading, "Email address");

            let error = result.as_ref().err().cloned();
            sender
                .send(crate::worker::Event::Restored {
                    generation: app.generation,
                    result,
                })
                .unwrap();
            app.receive();
            assert!(!app.loading);
            let output = render(&mut app, &context, vec![]);
            text_position(&output, "Welcome to Caper");
            text_position(&output, "Email me a code");
            text_position(
                &output,
                "Use your email to create an account or return to one. We’ll send a code to your email.",
            );
            if let Some(error) = error {
                text_position(&output, &error);
            }
            assert!(!output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text)
                    if ["Guest", "Channels", "Message #general", "WELCOME TO CAPER", "Come on in."].contains(&text.galley.job.text.as_str())
                        || text.galley.job.text.contains("No password needed"))
            }));
        }
    }

    #[test]
    fn logout_returns_to_email_entry_instead_of_an_old_challenge_or_guest_shell() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.challenge = Some("previous-challenge".into());
        app.code = "ABC234".into();
        app.attempts_remaining = Some(0);
        app.loading = true;
        app.logout();
        assert!(app.challenge.is_none());
        assert!(app.code.is_empty());
        assert!(app.attempts_remaining.is_none());
        assert!(!app.loading);
        assert!(app.account.is_none());
        assert!(app.selected_channel.is_none());
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        text_position(&output, "Welcome to Caper");
        text_position(&output, "Email me a code");
    }

    #[test]
    fn login_spacing_does_not_add_automatic_gaps_to_explicit_spacers() {
        for (width, verifying) in [
            (1440.0, false),
            (1440.0, true),
            (390.0, false),
            (390.0, true),
        ] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("signed-out"),
            );
            app.email = "fixture@example.test".into();
            if verifying {
                app.challenge = Some("fixture-challenge".into());
                app.code = "ABC234".into();
            }
            let mut frame = || {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 844.0),
                        )),
                        ..Default::default()
                    },
                    |context| app.page(context),
                )
            };
            frame();
            let output = frame();
            let text_rect = |label: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.job.text == label => {
                            Some(egui::Rect::from_min_size(text.pos, text.galley.size()))
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("missing {label}"))
            };
            let heading = text_rect(if verifying {
                "Check your email."
            } else {
                "Welcome to Caper"
            });
            let description = text_rect(if verifying {
                "Enter the six-character code sent to fixture@example.test. It expires in 10 minutes."
            } else {
                "Use your email to create an account or return to one. We’ll send a code to your email."
            });
            assert_eq!(description.top() - heading.bottom(), 4.0);
            let control_rect = |height: f32| {
                output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if rect.rect.width() > 80.0 && rect.rect.height() == height =>
                        {
                            Some(rect.rect)
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("missing {height}px control"))
            };
            assert_eq!(control_rect(58.0).top() - control_rect(52.0).bottom(), 12.0);
            if !verifying {
                assert_eq!(control_rect(58.0).right(), control_rect(52.0).right());
                assert!(control_rect(58.0).width() < control_rect(52.0).width());
            }
            for rect in [heading, description, control_rect(52.0), control_rect(58.0)] {
                assert!(
                    rect.left() >= 8.0 && rect.right() <= width - 8.0,
                    "clipped: {rect:?}"
                );
            }
        }
    }

    #[test]
    fn verification_requires_profile_and_empty_account_stays_without_chat() {
        let context = eframe::egui::Context::default();
        let api = crate::api::Api::new("http://127.0.0.1:9").unwrap();
        let mut app = CaperApp::new(&context, api, Some("signed-out"));
        app.session = Some(session());
        app.draft = "guest draft".into();
        app.selected_space = Some("stale-space".into());
        app.selected_channel = Some("stale-channel".into());

        app.establish("account-token".into(), account(false), spaces());
        assert!(matches!(app.dialog, Some(Dialog::Profile)));
        assert_eq!(
            app.selected_channel, None,
            "onboarding must not open guest chat"
        );
        render(&mut app, &context, vec![]);
        text_position(
            &render(&mut app, &context, vec![]),
            "Choose how you show up.",
        );

        app.profiled(
            account(true),
            Spaces {
                spaces: vec![],
                invitations: Vec::new(),
                limits: Some(crate::model::SpaceLimits {
                    owned_spaces: 5,
                    total_spaces: 20,
                    channels_per_space: 20,
                }),
            },
        );
        assert!(app.dialog.is_none());
        assert!(app.selected_space.is_none());
        assert!(app.selected_channel.is_none());
        assert!(app.session.is_none(), "guest capability must be discarded");
        assert!(
            app.draft.is_empty(),
            "guest draft must not cross auth transition"
        );
        render(&mut app, &context, vec![]);
        text_position(&render(&mut app, &context, vec![]), "Name your space");
    }

    #[test]
    fn established_account_opens_its_first_space() {
        let context = eframe::egui::Context::default();
        let api = crate::api::Api::new("http://127.0.0.1:9").unwrap();
        let mut app = CaperApp::new(&context, api, Some("signed-out"));
        let first = Space {
            id: "first-account-space".into(),
            name: "First".into(),
            owner_id: "account".into(),
            inviter: None,
            demo: false,
        };

        app.establish(
            "account-token".into(),
            account(true),
            Spaces {
                spaces: vec![first.clone()],
                invitations: Vec::new(),
                limits: None,
            },
        );

        assert!(app.opening);
        assert_eq!(
            app.navigation_target
                .as_ref()
                .and_then(|target| target.space.as_ref()),
            Some(&first.id)
        );
        assert!(app.selected_channel.is_none());
    }

    #[test]
    fn older_failure_keeps_conversation_and_send_error_and_retries_separately() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        let channel = app.selected_channel.clone().unwrap();
        let count = app.timeline.messages().count();
        app.has_more = true;
        app.error = Some("Send failed".into());
        app.draft = "Keep my draft".into();
        app.accept_older(
            &channel,
            Err(LoadError {
                message: "History unavailable".into(),
                access_denied: false,
                space_access_denied: false,
            }),
        );
        assert_eq!(app.older_error.as_deref(), Some("History unavailable"));
        assert_eq!(app.error.as_deref(), Some("Send failed"));
        assert_eq!(app.timeline.messages().count(), count);
        assert_eq!(app.draft, "Keep my draft");
        app.load_older();
        assert!(app.loading_older);
        assert!(app.older_error.is_none());
        app.older_error = Some("Sentinel".into());
        app.load_older();
        assert_eq!(
            app.older_error.as_deref(),
            Some("Sentinel"),
            "duplicate request must be ignored"
        );
        app.reload_selected_channel("other".into(), false);
        assert!(!app.loading_older);
        assert!(app.older_error.is_none());
        assert!(!app.has_more);
    }

    #[test]
    fn mismatched_channel_history_and_revoked_pagination_clear_visible_data() {
        let context = eframe::egui::Context::default();
        let api = crate::api::Api::new("http://127.0.0.1:9").unwrap();
        let mut app = CaperApp::new(&context, api, Some("signed-out"));
        app.selected_channel = Some("requested".into());
        app.session = Some(session());
        app.draft = "private draft".into();

        app.accept_channel(history("different"), Ok(session()), false, "requested");
        assert!(app.selected_channel.is_none());
        assert!(app.session.is_none());
        assert!(app.draft.is_empty());
        assert_eq!(
            app.error.as_deref(),
            Some("Caper returned history for another channel.")
        );

        app.selected_channel = Some("requested".into());
        app.session = Some(session());
        app.draft = "revoked draft".into();
        app.accept_older(
            "requested",
            Err(LoadError {
                message: "Channel access denied.".into(),
                access_denied: true,
                space_access_denied: false,
            }),
        );
        assert!(app.selected_channel.is_none());
        assert!(app.session.is_none());
        assert!(app.draft.is_empty());
        assert_eq!(app.error.as_deref(), Some("Channel access denied."));
    }

    #[test]
    fn member_presence_pages_use_current_web_page_size() {
        let members = (0..26)
            .map(|index| Member {
                id: format!("member-{index}"),
                avatar_id: Some(index),
                username: format!("member_{index}"),
                display_name: format!("Member {index}"),
                owner: index == 0,
            })
            .collect();
        let detail = SpaceDetail {
            space: Space {
                id: "space".into(),
                name: "Space".into(),
                owner_id: "member-0".into(),
                inviter: None,
                demo: false,
            },
            channels: Vec::new(),
            members,
            channel_invitations: Vec::new(),
        };
        assert_eq!(member_page_ids(&detail, 0).len(), 25);
        assert_eq!(member_page_ids(&detail, 1), ["member-25"]);
    }

    #[test]
    fn matching_gateway_event_confirms_pending_send() {
        let pending = PendingSend::prepare(None, "delivered despite lost HTTP response");
        let message = Message {
            id: "server-id".into(),
            channel_id: "channel".into(),
            seq: "8".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            client_message_id: pending.id.clone(),
            author: Author {
                id: "author".into(),
                avatar_id: None,
                name: "Member".into(),
                is_guest: false,
            },
            content: Content {
                version: 1,
                kind: "text".into(),
                text: pending.text.clone(),
                mentions: Vec::new(),
            },
            reactions: Vec::new(),
            reaction_seq: None,
            pin: None,
            pin_seq: None,
            thread_root_id: None,
            broadcast: false,
            thread: None,
            forward: None,
            forward_seq: None,
            revision: 1,
            edited_at: None,
            edit_seq: None,
        };
        assert!(pending.confirmed_by(&message, "author"));
        assert!(!pending.confirmed_by(&message, "another-author"));
    }

    #[test]
    fn typing_tombstones_reject_stale_events_and_clear_on_message_or_disconnect() {
        use std::time::{Duration, Instant};

        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("signed-out"),
        );
        app.selected_channel = Some("channel".into());
        app.session = Some(ChatSession {
            token: "token".into(),
            author: Author {
                id: "self".into(),
                avatar_id: None,
                name: "Self".into(),
                is_guest: false,
            },
        });
        let author = Author {
            id: "other".into(),
            avatar_id: None,
            name: "Other".into(),
            is_guest: false,
        };
        let typing = |author: Author, typing, revision: &str| GatewayEvent::Typing {
            generation: 1,
            channel: "channel".into(),
            author,
            typing,
            revision: revision.into(),
        };

        app.gateway(typing(author.clone(), true, "4"));
        app.gateway(typing(author.clone(), false, "6"));
        app.gateway(typing(author.clone(), true, "5"));
        app.gateway(typing(author.clone(), false, "6"));
        let tombstone = app.typers.get("other").unwrap();
        assert!(!tombstone.typing);
        assert_eq!(tombstone.revision, 6);
        assert!(app.typers.values().filter(|entry| entry.typing).count() == 0);

        app.gateway(typing(
            app.session.as_ref().unwrap().author.clone(),
            true,
            "7",
        ));
        assert!(!app.typers.contains_key("self"), "own typing stays hidden");

        app.typers.get_mut("other").unwrap().expires = Instant::now() - Duration::from_millis(1);
        app.periodic(&context);
        assert!(!app.typers.contains_key("other"), "stop tombstones expire");

        app.gateway(typing(author.clone(), true, "8"));
        app.gateway(GatewayEvent::Message {
            generation: 1,
            channel: "channel".into(),
            message: Box::new(Message {
                id: "message".into(),
                channel_id: "channel".into(),
                seq: "1".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                client_message_id: "client-message".into(),
                author: author.clone(),
                content: Content {
                    version: 1,
                    kind: "text".into(),
                    text: "sent".into(),
                    mentions: Vec::new(),
                },
                reactions: Vec::new(),
                reaction_seq: None,
                pin: None,
                pin_seq: None,
                thread_root_id: None,
                broadcast: false,
                thread: None,
                forward: None,
                forward_seq: None,
                revision: 1,
                edited_at: None,
                edit_seq: None,
            }),
        });
        assert!(!app.typers.get("other").unwrap().typing);

        app.gateway(typing(author, true, "9"));
        app.gateway(GatewayEvent::Status {
            generation: 1,
            channel: "channel".into(),
            online: false,
            detail: "Offline".into(),
        });
        assert!(app.typers.is_empty());
    }

    #[test]
    fn typing_stop_tombstones_are_not_rendered() {
        for stopped in [false, true] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-typing"),
            );
            if stopped {
                let author = app.typers.values().next().unwrap().author.clone();
                app.gateway(GatewayEvent::Typing {
                    generation: app.generation,
                    channel: app.selected_channel.clone().unwrap(),
                    author,
                    typing: false,
                    revision: "2".into(),
                });
                assert_eq!(app.typers.len(), 1, "the stop revision stays retained");
            }
            render(&mut app, &context, vec![]);
            let output = render(&mut app, &context, vec![]);
            let visible = output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text == "Maya is typing…")
            });
            assert_eq!(visible, !stopped);
        }
    }

    #[test]
    fn navigation_retains_conversation_and_rejects_stale_completions() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        let original = app.selected_channel.clone();
        app.selected_direct = original.clone();
        let count = app.timeline.messages().count();
        app.draft = "Unsent draft".into();
        app.select_channel("next".into(), false);
        let old_request = app.navigation;
        assert_eq!(app.selected_channel, original);
        assert_eq!(app.selected_direct, original);
        assert_eq!(app.timeline.messages().count(), count);
        assert_eq!(app.draft, "Unsent draft");
        app.accept_navigation(
            app.generation,
            old_request,
            Err(LoadError {
                message: "Unavailable".into(),
                access_denied: false,
                space_access_denied: false,
            }),
        );
        assert_eq!(app.navigation_error.as_deref(), Some("Unavailable"));
        assert_eq!(app.selected_channel, original);
        assert_eq!(app.selected_direct, original);
        assert_eq!(app.draft, "Unsent draft");
        app.navigate(app.navigation_target.clone().unwrap());
        assert!(app.opening);
        app.accept_navigation(
            app.generation,
            old_request,
            Ok(crate::worker::PreparedNavigation {
                detail: None,
                conversation: Some((history("stale"), Ok(session()))),
            }),
        );
        assert!(app.opening);
        assert_eq!(app.selected_channel, original);
        app.accept_navigation(
            app.generation,
            app.navigation,
            Ok(crate::worker::PreparedNavigation {
                detail: None,
                conversation: Some((history("next"), Ok(session()))),
            }),
        );
        assert_eq!(app.selected_channel.as_deref(), Some("next"));
        assert!(app.selected_direct.is_none());
        assert!(!app.opening);
        assert!(app.navigation_error.is_none());
        assert!(app.draft.is_empty());
    }

    #[test]
    fn navigation_loading_and_retry_do_not_shift_the_shell_or_consume_scroll_state() {
        for size in [egui::vec2(1440.0, 900.0), egui::vec2(390.0, 844.0)] {
            let context = egui::Context::default();
            context.enable_accesskit();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-desktop"),
            );
            app.draft = "Unsent draft".into();
            let frame = |app: &mut CaperApp, events| {
                context.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |context| app.page(context),
                )
            };
            frame(&mut app, vec![]);
            let ready = frame(&mut app, vec![]);
            let labels = if size.x > 760.0 {
                vec![
                    "Fixture Studio",
                    "Channels",
                    "# general",
                    "Unsent draft",
                    "Members",
                ]
            } else {
                vec!["# general", "Unsent draft"]
            };
            let positions: Vec<_> = labels
                .iter()
                .map(|label| text_position(&ready, label))
                .collect();
            let selected = app.selected_channel.clone().unwrap();
            let target = navigation::Target {
                space: app.selected_space.clone(),
                channel: Some("chan00000002".into()),
            };
            app.navigation_cache.begin_prefetch(target, Instant::now());
            app.select_channel("chan00000002".into(), false);
            app.history_offset = Some(63.0);
            app.older_anchor = Some(123.0);
            app.history_height = 987.0;
            app.older_armed = false;
            app.has_more = true;
            let opening = frame(&mut app, vec![]);
            assert!(
                opening
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some("Loading messages"))
            );
            assert!(
                opening
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|(_, node)| node.role() == egui::accesskit::Role::MultilineTextInput)
                    .unwrap()
                    .1
                    .is_disabled()
            );
            assert!(!opening.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if ["Opening conversation…", "Loading messages…", "No messages yet.",
                    "TEST FIXTURE — local sample data, not a live conversation."].contains(&text.galley.job.text.as_str()))));
            app.accept_navigation(
                app.generation,
                app.navigation,
                Err(LoadError {
                    message: "Connection unavailable".into(),
                    access_denied: false,
                    space_access_denied: false,
                }),
            );
            let failed = frame(&mut app, vec![]);
            assert!(
                text_position(&failed, "Retry opening").y > text_position(&failed, "# general").y
            );
            for output in [&opening, &failed] {
                for (label, position) in labels.iter().zip(&positions) {
                    assert_eq!(
                        text_position(output, label),
                        *position,
                        "{size:?}: {label} moved"
                    );
                }
            }
            assert_eq!(app.history_offset, Some(63.0));
            assert_eq!(app.older_anchor, Some(123.0));
            assert_eq!(app.history_height, 987.0);
            assert!(!app.older_armed);
            assert!(
                !app.loading_older,
                "hidden history must not load older pages"
            );
            assert_eq!(app.draft, "Unsent draft");
            assert_eq!(app.timeline.messages().count(), 4);
            let retry = text_position(&failed, "Retry opening");
            for pressed in [true, false] {
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(retry),
                        egui::Event::PointerButton {
                            pos: retry,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            assert!(
                app.opening,
                "{size:?}: retry at {retry:?} did not start navigation; error {:?}",
                app.navigation_error
            );
            assert!(app.navigation_error.is_none());
            app.select_channel(selected.clone(), false);
            assert!(!app.opening);
            app.history_offset = None;
            app.older_anchor = None;
            let restored = frame(&mut app, vec![]);
            assert_eq!(app.selected_channel.as_deref(), Some(selected.as_str()));
            assert_eq!(app.draft, "Unsent draft");
            assert!(
                !restored
                    .platform_output
                    .accesskit_update
                    .as_ref()
                    .unwrap()
                    .nodes
                    .iter()
                    .find(|(_, node)| node.role() == egui::accesskit::Role::MultilineTextInput)
                    .unwrap()
                    .1
                    .is_disabled()
            );
            assert!(restored.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.job.text == "TEST FIXTURE — local sample data, not a live conversation.")));
        }
    }

    #[test]
    fn initial_history_uses_skeleton_and_failed_navigation_without_a_channel_can_retry() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-loading"),
        );
        render(&mut app, &context, vec![]);
        let loading = render(&mut app, &context, vec![]);
        assert!(
            loading
                .platform_output
                .accesskit_update
                .as_ref()
                .unwrap()
                .nodes
                .iter()
                .any(|(_, node)| node.label() == Some("Loading messages"))
        );
        assert!(!loading.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if ["Loading messages…", "No messages yet."].contains(&text.galley.job.text.as_str()))));
        app.loading = false;
        let empty = render(&mut app, &context, vec![]);
        text_position(&empty, "No messages yet.");

        app.selected_channel = None;
        app.navigation_target = Some(NavigationTarget {
            space: app.selected_space.clone(),
            channel: Some("chan00000002".into()),
        });
        app.navigation_error = Some("Channel unavailable".into());
        let failed = render(&mut app, &context, vec![]);
        click(&mut app, &context, text_position(&failed, "Retry opening"));
        assert!(app.opening);
        assert!(app.navigation_error.is_none());
        let request = app.navigation;
        app.accept_navigation(
            app.generation,
            request,
            Err(LoadError {
                message: "Channel unavailable".into(),
                access_denied: false,
                space_access_denied: false,
            }),
        );
        let failed = render(&mut app, &context, vec![]);
        click(&mut app, &context, text_position(&failed, "Dismiss"));
        assert!(app.navigation_error.is_none());
        assert!(app.navigation_target.is_none());
        text_position(&render(&mut app, &context, vec![]), "No joined channels");
    }

    #[test]
    fn channel_selection_is_noop_duplicate_and_cancels_stale_target() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        let selected = app.selected_channel.clone().unwrap();
        app.draft = "keep me".into();
        let initial_navigation = app.navigation;
        app.select_channel(selected.clone(), false);
        assert_eq!(app.navigation, initial_navigation);
        assert_eq!(app.draft, "keep me");

        app.select_channel("pending".into(), false);
        let pending_navigation = app.navigation;
        app.select_channel("pending".into(), false);
        assert_eq!(
            app.navigation, pending_navigation,
            "duplicate target is ignored"
        );
        assert!(app.opening);

        app.select_channel(selected, false);
        assert!(!app.opening);
        assert!(app.navigation_target.is_none());
        assert!(app.navigation > pending_navigation);
        assert_eq!(app.draft, "keep me");
        app.accept_navigation(
            app.generation,
            pending_navigation,
            Ok(crate::worker::PreparedNavigation {
                detail: app.detail.clone(),
                conversation: Some((history("pending"), Ok(session()))),
            }),
        );
        assert_ne!(app.selected_channel.as_deref(), Some("pending"));
    }

    #[test]
    fn joining_the_displayed_preview_still_opens_a_participating_session() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.session = None;
        app.session_error = Some("Join this channel to chat.".into());
        let selected = app.selected_channel.clone().unwrap();
        assert!(app.selected_is_joined(), "membership refresh has completed");
        app.select_channel(selected, false);
        assert!(
            app.opening,
            "a preview-to-member transition is not a repeat click"
        );
    }

    #[test]
    fn click_waits_for_inflight_hover_and_consumes_its_history() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        let target = navigation::Target {
            space: app.selected_space.clone(),
            channel: Some("hovered".into()),
        };
        let request = app
            .navigation_cache
            .begin_prefetch(target.clone(), Instant::now())
            .unwrap();
        app.select_channel("hovered".into(), false);
        assert!(app.opening);
        assert_eq!(
            app.navigation_cache.prefetch_state(&target, Instant::now()),
            navigation::PrefetchState::Pending
        );

        assert!(app.navigation_cache.finish_prefetch(
            &target,
            request,
            navigation::Read {
                detail: app.detail.clone(),
                history: Some(history("hovered")),
            },
            Instant::now(),
        ));
        app.periodic(&context);
        assert!(app.navigation_prefetch.is_none());
        assert_eq!(
            app.navigation_cache.prefetch_state(&target, Instant::now()),
            navigation::PrefetchState::Missing,
            "the click reuses and consumes the completed hover read"
        );
    }

    #[test]
    fn expired_hover_falls_back_once_without_stalling_navigation() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        let target = navigation::Target {
            space: app.selected_space.clone(),
            channel: Some("slow-hover".into()),
        };
        let now = Instant::now();
        app.navigation_cache.begin_prefetch(target.clone(), now);
        app.select_channel("slow-hover".into(), false);
        let request = app.navigation;
        assert_eq!(app.navigation_prefetch.as_ref(), Some(&target));
        assert_eq!(
            app.navigation_cache
                .prefetch_state(&target, now + Duration::from_secs(5)),
            navigation::PrefetchState::Missing
        );
        app.periodic(&context);
        assert!(
            app.navigation_prefetch.is_none(),
            "normal preparation has started"
        );
        assert!(app.opening);
        app.periodic(&context);
        assert!(app.navigation_prefetch.is_none());
        assert_eq!(
            app.navigation, request,
            "fallback keeps the same navigation attempt"
        );
    }

    #[test]
    fn presence_survives_same_space_navigation_and_clears_across_spaces() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.presence.insert("member".into(), "online".into());
        let detail = app.detail.clone();
        app.select_channel("same-space".into(), false);
        app.accept_navigation(
            app.generation,
            app.navigation,
            Ok(crate::worker::PreparedNavigation {
                detail,
                conversation: Some((history("same-space"), Ok(session()))),
            }),
        );
        assert_eq!(
            app.presence.get("member").map(String::as_str),
            Some("online")
        );

        let mut other_detail = app.detail.clone().unwrap();
        other_detail.space.id = "other-space".into();
        let mut other_history = history("other-channel");
        other_history.space.id = "other-space".into();
        app.navigate(NavigationTarget {
            space: Some("other-space".into()),
            channel: Some("other-channel".into()),
        });
        app.accept_navigation(
            app.generation,
            app.navigation,
            Ok(crate::worker::PreparedNavigation {
                detail: Some(other_detail),
                conversation: Some((other_history, Ok(session()))),
            }),
        );
        assert!(app.presence.is_empty());
    }

    #[test]
    fn direct_read_cursor_requires_foreground_and_never_regresses() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.selected_direct = app.selected_channel.clone();
        app.token = Some("fixture-only-token".into());
        app.directs = vec![crate::model::DirectConversation {
            id: app.selected_channel.clone().unwrap(),
            peer: crate::model::DirectPeer {
                id: "peer".into(),
                username: "peer".into(),
                display_name: "TEST FIXTURE peer".into(),
                avatar_id: None,
            },
            last_seq: "9".into(),
            read_seq: "1".into(),
            status: model::DirectStatus::Accepted,
            blocked: false,
        }];
        app.mark_selected_direct_read();
        assert_eq!(
            app.directs[0].read_seq, "1",
            "background arrival is not a read"
        );
        app.foreground = true;
        app.mark_selected_direct_read();
        assert_eq!(
            app.directs[0].read_seq, "4",
            "only the retained timeline is read"
        );
        let channel = app.selected_channel.clone().unwrap();
        app.gateway(GatewayEvent::Reactions {
            generation: app.generation,
            channel: channel.clone(),
            update: model::ReactionUpdate {
                kind: "message.reactions".into(),
                schema_version: 1,
                channel_id: channel,
                seq: "5".into(),
                message_id: "fixture-message-1".into(),
                reactions: vec![model::Reaction {
                    emoji: "👍".into(),
                    author_ids: vec!["fixture-maya".into()],
                }],
            },
        });
        assert_eq!(
            app.directs[0].read_seq, "5",
            "a foreground reaction event advances the direct read cursor"
        );
        app.directs[0].read_seq = "8".into();
        app.mark_selected_direct_read();
        assert_eq!(
            app.directs[0].read_seq, "8",
            "older retained history cannot undo another device's read"
        );
    }

    #[test]
    fn revoked_channel_fences_inflight_navigation_and_cached_history() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        app.select_channel("next".into(), false);
        let pending = app.navigation;
        app.clear_channel("Access revoked");
        app.accept_navigation(
            app.generation,
            pending,
            Ok(crate::worker::PreparedNavigation {
                detail: None,
                conversation: Some((history("next"), Ok(session()))),
            }),
        );
        assert!(app.selected_channel.is_none());
        assert!(app.session.is_none());
        assert!(!app.opening);
        assert_eq!(app.error.as_deref(), Some("Access revoked"));
    }

    #[test]
    fn navigation_distinguishes_revoked_space_from_another_private_channel() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        let original = app.selected_channel.clone();
        app.draft = "Private draft".into();
        app.select_channel("other-private-channel".into(), false);
        app.accept_navigation(
            app.generation,
            app.navigation,
            Err(LoadError {
                message: "Channel denied".into(),
                access_denied: true,
                space_access_denied: false,
            }),
        );
        assert_eq!(app.selected_channel, original);
        assert_eq!(app.draft, "Private draft");
        assert!(app.detail.is_some());
        app.select_channel("other-private-channel".into(), false);
        app.accept_navigation(
            app.generation,
            app.navigation,
            Err(LoadError {
                message: "Space denied".into(),
                access_denied: true,
                space_access_denied: true,
            }),
        );
        assert!(app.selected_channel.is_none());
        assert!(app.selected_space.is_none());
        assert!(app.detail.is_none());
        assert!(app.draft.is_empty());
        assert_eq!(app.timeline.messages().count(), 0);
    }

    #[test]
    fn refresh_preserves_history_only_when_it_reaches_the_replay_cursor() {
        let context = egui::Context::default();
        // Above JavaScript's safe integer range, but inside the API's i64 contract.
        let base = 9_007_199_254_740_992_u64;
        for (fresh, cursor, old_more, fresh_more, expected, expected_more) in [
            (vec![4], 4, false, true, vec![2, 4, 9], false),
            (vec![5], 5, true, false, vec![2, 4, 5, 9], true),
            (vec![5, 6], 6, true, false, vec![2, 4, 5, 6, 9], true),
            (vec![6], 6, false, true, vec![6], true),
            (vec![2], 2, true, false, vec![2], false),
            (vec![], 0, true, false, vec![], false),
            // The missing sequence may update a reaction on message 2.
            (vec![5], 6, true, false, vec![5], false),
            (vec![4, 5, 7], 7, true, false, vec![4, 5, 7], false),
        ] {
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some("parity-desktop"),
            );
            let template = app.timeline.messages().next().unwrap().clone();
            let channel = app.selected_channel.clone().unwrap();
            let message = |offset: u64| {
                let mut message = template.clone();
                message.id = format!("message-{offset}");
                message.seq = (base + offset).to_string();
                message
            };
            let mut cached = message(4);
            cached.reaction_seq = Some((base + 10).to_string());
            cached.reactions = vec![model::Reaction {
                emoji: "👍".into(),
                author_ids: vec!["newer-actor".into()],
            }];
            app.timeline
                .reset(vec![message(2), cached], &(base + 4).to_string())
                .unwrap();
            // An HTTP confirmation must not bridge the missing replay range.
            app.timeline.merge_sent(message(9)).unwrap();
            app.has_more = old_more;
            app.draft = "unsent draft".into();
            app.reload_channel();
            assert_eq!(
                app.timeline.messages().count(),
                3,
                "refresh must not blank the conversation"
            );
            assert_eq!(app.draft, "unsent draft");
            app.load_older();
            assert!(
                !app.loading_older,
                "pagination waits for the refresh to finish"
            );

            let mut refreshed = history(&channel);
            refreshed.cursor = (base + cursor).to_string();
            refreshed.has_more = fresh_more;
            for offset in &fresh {
                let mut fresh_message = message(*offset);
                fresh_message.author.name = "Refreshed author".into();
                refreshed.messages.push(fresh_message);
            }
            app.accept_channel(refreshed, Ok(session()), false, &channel);
            assert_eq!(
                app.timeline
                    .messages()
                    .map(|message| message.seq.clone())
                    .collect::<Vec<_>>(),
                expected
                    .iter()
                    .map(|offset| (base + offset).to_string())
                    .collect::<Vec<_>>()
            );
            assert_eq!(app.has_more, expected_more);
            for offset in &fresh {
                assert_eq!(
                    app.timeline
                        .messages()
                        .find(|item| item.id == format!("message-{offset}"))
                        .unwrap()
                        .author
                        .name,
                    "Refreshed author"
                );
            }
            if fresh.contains(&4) {
                let retained = app
                    .timeline
                    .messages()
                    .find(|item| item.id == "message-4")
                    .unwrap();
                assert_eq!(retained.reaction_seq, Some((base + 10).to_string()));
                assert_eq!(retained.reactions[0].author_ids, ["newer-actor"]);
            }
            app.clear_channel("Access revoked");
            assert_eq!(app.timeline.messages().count(), 0);
            assert!(app.draft.is_empty());
        }
    }

    #[test]
    fn resync_retains_uncertain_send_but_channel_switch_discards_it() {
        let context = eframe::egui::Context::default();
        let api = crate::api::Api::new("http://127.0.0.1:9").unwrap();
        let mut app = CaperApp::new(&context, api, Some("signed-out"));
        app.selected_channel = Some("one".into());
        let pending = PendingSend::prepare(None, "unconfirmed payload");
        let id = pending.id.clone();
        app.pending = Some(pending);
        app.draft = "different draft".into();
        app.reload_channel();
        let retry = app.pending.as_ref().unwrap();
        assert_eq!(retry.id, id);
        assert_eq!(retry.text, "unconfirmed payload");
        assert!(!retry.sending);
        assert_eq!(app.draft, "different draft");
        app.select_channel("two".into(), false);
        assert_eq!(
            app.draft, "different draft",
            "a pending navigation must not discard text"
        );
        assert!(app.pending.is_some());
        app.accept_navigation(
            app.generation,
            app.navigation,
            Ok(crate::worker::PreparedNavigation {
                detail: None,
                conversation: Some((history("two"), Ok(session()))),
            }),
        );
        assert!(app.pending.is_none());
        assert!(app.draft.is_empty());
    }

    #[test]
    fn message_editor_retains_conflicts_keeps_replay_cursor_and_ignores_obsolete_requests() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-edits-editor"),
        );
        let original = app.message_editor.as_ref().unwrap().original.clone();
        let request = app.message_editor.as_ref().unwrap().request;
        let cursor = app.timeline.cursor();
        app.message_editor.as_mut().unwrap().draft = "Keep my unsaved draft".into();
        app.accept_edit(
            request,
            &original.id,
            false,
            Err("This message changed. Load latest before retrying.".into()),
        );
        assert_eq!(
            app.message_editor.as_ref().unwrap().draft,
            "Keep my unsaved draft"
        );
        let mut latest = original.clone();
        latest.content.text = "A correction made on another device".into();
        latest.revision = 3;
        latest.edit_seq = Some("7".into());
        app.timeline.merge_edit_ack(latest.clone()).unwrap();
        assert_eq!(
            app.message_editor.as_ref().unwrap().original.revision,
            2,
            "Live updates must not silently rebase the draft"
        );
        assert_eq!(app.timeline.cursor(), cursor);
        app.accept_edit(request, &original.id, true, Ok(Box::new(latest.clone())));
        assert_eq!(
            app.message_editor.as_ref().unwrap().draft,
            "A correction made on another device"
        );
        assert_eq!(app.message_editor.as_ref().unwrap().original.revision, 3);
        app.open_editor(&latest);
        app.accept_edit(request, &original.id, false, Ok(Box::new(latest.clone())));
        assert!(
            app.message_editor.is_some(),
            "A late response must not close a reopened editor"
        );
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        assert!(output.shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains("Save changes"))
        }));
        assert_eq!(app.timeline.cursor(), cursor);
        app.session.as_mut().unwrap().author.id = "somebody-else".into();
        app.open_editor(&latest);
        render(&mut app, &context, vec![]);
        assert!(
            app.message_editor.is_none(),
            "Author/account loss closes the obsolete editor"
        );
    }

    #[test]
    fn forwarded_wrapper_author_cannot_open_editor_or_source_history() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-edits-editor"),
        );
        let original = app.message_editor.take().unwrap().original;
        assert!(
            app.can_edit(&original),
            "The actual source author retains editing"
        );
        let mut wrapper = original.clone();
        wrapper.id = "forward-wrapper".into();
        wrapper.forward = Some(Box::new(model::MessageForward {
            message: Some(original),
            seq: "7".into(),
        }));
        assert!(
            !app.can_edit(&wrapper),
            "Wrapper authors cannot edit the shared content or note"
        );
        app.open_editor(&wrapper);
        app.open_edit_history(&wrapper);
        assert!(app.message_editor.is_none());
        assert!(app.edit_history.is_none());
    }

    /// A signed-in `parity-edits` timeline, so rows carry the forward menu.
    fn signed_in_edits(context: &egui::Context) -> (CaperApp, egui::FullOutput) {
        let mut app = CaperApp::new(
            context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-edits"),
        );
        app.token = Some("account-token".into());
        render(&mut app, context, vec![]);
        let output = render(&mut app, context, vec![]);
        (app, output)
    }

    fn text_ending(output: &egui::FullOutput, suffix: &str) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text.ends_with(suffix) => {
                    Some(text.pos + text.galley.rect.center().to_vec2())
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("nothing ending in {suffix} is drawn"))
    }

    #[test]
    fn signed_in_edited_marker_opens_history_beneath_the_forward_menu() {
        let context = egui::Context::default();
        let (mut app, output) = signed_in_edits(&context);
        click(&mut app, &context, text_ending(&output, "(edited)"));
        assert!(app.edit_history.is_some());
    }

    #[test]
    fn right_clicking_signed_in_message_text_offers_forwarding() {
        let context = egui::Context::default();
        let (mut app, output) = signed_in_edits(&context);
        let pos = text_ending(&output, "feel familiar on every platform.");
        for pressed in [true, false] {
            render(
                &mut app,
                &context,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        // New egui areas spend their first pass measuring, invisibly.
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        assert!(texts(&output).contains(&"Forward message"));
    }

    #[test]
    fn message_history_keeps_latest_pair_when_browsing_the_original() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-edits-history"),
        );
        render(&mut app, &context, vec![]);
        render(&mut app, &context, vec![]);
        app.edit_history.as_mut().unwrap().selected = Some(1);
        render(&mut app, &context, vec![]);
        let output = render(&mut app, &context, vec![]);
        for label in [
            "Previous version",
            "Current version",
            "View previous versions",
            "Original version",
        ] {
            assert!(output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.job.text.contains(label))
            }), "Missing {label}");
        }
        let heading_y = |label: &str| {
            output
                .shapes
                .iter()
                .find_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        (text.galley.job.text == label).then_some(text.pos.y)
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| panic!("Missing exact heading {label}"))
        };
        assert!(
            heading_y("Original version") > heading_y("View previous versions"),
            "The selected original must be rendered below the selector, not only in the latest pair"
        );
        let history = app.edit_history.as_ref().unwrap();
        assert_eq!(history.versions[0].revision, 2);
        assert_eq!(history.selected, Some(1));
        let request = history.request;
        let id = history.message.id.clone();
        app.accept_versions(request + 1, &id, Err("Obsolete request".into()));
        assert!(app.edit_history.as_ref().unwrap().error.is_none());
    }
}
