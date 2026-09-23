mod api;
mod credentials;
mod gateway;
mod model;
mod worker;

use eframe::egui::{self, Color32, RichText, Stroke};
use gateway::GatewayEvent;
use model::{Account, ChatSession, SpaceDetail, Spaces, Timeline};
use worker::{Command, Event, Worker, current};

const BLACKOUT: Color32 = Color32::from_rgb(12, 13, 15);
const SURFACE: Color32 = Color32::from_rgb(21, 23, 25);
const RAISED: Color32 = Color32::from_rgb(28, 31, 33);
const BORDER: Color32 = Color32::from_rgb(52, 56, 59);
const TEXT: Color32 = Color32::from_rgb(243, 244, 245);
const MUTED: Color32 = Color32::from_rgb(185, 188, 190);
const TERRACOTTA: Color32 = Color32::from_rgb(182, 77, 50);
const CAPER: Color32 = Color32::from_rgb(99, 122, 67);

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

struct CaperApp {
    worker: Worker,
    generation: u64,
    loading: bool,
    error: Option<String>,
    warning: Option<String>,
    token: Option<String>,
    account: Option<Account>,
    spaces: Vec<model::Space>,
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
    fixture: Option<String>,
}

impl CaperApp {
    fn new(context: &egui::Context, api: api::Api, fixture: Option<&str>) -> Self {
        configure(context);
        let worker = Worker::new(api, context.clone());
        let mut app = Self {
            worker,
            generation: 1,
            loading: fixture.is_none(),
            error: None,
            warning: None,
            token: None,
            account: None,
            spaces: Vec::new(),
            selected_space: None,
            detail: None,
            selected_channel: None,
            session: None,
            timeline: Timeline::default(),
            live: "Offline".into(),
            email: String::new(),
            challenge: None,
            code: String::new(),
            username: String::new(),
            display_name: String::new(),
            draft: String::new(),
            pending: None,
            fixture: fixture.map(str::to_owned),
        };
        match fixture {
            Some("error") => {
                app.error =
                    Some("Could not reach Caper. Check your connection and try again.".into())
            }
            Some(_) => {}
            None => app.worker.send(Command::Restore {
                generation: app.generation,
            }),
        }
        app
    }

    fn receive(&mut self) {
        let events: Vec<_> = self.worker.events.try_iter().collect();
        for event in events {
            match event {
                Event::Restored { generation, result }
                    if current(generation, self.generation, None, None) =>
                {
                    self.loading = false;
                    match result {
                        Ok(Some((token, account, spaces))) => {
                            self.establish(token, account, spaces)
                        }
                        Ok(None) => {}
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::CodeRequested { generation, result }
                    if current(generation, self.generation, None, None) =>
                {
                    self.loading = false;
                    match result {
                        Ok(challenge) => {
                            self.challenge = Some(challenge);
                            self.code.clear();
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::Verified { generation, result }
                    if current(generation, self.generation, None, None) =>
                {
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
                Event::Profiled { generation, result }
                    if current(generation, self.generation, None, None) =>
                {
                    self.loading = false;
                    match result {
                        Ok((account, spaces)) => {
                            self.account = Some(account);
                            self.set_spaces(spaces);
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::SpaceLoaded {
                    generation,
                    space,
                    result,
                } if current(generation, self.generation, None, None)
                    && self.selected_space.as_deref() == Some(&space) =>
                {
                    self.loading = false;
                    match result {
                        Ok(detail) => self.detail = Some(detail),
                        Err(error) => self.error = Some(error),
                    }
                }
                Event::ChannelLoaded {
                    generation,
                    channel,
                    result,
                } if current(
                    generation,
                    self.generation,
                    Some(&channel),
                    self.selected_channel.as_deref(),
                ) =>
                {
                    self.loading = false;
                    match result {
                        Ok((history, session)) => {
                            if history.channel.id != channel
                                || history
                                    .messages
                                    .iter()
                                    .any(|message| message.channel_id != channel)
                            {
                                self.clear_channel("Caper returned messages from another channel.");
                            } else if let Err(error) =
                                self.timeline.reset(history.messages, &history.cursor)
                            {
                                self.clear_channel(&error);
                            } else {
                                self.session = Some(session);
                                self.live = "Connecting…".into();
                                self.worker.send(Command::Connect {
                                    generation: self.generation,
                                    token: self.token.clone().unwrap_or_default(),
                                    channel,
                                    cursor: self.timeline.cursor(),
                                });
                            }
                        }
                        Err(error) => {
                            if error.access_denied {
                                self.clear_channel(&error.message);
                            } else {
                                self.error = Some(error.message);
                            }
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
                    self.generation,
                    Some(&channel),
                    self.selected_channel.as_deref(),
                ) && self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.id == client_id) =>
                {
                    match result {
                        Ok(message) => {
                            // HTTP proves persistence but never advances the replay cursor.
                            let confirmed = message.channel_id == channel
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
                        Err(error) => {
                            if matches!(error.status, Some(401 | 403)) {
                                self.clear_channel("Your messaging session expired. Select the channel to reconnect.");
                            } else if permanent_send_rejection(error.status) {
                                self.pending = None;
                                self.error = Some(format!(
                                    "{} Edit the message before sending again.",
                                    error.message
                                ));
                            } else {
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
                }
                Event::Credential {
                    generation,
                    result: Err(error),
                } if current(generation, self.generation, None, None) => {
                    self.warning = Some(error);
                }
                Event::LoggedOut {
                    generation,
                    result: Err(error),
                } if current(generation, self.generation, None, None) => {
                    self.error = Some(format!(
                        "Signed out locally, but server revocation failed: {error}"
                    ));
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
        self.account = Some(account);
        self.error = None;
        self.set_spaces(spaces);
    }

    fn set_spaces(&mut self, spaces: Spaces) {
        self.spaces = spaces.spaces;
        if self.selected_space.is_none()
            && let Some(space) = self.spaces.first()
        {
            self.select_space(space.id.clone());
        }
    }

    fn select_space(&mut self, id: String) {
        self.generation += 1;
        self.selected_space = Some(id.clone());
        self.detail = None;
        self.clear_channel_state();
        self.loading = true;
        self.worker.send(Command::LoadSpace {
            generation: self.generation,
            token: self.token.clone().unwrap_or_default(),
            space: id,
        });
    }

    fn select_channel(&mut self, id: String) {
        self.generation += 1;
        self.selected_channel = Some(id.clone());
        self.session = None;
        self.timeline = Timeline::default();
        self.pending = None;
        self.draft.clear();
        self.error = None;
        self.loading = true;
        self.worker.send(Command::StopGateway);
        let name = self
            .account
            .as_ref()
            .and_then(|account| account.display_name.clone())
            .unwrap_or_else(|| "Caper member".into());
        self.worker.send(Command::LoadChannel {
            generation: self.generation,
            token: self.token.clone().unwrap_or_default(),
            channel: id,
            name,
        });
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
                self.live = if online { "Live".into() } else { detail };
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
                if message.channel_id != channel || message.validate().is_err() {
                    self.reload_channel();
                    return;
                }
                if self.pending.as_ref().is_some_and(|pending| {
                    self.session
                        .as_ref()
                        .is_some_and(|session| pending.confirmed_by(&message, &session.author.id))
                }) {
                    self.pending = None;
                    self.draft.clear();
                    self.error = None;
                }
                match self.timeline.apply(*message) {
                    Ok(model::Apply::Resync) | Err(_) => self.reload_channel(),
                    _ => {}
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
                self.clear_channel(&detail);
            }
            _ => {}
        }
    }

    fn reload_channel(&mut self) {
        if let Some(channel) = self.selected_channel.clone() {
            let mut pending = self.pending.take();
            let draft = std::mem::take(&mut self.draft);
            if let Some(pending) = &mut pending {
                pending.sending = false;
            }
            self.select_channel(channel);
            self.pending = pending;
            self.draft = draft;
        }
    }

    fn clear_channel(&mut self, message: &str) {
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
        self.live = "Offline".into();
    }

    fn logout(&mut self) {
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
        self.selected_space = None;
        self.detail = None;
        self.clear_channel_state();
        self.error = None;
        self.warning = None;
        self.challenge = None;
        self.loading = false;
        self.code.clear();
        self.email.clear();
        self.username.clear();
        self.display_name.clear();
    }

    fn send_message(&mut self) {
        if self.session.is_none()
            || self.selected_channel.is_none()
            || self.pending.as_ref().is_some_and(|pending| pending.sending)
        {
            return;
        }
        let text = self
            .pending
            .as_ref()
            .map_or_else(|| self.draft.clone(), |pending| pending.text.clone());
        let count = text.chars().count();
        if text.trim().is_empty() || count > 4_000 {
            self.error = Some(
                if count > 4_000 {
                    "Messages can be at most 4,000 characters."
                } else {
                    "Write a message first."
                }
                .into(),
            );
            return;
        }
        let pending = PendingSend::prepare(self.pending.as_ref(), &text);
        let id = pending.id.clone();
        self.pending = Some(pending);
        self.error = None;
        self.worker.send(Command::Send {
            generation: self.generation,
            token: self.token.clone().unwrap_or_default(),
            chat_token: self
                .session
                .as_ref()
                .map_or_else(String::new, |session| session.token.clone()),
            channel: self.selected_channel.clone().unwrap_or_default(),
            client_id: id,
            text,
        });
    }
}

impl eframe::App for CaperApp {
    fn update(&mut self, context: &egui::Context, _: &mut eframe::Frame) {
        if let Some(fixture) = &self.fixture {
            egui::TopBottomPanel::top("fixture-label").show(context, |ui| {
                ui.label(format!(
                    "TEST FIXTURE: {fixture} — no live conversation data"
                ));
            });
        }
        if context.input(|input| !input.events.is_empty()) {
            self.worker.send(Command::Activity);
        }
        self.receive();
        if self.account.is_none() {
            self.login_ui(context);
        } else if self
            .account
            .as_ref()
            .is_some_and(|account| account.username.is_none())
        {
            self.profile_ui(context);
        } else {
            self.main_ui(context);
        }
    }
}

impl CaperApp {
    fn login_ui(&mut self, context: &egui::Context) {
        egui::CentralPanel::default().frame(panel(BLACKOUT)).show(context, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(56.0); ui.label(RichText::new("caper.").size(30.0).strong().color(TEXT));
                ui.add_space(44.0); ui.label(RichText::new("WELCOME TO CAPER").size(11.0).strong().color(MUTED));
                ui.heading(if self.challenge.is_some() { "Check your email." } else { "Come on in." });
                ui.set_max_width(430.0);
                if self.challenge.is_some() {
                    ui.label(RichText::new("Enter the six-character code. It expires in 10 minutes.").color(MUTED));
                    ui.add_space(16.0); ui.label("Sign-in code");
                    let response = ui.add_sized([430.0, 42.0], egui::TextEdit::singleline(&mut self.code).char_limit(6));
                    self.code.make_ascii_uppercase(); self.code.retain(|character| "ABCDEFGHJKMNPQRSTWXYZ23456789".contains(character));
                    if primary(ui, if self.loading { "Checking…" } else { "Continue" }, self.loading || self.code.len() != 6).clicked() || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))) { self.verify(); }
                    if ui.button("Use a different email").clicked() { self.generation += 1; self.loading = false; self.challenge = None; self.error = None; }
                } else {
                    ui.label(RichText::new("Use your email to create an account or return to one. No password needed.").color(MUTED));
                    ui.add_space(16.0); ui.label("Email address");
                    let response = ui.add_sized([430.0, 42.0], egui::TextEdit::singleline(&mut self.email).hint_text("you@example.com"));
                    if primary(ui, if self.loading { "Sending…" } else { "Email me a code" }, self.loading).clicked() || (response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter))) { self.request_code(); }
                }
                notices(ui, &self.error, &self.warning);
            });
        });
    }

    fn profile_ui(&mut self, context: &egui::Context) {
        egui::CentralPanel::default()
            .frame(panel(BLACKOUT))
            .show(context, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(70.0);
                    ui.label(RichText::new("caper.").size(30.0).strong());
                    ui.heading("Make yourself at home.");
                    ui.set_max_width(430.0);
                    ui.label(RichText::new("Choose how people will recognize you.").color(MUTED));
                    ui.add_space(14.0);
                    ui.label("Username");
                    ui.add_sized(
                        [430.0, 42.0],
                        egui::TextEdit::singleline(&mut self.username).hint_text("lowercase_name"),
                    );
                    ui.label("Display name");
                    ui.add_sized(
                        [430.0, 42.0],
                        egui::TextEdit::singleline(&mut self.display_name),
                    );
                    if primary(ui, "Continue", self.loading).clicked() {
                        self.loading = true;
                        self.error = None;
                        self.worker.send(Command::Profile {
                            generation: self.generation,
                            token: self.token.clone().unwrap_or_default(),
                            username: self.username.trim().to_ascii_lowercase(),
                            display_name: self.display_name.trim().into(),
                        });
                    }
                    notices(ui, &self.error, &self.warning);
                })
            });
    }

    fn main_ui(&mut self, context: &egui::Context) {
        egui::SidePanel::left("spaces")
            .exact_width(86.0)
            .frame(panel(BLACKOUT))
            .show(context, |ui| {
                ui.label(RichText::new("c.").size(25.0).strong());
                ui.add_space(20.0);
                let spaces: Vec<_> = self
                    .spaces
                    .iter()
                    .map(|space| (space.id.clone(), space.name.clone()))
                    .collect();
                for (id, name) in spaces {
                    if ui
                        .selectable_label(
                            self.selected_space.as_deref() == Some(&id),
                            RichText::new(name.chars().next().unwrap_or('C')).size(18.0),
                        )
                        .on_hover_text(name)
                        .clicked()
                    {
                        self.select_space(id);
                    }
                }
            });
        egui::SidePanel::left("channels")
            .exact_width(220.0)
            .frame(panel(SURFACE))
            .show(context, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(
                        self.detail
                            .as_ref()
                            .map_or("Spaces", |detail| detail.space.name.as_str()),
                    );
                });
                ui.separator();
                ui.label(
                    RichText::new("TEXT CHANNELS")
                        .size(10.0)
                        .strong()
                        .color(MUTED),
                );
                let channels: Vec<_> = self.detail.as_ref().map_or_else(Vec::new, |detail| {
                    detail
                        .channels
                        .iter()
                        .map(|channel| (channel.id.clone(), channel.name.clone(), channel.private))
                        .collect()
                });
                for (id, name, private) in channels {
                    let label = format!("{} {name}", if private { "▣" } else { "#" });
                    if ui
                        .selectable_label(self.selected_channel.as_deref() == Some(&id), label)
                        .clicked()
                    {
                        self.select_channel(id);
                    }
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    if ui.button("Log out").clicked() {
                        self.logout();
                        return;
                    }
                    let account = self.account.as_ref().unwrap();
                    ui.label(
                        RichText::new(account.display_name.as_deref().unwrap_or("Caper member"))
                            .strong(),
                    );
                    ui.label(
                        RichText::new(
                            account
                                .username
                                .as_deref()
                                .map_or(String::new(), |name| format!("@{name}")),
                        )
                        .size(11.0)
                        .color(MUTED),
                    );
                });
            });
        egui::CentralPanel::default()
            .frame(panel(Color32::from_rgb(25, 33, 35)))
            .show(context, |ui| {
                notices(ui, &self.error, &self.warning);
                if let Some(channel) = self.selected_channel.clone() {
                    let channel_name = self.channel_name(&channel).to_owned();
                    ui.horizontal(|ui| {
                        ui.heading(format!("# {channel_name}"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(&self.live).color(if self.live == "Live" {
                                CAPER
                            } else {
                                MUTED
                            }));
                        });
                    });
                    ui.separator();
                    egui::TopBottomPanel::bottom("composer").show_inside(ui, |ui| {
                        ui.horizontal(|ui| {
                            let response = ui.add_sized(
                                [ui.available_width() - 82.0, 42.0],
                                egui::TextEdit::singleline(&mut self.draft)
                                    .hint_text(format!("Message #{channel_name}")),
                            );
                            let sending =
                                self.pending.as_ref().is_some_and(|pending| pending.sending);
                            if ui
                                .add_enabled(
                                    !sending && self.session.is_some(),
                                    egui::Button::new(if self.pending.is_some() {
                                        "Retry"
                                    } else {
                                        "Send"
                                    })
                                    .fill(TERRACOTTA),
                                )
                                .clicked()
                                || (response.lost_focus()
                                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                                    && !sending)
                            {
                                self.send_message();
                            }
                        });
                    });
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for message in self.timeline.messages() {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(RichText::new(&message.author.name).strong());
                                    ui.label(
                                        RichText::new(&message.created_at).size(10.0).color(MUTED),
                                    );
                                });
                                ui.label(&message.content.text);
                                ui.add_space(10.0);
                            }
                            if self.loading {
                                ui.spinner();
                            }
                        });
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new("Choose a channel to join the conversation.")
                                .color(MUTED),
                        )
                    });
                }
            });
    }

    fn channel_name(&self, id: &str) -> &str {
        self.detail
            .as_ref()
            .and_then(|detail| detail.channels.iter().find(|channel| channel.id == id))
            .map_or("channel", |channel| channel.name.as_str())
    }

    fn request_code(&mut self) {
        if self.loading || !self.email.contains('@') {
            return;
        }
        self.loading = true;
        self.error = None;
        self.worker.send(Command::RequestCode {
            generation: self.generation,
            email: self.email.trim().into(),
        });
    }

    fn verify(&mut self) {
        let Some(challenge) = self.challenge.clone() else {
            return;
        };
        if self.loading || self.code.len() != 6 {
            return;
        }
        self.loading = true;
        self.error = None;
        self.worker.send(Command::VerifyCode {
            generation: self.generation,
            challenge,
            code: self.code.clone(),
        });
    }
}

fn configure(context: &egui::Context) {
    let mut style = (*context.style()).clone();
    style.visuals.dark_mode = true;
    style.visuals.panel_fill = BLACKOUT;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = RAISED;
    style.visuals.widgets.inactive.bg_fill = RAISED;
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, TERRACOTTA);
    style.visuals.selection.bg_fill = TERRACOTTA;
    style.visuals.override_text_color = Some(TEXT);
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    context.set_style(style);
}

fn panel(color: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(color)
        .inner_margin(egui::Margin::same(18))
}
fn primary(ui: &mut egui::Ui, text: &str, disabled: bool) -> egui::Response {
    ui.add_enabled(
        !disabled,
        egui::Button::new(text)
            .fill(TERRACOTTA)
            .min_size(egui::vec2(430.0, 44.0)),
    )
}
fn notices(ui: &mut egui::Ui, error: &Option<String>, warning: &Option<String>) {
    if let Some(error) = error {
        ui.add_space(8.0);
        ui.colored_label(Color32::from_rgb(255, 155, 130), error);
    }
    if let Some(warning) = warning {
        ui.add_space(8.0);
        ui.colored_label(Color32::from_rgb(232, 189, 113), warning);
    }
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let fixture = args
        .windows(2)
        .find(|pair| pair[0] == "--fixture")
        .map(|pair| pair[1].clone());
    let endpoint = args
        .windows(2)
        .find(|pair| pair[0] == "--api-url")
        .map(|pair| pair[1].clone())
        .or_else(|| std::env::var("CAPER_API_URL").ok())
        .unwrap_or_else(|| "https://caper.chat".into());
    let api = api::Api::new(&endpoint).unwrap_or_else(|message| {
        eprintln!("Caper could not start: {message}");
        std::process::exit(2)
    });
    eframe::run_native(
        "Caper",
        eframe::NativeOptions {
            renderer: eframe::Renderer::Wgpu,
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1180.0, 760.0])
                .with_min_inner_size([820.0, 560.0]),
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
    use super::{CaperApp, PendingSend, permanent_send_rejection};
    use crate::model::{Author, Content, Message};

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
        assert!(
            !permanent_send_rejection(None),
            "timeout is an unknown outcome"
        );
        assert!(
            !permanent_send_rejection(Some(503)),
            "availability failure is uncertain"
        );
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
        app.select_channel("two".into());
        assert!(app.pending.is_none());
        assert!(app.draft.is_empty());
    }
}
