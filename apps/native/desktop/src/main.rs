mod api;
mod credentials;
mod effects;
mod gateway;
#[path = "../voice-spike/src/media.rs"]
mod media;
#[path = "../voice-spike/src/media_gateway.rs"]
mod media_gateway;
mod model;
mod navigation;
#[path = "../voice-spike/src/state.rs"]
mod state;
mod voice;
mod worker;

use effects::{Effect, Effects};
use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};
use gateway::GatewayEvent;
use model::{
    Account, Author, ChatSession, Member, Presence, SpaceDetail, SpaceLimits, Spaces, Timeline,
};
use state::{CallContext, Phase};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};
use voice::{MicrophoneState, Voice, VoiceOperation};
use worker::{AdminOperation, AdminResult, Command, Event, Worker, current};

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
const ERROR: Color32 = Color32::from_rgb(255, 155, 130);
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingSend {
    id: String,
    text: String,
    sending: bool,
    rejection: Option<String>,
}

impl PendingSend {
    fn prepare(previous: Option<&Self>, draft: &str) -> Self {
        previous.map_or_else(
            || Self {
                id: uuid::Uuid::new_v4().to_string(),
                text: draft.into(),
                sending: true,
                rejection: None,
            },
            |pending| Self {
                id: pending.id.clone(),
                text: pending.text.clone(),
                sending: true,
                rejection: None,
            },
        )
    }

    fn confirmed_by(&self, message: &model::Message, author: &str) -> bool {
        self.id == message.client_message_id && message.author.id == author
    }
}

fn permanent_send_rejection(status: Option<u16>) -> bool {
    matches!(status, Some(400 | 404 | 409 | 413 | 422))
}

#[derive(Clone)]
enum Dialog {
    SignIn,
    Profile,
    Audio,
    Connection,
    Diagnostics,
    CreateSpace,
    ManageSpace,
    LeaveSpace {
        id: String,
        name: String,
    },
    ConfirmDelete {
        space: String,
        channel: Option<String>,
        name: String,
    },
    CreateChannel,
    ManageChannel(String),
}

#[derive(Clone)]
struct Typer {
    author: Author,
    revision: u64,
    expires: Instant,
}

#[derive(Clone)]
struct NavigationTarget {
    space: Option<String>,
    channel: Option<String>,
}

struct CaperApp {
    worker: Worker,
    voice: Voice,
    effects: Effects,
    sound_effects: bool,
    announced_voice: Option<u64>,
    generation: u64,
    loading: bool,
    loading_older: bool,
    older_error: Option<String>,
    has_more: bool,
    error: Option<String>,
    warning: Option<String>,
    token: Option<String>,
    account: Option<Account>,
    spaces: Vec<model::Space>,
    limits: Option<SpaceLimits>,
    selected_space: Option<String>,
    detail: Option<SpaceDetail>,
    selected_channel: Option<String>,
    session: Option<ChatSession>,
    timeline: Timeline,
    live: String,
    email: String,
    challenge: Option<String>,
    code: String,
    username: String,
    display_name: String,
    draft: String,
    pending: Option<PendingSend>,
    typers: BTreeMap<String, Typer>,
    typing_sent: bool,
    typing_edited: Instant,
    typing_pulse: Instant,
    presence: BTreeMap<String, String>,
    member_page: usize,
    members_visible: bool,
    narrow_members_visible: bool,
    channels_expanded: bool,
    roster_generation: u64,
    voice_join_request: u64,
    channel_rosters: BTreeMap<String, Vec<model::VoiceOccupant>>,
    unavailable_rosters: BTreeSet<String>,
    collapsed_rosters: BTreeSet<String>,
    sidebar_width: f32,
    navigation_open: bool,
    navigation: u64,
    opening: bool,
    navigation_target: Option<NavigationTarget>,
    navigation_error: Option<String>,
    navigation_cache: navigation::NavigationCache,
    navigation_cache_generation: u64,
    dialog: Option<Dialog>,
    form_name: String,
    form_private: bool,
    member_username: String,
    managed_members: Vec<Member>,
    managed_channel: Option<String>,
    persist_preferences: bool,
}

impl CaperApp {
    fn new(context: &egui::Context, api: api::Api, fixture: Option<&str>) -> Self {
        configure(context);
        let voice = Voice::new(api.base().clone(), context.clone());
        let worker = Worker::new(api, context.clone());
        let now = Instant::now();
        let mut app = Self {
            worker,
            voice,
            effects: Effects::new(fixture.is_none()),
            sound_effects: true,
            announced_voice: None,
            generation: 1,
            loading: fixture.is_none(),
            loading_older: false,
            older_error: None,
            has_more: false,
            error: None,
            warning: None,
            token: None,
            account: None,
            spaces: Vec::new(),
            limits: None,
            selected_space: None,
            detail: None,
            selected_channel: None,
            session: None,
            timeline: Timeline::default(),
            live: "Connecting…".into(),
            email: String::new(),
            challenge: None,
            code: String::new(),
            username: String::new(),
            display_name: String::new(),
            draft: String::new(),
            pending: None,
            typers: BTreeMap::new(),
            typing_sent: false,
            typing_edited: now,
            typing_pulse: now,
            presence: BTreeMap::new(),
            member_page: 0,
            members_visible: true,
            narrow_members_visible: false,
            channels_expanded: true,
            roster_generation: 0,
            voice_join_request: 0,
            channel_rosters: BTreeMap::new(),
            unavailable_rosters: BTreeSet::new(),
            collapsed_rosters: BTreeSet::new(),
            sidebar_width: 280.0,
            navigation_open: false,
            navigation: 0,
            opening: false,
            navigation_target: None,
            navigation_error: None,
            navigation_cache: navigation::NavigationCache::default(),
            navigation_cache_generation: 1,
            dialog: None,
            form_name: String::new(),
            form_private: false,
            member_username: String::new(),
            managed_members: Vec::new(),
            managed_channel: None,
            persist_preferences: fixture.is_none(),
        };
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
                if name == "parity-admin" {
                    app.form_name = "Fixture Studio".into();
                    app.managed_members = app
                        .detail
                        .as_ref()
                        .map_or_else(Vec::new, |detail| detail.members.clone());
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
                } else if name == "parity-browse" {
                    app.navigation_open = true;
                } else if matches!(name, "parity-voice-rosters" | "parity-voice-rosters-narrow") {
                    app.channel_rosters.insert(
                        "chan00000002".into(),
                        vec![
                            model::VoiceOccupant {
                                id: "fixture-maya".into(),
                                name: "Maya".into(),
                                country_code: None,
                                muted: false,
                                deafened: false,
                            },
                            model::VoiceOccupant {
                                id: "fixture-alex".into(),
                                name: "Alex".into(),
                                country_code: None,
                                muted: true,
                                deafened: false,
                            },
                        ],
                    );
                    app.navigation_open = name.ends_with("-narrow");
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
                } else if name == "parity-member" {
                    app.account.as_mut().unwrap().id = "fixture-maya".into();
                    app.account.as_mut().unwrap().display_name = Some("Maya".into());
                } else if name == "parity-audio-debug" {
                    app.account.as_mut().unwrap().debug_enabled = true;
                    app.dialog = Some(Dialog::Diagnostics);
                } else if matches!(name, "parity-audio" | "parity-audio-recorded") {
                    app.dialog = Some(Dialog::Audio);
                    if name == "parity-audio-recorded" {
                        app.voice.microphone = MicrophoneState::Ready(3.6);
                    }
                } else if matches!(name, "parity-voice-joining" | "parity-voice-connected") {
                    // Explicit visual fixtures only: no media transport is started.
                    let call = CallContext {
                        channel_id: "chan00000001".into(),
                        channel_name: "general".into(),
                        space_name: "Fixture Studio".into(),
                    };
                    app.voice.active_space = Some("space0000001".into());
                    app.voice.state.phase = if name == "parity-voice-connected" {
                        app.voice.self_id = "fixture-owner".into();
                        app.voice.participants = app
                            .detail
                            .as_ref()
                            .unwrap()
                            .members
                            .iter()
                            .map(|member| media::Participant {
                                id: member.id.clone(),
                                name: member.display_name.clone(),
                                country_code: None,
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
                            },
                            Instant::now(),
                        ));
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
            demo: true,
        };
        let space = Space {
            id: "space0000001".into(),
            name: "Fixture Studio".into(),
            owner_id: "fixture-owner".into(),
            demo: false,
        };
        let channel = Channel {
            id: "chan00000001".into(),
            space_id: space.id.clone(),
            name: "general".into(),
            private: false,
        };
        self.account = Some(Account {
            id: "fixture-owner".into(),
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
            },
            Channel {
                id: "chan00000003".into(),
                space_id: space.id.clone(),
                name: "planning".into(),
                private: true,
            },
        ];
        self.spaces = vec![demo, space.clone()];
        self.selected_space = Some(space.id.clone());
        self.selected_channel = Some(channel.id.clone());
        self.detail = Some(SpaceDetail {
            space,
            channels,
            members: vec![
                Member {
                    id: "fixture-owner".into(),
                    username: "fixture_owner".into(),
                    display_name: "Fixture Owner".into(),
                    owner: true,
                },
                Member {
                    id: "fixture-maya".into(),
                    username: "maya".into(),
                    display_name: "Maya".into(),
                    owner: false,
                },
                Member {
                    id: "fixture-alex".into(),
                    username: "alex".into(),
                    display_name: "Alex".into(),
                    owner: false,
                },
            ],
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
                    name: name.into(),
                    is_guest: false,
                },
                content: Content {
                    version: 1,
                    kind: "text".into(),
                    text: text.into(),
                },
            })
            .collect();
        self.timeline.reset(messages, "4").expect("valid fixture");
        self.live = "Live".into();
    }

    fn receive(&mut self) {
        self.voice.receive();
        if matches!(self.voice.state.phase, Phase::Connected(_))
            && self.announced_voice != Some(self.voice.state.generation)
        {
            self.announced_voice = Some(self.voice.state.generation);
            self.effects.play(Effect::Join);
        }
        let events: Vec<_> = self.worker.events.try_iter().collect();
        for event in events {
            match event {
                Event::Restored { generation, result } if generation == self.generation => {
                    self.loading = false;
                    match result {
                        Ok(Some((token, account, spaces))) => {
                            self.establish(token, account, spaces)
                        }
                        Ok(None) => self.open_general(),
                        Err(error) => {
                            self.warning = Some(error);
                            self.open_general();
                        }
                    }
                }
                Event::CodeRequested { generation, result } if generation == self.generation => {
                    self.loading = false;
                    match result {
                        Ok(challenge) => {
                            self.challenge = Some(challenge);
                            self.code.clear();
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::Verified { generation, result } if generation == self.generation => {
                    self.loading = false;
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
                        Ok(read) => self.navigation_cache.finish_prefetch(
                            &target,
                            request,
                            read,
                            Instant::now(),
                        ),
                        Err(error) if error.access_denied => {
                            if error.space_access_denied {
                                if let Some(space) = &target.space {
                                    self.navigation_cache.forget_space(space);
                                }
                            } else if let Some(channel) = &target.channel {
                                self.navigation_cache.forget_channel(channel);
                            }
                        }
                        Err(_) => {}
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
                        Err(error) => self.error = Some(error.message),
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
                _ => {}
            }
        }
    }

    fn establish(&mut self, token: String, account: Account, spaces: Spaces) {
        self.invalidate_navigation_cache();
        self.token = Some(token);
        self.username = account.username.clone().unwrap_or_default();
        self.display_name = account.display_name.clone().unwrap_or_default();
        let needs_profile = account.username.is_none() || account.display_name.is_none();
        self.account = Some(account);
        self.set_spaces(spaces);
        self.error = None;
        self.dialog = None;
        if needs_profile {
            self.dialog = Some(Dialog::Profile);
        } else {
            self.open_general();
        }
    }

    fn profiled(&mut self, account: Account, spaces: Spaces) {
        self.invalidate_navigation_cache();
        self.account = Some(account);
        self.set_spaces(spaces);
        self.dialog = None;
        self.open_general();
    }

    fn set_spaces(&mut self, spaces: Spaces) {
        self.spaces.retain(|space| space.demo);
        self.spaces.extend(spaces.spaces);
        self.limits = spaces.limits;
    }

    fn open_general(&mut self) {
        self.navigation += 1;
        self.opening = false;
        self.navigation_target = None;
        self.navigation_error = None;
        self.voice.state.browse("general".into());
        self.generation += 1;
        self.loading = true;
        self.clear_channel_state();
        self.selected_space = Some("general".into());
        self.selected_channel = Some("general".into());
        self.worker.send(Command::LoadChannel {
            generation: self.generation,
            token: self.token.clone(),
            channel: "general".into(),
            name: self.identity_name(),
            general: true,
        });
    }

    fn accept_channel(
        &mut self,
        history: model::History,
        session: ChatSession,
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
        if let Err(error) = self.timeline.reset(history.messages, &history.cursor) {
            self.clear_channel(&error);
            return;
        }
        self.has_more = history.has_more;
        self.selected_channel = Some(history.channel.id.clone());
        if general {
            let demo = model::Space {
                id: history.space.id.clone(),
                name: history.space.name.clone(),
                owner_id: String::new(),
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
                }],
                members: vec![],
            });
        }
        self.session = Some(session);
        self.live = "Connecting…".into();
        self.connect_gateway();
    }

    fn connect_gateway(&mut self) {
        let Some(channel) = self.selected_channel.clone() else {
            return;
        };
        self.roster_generation += 1;
        self.channel_rosters.clear();
        self.unavailable_rosters.clear();
        let media = gateway::MediaWatch {
            epoch: self.roster_generation,
            channels: self.detail.as_ref().map_or_else(Vec::new, |detail| {
                detail
                    .channels
                    .iter()
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
        if self.spaces.iter().any(|space| space.id == id && space.demo) {
            self.navigate(NavigationTarget {
                space: None,
                channel: None,
            });
            return;
        }
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
        self.remember_conversation();
        let mut cache_target = navigation::Target {
            space: target.space.clone(),
            channel: target.channel.clone(),
        };
        cache_target = self.navigation_cache.resolve(&cache_target);
        let target = NavigationTarget {
            space: cache_target.space.clone(),
            channel: cache_target.channel.clone(),
        };
        let prefetched = self
            .navigation_cache
            .take_prefetch(&cache_target, Instant::now());
        let cached = self.navigation_cache.history(&cache_target).or_else(|| {
            prefetched.and_then(|read| {
                let _fresh_detail = read.detail;
                read.history
            })
        });
        self.navigation += 1;
        self.opening = true;
        self.navigation_error = None;
        self.worker.send(Command::PrepareNavigation {
            generation: self.generation,
            navigation: self.navigation,
            token: self.token.clone(),
            space: target.space.clone(),
            channel: target.channel.clone(),
            name: self.identity_name(),
            cached,
        });
        self.navigation_target = Some(target);
    }

    fn prefetch(&mut self, target: NavigationTarget) {
        let target = self.navigation_cache.resolve(&navigation::Target {
            space: target.space,
            channel: target.channel,
        });
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
                messages: self.timeline.messages().cloned().collect(),
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
                if self.selected_space.as_deref()
                    != prepared
                        .detail
                        .as_ref()
                        .map(|detail| detail.space.id.as_str())
                {
                    self.voice_join_request += 1;
                }
                self.generation += 1;
                self.clear_channel_state();
                self.loading = false;
                self.error = None;
                self.navigation_error = None;
                self.navigation_target = None;
                self.navigation_open = false;
                self.member_page = 0;
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
        self.loading_older = false;
        self.older_error = None;
        self.has_more = false;
        self.timeline = Timeline::default();
        self.pending = None;
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

    fn join_voice(&mut self) {
        let Some(channel) = self.selected_channel.clone() else {
            return;
        };
        self.join_voice_channel(&channel);
    }

    fn voice_target(&self, channel: &str) -> Option<(CallContext, Option<String>)> {
        let detail = self.detail.as_ref()?;
        let target = detail.channels.iter().find(|item| item.id == channel)?;
        if self.unavailable_rosters.contains(channel)
            || (!detail.space.demo && self.token.is_none())
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

    fn join_voice_channel(&mut self, channel: &str) {
        if self.voice.state.active_channel() == Some(channel)
            && !matches!(self.voice.state.phase, Phase::Failed(_))
        {
            return;
        }
        let Some((context, space)) = self.voice_target(channel) else {
            return;
        };
        self.voice_join_request += 1;
        if let Some(space) = space {
            self.worker.send(Command::CheckVoice {
                request: self.voice_join_request,
                voice_generation: self.voice.state.generation,
                token: self.token.clone().unwrap_or_default(),
                space,
                channel: channel.into(),
            });
        } else {
            self.voice
                .join(context, None, self.token.clone(), self.identity_name());
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
        if request != self.voice_join_request || voice_generation != self.voice.state.generation {
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
            ),
            Err(error) => {
                if error.access_denied {
                    self.unavailable_rosters.insert(channel.into());
                    self.channel_rosters.remove(channel);
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
            } if generation == self.roster_generation
                && self.selected_channel.is_some()
                && self.detail.as_ref().is_some_and(|detail| {
                    detail.channels.iter().any(|item| item.id == channel)
                }) =>
            {
                self.unavailable_rosters.remove(&channel);
                self.channel_rosters.insert(channel, participants);
            }
            GatewayEvent::VoiceUnavailable {
                generation,
                channel,
                revoked,
            } if generation == self.roster_generation => {
                self.channel_rosters.remove(&channel);
                self.unavailable_rosters.insert(channel.clone());
                if revoked {
                    self.voice.revoke_channel(&channel);
                }
            }
            GatewayEvent::VoiceReset { generation } if generation == self.roster_generation => {
                self.channel_rosters.clear();
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
                self.live = if online { "Live".into() } else { detail }
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
                let remote = self
                    .session
                    .as_ref()
                    .is_some_and(|session| session.author.id != message.author.id);
                match self.timeline.apply(*message) {
                    Ok(model::Apply::Applied) if remote => self.effects.play(Effect::Message),
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
                if typing
                    && author.id
                        != self
                            .session
                            .as_ref()
                            .map_or("", |session| session.author.id.as_str())
                {
                    let replace = self
                        .typers
                        .get(&author.id)
                        .is_none_or(|entry| revision > entry.revision);
                    if replace {
                        self.typers.insert(
                            author.id.clone(),
                            Typer {
                                author,
                                revision,
                                expires: Instant::now() + Duration::from_secs(6),
                            },
                        );
                    }
                } else if self
                    .typers
                    .get(&author.id)
                    .is_some_and(|entry| revision > entry.revision)
                {
                    self.typers.remove(&author.id);
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
            if let Some(pending) = &mut pending {
                pending.sending = false;
            }
            self.reload_selected_channel(channel, general);
            self.pending = pending;
            self.draft = draft;
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
        self.unavailable_rosters.clear();
        self.selected_channel = None;
        self.session = None;
        self.loading_older = false;
        self.older_error = None;
        self.has_more = false;
        self.timeline = Timeline::default();
        self.pending = None;
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
        self.invalidate_navigation_cache();
        self.spaces.clear();
        self.detail = None;
        self.error = None;
        self.dialog = None;
        self.open_general();
    }

    fn send_message(&mut self) {
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
        let text = self
            .pending
            .as_ref()
            .map_or_else(|| self.draft.clone(), |pending| pending.text.clone());
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
        let pending = PendingSend::prepare(self.pending.as_ref(), &text);
        let id = pending.id.clone();
        if self.pending.is_none() && self.draft == text {
            self.draft.clear();
        }
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
        if self.loading_older || !self.has_more {
            return;
        }
        let Some(channel) = self.selected_channel.clone() else {
            return;
        };
        let Some(before) = self
            .timeline
            .messages()
            .next()
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
                self.spaces.retain(|space| space.id != id);
                self.dialog = None;
                self.detail = None;
                self.managed_members.clear();
                self.presence.clear();
                self.open_general();
            }
            AdminResult::ChannelCreated(channel) => {
                if let Some(detail) = &mut self.detail {
                    detail.channels.push(channel.clone());
                }
                self.dialog = None;
                self.select_channel(channel.id, false);
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
            AdminResult::Members { channel, members } => {
                if channel == self.managed_channel {
                    self.managed_members = members;
                }
            }
            AdminResult::MemberAdded { channel, member } => {
                if channel == self.managed_channel {
                    self.managed_members.retain(|item| item.id != member.id);
                    self.managed_members.push(member);
                    self.member_username.clear();
                }
            }
            AdminResult::MemberRemoved { channel, member } => {
                if channel == self.managed_channel {
                    self.managed_members.retain(|item| item.id != member);
                }
            }
        }
    }

    fn periodic(&mut self, context: &egui::Context) {
        let now = Instant::now();
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
        if context.input(|input| !input.events.is_empty()) {
            self.worker.send(Command::Activity);
        }
        self.receive();
        self.periodic(context);
        if matches!(self.dialog, Some(Dialog::SignIn)) {
            self.login_page(context);
        } else {
            self.shell(context);
            self.dialogs(context);
        }
        if !matches!(self.dialog, Some(Dialog::Audio))
            && !matches!(self.voice.microphone, MicrophoneState::Idle)
        {
            self.voice.stop_mic_test();
        }
    }
}

impl CaperApp {
    fn login_page(&mut self, context: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(BLACKOUT))
            .show(context, |ui| {
                ui.vertical_centered(|ui| {
                    ui.set_max_width(440.0);
                    ui.add_space(100.0);
                    ui.horizontal(|ui| {
                        ui.label(bold("caper").size(23.0));
                        ui.label(bold(".").size(23.0).color(TERRACOTTA_BRIGHT));
                    });
                    ui.add_space(58.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                        ui.label(
                            bold("WELCOME TO CAPER")
                                .size(11.0)
                                .extra_letter_spacing(1.5)
                                .color(MUTED),
                        );
                        ui.add_space(24.0);
                        ui.label(black(if self.challenge.is_some() {
                            "Check your email."
                        } else {
                            "Come on in."
                        }).size(40.0));
                        ui.add_space(28.0);
                        ui.label(
                            RichText::new(if self.challenge.is_some() {
                                "Enter the six-character code from your email. It expires in 10 minutes."
                            } else {
                                "Use your email to create an account or return to one. No password needed."
                            })
                            .size(16.0)
                            .color(MUTED),
                        );
                        ui.add_space(28.0);
                        if self.challenge.is_some() {
                            ui.label(bold("Sign-in code").size(14.0));
                            ui.add_space(4.0);
                            let response = ui.add_sized(
                                [440.0, 52.0],
                                egui::TextEdit::singleline(&mut self.code)
                                    .vertical_align(egui::Align::Center).char_limit(6),
                            );
                            self.code.make_ascii_uppercase();
                            self.code.retain(|character| {
                                "ABCDEFGHJKMNPQRSTWXYZ23456789".contains(character)
                            });
                            ui.add_space(20.0);
                            let submit = login_action(
                                ui,
                                if self.loading { "Checking…" } else { "Continue" },
                                self.loading || self.code.len() != 6,
                            );
                            if submit.clicked()
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
                            if ui.button("Use a different email").clicked() {
                                self.challenge = None;
                                self.code.clear();
                                self.error = None;
                            }
                        } else {
                            ui.label(bold("Email address").size(14.0));
                            ui.add_space(4.0);
                            let response = ui.add_sized(
                                [440.0, 52.0],
                                egui::TextEdit::singleline(&mut self.email)
                                    .vertical_align(egui::Align::Center)
                                    .hint_text("you@example.com"),
                            );
                            if let Some(error) = &self.error {
                                ui.add_space(12.0);
                                egui::Frame::new()
                                    .stroke(Stroke::new(1.0, TERRACOTTA))
                                    .corner_radius(6)
                                    .inner_margin(egui::Margin::symmetric(12, 14))
                                    .show(ui, |ui| {
                                        ui.set_width(414.0);
                                        ui.label(RichText::new(error).size(13.0).color(ERROR));
                                    });
                            }
                            ui.add_space(20.0);
                            let submit = login_action(
                                ui,
                                if self.loading { "Sending…" } else { "Email me a code" },
                                self.loading || !self.email.contains('@'),
                            );
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
                        ui.add_space(12.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(
                                    "We only send a code when you ask. Prefer to look around first?",
                                )
                                .size(14.0)
                                .color(MUTED),
                            );
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Join #general as a guest.")
                                            .size(14.0)
                                            .color(MUTED),
                                    )
                                    .frame(false),
                                )
                                .clicked()
                            {
                                self.dialog = None;
                                self.error = None;
                                self.open_general();
                            }
                        });
                    });
                });
            });
    }

    fn shell(&mut self, context: &egui::Context) {
        let narrow = context.viewport_rect().width() <= 760.0;
        if self.opening || self.navigation_error.is_some() {
            egui::TopBottomPanel::top("navigation-status")
                .frame(egui::Frame::new().fill(SURFACE).inner_margin(12.0))
                .show(context, |ui| {
                    ui.add_enabled_ui(self.dialog.is_none(), |ui| {
                        if self.opening {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Opening conversation…");
                            });
                        }
                        if let Some(error) = &self.navigation_error {
                            ui.colored_label(ERROR, error);
                            if ui.button("Retry opening conversation").clicked()
                                && let Some(target) = self.navigation_target.clone()
                            {
                                self.navigate(target);
                            }
                        }
                    });
                });
        }
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
                        if self.narrow_members_visible {
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
                    let wide_members = context.viewport_rect().width() >= 1100.0;
                    let (conversation_rect, members_rect) = if self.members_visible && wide_members
                    {
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
                    } else if self.members_visible {
                        let members = egui::Rect::from_min_max(
                            egui::pos2(stage_rect.left(), stage_rect.bottom() - 220.0),
                            stage_rect.max,
                        );
                        (
                            egui::Rect::from_min_max(
                                stage_rect.min,
                                egui::pos2(stage_rect.right(), members.top()),
                            ),
                            Some(members),
                        )
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
                        .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
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
                        ui.painter().vline(
                            separator_rect.center().x,
                            separator_rect.y_range(),
                            Stroke::new(2.0, TERRACOTTA),
                        );
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
                    let text = if demo {
                        "C".into()
                    } else {
                        name.chars()
                            .next()
                            .unwrap_or('C')
                            .to_uppercase()
                            .to_string()
                    };
                    let button = egui::Button::new(RichText::new(text).strong().color(if active {
                        TEXT
                    } else {
                        MUTED
                    }))
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
                        .inner
                        .on_hover_text(name);
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
                let (rect, add) =
                    ui.allocate_exact_size(egui::vec2(40.0, 40.0), egui::Sense::click());
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
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        ui.is_enabled(),
                        "Create space",
                    )
                });
                paint_icon(
                    ui.painter(),
                    egui::Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0)),
                    NavIcon::Plus,
                    TERRACOTTA_BRIGHT,
                );
                if add
                    .on_hover_text(if self.account.is_some() {
                        "Create space"
                    } else {
                        "Sign in to create a space"
                    })
                    .clicked()
                {
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
                                    .map_or("Caper", |detail| detail.space.name.as_str());
                                let title_rect = rect.shrink2(egui::vec2(8.0, 0.0));
                                ui.painter()
                                    .with_clip_rect(egui::Rect::from_min_max(
                                        title_rect.min,
                                        egui::pos2(title_rect.right() - 24.0, title_rect.bottom()),
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
                                    egui::Popup::menu(&actions).width(width).show(|ui| {
                                        if self.can_leave_space() {
                                            if ui
                                                .button(RichText::new("Leave space").color(ERROR))
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
                                            self.form_name = self
                                                .detail
                                                .as_ref()
                                                .map_or_else(String::new, |detail| {
                                                    detail.space.name.clone()
                                                });
                                            self.managed_members = self
                                                .detail
                                                .as_ref()
                                                .map_or_else(Vec::new, |detail| {
                                                    detail.members.clone()
                                                });
                                            self.managed_channel = None;
                                            if let (Some(token), Some(space)) =
                                                (self.token.clone(), self.selected_space.clone())
                                            {
                                                self.worker.send(Command::Admin {
                                                    generation: self.generation,
                                                    token,
                                                    operation: AdminOperation::LoadMembers {
                                                        space,
                                                        channel: None,
                                                    },
                                                });
                                            }
                                            self.dialog = Some(Dialog::ManageSpace);
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
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.spacing_mut().item_spacing.x = 2.0;
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
                                                        .add(egui::Button::image_and_text(
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
                                            && drawn_icon_button(
                                                ui,
                                                NavIcon::Plus,
                                                "Create channel",
                                            )
                                            .clicked()
                                        {
                                            self.form_name.clear();
                                            self.form_private = false;
                                            self.dialog = Some(Dialog::CreateChannel);
                                        }
                                        ui.label(
                                            RichText::new(count.to_string())
                                                .size(10.0)
                                                .color(MUTED),
                                        );
                                    },
                                );
                            },
                        );
                        ui.add_space(4.0);
                        let channels: Vec<_> =
                            self.detail.as_ref().map_or_else(Vec::new, |detail| {
                                detail
                                    .channels
                                    .iter()
                                    .map(|channel| {
                                        (channel.id.clone(), channel.name.clone(), channel.private)
                                    })
                                    .collect()
                            });
                        for (id, name, private) in channels {
                            if !self.channels_expanded {
                                break;
                            }
                            let active = self.selected_channel.as_deref() == Some(&id);
                            let (response, settings) = channel_button(
                                ui,
                                ui.available_width(),
                                &name,
                                private,
                                active,
                                self.owner(),
                            );
                            if settings.is_some_and(|response| response.clicked()) {
                                self.form_name = name.clone();
                                self.form_private = private;
                                self.managed_channel = Some(id.clone());
                                if private
                                    && let (Some(token), Some(space)) =
                                        (self.token.clone(), self.selected_space.clone())
                                {
                                    self.worker.send(Command::Admin {
                                        generation: self.generation,
                                        token,
                                        operation: AdminOperation::LoadMembers {
                                            space,
                                            channel: Some(id.clone()),
                                        },
                                    });
                                }
                                self.dialog = Some(Dialog::ManageChannel(id.clone()));
                            } else if response.clicked() {
                                self.select_channel(id.clone(), false);
                            } else if response.hovered() || response.has_focus() {
                                self.prefetch(NavigationTarget {
                                    space: self.selected_space.clone(),
                                    channel: Some(id.clone()),
                                });
                            }
                            ui.push_id(&id, |ui| self.channel_voice(ui, &id, &name, active));
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
                    });
            });
    }

    fn roster_for_active_call(&self) -> Vec<model::VoiceOccupant> {
        self.voice
            .participants
            .iter()
            .map(|participant| model::VoiceOccupant {
                id: participant.id.clone(),
                name: participant.name.clone(),
                country_code: participant.country_code.clone(),
                muted: participant.muted,
                deafened: participant.deafened,
            })
            .collect()
    }

    fn channel_voice(&mut self, ui: &mut egui::Ui, id: &str, name: &str, viewed: bool) {
        let own = self.voice.state.active_channel() == Some(id)
            && !matches!(self.voice.state.phase, Phase::Failed(_));
        let people = if own {
            self.roster_for_active_call()
        } else {
            self.channel_rosters.get(id).cloned().unwrap_or_default()
        };
        if people.is_empty() && (!viewed || own) {
            return;
        }
        let open = !self.collapsed_rosters.contains(id);
        ui.horizontal(|ui| {
            ui.add_space(32.0);
            if !people.is_empty() {
                let faces_width = people.len().min(3) as f32 * 16.0 + 8.0;
                let width = faces_width + 18.0 + if people.len() > 3 { 24.0 } else { 0.0 };
                let (rect, stack) =
                    ui.allocate_exact_size(egui::vec2(width, 30.0), egui::Sense::click());
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
                for (index, person) in people.iter().take(3).enumerate() {
                    let center =
                        egui::pos2(rect.left() + 12.0 + index as f32 * 16.0, rect.center().y);
                    ui.painter().circle_filled(center, 11.0, SURFACE);
                    ui.painter()
                        .circle_stroke(center, 11.0, Stroke::new(1.0, BORDER));
                    ui.painter().text(
                        center,
                        egui::Align2::CENTER_CENTER,
                        person
                            .name
                            .chars()
                            .next()
                            .unwrap_or('?')
                            .to_uppercase()
                            .to_string(),
                        egui::FontId::proportional(11.0),
                        TEXT,
                    );
                }
                if people.len() > 3 {
                    ui.painter().text(
                        egui::pos2(rect.left() + faces_width, rect.center().y),
                        egui::Align2::LEFT_CENTER,
                        format!("+{}", people.len() - 3),
                        egui::FontId::proportional(10.0),
                        MUTED,
                    );
                }
                paint_icon(
                    ui.painter(),
                    egui::Rect::from_center_size(
                        egui::pos2(rect.right() - 8.0, rect.center().y),
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
                        self.collapsed_rosters.insert(id.into());
                    } else {
                        self.collapsed_rosters.remove(id);
                    }
                }
            }
            if !own {
                ui.add_enabled_ui(self.voice_target(id).is_some(), |ui| {
                    let button = voice_join_button(ui, "Join");
                    let label = format!("Join voice in #{name}");
                    button.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), &label)
                    });
                    if button.on_hover_text(label).clicked() {
                        self.join_voice_channel(id);
                    }
                });
            }
        });
        if open {
            self.voice_roster(ui, people, own);
        }
    }

    fn voice_roster(&mut self, ui: &mut egui::Ui, people: Vec<model::VoiceOccupant>, own: bool) {
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
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 7.0;
                    avatar(ui, &participant.name, 28.0, false);
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
                        (deafened, NavIcon::VolumeX, "Deafened"),
                    ] {
                        if active {
                            let (rect, response) = ui
                                .allocate_exact_size(egui::vec2(16.0, 16.0), egui::Sense::hover());
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
                        egui::Popup::menu(&options).width(240.0).show(|ui| {
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
                        });
                    }
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
            ui.separator();
            ui.horizontal(|ui| {
                let status = ui.vertical(|ui| {
                    ui.add(
                        egui::Label::new(
                            bold(if connected {
                                "Voice connected"
                            } else {
                                "Connecting voice…"
                            })
                            .size(12.0),
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
                if drawn_icon_button(ui, NavIcon::Close, "Disconnect voice").clicked() {
                    if connected {
                        self.effects.play(Effect::Leave);
                    }
                    self.voice.leave();
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
            .on_hover_text("Edit profile")
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
                .clicked()
                {
                    self.voice.command(VoiceOperation::Mute(!muted));
                    self.effects.toggle(muted);
                }
                let input = audio_icon_button(ui, NavIcon::Chevron, 16.0, "Input Options", muted);
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
                .clicked()
                {
                    self.voice.command(VoiceOperation::Deafen(!deafened));
                    self.effects.toggle(deafened);
                }
                let output =
                    audio_icon_button(ui, NavIcon::Chevron, 16.0, "Output Options", deafened);
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
                    audio_icon_button(ui, NavIcon::Settings, 28.0, "User Settings", false);
                if settings.clicked() {
                    self.effects.toggle(!egui::Popup::menu(&settings).is_open());
                }
                egui::Popup::menu(&settings)
                    .align(egui::RectAlign::TOP_END)
                    .width(232.0)
                    .show(|ui| {
                        if ui
                            .checkbox(&mut self.sound_effects, "Caper sound effects")
                            .changed()
                        {
                            self.effects =
                                Effects::new(self.sound_effects && self.persist_preferences);
                            if self.sound_effects {
                                self.effects.play(Effect::ToggleOn);
                            }
                        }
                        if ui.button("Audio preferences").clicked() {
                            self.voice.refresh_devices();
                            self.dialog = Some(Dialog::Audio);
                            ui.close();
                        }
                        if self.voice.diagnostics.is_some()
                            && ui.button("Connection details").clicked()
                        {
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
                        if ui
                            .button(if self.account.is_some() {
                                "Edit profile"
                            } else {
                                "Sign in"
                            })
                            .clicked()
                        {
                            self.dialog = Some(if self.account.is_some() {
                                Dialog::Profile
                            } else {
                                Dialog::SignIn
                            });
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

    fn audio_preferences(&mut self, ui: &mut egui::Ui) {
        self.input_processing(ui);
        ui.add_space(12.0);
        self.output_gain(ui);
        ui.add_space(12.0);
        for input in [true, false] {
            let preferred = if input {
                &self.voice.preferences.input
            } else {
                &self.voice.preferences.output
            };
            let devices = if input {
                &self.voice.inputs
            } else {
                &self.voice.outputs
            };
            let label = preferred
                .as_ref()
                .map_or("System default", |id| {
                    devices
                        .iter()
                        .find(|(guid, _)| guid == id)
                        .map_or("Saved device (unavailable)", |(_, name)| name.as_str())
                })
                .to_owned();
            ui.label(bold(if input { "Microphone" } else { "Output device" }).size(13.0));
            let button = ui.add_sized(
                [ui.available_width(), 36.0],
                egui::Button::new(label)
                    .fill(COMPOSER)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(8),
            );
            egui::Popup::menu(&button).width(300.0).show(|ui| {
                ui.add_enabled_ui(
                    matches!(self.voice.microphone, MicrophoneState::Idle),
                    |ui| self.device_options(ui, input),
                );
            });
            ui.add_space(12.0);
        }
        if let Some(error) = &self.voice.device_error {
            ui.label(RichText::new(error).color(ERROR));
        }
        ui.add_space(16.0);
        ui.separator();
        ui.add_space(12.0);
        ui.label(bold("Microphone test").size(14.0));
        ui.add_space(10.0);
        if !self.persist_preferences {
            ui.label(
                RichText::new("TEST FIXTURE — no recording or playback.")
                    .size(11.0)
                    .color(MUTED),
            );
        }
        ui.add_enabled_ui(self.persist_preferences, |ui| {
            match self.voice.microphone.clone() {
                MicrophoneState::Idle => {
                    if primary(ui, "Record microphone", false).clicked() {
                        self.voice.start_mic_test();
                    }
                }
                MicrophoneState::Preparing => {
                    ui.label("Preparing local audio…");
                    if ui.button("Cancel test").clicked() {
                        self.voice.stop_mic_test();
                    }
                }
                MicrophoneState::Recording(started) => {
                    ui.label(format!(
                        "Recording · {} / 30 seconds",
                        started.elapsed().as_secs().min(30)
                    ));
                    ui.ctx().request_repaint_after(Duration::from_millis(100));
                    ui.horizontal(|ui| {
                        if ui.button("Finish recording").clicked() {
                            self.voice.finish_mic_recording();
                        }
                        if ui.button("Cancel test").clicked() {
                            self.voice.stop_mic_test();
                        }
                    });
                }
                MicrophoneState::Ready(seconds) => {
                    ui.label(format!("Recorded {seconds:.1} seconds"));
                    ui.horizontal_wrapped(|ui| {
                        if ui.button("Play natural").clicked() {
                            self.voice.play_mic_sample(false);
                        }
                        if ui.button("Play enhanced").clicked() {
                            self.voice.play_mic_sample(true);
                        }
                    });
                    if ui.button("Discard recording").clicked() {
                        self.voice.stop_mic_test();
                    }
                }
                MicrophoneState::Playing { seconds, enhanced } => {
                    ui.label(format!(
                        "Playing {} · {seconds:.1} seconds",
                        if enhanced { "enhanced" } else { "natural" }
                    ));
                    if ui.button("Stop playback").clicked() {
                        self.voice.stop_mic_playback();
                    }
                }
            }
        });
        if let Some(error) = &self.voice.microphone_error {
            ui.label(RichText::new(error).color(ERROR));
        }
        if self
            .account
            .as_ref()
            .is_some_and(|account| account.debug_enabled)
        {
            ui.add_space(12.0);
            ui.collapsing("Audio diagnostics", |ui| self.audio_diagnostics(ui));
        }
    }

    fn audio_diagnostics(&self, ui: &mut egui::Ui) {
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
            ui.label("No processing data.");
        }
        let report = serde_json::to_string_pretty(&serde_json::json!({
            "platform": std::env::consts::OS,
            "processing": processing,
        }))
        .expect("numeric diagnostics serialize");
        if ui.button("Copy diagnostics").clicked() {
            ui.ctx().copy_text(report.clone());
        }
        ui.add(egui::Label::new(RichText::new(report).monospace()).selectable(true));
        ui.ctx().request_repaint_after(Duration::from_secs(1));
    }

    fn connection_details(&self, ui: &mut egui::Ui) {
        if !self.persist_preferences {
            ui.label(
                RichText::new("TEST FIXTURE — synthetic statistics, no live connection.")
                    .color(MUTED),
            );
            ui.add_space(12.0);
        }
        let Some((stats, sampled)) = &self.voice.diagnostics else {
            ui.label(if matches!(self.voice.state.phase, Phase::Idle) {
                "Not connected"
            } else {
                "Waiting for connection statistics…"
            });
            return;
        };
        egui::Grid::new("connection-statistics")
            .num_columns(2)
            .spacing([28.0, 12.0])
            .show(ui, |ui| {
                for (label, value) in [
                    ("Received", format!("{} bytes", stats.received_bytes)),
                    (
                        "Live receive",
                        format!("{:.1} kbps", stats.receive_bitrate / 1000.0),
                    ),
                    ("Sent", format!("{} bytes", stats.sent_bytes)),
                    (
                        "Live send",
                        format!("{:.1} kbps", stats.send_bitrate / 1000.0),
                    ),
                    ("Packets lost", stats.packets_lost.to_string()),
                    ("Max jitter", format!("{:.0} ms", stats.max_jitter_ms)),
                    ("RTT", format!("{:.0} ms", stats.round_trip_ms)),
                    (
                        "Route",
                        match stats.route {
                            "relay" => "TURN relay",
                            "direct" => "Direct",
                            _ => "Not observed yet",
                        }
                        .into(),
                    ),
                ] {
                    ui.label(RichText::new(label).color(MUTED));
                    ui.label(value);
                    ui.end_row();
                }
            });
        ui.add_space(16.0);
        ui.label(
            RichText::new(format!("Sampled {}s ago", sampled.elapsed().as_secs()))
                .size(11.0)
                .color(MUTED),
        );
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
        for member in members {
            ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),44.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                ui.add_space(16.0);
                let status = self
                    .presence
                    .get(&member.id)
                    .map_or("unknown", String::as_str);
                presence_avatar(ui, &member.display_name, 30.0, status);
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

    fn conversation(&mut self, ui: &mut egui::Ui, narrow: bool) {
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
                            if narrow
                                && drawn_icon_button(ui, NavIcon::Menu, "Browse channels").clicked()
                            {
                                self.navigation_open = true;
                            }
                            ui.label(
                                RichText::new(format!("# {}", self.channel_name())).size(13.76),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if self
                                        .detail
                                        .as_ref()
                                        .is_some_and(|detail| !detail.members.is_empty())
                                        && users_button(ui, if narrow { self.narrow_members_visible } else { self.members_visible }).clicked()
                                    {
                                        if narrow { self.narrow_members_visible = !self.narrow_members_visible; }
                                        else { self.members_visible = !self.members_visible; }
                                        self.effects.toggle(if narrow { self.narrow_members_visible } else { self.members_visible });
                                    }
                                    let already_here =
                                        self.voice.state.active_channel().is_some_and(|channel| {
                                            Some(channel) == self.selected_channel.as_deref()
                                        });
                                    let label = if already_here {
                                        if matches!(self.voice.state.phase, Phase::Connected(_)) {
                                            "Leave"
                                        } else {
                                            "Cancel"
                                        }
                                    } else {
                                        "Join"
                                    };
                                    if self.selected_channel.is_some()
                                        && voice_join_button(ui, label).clicked()
                                    {
                                        if already_here {
                                            if matches!(self.voice.state.phase, Phase::Connected(_)) { self.effects.play(Effect::Leave); }
                                            self.voice.leave();
                                        } else {
                                            self.join_voice();
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
            let composer = egui::TopBottomPanel::bottom("composer")
                .min_height(editor_height + 24.0)
                .show_separator_line(false)
                .frame(
                    egui::Frame::new()
                        .fill(CONVERSATION)
                        .inner_margin(egui::Margin::symmetric(18, 12)),
                )
                .show_inside(ui, |ui| {
                    ui.visuals_mut().widgets.inactive.corner_radius = CornerRadius::same(6);
                    ui.visuals_mut().widgets.hovered.corner_radius = CornerRadius::same(6);
                    ui.visuals_mut().widgets.active.corner_radius = CornerRadius::same(6);
                    if let Some(error) = &self.error {
                        ui.colored_label(ERROR, error);
                    }
                    if let Some(error) = &self.voice.error {
                        ui.colored_label(ERROR, error);
                    }
                    if self.live != "Live" {
                        ui.label(RichText::new(&self.live).size(11.0).color(MUTED));
                    }
                    let before = self.draft.clone();
                    let channel_name = self.channel_name().to_owned();
                    let editor = egui::ScrollArea::vertical()
                        .id_salt("composer-scroll")
                        .max_height((ui.ctx().viewport_rect().height() * 0.4).min(320.0))
                        .min_scrolled_height(42.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| ui.add(
                        egui::TextEdit::multiline(&mut self.draft)
                            .id(egui::Id::new("message-composer"))
                            .desired_width(f32::INFINITY)
                            .min_size(egui::vec2(0.0, 42.0))
                            .desired_rows(1)
                            .return_key(Some(egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::Enter)))
                            .font(egui::FontId::proportional(13.6))
                            .margin(egui::vec2(11.0, 10.0))
                            .hint_text(
                                RichText::new(format!("Message #{channel_name}"))
                                    .color(Color32::from_rgb(142, 149, 152)),
                            )
                            .background_color(COMPOSER)
                            .char_limit(4_000),
                    ));
                    let response = editor.inner;
                    ui.painter().rect_stroke(
                        editor.inner_rect,
                        6.0,
                        Stroke::new(1.0, BORDER),
                        egui::StrokeKind::Inside,
                    );
                    if self.draft != before {
                        self.typing_edited = Instant::now();
                    }
                    let send = response.has_focus()
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
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{count} / 4,000"))
                                    .size(10.0)
                                    .color(if count >= 3_900 { ERROR } else { MUTED }),
                            );
                        });
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
                        .map(|typer| typer.author.name.as_str())
                        .collect();
                    if !names.is_empty() {
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
                    }
                });
            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if self.has_more
                        && ui
                            .add_enabled(
                                !self.loading_older,
                                egui::Button::new(if self.loading_older {
                                    "Loading…"
                                } else if self.older_error.is_some() {
                                    "Retry older messages"
                                } else {
                                    "Load older messages"
                                }),
                            )
                            .clicked()
                    {
                        self.load_older();
                    } else if !self.has_more {
                        let (history, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 44.0),
                            egui::Sense::hover(),
                        );
                        ui.painter().text(
                            history.center(),
                            egui::Align2::CENTER_CENTER,
                            "Beginning of conversation",
                            egui::FontId::proportional(11.52),
                            MUTED,
                        );
                    }
                    if let Some(error) = &self.older_error {
                        ui.colored_label(egui::Color32::LIGHT_RED, error);
                    }
                    if self.loading && self.timeline.messages().next().is_none() {
                        ui.centered_and_justified(|ui| {
                            ui.spinner();
                        });
                    }
                    let messages: Vec<_> = self.timeline.messages().cloned().collect();
                    for message in messages {
                        self.message(ui, &message);
                    }
                    if let Some(pending) = self.pending.clone() {
                        let author = self.session.as_ref().map_or_else(
                            || self.identity_name(),
                            |session| session.author.name.clone(),
                        );
                        message_row(ui, &author, "Now", &pending.text, false, true);
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
                                    RichText::new(format!(
                                        "Start the conversation in #{}.",
                                        self.channel_name()
                                    ))
                                    .color(MUTED),
                                );
                            });
                        });
                    }
                });
        });
    }

    fn message(&self, ui: &mut egui::Ui, message: &model::Message) {
        let time = display_time(&message.created_at);
        message_row(
            ui,
            &message.author.name,
            &time,
            &message.content.text,
            message.author.is_guest,
            false,
        );
    }

    fn channel_name(&self) -> &str {
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

    fn dialogs(&mut self, context: &egui::Context) {
        let Some(dialog) = self.dialog.clone() else {
            return;
        };
        if matches!(dialog, Dialog::SignIn) {
            return;
        }
        let title = match &dialog {
            Dialog::SignIn => {
                if self.challenge.is_some() {
                    "Check your email"
                } else {
                    "Sign in to Caper"
                }
            }
            Dialog::Profile => "Edit profile",
            Dialog::Audio => "Audio preferences",
            Dialog::Connection => "Connection details",
            Dialog::Diagnostics => "Audio diagnostics",
            Dialog::CreateSpace => "Create a space",
            Dialog::ManageSpace => "Manage space",
            Dialog::LeaveSpace { .. } => "Leave space?",
            Dialog::ConfirmDelete { channel, .. } => {
                if channel.is_some() {
                    "Delete channel"
                } else {
                    "Delete space"
                }
            }
            Dialog::CreateChannel => "Create a channel",
            Dialog::ManageChannel(_) => "Overview",
        };
        context
            .layer_painter(egui::LayerId::new(
                egui::Order::Background,
                egui::Id::new("dialog-backdrop"),
            ))
            .rect_filled(context.viewport_rect(), 0.0, Color32::from_black_alpha(190));
        let wide = matches!(dialog, Dialog::ManageSpace | Dialog::ManageChannel(_));
        let width: f32 = if wide { 600.0 } else { 440.0 };
        let minimum: f32 = if matches!(dialog, Dialog::ManageSpace) {
            658.0
        } else if matches!(dialog, Dialog::ManageChannel(_)) {
            618.0
        } else {
            0.0
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
        egui::Area::new(egui::Id::new("caper-dialog"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
                // An Area remembers its previous size. Let settings grow when
                // recording/replay controls appear instead of pinning the old
                // short scroll viewport; the screen remains the upper bound.
                ui.set_max_height(available.y);
                egui::Frame::new()
                    .fill(SURFACE)
                    .stroke(Stroke::new(1.0, BORDER))
                    .corner_radius(8)
                    .show(ui, |ui| {
                        ui.set_width(width.min(available.x));
                        ui.set_min_height(minimum.min(available.y));
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(22, 18))
                            .show(ui, |ui| {
                                ui.horizontal_top(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(bold(title).size(19.0));
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
                        egui::ScrollArea::vertical()
                            .max_height((available.y - 92.0).max(120.0))
                            .show(ui, |ui| {
                                egui::Frame::new()
                                    .inner_margin(egui::Margin::symmetric(22, 20))
                                    .show(ui, |ui| {
                                        ui.set_width((width.min(available.x) - 44.0).max(1.0));
                                        match dialog {
                                            Dialog::SignIn => unreachable!("sign-in is rendered as a full page"),
                                            Dialog::Profile => self.profile_dialog(ui),
                                            Dialog::Audio => self.audio_preferences(ui),
                                            Dialog::Connection => self.connection_details(ui),
                                            Dialog::Diagnostics => self.audio_diagnostics(ui),
                                            Dialog::CreateSpace => self.space_dialog(ui, false),
                                            Dialog::ManageSpace => self.space_dialog(ui, true),
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
                                            Dialog::LeaveSpace { id, name } => {
                                                ui.label(format!("Leave {name}? You will lose access to its channels and conversations. An owner can add you again later."));
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
                                            Dialog::CreateChannel => self.channel_dialog(ui, None),
                                            Dialog::ManageChannel(id) => self.channel_dialog(ui, Some(id)),
                                        }
                                        notices(ui, &self.error, &self.warning);
                                    });
                            });
                    });
            });
        if close && !(self.loading && return_to.is_some()) {
            self.dialog = return_to;
            self.error = None;
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
        ui.label("Space name");
        ui.add_sized(
            [ui.available_width(), 42.0],
            egui::TextEdit::singleline(&mut self.form_name)
                .vertical_align(egui::Align::Center)
                .char_limit(80),
        );
        let disabled = self.loading || self.form_name.trim().is_empty();
        let save = if manage {
            let mut clicked = false;
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    clicked = ui
                        .add_enabled(
                            !disabled,
                            egui::Button::new(if self.loading {
                                "Saving…"
                            } else {
                                "Save name"
                            })
                            .min_size(egui::vec2(88.0, 36.0)),
                        )
                        .clicked();
                });
            });
            clicked
        } else {
            primary(
                ui,
                if self.loading {
                    "Saving…"
                } else {
                    "Create space"
                },
                disabled,
            )
            .clicked()
        };
        if save {
            if manage {
                if let Some(space) = self.selected_space.clone() {
                    self.admin(AdminOperation::UpdateSpace {
                        space,
                        name: self.form_name.clone(),
                    });
                }
            } else {
                self.admin(AdminOperation::CreateSpace {
                    name: self.form_name.clone(),
                });
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

    fn channel_dialog(&mut self, ui: &mut egui::Ui, channel: Option<String>) {
        ui.label("Channel name");
        ui.add_sized(
            [ui.available_width(), 42.0],
            egui::TextEdit::singleline(&mut self.form_name)
                .vertical_align(egui::Align::Center)
                .char_limit(80),
        );
        self.form_name = normalize_channel(&self.form_name);
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
        if primary(
            ui,
            if self.loading {
                "Saving…"
            } else if channel.is_some() {
                "Save changes"
            } else {
                "Create channel"
            },
            self.loading || self.form_name.trim_end_matches('-').is_empty(),
        )
        .clicked()
        {
            let Some(space) = self.selected_space.clone() else {
                return;
            };
            let name = self.form_name.trim_end_matches('-').to_owned();
            if let Some(channel) = channel.clone() {
                self.admin(AdminOperation::UpdateChannel {
                    space,
                    channel,
                    name,
                    private: self.form_private,
                });
            } else {
                self.admin(AdminOperation::CreateChannel {
                    space,
                    name,
                    private: self.form_private,
                });
            }
        }
        if let Some(channel) = channel {
            if self.form_private {
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
                    .detail
                    .as_ref()
                    .and_then(|detail| detail.channels.iter().find(|entry| entry.id == channel))
                    .map_or("this channel", |entry| entry.name.as_str())
                    .to_owned();
                self.dialog = Some(Dialog::ConfirmDelete {
                    space,
                    channel: Some(channel),
                    name,
                });
            }
        }
    }

    fn members_dialog(&mut self, ui: &mut egui::Ui, channel: Option<String>) {
        ui.horizontal(|ui| {
            ui.add_sized(
                [(ui.available_width() - 80.0).max(1.0), 38.0],
                egui::TextEdit::singleline(&mut self.member_username)
                    .vertical_align(egui::Align::Center)
                    .hint_text("Exact username"),
            );
            if ui
                .add(egui::Button::new(bold("Add").size(12.0)).min_size(egui::vec2(64.0, 38.0)))
                .clicked()
                && !self.member_username.trim().is_empty()
                && let Some(space) = self.selected_space.clone()
            {
                self.admin(AdminOperation::AddMember {
                    space,
                    channel: channel.clone(),
                    username: self.member_username.trim().into(),
                });
            }
        });
        ui.separator();
        let members = self.managed_members.clone();
        egui::ScrollArea::vertical()
            .max_height(280.0)
            .show(ui, |ui| {
                for member in members {
                    ui.horizontal(|ui| {
                        avatar(ui, &member.display_name, 30.0, false);
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
    response.on_hover_text(label)
}

fn voice_join_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(if label == "Join" { 76.0 } else { 88.0 }, 36.0),
        egui::Sense::click(),
    );
    let hover = response.hovered() || response.has_focus();
    ui.painter().rect_filled(
        rect,
        6.0,
        Color32::from_rgba_unmultiplied(182, 77, 50, if hover { 61 } else { 36 }),
    );
    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(
            1.0,
            if hover {
                TERRACOTTA_BRIGHT
            } else {
                Color32::from_rgb(137, 70, 53)
            },
        ),
        egui::StrokeKind::Inside,
    );
    let color = Color32::from_rgb(227, 153, 133);
    paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 20.0, rect.center().y),
            egui::vec2(16.0, 16.0),
        ),
        NavIcon::Speech,
        color,
    );
    ui.painter().text(
        egui::pos2(rect.left() + 36.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::new(12.48, egui::FontFamily::Name("Satoshi Bold".into())),
        color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            format!("{label} voice"),
        )
    });
    response.on_hover_text(format!("{label} voice"))
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

fn channel_button(
    ui: &mut egui::Ui,
    width: f32,
    name: &str,
    private: bool,
    active: bool,
    manageable: bool,
) -> (egui::Response, Option<egui::Response>) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 38.0), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            active,
            name,
        )
    });
    if active || response.hovered() {
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
    let color = if active { TEXT } else { MUTED };
    paint_icon(
        ui.painter(),
        egui::Rect::from_center_size(
            egui::pos2(rect.left() + 17.5, rect.center().y),
            egui::vec2(17.0, 17.0),
        ),
        if private {
            NavIcon::Lock
        } else {
            NavIcon::Hash
        },
        if active { TERRACOTTA_BRIGHT } else { color },
    );
    ui.painter().text(
        egui::pos2(rect.left() + 35.0, rect.center().y - 1.0),
        egui::Align2::LEFT_CENTER,
        name,
        egui::FontId::new(13.0, egui::FontFamily::Name("Satoshi Medium".into())),
        color,
    );
    // The nested settings hit target owns hover while the pointer is over it,
    // so parent-response hover alone makes the control disappear between the
    // mouse press and release. Geometry remains stable for the whole gesture.
    let pointer_over_row = ui.input(|input| {
        input
            .pointer
            .hover_pos()
            .is_some_and(|position| rect.contains(position))
    });
    let settings = (manageable && (pointer_over_row || response.has_focus())).then(|| {
        let rect = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 18.0, rect.center().y),
            egui::vec2(28.0, 28.0),
        );
        let settings = ui.interact(rect, response.id.with("settings"), egui::Sense::click());
        if settings.hovered() || settings.has_focus() {
            ui.painter().rect_filled(rect, 6.0, SURFACE);
        }
        paint_icon(
            ui.painter(),
            rect.shrink(6.0),
            NavIcon::Settings,
            if settings.hovered() { TEXT } else { MUTED },
        );
        settings.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Button,
                ui.is_enabled(),
                format!("Manage {name}"),
            )
        });
        settings.on_hover_text(format!("Manage {name}"))
    });
    (response, settings)
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

fn message_row(
    ui: &mut egui::Ui,
    author: &str,
    time: &str,
    text: &str,
    guest: bool,
    pending: bool,
) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(18, 10))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 4.0);
            ui.horizontal_top(|ui| {
                avatar(ui, author, 34.0, false);
                ui.vertical(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(bold(author).size(13.0));
                        if guest {
                            ui.label(RichText::new("GUEST").size(9.0).color(MUTED));
                        }
                        ui.label(RichText::new(time).size(10.0).color(MUTED));
                    });
                    ui.label(RichText::new(text).size(14.0).color(if pending {
                        MUTED
                    } else {
                        Color32::from_rgb(222, 223, 224)
                    }));
                });
            });
        });
}

fn avatar(ui: &mut egui::Ui, name: &str, size: f32, online: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), size / 2.0, RAISED);
    if online {
        ui.painter()
            .circle_stroke(rect.center(), size / 2.0 - 1.0, Stroke::new(2.0, CAPER));
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        name.chars()
            .next()
            .unwrap_or('C')
            .to_uppercase()
            .to_string(),
        egui::FontId::proportional(size * 0.38),
        TEXT,
    );
}

fn presence_avatar(ui: &mut egui::Ui, name: &str, size: f32, status: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    ui.painter()
        .circle_filled(rect.center(), size / 2.0, RAISED);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        name.chars()
            .next()
            .unwrap_or('C')
            .to_uppercase()
            .to_string(),
        egui::FontId::proportional(size * 0.38),
        TEXT,
    );
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

fn display_time(timestamp: &str) -> String {
    let Some(value) = timestamp.get(11..16) else {
        return timestamp.to_owned();
    };
    let Some((hour, minute)) = value.split_once(':') else {
        return value.to_owned();
    };
    let Ok(hour) = hour.parse::<u8>() else {
        return value.to_owned();
    };
    let suffix = if hour < 12 { "AM" } else { "PM" };
    let hour = match hour % 12 {
        0 => 12,
        value => value,
    };
    format!("{hour}:{minute} {suffix}")
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
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, TERRACOTTA_BRIGHT);
    style.visuals.selection.bg_fill = TERRACOTTA;
    style.visuals.override_text_color = Some(TEXT);
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
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

fn primary(ui: &mut egui::Ui, text: &str, disabled: bool) -> egui::Response {
    ui.add_enabled(
        !disabled,
        egui::Button::new(text)
            .fill(TERRACOTTA)
            .min_size(egui::vec2(ui.available_width(), 44.0))
            .corner_radius(8),
    )
}

fn destructive(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::from_rgb(255, 128, 149)))
            .stroke(Stroke::new(1.0, Color32::from_rgb(185, 54, 77)))
            .fill(Color32::TRANSPARENT)
            .min_size(egui::vec2(112.0, 36.0)),
    )
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
            "parity-narrow" | "parity-browse" | "parity-voice-rosters-narrow"
        )
    }) {
        [390.0, 844.0]
    } else {
        [1440.0, 900.0]
    };
    eframe::run_native(
        "Caper",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Wgpu,
            persist_window: fixture.is_none(),
            viewport: egui::ViewportBuilder::default()
                .with_inner_size(viewport_size)
                .with_min_inner_size([320.0, 560.0]),
            ..Default::default()
        },
        Box::new(move |creation| {
            let mut app = CaperApp::new(&creation.egui_ctx, api, fixture.as_deref());
            if let Some(storage) = creation.storage {
                app.restore_preferences(storage);
            }
            Ok(Box::new(app))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CaperApp, Dialog, GatewayEvent, PendingSend, Phase, endpoint, member_page_ids,
        normalize_channel, permanent_send_rejection,
    };
    use crate::model::{
        Account, Author, ChatSession, Content, History, HistoryPlace, Member, Message, Space,
        SpaceDetail, Spaces,
    };
    use crate::worker::LoadError;
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
            |context| {
                app.shell(context);
                app.dialogs(context);
            },
        )
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
        let output = render(&mut app, &context, vec![]);
        let sidebar_text = |output: &egui::FullOutput, label: &str| {
            output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.pos.x < 340.0 && text.galley.job.text == label => {
                    Some(text.pos)
                }
                _ => None,
            })
        };
        assert!(sidebar_text(&output, "Maya").is_some());
        let stack = sidebar_text(&output, "M").unwrap() + egui::vec2(3.0, 3.0);
        click(&mut app, &context, stack);
        assert!(app.collapsed_rosters.contains("chan00000002"));
        assert!(sidebar_text(&render(&mut app, &context, vec![]), "Maya").is_none());
        assert_eq!(app.selected_channel, selected);
        click(&mut app, &context, stack);
        let reopened = render(&mut app, &context, vec![]);
        assert!(sidebar_text(&reopened, "Maya").is_some());
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
        });
        assert!(app.channel_rosters.is_empty());
        app.selected_channel = selected;
        app.gateway(GatewayEvent::VoiceRoster {
            generation: 43,
            channel: "chan00000003".into(),
            participants: people,
        });
        assert!(app.channel_rosters.contains_key("chan00000003"));
        app.gateway(GatewayEvent::VoiceReset { generation: 42 });
        assert!(!app.channel_rosters.is_empty());
        app.gateway(GatewayEvent::VoiceReset { generation: 43 });
        assert!(app.channel_rosters.is_empty());
    }

    #[test]
    fn voice_roster_names_align_and_participant_menu_changes_only_that_listener() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-voice-connected"),
        );
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
                    conversation: Some((history, session())),
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
        app.draft = "private draft".into();
        app.pending = Some(PendingSend::prepare(None, "private draft"));
        assert!(app.timeline.messages().next().is_some());
        app.admin_result(crate::worker::AdminResult::SpaceLeft(space.clone()));
        assert!(!app.spaces.iter().any(|entry| entry.id == space));
        assert!(app.detail.is_none());
        assert_eq!(app.selected_channel.as_deref(), Some("general"));
        assert!(app.timeline.messages().next().is_none());
        assert!(app.draft.is_empty());
        assert!(app.pending.is_none());
        assert!(!app.can_leave_space());
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
            let output = render(&mut app, &context, vec![]);
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
        assert!(contains(&audio, "Microphone test"));
        for removed in [
            "contour",
            "noise suppression",
            "Only you can hear",
            "preferences are saved",
        ] {
            assert!(
                !contains(&audio, removed),
                "Unexpected explanatory copy: {removed}"
            );
        }
        app.dialog = Some(Dialog::Connection);
        assert!(contains(
            &render(&mut app, &context, vec![]),
            "Not connected"
        ));
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
                .find(|text| {
                    text.galley.job.text == label && (label != "Join" || text.pos.x > 340.0)
                })
                .unwrap_or_else(|| panic!("missing {label}"))
        };
        // Independent values from the current web CSS: 54px header, 44px history.
        for (label, center) in [
            ("Fixture Studio", 27.0),
            ("# general", 27.0),
            ("Members", 27.0),
            ("Join", 27.0),
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
        assert!(
            (text("planning").pos.y - text("design").pos.y - 41.0).abs() < 1.0,
            "38px rows plus 3px gap"
        );
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
    fn audio_controls_and_voice_header_act_on_current_intent() {
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
        let output = render(&mut app, &context, vec![]);
        assert!(app.sound_effects);
        click(
            &mut app,
            &context,
            text_position(&output, "Caper sound effects"),
        );
        assert!(!app.sound_effects);
        click(&mut app, &context, egui::pos2(307.0, 867.0));
        let output = render(&mut app, &context, vec![]);
        click(
            &mut app,
            &context,
            text_position(&output, "Caper sound effects"),
        );
        assert!(app.sound_effects);
        for fixture in ["parity-voice-joining", "parity-voice-connected"] {
            let context = egui::Context::default();
            let mut app = CaperApp::new(
                &context,
                crate::api::Api::new("http://127.0.0.1:9").unwrap(),
                Some(fixture),
            );
            render(&mut app, &context, vec![]);
            click(&mut app, &context, egui::pos2(1110.0, 27.0));
            assert!(
                matches!(app.voice.state.phase, crate::state::Phase::Idle),
                "Cancel/Leave must end the current call"
            );
        }
    }

    fn account(profile: bool) -> Account {
        Account {
            id: "account".into(),
            username: profile.then(|| "member".into()),
            display_name: profile.then(|| "Member".into()),
            debug_enabled: false,
        }
    }

    fn spaces() -> Spaces {
        Spaces {
            spaces: Vec::new(),
            limits: None,
        }
    }

    fn history(channel: &str) -> History {
        History {
            messages: Vec::new(),
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
                name: "Member".into(),
                is_guest: false,
            },
        }
    }

    #[test]
    fn unknown_send_retry_preserves_id_and_original_text() {
        let first = PendingSend::prepare(None, "first payload");
        let retry = PendingSend::prepare(Some(&first), "edited payload");
        assert_eq!(retry.id, first.id);
        assert_eq!(retry.text, "first payload");
        assert!(retry.sending);
    }

    #[test]
    fn definitive_rejection_unlocks_editing_but_transport_failure_preserves_retry() {
        assert!(permanent_send_rejection(Some(400)));
        assert!(permanent_send_rejection(Some(422)));
        assert!(!permanent_send_rejection(None));
        assert!(!permanent_send_rejection(Some(503)));
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
    fn verification_requires_profile_and_profile_completion_reopens_account_chat() {
        let context = eframe::egui::Context::default();
        let api = crate::api::Api::new("http://127.0.0.1:9").unwrap();
        let mut app = CaperApp::new(&context, api, Some("signed-out"));
        app.session = Some(session());
        app.draft = "guest draft".into();

        app.establish("account-token".into(), account(false), spaces());
        assert!(matches!(app.dialog, Some(Dialog::Profile)));
        assert_eq!(
            app.selected_channel, None,
            "onboarding must not open guest chat"
        );

        app.profiled(account(true), spaces());
        assert!(app.dialog.is_none());
        assert_eq!(app.selected_channel.as_deref(), Some("general"));
        assert!(app.session.is_none(), "guest capability must be discarded");
        assert!(
            app.draft.is_empty(),
            "guest draft must not cross auth transition"
        );
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

        app.accept_channel(history("different"), session(), false, "requested");
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
                demo: false,
            },
            channels: Vec::new(),
            members,
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
                name: "Member".into(),
                is_guest: false,
            },
            content: Content {
                version: 1,
                kind: "text".into(),
                text: pending.text.clone(),
            },
        };
        assert!(pending.confirmed_by(&message, "author"));
        assert!(!pending.confirmed_by(&message, "another-author"));
    }

    #[test]
    fn navigation_keeps_visible_conversation_and_rejects_stale_completions() {
        let context = egui::Context::default();
        let mut app = CaperApp::new(
            &context,
            crate::api::Api::new("http://127.0.0.1:9").unwrap(),
            Some("parity-desktop"),
        );
        let original = app.selected_channel.clone();
        let count = app.timeline.messages().count();
        app.draft = "Unsent draft".into();
        app.select_channel("next".into(), false);
        let old_request = app.navigation;
        assert_eq!(app.selected_channel, original);
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
        assert_eq!(app.draft, "Unsent draft");
        app.navigate(app.navigation_target.clone().unwrap());
        assert!(app.opening);
        app.accept_navigation(
            app.generation,
            old_request,
            Ok(crate::worker::PreparedNavigation {
                detail: None,
                conversation: Some((history("stale"), session())),
            }),
        );
        assert!(app.opening);
        assert_eq!(app.selected_channel, original);
        app.accept_navigation(
            app.generation,
            app.navigation,
            Ok(crate::worker::PreparedNavigation {
                detail: None,
                conversation: Some((history("next"), session())),
            }),
        );
        assert_eq!(app.selected_channel.as_deref(), Some("next"));
        assert!(!app.opening);
        assert!(app.navigation_error.is_none());
        assert!(app.draft.is_empty());
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
                conversation: Some((history("next"), session())),
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
                conversation: Some((history("two"), session())),
            }),
        );
        assert!(app.pending.is_none());
        assert!(app.draft.is_empty());
    }
}
