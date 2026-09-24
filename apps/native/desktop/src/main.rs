mod api;
mod credentials;
mod gateway;
#[path = "../voice-spike/src/media.rs"]
mod media;
#[path = "../voice-spike/src/media_gateway.rs"]
mod media_gateway;
mod model;
#[path = "../voice-spike/src/state.rs"]
mod state;
mod voice;
mod worker;

use eframe::egui::{self, Color32, CornerRadius, RichText, Stroke};
use gateway::GatewayEvent;
use model::{
    Account, Author, ChatSession, Member, Presence, SpaceDetail, SpaceLimits, Spaces, Timeline,
};
use state::{CallContext, Phase};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};
use voice::{Voice, VoiceOperation};
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
    Close,
    More,
    Plus,
    Settings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingSend {
    id: String,
    text: String,
    sending: bool,
}

impl PendingSend {
    fn prepare(previous: Option<&Self>, draft: &str) -> Self {
        previous.map_or_else(
            || Self {
                id: uuid::Uuid::new_v4().to_string(),
                text: draft.into(),
                sending: true,
            },
            |pending| Self {
                id: pending.id.clone(),
                text: pending.text.clone(),
                sending: true,
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
    CreateSpace,
    ManageSpace,
    CreateChannel,
    ManageChannel(String),
}

#[derive(Clone)]
struct Typer {
    author: Author,
    revision: u64,
    expires: Instant,
}

struct CaperApp {
    worker: Worker,
    voice: Voice,
    generation: u64,
    loading: bool,
    loading_older: bool,
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
    sidebar_width: f32,
    navigation_open: bool,
    dialog: Option<Dialog>,
    form_name: String,
    form_private: bool,
    member_username: String,
    managed_members: Vec<Member>,
    managed_channel: Option<String>,
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
            generation: 1,
            loading: fixture.is_none(),
            loading_older: false,
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
            sidebar_width: 280.0,
            navigation_open: false,
            dialog: None,
            form_name: String::new(),
            form_private: false,
            member_username: String::new(),
            managed_members: Vec::new(),
            managed_channel: None,
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
                Event::SpaceLoaded {
                    generation,
                    space,
                    result,
                } if generation == self.generation
                    && self.selected_space.as_deref() == Some(&space) =>
                {
                    self.loading = false;
                    match result {
                        Ok(detail) => {
                            self.detail = Some(detail);
                            self.member_page = 0;
                            if let Some(channel) = self
                                .detail
                                .as_ref()
                                .and_then(|detail| detail.channels.first())
                            {
                                self.select_channel(channel.id.clone(), false);
                            }
                        }
                        Err(error) => self.error = Some(error),
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

    fn connect_gateway(&self) {
        let Some(channel) = self.selected_channel.clone() else {
            return;
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
        });
    }

    fn select_space(&mut self, id: String) {
        if self.spaces.iter().any(|space| space.id == id && space.demo) {
            self.open_general();
            return;
        }
        let Some(token) = self.token.clone() else {
            self.dialog = Some(Dialog::SignIn);
            return;
        };
        self.generation += 1;
        self.selected_space = Some(id.clone());
        self.detail = None;
        self.clear_channel_state();
        self.loading = true;
        self.navigation_open = false;
        self.worker.send(Command::LoadSpace {
            generation: self.generation,
            token,
            space: id,
        });
    }

    fn select_channel(&mut self, id: String, general: bool) {
        self.voice.state.browse(id.clone());
        self.generation += 1;
        self.selected_channel = Some(id.clone());
        self.session = None;
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
        let space = self
            .selected_space
            .as_ref()
            .filter(|space| *space != "general")
            .cloned();
        let context = CallContext {
            channel_id: channel,
            channel_name: self.channel_name().into(),
            space_name: self
                .detail
                .as_ref()
                .map_or("General", |detail| detail.space.name.as_str())
                .into(),
        };
        self.voice
            .join(context, space, self.token.clone(), self.identity_name());
    }

    fn gateway(&mut self, event: GatewayEvent) {
        match event {
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
                    self.draft.clear();
                    self.error = None;
                }
                if matches!(
                    self.timeline.apply(*message),
                    Ok(model::Apply::Resync) | Err(_)
                ) {
                    self.reload_channel();
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
                    self.draft.clear();
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
                self.pending = None;
                self.error = Some(format!(
                    "{} Edit the message before sending again.",
                    error.message
                ));
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
            self.select_channel(channel, general);
            self.pending = pending;
            self.draft = draft;
        }
    }

    fn accept_older(
        &mut self,
        requested_channel: &str,
        result: Result<model::History, worker::LoadError>,
    ) {
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
                    self.error = Some(error);
                }
            }
            Ok(_) => self.error = Some("Caper returned messages from another channel.".into()),
            Err(error) if error.access_denied => self.clear_channel(&error.message),
            Err(error) => self.error = Some(error.message),
        }
    }

    fn clear_channel(&mut self, message: &str) {
        if let Some(channel) = &self.selected_channel {
            self.voice.revoke_channel(channel);
        }
        self.clear_channel_state();
        self.error = Some(message.into());
    }

    fn clear_channel_state(&mut self) {
        self.worker.send(Command::StopGateway);
        self.selected_channel = None;
        self.session = None;
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
        if self.pending.as_ref().is_some_and(|pending| pending.sending) {
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
        self.loading = true;
        self.error = None;
        self.worker.send(Command::Admin {
            generation: self.generation,
            token,
            operation,
        });
    }

    fn admin_result(&mut self, result: AdminResult) {
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
            AdminResult::SpaceDeleted(id) => {
                self.voice.revoke_space(&id);
                self.spaces.retain(|space| space.id != id);
                self.dialog = None;
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
}

impl eframe::App for CaperApp {
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
                                egui::TextEdit::singleline(&mut self.code).char_limit(6),
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
                                        RichText::new("Join general as a guest.")
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
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(SURFACE))
            .show(context, |ui| {
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
                        if self.members_visible {
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
                    let separator = ui.interact(
                        separator_rect,
                        ui.id().with("sidebar-resize"),
                        egui::Sense::drag(),
                    );
                    if separator.dragged() {
                        self.sidebar_width =
                            (self.sidebar_width + separator.drag_delta().x).clamp(220.0, 440.0);
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
                    .stroke(Stroke::new(1.0, if active { TERRACOTTA } else { BORDER }))
                    .corner_radius(if active { 8 } else { 12 });
                    if ui
                        .with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.add(button)
                        })
                        .inner
                        .on_hover_text(name)
                        .clicked()
                    {
                        self.select_space(id);
                    }
                    ui.add_space(6.0);
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("+").size(22.0).color(TERRACOTTA_BRIGHT))
                            .min_size(egui::vec2(40.0, 40.0))
                            .fill(SURFACE)
                            .stroke(Stroke::new(1.0, BORDER))
                            .corner_radius(12),
                    )
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
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(width - 24.0);
                ui.set_height(ui.available_height());
                egui::ScrollArea::vertical()
                    .id_salt("sidebar-scroll")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                bold(
                                    self.detail
                                        .as_ref()
                                        .map_or("Caper", |detail| detail.space.name.as_str()),
                                )
                                .size(16.0),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if self.owner()
                                        && drawn_icon_button(ui, NavIcon::Chevron, "Space actions")
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
                                            .map_or_else(Vec::new, |detail| detail.members.clone());
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
                                    }
                                    if self.navigation_open
                                        && drawn_icon_button(ui, NavIcon::Close, "Close navigation")
                                            .clicked()
                                    {
                                        self.navigation_open = false;
                                    }
                                },
                            );
                        });
                        ui.separator();
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            drawn_icon(ui, NavIcon::Chevron, egui::vec2(18.0, 28.0), MUTED);
                            ui.label(bold("Channels").size(12.0).color(MUTED));
                            let count = self
                                .detail
                                .as_ref()
                                .map_or(0, |detail| detail.channels.len());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if self.owner()
                                        && drawn_icon_button(ui, NavIcon::More, "Channel options")
                                            .clicked()
                                    {
                                        self.form_name.clear();
                                        self.form_private = false;
                                        self.dialog = Some(Dialog::CreateChannel);
                                    }
                                    if self.owner()
                                        && drawn_icon_button(ui, NavIcon::Plus, "Create channel")
                                            .clicked()
                                    {
                                        self.form_name.clear();
                                        self.form_private = false;
                                        self.dialog = Some(Dialog::CreateChannel);
                                    }
                                    ui.label(
                                        RichText::new(count.to_string()).size(10.0).color(MUTED),
                                    );
                                },
                            );
                        });
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
                            }
                        }
                        if !self.voice.participants.is_empty() {
                            ui.add_space(14.0);
                            ui.label(
                                RichText::new(format!(
                                    "IN VOICE · {}",
                                    self.voice.participants.len()
                                ))
                                .size(11.0)
                                .color(MUTED),
                            );
                            for participant in &self.voice.participants {
                                ui.horizontal(|ui| {
                                    avatar(ui, &participant.name, 28.0, false);
                                    ui.label(
                                        bold(if participant.id == self.voice.self_id {
                                            format!("{} (you)", participant.name)
                                        } else {
                                            participant.name.clone()
                                        })
                                        .size(12.0),
                                    );
                                    if participant.deafened {
                                        ui.label(RichText::new("Deafened").size(10.0).color(MUTED));
                                    } else if participant.muted {
                                        ui.label(RichText::new("Muted").size(10.0).color(MUTED));
                                    }
                                });
                            }
                        }
                    });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    egui::Frame::new()
                        .fill(RAISED)
                        .stroke(Stroke::new(1.0, BORDER))
                        .corner_radius(6)
                        .inner_margin(4)
                        .show(ui, |ui| {
                            ui.set_width(width - 34.0);
                            ui.horizontal(|ui| {
                                avatar(ui, &self.identity_name(), 30.0, false);
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(self.identity_name()).strong(),
                                        )
                                        .frame(false),
                                    )
                                    .clicked()
                                {
                                    self.dialog = Some(if self.account.is_some() {
                                        Dialog::Profile
                                    } else {
                                        Dialog::SignIn
                                    });
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if drawn_icon_button(ui, NavIcon::Settings, "User settings")
                                            .clicked()
                                        {
                                            if self.account.is_some() {
                                                self.dialog = Some(Dialog::Profile);
                                            } else {
                                                self.dialog = Some(Dialog::SignIn);
                                            }
                                        }
                                    },
                                );
                            });
                        });
                    ui.add_space(8.0);
                    if let Phase::Joining(context)
                    | Phase::Connected(context)
                    | Phase::Reconnecting(context) = &self.voice.state.phase
                    {
                        let label = format!("{} / {}", context.space_name, context.channel_name);
                        let connected = matches!(self.voice.state.phase, Phase::Connected(_));
                        egui::Frame::new()
                            .fill(RAISED)
                            .stroke(Stroke::new(1.0, BORDER))
                            .inner_margin(8)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            bold(if connected {
                                                "Voice connected"
                                            } else {
                                                "Connecting voice…"
                                            })
                                            .size(12.0),
                                        );
                                        ui.label(RichText::new(label).size(11.0).color(MUTED));
                                    });
                                    if ui.button("Leave").clicked() {
                                        self.voice.leave();
                                    }
                                });
                            });
                        ui.add_space(8.0);
                    }
                    ui.horizontal(|ui| {
                        if ui
                            .add_sized(
                                [76.0, 32.0],
                                egui::Button::new(if self.voice.state.audio.muted {
                                    "Unmute"
                                } else {
                                    "Mute"
                                })
                                .fill(RAISED)
                                .stroke(Stroke::new(1.0, BORDER))
                                .corner_radius(8),
                            )
                            .clicked()
                        {
                            self.voice
                                .command(VoiceOperation::Mute(!self.voice.state.audio.muted));
                        }
                        if ui
                            .add_sized(
                                [84.0, 32.0],
                                egui::Button::new(if self.voice.state.audio.deafened {
                                    "Undeafen"
                                } else {
                                    "Deafen"
                                })
                                .fill(RAISED)
                                .stroke(Stroke::new(1.0, BORDER))
                                .corner_radius(8),
                            )
                            .clicked()
                        {
                            self.voice
                                .command(VoiceOperation::Deafen(!self.voice.state.audio.deafened));
                        }
                        ui.style_mut().spacing.interact_size.y = 32.0;
                        ui.menu_button("Audio", |ui| {
                            ui.label(bold("Microphone"));
                            for (guid, name) in self.voice.inputs.clone() {
                                if ui
                                    .selectable_label(
                                        self.voice.input.as_deref() == Some(&guid),
                                        name,
                                    )
                                    .clicked()
                                {
                                    self.voice.command(VoiceOperation::Input(guid));
                                    ui.close();
                                }
                            }
                            if self.voice.inputs.is_empty() {
                                ui.label("Available after joining voice");
                            }
                            ui.separator();
                            ui.label(bold("Speaker"));
                            for (guid, name) in self.voice.outputs.clone() {
                                if ui
                                    .selectable_label(
                                        self.voice.output.as_deref() == Some(&guid),
                                        name,
                                    )
                                    .clicked()
                                {
                                    self.voice.command(VoiceOperation::Output(guid));
                                    ui.close();
                                }
                            }
                            if self.voice.outputs.is_empty() {
                                ui.label("Available after joining voice");
                            }
                        });
                    });
                    if let Some(error) = &self.voice.error {
                        ui.colored_label(ERROR, error);
                    }
                    ui.add_space(8.0);
                });
            });
    }

    fn member_presence(&mut self, ui: &mut egui::Ui) {
        let Some(detail) = &self.detail else { return };
        let member_count = detail.members.len();
        let members = detail.members.clone();
        let demo = detail.space.demo;
        egui::Frame::new()
            .fill(SIDEBAR)
            .stroke(Stroke::new(1.0, BORDER))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
        ui.set_min_size(ui.available_size());
        ui.horizontal(|ui| {
            ui.label(RichText::new("Members").size(12.0).strong().color(MUTED));
            if !demo {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(member_count.to_string()).size(10.0).color(MUTED));
                });
            }
        });
        ui.separator();
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
            ui.horizontal(|ui| {
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
            egui::TopBottomPanel::top("chat-heading")
                .exact_height(52.0)
                .frame(
                    egui::Frame::new()
                        .fill(CONVERSATION)
                        .stroke(Stroke::new(0.0, BORDER))
                        .inner_margin(egui::Margin::symmetric(18, 8)),
                )
                .show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        if narrow && ui.button("☰  Browse").clicked() {
                            self.navigation_open = true;
                        }
                        ui.heading(RichText::new(format!("# {}", self.channel_name())).size(14.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self
                                .detail
                                .as_ref()
                                .is_some_and(|detail| !detail.members.is_empty())
                                && users_button(ui, self.members_visible).clicked()
                            {
                                self.members_visible = !self.members_visible;
                            }
                            ui.label(
                                RichText::new(&self.live)
                                    .size(11.0)
                                    .color(if self.live == "Live" { CAPER } else { MUTED }),
                            );
                            let already_here = self.voice.state.active_channel()
                                == self.selected_channel.as_deref();
                            if !already_here
                                && self.selected_channel.is_some()
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("Join voice").strong().color(TEXT),
                                        )
                                        .fill(TERRACOTTA)
                                        .corner_radius(8),
                                    )
                                    .clicked()
                            {
                                self.join_voice();
                            }
                        });
                    });
                });
            egui::TopBottomPanel::bottom("composer")
                .frame(
                    egui::Frame::new()
                        .fill(CONVERSATION)
                        .stroke(Stroke::new(1.0, BORDER))
                        .inner_margin(egui::Margin::symmetric(18, 12)),
                )
                .show_inside(ui, |ui| {
                    if let Some(error) = &self.error {
                        ui.colored_label(ERROR, error);
                    }
                    let before = self.draft.clone();
                    let channel_name = self.channel_name().to_owned();
                    let response = ui.add_sized(
                        [ui.available_width(), 44.0],
                        egui::TextEdit::multiline(&mut self.draft)
                            .desired_rows(1)
                            .hint_text(format!("Message #{channel_name}"))
                            .background_color(COMPOSER)
                            .char_limit(4_000),
                    );
                    if self.draft != before {
                        self.typing_edited = Instant::now();
                    }
                    let send = response.has_focus()
                        && ui.input(|input| {
                            input.key_pressed(egui::Key::Enter) && !input.modifiers.shift
                        });
                    if send {
                        let trimmed = self.draft.trim_end_matches('\n').to_owned();
                        self.draft = trimmed;
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
            egui::TopBottomPanel::bottom("typing")
                .exact_height(24.0)
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
                                } else {
                                    "Load older messages"
                                }),
                            )
                            .clicked()
                    {
                        self.load_older();
                    } else if !self.has_more {
                        ui.vertical_centered(|ui| {
                            ui.label(
                                RichText::new("Beginning of conversation")
                                    .size(11.0)
                                    .color(MUTED),
                            );
                        });
                        ui.add_space(24.0);
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
                    if let Some(pending) = &self.pending {
                        let author = self.session.as_ref().map_or_else(
                            || self.identity_name(),
                            |session| session.author.name.clone(),
                        );
                        message_row(ui, &author, "Now", &pending.text, false, true);
                        if !pending.sending {
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
            Dialog::CreateSpace => "Create a space",
            Dialog::ManageSpace => "Manage space",
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
        let mut close = context.input(|input| input.key_pressed(egui::Key::Escape));
        egui::Area::new(egui::Id::new("caper-dialog"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(context, |ui| {
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
                                            Dialog::CreateSpace => self.space_dialog(ui, false),
                                            Dialog::ManageSpace => self.space_dialog(ui, true),
                                            Dialog::CreateChannel => self.channel_dialog(ui, None),
                                            Dialog::ManageChannel(id) => self.channel_dialog(ui, Some(id)),
                                        }
                                        notices(ui, &self.error, &self.warning);
                                    });
                            });
                    });
            });
        if close {
            self.dialog = None;
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
            egui::TextEdit::singleline(&mut self.username),
        );
        ui.label("Display name");
        ui.add_sized(
            [ui.available_width(), 42.0],
            egui::TextEdit::singleline(&mut self.display_name),
        );
        if primary(
            ui,
            if self.loading {
                "Saving…"
            } else {
                "Save profile"
            },
            self.loading,
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
            egui::TextEdit::singleline(&mut self.form_name).char_limit(80),
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
                self.admin(AdminOperation::DeleteSpace { space });
            }
        }
    }

    fn channel_dialog(&mut self, ui: &mut egui::Ui, channel: Option<String>) {
        ui.label("Channel name");
        ui.add_sized(
            [ui.available_width(), 42.0],
            egui::TextEdit::singleline(&mut self.form_name).char_limit(80),
        );
        self.form_name = normalize_channel(&self.form_name);
        ui.checkbox(&mut self.form_private, "Private channel");
        ui.label(
            RichText::new(if self.form_private {
                "Only explicitly granted space members can open this channel."
            } else {
                "Everyone in the space can open this channel."
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
                self.admin(AdminOperation::DeleteChannel { space, channel });
            }
        }
    }

    fn members_dialog(&mut self, ui: &mut egui::Ui, channel: Option<String>) {
        ui.horizontal(|ui| {
            ui.add_sized(
                [(ui.available_width() - 80.0).max(1.0), 38.0],
                egui::TextEdit::singleline(&mut self.member_username).hint_text("username"),
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
                                RichText::new(format!("@{}", member.username))
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

fn drawn_icon(ui: &mut egui::Ui, icon: NavIcon, size: egui::Vec2, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    paint_icon(ui.painter(), rect, icon, color);
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
    response.on_hover_text(label)
}

fn paint_icon(painter: &egui::Painter, rect: egui::Rect, icon: NavIcon, color: Color32) {
    let center = rect.center();
    let stroke = Stroke::new(1.7, color);
    match icon {
        NavIcon::Chevron => {
            painter.line_segment([egui::pos2(center.x - 4.0, center.y - 2.0), center], stroke);
            painter.line_segment([center, egui::pos2(center.x + 4.0, center.y - 2.0)], stroke);
        }
        NavIcon::Close => {
            painter.line_segment(
                [
                    egui::pos2(center.x - 4.0, center.y - 4.0),
                    egui::pos2(center.x + 4.0, center.y + 4.0),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x + 4.0, center.y - 4.0),
                    egui::pos2(center.x - 4.0, center.y + 4.0),
                ],
                stroke,
            );
        }
        NavIcon::Plus => {
            painter.line_segment(
                [
                    egui::pos2(center.x - 5.0, center.y),
                    egui::pos2(center.x + 5.0, center.y),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x, center.y - 5.0),
                    egui::pos2(center.x, center.y + 5.0),
                ],
                stroke,
            );
        }
        NavIcon::More => {
            for x in [-5.0, 0.0, 5.0] {
                painter.circle_filled(egui::pos2(center.x + x, center.y), 1.4, color);
            }
        }
        NavIcon::Settings => {
            painter.circle_stroke(center, 5.0, stroke);
            painter.circle_stroke(center, 1.7, stroke);
            for angle in [
                0.0_f32,
                std::f32::consts::FRAC_PI_2,
                std::f32::consts::PI,
                std::f32::consts::PI + std::f32::consts::FRAC_PI_2,
            ] {
                let direction = egui::vec2(angle.cos(), angle.sin());
                painter.line_segment([center + direction * 5.0, center + direction * 7.0], stroke);
            }
        }
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
    let icon_center = egui::pos2(rect.left() + 15.0, rect.center().y);
    if private {
        let body = egui::Rect::from_center_size(
            egui::pos2(icon_center.x, icon_center.y + 2.0),
            egui::vec2(10.0, 9.0),
        );
        ui.painter()
            .rect_stroke(body, 2.0, Stroke::new(1.5, color), egui::StrokeKind::Middle);
        ui.painter().line_segment(
            [
                egui::pos2(icon_center.x - 4.0, icon_center.y - 2.0),
                egui::pos2(icon_center.x - 4.0, icon_center.y - 6.0),
            ],
            Stroke::new(1.5, color),
        );
        ui.painter().line_segment(
            [
                egui::pos2(icon_center.x - 4.0, icon_center.y - 6.0),
                egui::pos2(icon_center.x + 4.0, icon_center.y - 6.0),
            ],
            Stroke::new(1.5, color),
        );
        ui.painter().line_segment(
            [
                egui::pos2(icon_center.x + 4.0, icon_center.y - 6.0),
                egui::pos2(icon_center.x + 4.0, icon_center.y - 2.0),
            ],
            Stroke::new(1.5, color),
        );
    } else {
        ui.painter().text(
            icon_center,
            egui::Align2::CENTER_CENTER,
            "#",
            egui::FontId::new(17.0, egui::FontFamily::Name("Satoshi Medium".into())),
            if active { TERRACOTTA_BRIGHT } else { color },
        );
    }
    ui.painter().text(
        egui::pos2(rect.left() + 31.0, rect.center().y - 1.0),
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
    let center = rect.center();
    ui.painter().circle_stroke(
        egui::pos2(center.x - 3.0, center.y - 4.0),
        3.0,
        Stroke::new(1.5, color),
    );
    ui.painter().circle_stroke(
        egui::pos2(center.x + 5.0, center.y - 2.5),
        2.4,
        Stroke::new(1.4, color),
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x - 9.0, center.y + 6.0),
            egui::pos2(center.x - 6.0, center.y + 2.0),
        ],
        Stroke::new(1.5, color),
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x - 6.0, center.y + 2.0),
            egui::pos2(center.x, center.y + 2.0),
        ],
        Stroke::new(1.5, color),
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x, center.y + 2.0),
            egui::pos2(center.x + 3.0, center.y + 6.0),
        ],
        Stroke::new(1.5, color),
    );
    ui.painter().line_segment(
        [
            egui::pos2(center.x + 3.0, center.y + 2.5),
            egui::pos2(center.x + 9.0, center.y + 6.0),
        ],
        Stroke::new(1.4, color),
    );
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
        .inner_margin(egui::Margin::symmetric(18, 6))
        .show(ui, |ui| {
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
    let viewport_size = if fixture
        .as_deref()
        .is_some_and(|name| matches!(name, "parity-narrow" | "parity-browse"))
    {
        [390.0, 844.0]
    } else {
        [1440.0, 900.0]
    };
    eframe::run_native(
        "Caper",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Wgpu,
            viewport: egui::ViewportBuilder::default()
                .with_inner_size(viewport_size)
                .with_min_inner_size([320.0, 560.0]),
            ..Default::default()
        },
        Box::new(move |creation| {
            Ok(Box::new(CaperApp::new(
                &creation.egui_ctx,
                api,
                fixture.as_deref(),
            )))
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CaperApp, Dialog, PendingSend, endpoint, member_page_ids, normalize_channel,
        permanent_send_rejection,
    };
    use crate::model::{
        Account, Author, ChatSession, Content, History, HistoryPlace, Member, Message, Space,
        SpaceDetail, Spaces,
    };
    use crate::worker::LoadError;

    fn account(profile: bool) -> Account {
        Account {
            id: "account".into(),
            username: profile.then(|| "member".into()),
            display_name: profile.then(|| "Member".into()),
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
        assert!(app.pending.is_none());
        assert!(app.draft.is_empty());
    }
}
