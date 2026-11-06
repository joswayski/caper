//! Voice, as on the egui desktop: who is in voice under each channel, Join
//! (or Switch) in the conversation header, the call card above the account
//! bar with the call's people, connection progress, session timer, Mute,
//! Deafen and Leave, and the account bar's microphone and deafen toggles.
//! The call itself is `caper_client::voice::Voice`. A child of `app`.

use super::{App, State, clear};
use crate::images::{Avatar, AvatarView, Icon};
use crate::ui::label;
use caper_client::effects::Effect;
use caper_client::gateway::{GatewayEvent, MAX_MEDIA_CHANNELS, MediaChannel, MediaWatch};
use caper_client::model::VoiceOccupant;
use caper_client::state::{CallContext, Phase};
use caper_client::voice::{self, Preferences, VoiceOperation};
use caper_client::worker::{Command, LoadError};
use gtk::{glib, prelude::*};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How often the call card redraws speaking rings and its timer.
const TICK: Duration = Duration::from_millis(250);

/// Voice rosters and availability for the open space.
#[derive(Default)]
pub(super) struct Calls {
    rosters: HashMap<String, Vec<VoiceOccupant>>,
    starts: HashMap<String, u64>,
    unavailable: HashSet<String>,
    /// `GET …/media/status` per channel.
    available: HashMap<String, bool>,
    checked: HashSet<String>,
    /// Fences roster events: advanced by every gateway connection.
    generation: u64,
    /// A join waiting for its membership check: channel, call generation
    /// and when Join was clicked.
    pending: Option<(String, u64, u64)>,
    request: u64,
}

/// Web's `voiceAction` for one channel.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Action {
    Leave,
    Joining,
    Join { switching: bool, enabled: bool },
}

/// The call's sounds: Join once connected, then others leaving and arriving.
#[derive(Default)]
pub(super) struct Heard {
    announced: Option<u64>,
    others: Option<(u64, String, BTreeSet<String>)>,
}

fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

impl State {
    fn join_unavailable(&self, channel: &str) -> Option<&'static str> {
        match self.calls.available.get(channel) {
            Some(true) => None,
            Some(false) => Some("Joining is not available at this time."),
            None => Some("Checking voice availability…"),
        }
    }

    fn voice_target(&self, channel: &str) -> Option<(CallContext, Option<String>)> {
        let detail = self.detail.as_ref()?;
        let target = detail.channels.iter().find(|item| item.id == channel)?;
        if !target.joined
            || self.calls.unavailable.contains(channel)
            || (!detail.space.demo && self.token.is_none())
            || self.join_unavailable(channel).is_some()
        {
            return None;
        }
        Some((
            CallContext {
                channel_id: target.id.clone(),
                channel_name: target.name.clone(),
                space_name: detail.space.name.clone(),
            },
            (!detail.space.demo).then(|| detail.space.id.clone()),
        ))
    }
}

impl App {
    /// The saved device choices and volumes, applied before any call.
    pub(super) fn restore_voice(&self) {
        if let Some(saved) = self
            .prefs
            .get::<serde_json::Value>(super::audio::PREFERENCES)
        {
            self.voice.borrow_mut().preferences = Preferences::restore(&saved.to_string());
        }
    }

    pub(super) fn save_voice(&self) {
        self.prefs
            .set(super::audio::PREFERENCES, &self.voice.borrow().preferences);
    }

    /// Which channels' rosters a new gateway connection watches: the open
    /// space's joined channels, while the open channel is joined.
    pub(super) fn media_watch(&self, state: &mut State) -> MediaWatch {
        let calls = &mut state.calls;
        calls.generation += 1;
        calls.rosters.clear();
        calls.starts.clear();
        calls.unavailable.clear();
        let channels = state
            .detail
            .as_ref()
            .filter(|_| state.direct.is_none() && state.token.is_some() && state.joined())
            .map_or_else(Vec::new, |detail| {
                detail
                    .channels
                    .iter()
                    .filter(|channel| channel.joined)
                    .map(|channel| MediaChannel {
                        id: channel.id.clone(),
                        demo: detail.space.demo,
                    })
                    .take(MAX_MEDIA_CHANNELS)
                    .collect()
            });
        // Each joined channel's availability, once per sign-in.
        for channel in &channels {
            if state.calls.checked.insert(channel.id.clone()) {
                self.worker.send(Command::MediaStatus {
                    generation: state.epoch,
                    root: channel.id.clone(),
                    token: state.token.clone(),
                    channel: Some(channel.id.clone()),
                });
            }
        }
        MediaWatch {
            epoch: state.calls.generation,
            channels,
        }
    }

    pub(super) fn media_status(&self, state: &mut State, root: String, enabled: bool) {
        state.calls.available.insert(root, enabled);
        state.layout += 1;
    }

    /// Roster events; anything else comes back to `gateway`.
    pub(super) fn voice_gateway(
        &self,
        state: &mut State,
        event: GatewayEvent,
    ) -> Option<GatewayEvent> {
        let calls = &mut state.calls;
        match event {
            GatewayEvent::VoiceRoster {
                generation,
                channel,
                participants,
                session_started_at,
            } if generation == calls.generation => {
                calls.unavailable.remove(&channel);
                match session_started_at.filter(|_| !participants.is_empty()) {
                    Some(started) => calls.starts.insert(channel.clone(), started),
                    None => calls.starts.remove(&channel),
                };
                calls.rosters.insert(channel, participants);
            }
            GatewayEvent::VoiceUnavailable {
                generation,
                channel,
                revoked,
            } if generation == calls.generation => {
                calls.rosters.remove(&channel);
                calls.starts.remove(&channel);
                calls.unavailable.insert(channel.clone());
                if revoked {
                    self.voice.borrow_mut().revoke_channel(&channel);
                }
            }
            GatewayEvent::VoiceReset { generation } if generation == calls.generation => {
                calls.rosters.clear();
                calls.starts.clear();
            }
            event @ (GatewayEvent::VoiceRoster { .. }
            | GatewayEvent::VoiceUnavailable { .. }
            | GatewayEvent::VoiceReset { .. }) => {
                let _ = event;
                return None;
            }
            event => return Some(event),
        }
        state.layout += 1;
        None
    }

    fn action(&self, state: &State, id: &str) -> Action {
        let voice = self.voice.borrow();
        let own = voice.state.active_channel() == Some(id)
            && !matches!(voice.state.phase, Phase::Failed(_));
        let authorizing = state
            .calls
            .pending
            .as_ref()
            .map(|(channel, _, _)| channel.as_str());
        let connecting = matches!(
            voice.state.phase,
            Phase::Joining(_) | Phase::Reconnecting(_)
        );
        if own && matches!(voice.state.phase, Phase::Connected(_)) {
            Action::Leave
        } else if authorizing == Some(id) || own && connecting {
            Action::Joining
        } else {
            Action::Join {
                switching: !matches!(voice.state.phase, Phase::Idle | Phase::Failed(_)),
                enabled: authorizing.is_none() && !connecting && state.voice_target(id).is_some(),
            }
        }
    }

    /// Join `channel`'s call. Space channels recheck membership first.
    pub(super) fn join_voice(self: &Rc<Self>, channel: &str) {
        {
            let mut state = self.state.borrow_mut();
            let mut voice = self.voice.borrow_mut();
            if state
                .calls
                .pending
                .as_ref()
                .is_some_and(|(_, generation, _)| *generation == voice.state.generation)
                || matches!(
                    voice.state.phase,
                    Phase::Joining(_) | Phase::Reconnecting(_)
                )
                || voice.state.active_channel() == Some(channel)
                    && !matches!(voice.state.phase, Phase::Failed(_))
            {
                return;
            }
            let Some((context, space)) = state.voice_target(channel) else {
                return;
            };
            let clicked = now_ms();
            state.calls.request += 1;
            match space {
                Some(space) => {
                    state.calls.pending = Some((channel.into(), voice.state.generation, clicked));
                    self.worker.send(Command::CheckVoice {
                        request: state.calls.request,
                        voice_generation: voice.state.generation,
                        token: state.token.clone().unwrap_or_default(),
                        space,
                        channel: channel.into(),
                    });
                }
                None => voice.join(
                    context,
                    None,
                    state.token.clone(),
                    state.name(),
                    clicked,
                    state.calls.starts.get(channel).copied(),
                ),
            }
            state.layout += 1;
        }
        self.later_render();
    }

    /// The membership check answered: join, or say why not.
    pub(super) fn voice_checked(
        &self,
        state: &mut State,
        request: u64,
        voice_generation: u64,
        space: &str,
        channel: &str,
        result: Result<(), LoadError>,
    ) {
        if request != state.calls.request {
            return;
        }
        let Some((_, _, clicked)) = state.calls.pending.take() else {
            return;
        };
        state.layout += 1;
        let mut voice = self.voice.borrow_mut();
        if voice_generation != voice.state.generation {
            return;
        }
        let Some((context, target)) = state.voice_target(channel) else {
            return;
        };
        if target.as_deref() != Some(space) {
            return;
        }
        match result {
            Ok(()) => voice.join(
                context,
                target,
                state.token.clone(),
                state.name(),
                clicked,
                state.calls.starts.get(channel).copied(),
            ),
            Err(error) => {
                if error.access_denied {
                    state.calls.unavailable.insert(channel.into());
                    state.calls.rosters.remove(channel);
                    state.calls.starts.remove(channel);
                }
                voice.error = Some(error.message);
            }
        }
    }

    /// Leave, or Cancel while joining. Only a connected call plays the
    /// disconnect sound.
    pub(super) fn leave_voice(self: &Rc<Self>) {
        {
            let mut voice = self.voice.borrow_mut();
            if matches!(voice.state.phase, Phase::Connected(_)) {
                self.play(Effect::Disconnect);
            }
            voice.leave();
            voice.error = None;
        }
        if let Ok(mut state) = self.state.try_borrow_mut() {
            state.calls.pending = None;
            state.layout += 1;
        }
        self.later_render();
    }

    fn toggle_mute(self: &Rc<Self>) {
        let muted = {
            let mut voice = self.voice.borrow_mut();
            let muted = voice.state.audio.muted;
            voice.command(VoiceOperation::Mute(!muted));
            muted
        };
        self.effects.borrow().toggle(muted);
        self.later_render();
    }

    fn toggle_deafen(self: &Rc<Self>) {
        let deafened = {
            let mut voice = self.voice.borrow_mut();
            let deafened = voice.state.audio.deafened;
            voice.command(VoiceOperation::Deafen(!deafened));
            deafened
        };
        self.effects.borrow().toggle(deafened);
        self.later_render();
    }

    /// Redraw once the click being handled is over: its button may go.
    fn later_render(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(app) = weak.upgrade() {
                app.render();
            }
        });
    }

    /// Reads the call's reports and plays its sounds.
    pub(super) fn receive_voice(self: &Rc<Self>) {
        let mut voice = self.voice.borrow_mut();
        voice.receive();
        let mut heard = self.heard.borrow_mut();
        if let Ok(mut state) = self.state.try_borrow_mut()
            && state
                .calls
                .pending
                .as_ref()
                .is_some_and(|(_, generation, _)| *generation != voice.state.generation)
        {
            state.calls.pending = None;
        }
        let connected = matches!(voice.state.phase, Phase::Connected(_));
        if connected && heard.announced != Some(voice.state.generation) {
            heard.announced = Some(voice.state.generation);
            self.play(Effect::Join);
        }
        if !connected || voice.self_id.is_empty() {
            heard.others = None;
        } else {
            let generation = voice.state.generation;
            let others: BTreeSet<String> = voice
                .participants
                .iter()
                .map(|participant| participant.id.clone())
                .filter(|id| *id != voice.self_id)
                .collect();
            if let Some((previous_generation, previous_self, previous)) = &heard.others
                && *previous_generation == generation
                && *previous_self == voice.self_id
            {
                if previous.iter().any(|id| !others.contains(id)) {
                    self.play(Effect::Leave);
                } else if others.iter().any(|id| !previous.contains(id)) {
                    self.play(Effect::Join);
                }
            }
            heard.others = Some((generation, voice.self_id.clone(), others));
        }
        let busy = !matches!(voice.state.phase, Phase::Idle);
        drop(voice);
        drop(heard);
        if busy {
            self.voice_tick();
        }
    }

    /// Redraws the call card while a call is under way.
    fn voice_tick(self: &Rc<Self>) {
        if self.voice_ticking.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(TICK, move || {
            let Some(app) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            app.voice.borrow_mut().receive();
            app.render_voice();
            app.render_layout();
            if matches!(app.voice.borrow().state.phase, Phase::Idle) {
                app.voice_ticking.set(false);
                app.render();
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    /// Who is in voice in `id`: the call's own roster when it is yours.
    fn voice_people(&self, state: &State, id: &str) -> Vec<VoiceOccupant> {
        let voice = self.voice.borrow();
        if voice.state.active_channel() == Some(id)
            && !matches!(voice.state.phase, Phase::Failed(_))
        {
            let account = state.account.as_ref();
            return voice
                .participants
                .iter()
                .map(|participant| {
                    // Your own face shows the picture you just saved.
                    let own = account.filter(|_| participant.id == voice.self_id);
                    VoiceOccupant {
                        id: participant.id.clone(),
                        avatar_id: participant.avatar_id,
                        avatar_asset: own.map_or_else(
                            || participant.avatar_asset.clone(),
                            |account| account.avatar_asset.clone(),
                        ),
                        avatar_url: own.map_or_else(
                            || participant.avatar_url.clone(),
                            |account| account.avatar_url.clone(),
                        ),
                        name: participant.name.clone(),
                        muted: participant.muted,
                        deafened: participant.deafened,
                    }
                })
                .collect();
        }
        state.calls.rosters.get(id).cloned().unwrap_or_default()
    }

    /// A channel's live room under its sidebar row, as the desktop's room
    /// card: the session timer, Join or Switch, and who is there.
    pub(super) fn channel_voice(self: &Rc<Self>, state: &State, id: &str) -> Option<gtk::Box> {
        let people = self.voice_people(state, id);
        if people.is_empty() {
            return None;
        }
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        list.add_css_class("voice-roster");
        let own = self.voice.borrow().state.active_channel() == Some(id);
        let started = if own {
            self.voice.borrow().session_started_at
        } else {
            state.calls.starts.get(id).copied()
        };
        let name = state
            .detail
            .as_ref()
            .and_then(|detail| detail.channels.iter().find(|channel| channel.id == id))
            .map_or_else(String::new, |channel| channel.name.clone());
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let timer = label(
            &started.map_or_else(String::new, |started| {
                voice::session_duration(started, now_ms())
            }),
            &["small", "voice-timer"],
        );
        timer.set_hexpand(true);
        timer.set_xalign(0.0);
        if let Some(started) = started {
            self.room_timers.borrow_mut().push((timer.clone(), started));
            self.room_tick();
        }
        top.append(&timer);
        let action = match self.action(state, id) {
            Action::Join { switching, enabled } => {
                let button: gtk::Button = crate::app::named(
                    "GtkButton",
                    &format!("{} #{name}", if switching { "Switch to" } else { "Join" }),
                );
                button.set_label(if switching { "Switch" } else { "Join" });
                button.set_sensitive(enabled);
                button.set_tooltip_text(Some(
                    &state.join_unavailable(id).filter(|_| !enabled).map_or_else(
                        || {
                            format!(
                                "{} #{name}",
                                if switching {
                                    "Switch voice to"
                                } else {
                                    "Join voice in"
                                }
                            )
                        },
                        str::to_owned,
                    ),
                ));
                if !switching {
                    button.add_css_class("primary");
                }
                let (weak, channel) = (Rc::downgrade(self), id.to_owned());
                button.connect_clicked(move |_| {
                    if let Some(app) = weak.upgrade() {
                        app.join_voice(&channel);
                    }
                });
                Some(button)
            }
            Action::Joining => {
                let button = gtk::Button::with_label("Joining…");
                button.set_sensitive(false);
                button.set_tooltip_text(Some(&format!("Connecting to #{name}…")));
                Some(button)
            }
            Action::Leave => None,
        };
        if let Some(button) = action {
            button.add_css_class("room-action");
            button.set_valign(gtk::Align::Center);
            top.append(&button);
        }
        if started.is_some() || top.last_child() != top.first_child() {
            list.append(&top);
        }
        for person in &people {
            list.append(&self.occupant(person, own, 18));
        }
        Some(list)
    }

    /// Keeps the sidebar's room timers running while any show.
    fn room_tick(self: &Rc<Self>) {
        if self.room_ticking.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(app) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let mut timers = app.room_timers.borrow_mut();
            timers.retain(|(label, _)| label.parent().is_some());
            if timers.is_empty() {
                app.room_ticking.set(false);
                return glib::ControlFlow::Break;
            }
            let now = now_ms();
            for (label, started) in timers.iter() {
                label.set_label(&voice::session_duration(*started, now));
            }
            glib::ControlFlow::Continue
        });
    }

    fn occupant(&self, person: &VoiceOccupant, own: bool, size: i32) -> gtk::Box {
        let voice = self.voice.borrow();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.add_css_class("voice-person");
        let avatar = AvatarView::new(size, false);
        avatar.set(
            &self.images,
            &Avatar::new(
                &person.name,
                person.avatar_id,
                &person.avatar_asset,
                &person.avatar_url,
            ),
        );
        if own && voice.speaking(&person.id, person.muted, Instant::now()) {
            avatar.root.add_css_class("speaking");
        }
        row.append(&avatar.root);
        let name = label(&person.name, &["small"]);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        name.set_hexpand(true);
        row.append(&name);
        if person.deafened {
            let icon = self.images.icon_image(Icon::HeadphoneOff, "#e5735a", 13);
            icon.set_tooltip_text(Some("Deafened"));
            row.append(&icon);
        } else if person.muted {
            let icon = self.images.icon_image(Icon::MicOff, "#e5735a", 13);
            icon.set_tooltip_text(Some("Muted"));
            row.append(&icon);
        }
        row
    }

    /// The header's Join, Switch, Joining… or Leave voice, the call card and
    /// the account bar's toggles.
    pub(super) fn render_voice(self: &Rc<Self>) {
        let Ok(state) = self.state.try_borrow() else {
            return;
        };
        let ui = &self.ui;
        let channel = state
            .channel
            .clone()
            .filter(|_| state.direct.is_none() && state.joined() && state.detail.is_some());
        match channel.as_deref() {
            Some(id) if state.token.is_some() => {
                let action = self.action(&state, id);
                let (text, enabled, tooltip) = match action {
                    Action::Leave => ("Leave voice", true, "Leave voice".to_owned()),
                    Action::Joining => (
                        "Joining…",
                        false,
                        format!("Connecting to #{}…", state.channel_name),
                    ),
                    Action::Join { switching, enabled } => (
                        if switching { "Switch" } else { "Join" },
                        enabled,
                        state
                            .join_unavailable(id)
                            .map(str::to_owned)
                            .filter(|_| !enabled)
                            .unwrap_or_else(|| {
                                format!(
                                    "{} #{}",
                                    if switching {
                                        "Switch voice to"
                                    } else {
                                        "Join voice in"
                                    },
                                    state.channel_name
                                )
                            }),
                    ),
                };
                ui.voice_join.set_label(text);
                ui.voice_join.set_sensitive(enabled);
                ui.voice_join.set_tooltip_text(Some(&tooltip));
                // Leave lives on the call card; Switch is neutral.
                ui.voice_join.set_visible(action != Action::Leave);
                if matches!(
                    action,
                    Action::Join {
                        switching: false,
                        ..
                    }
                ) {
                    ui.voice_join.add_css_class("primary");
                } else {
                    ui.voice_join.remove_css_class("primary");
                }
            }
            _ => ui.voice_join.set_visible(false),
        }
        let voice = self.voice.borrow();
        let audio = voice.state.audio;
        let toggles = (audio.muted, audio.deafened);
        if self.rendered_toggles.replace(Some(toggles)) != Some(toggles) {
            ui.mic.set_child(Some(&self.images.icon_image(
                if audio.muted { Icon::MicOff } else { Icon::Mic },
                if audio.muted { "#e5735a" } else { "#b9bcbe" },
                18,
            )));
            ui.mic
                .set_tooltip_text(Some(if audio.muted { "Unmute" } else { "Mute" }));
            ui.deafen.set_child(Some(&self.images.icon_image(
                if audio.deafened {
                    Icon::VolumeOff
                } else {
                    Icon::Headphones
                },
                if audio.deafened { "#e5735a" } else { "#b9bcbe" },
                18,
            )));
            ui.deafen
                .set_tooltip_text(Some(if audio.deafened { "Undeafen" } else { "Deafen" }));
        }

        // The call card redraws when what it shows changes; the timer ticks.
        let now = Instant::now();
        let context = match &voice.state.phase {
            Phase::Idle => None,
            Phase::Joining(context)
            | Phase::Connected(context)
            | Phase::Reconnecting(context)
            | Phase::Failed(context) => Some(context.clone()),
        };
        let timer = voice
            .session_started_at
            .filter(|_| matches!(voice.state.phase, Phase::Connected(_)))
            .map(|started| voice::session_duration(started, now_ms()));
        let error = voice.error.clone();
        let people = context
            .as_ref()
            .map(|context| self.voice_people(&state, &context.channel_id))
            .unwrap_or_default();
        let key = format!(
            "{:?}|{:?}|{:?}|{}",
            voice.state.phase,
            error,
            toggles,
            people
                .iter()
                .map(|person| format!(
                    "{}:{}:{}:{}:{:?}",
                    person.id,
                    person.muted,
                    person.deafened,
                    voice.speaking(&person.id, person.muted, now),
                    person.avatar_asset
                ))
                .collect::<Vec<_>>()
                .join(",")
        );
        drop(voice);
        drop(state);
        if let Some(label) = self.voice_timer.borrow().as_ref() {
            label.set_label(timer.as_deref().unwrap_or_default());
        }
        if *self.rendered_card.borrow() == key {
            return;
        }
        *self.rendered_card.borrow_mut() = key;
        // The sidebar's roster shows the same people.
        if let Ok(mut state) = self.state.try_borrow_mut() {
            state.layout += 1;
        }
        self.build_call_card(context, error, timer, &people);
    }

    fn build_call_card(
        self: &Rc<Self>,
        context: Option<CallContext>,
        error: Option<String>,
        timer: Option<String>,
        people: &[VoiceOccupant],
    ) {
        let card = &self.ui.call_card;
        clear(card);
        *self.voice_timer.borrow_mut() = None;
        let phase = self.voice.borrow().state.phase.clone();
        let Some(context) = context else {
            // An error from a join that never started (a failed check).
            if let Some(error) = error {
                let text = label(&error, &["error", "small"]);
                text.set_wrap(true);
                card.append(&text);
                let dismiss = gtk::Button::with_label("Dismiss");
                dismiss.set_halign(gtk::Align::Start);
                let weak = Rc::downgrade(self);
                dismiss.connect_clicked(move |_| {
                    if let Some(app) = weak.upgrade() {
                        app.voice.borrow_mut().error = None;
                        app.later_render();
                    }
                });
                card.append(&dismiss);
            }
            card.set_visible(card.first_child().is_some());
            return;
        };
        card.set_visible(true);
        let failed = matches!(phase, Phase::Failed(_));
        let title = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        title.append(&self.images.icon_image(
            Icon::AudioLines,
            if failed { "#e5735a" } else { "#6fbf8a" },
            16,
        ));
        let name = gtk::Button::with_label(&format!("#{}", context.channel_name));
        name.add_css_class("flat");
        name.add_css_class("call-channel");
        name.set_tooltip_text(Some("Open the call's chat"));
        let weak = Rc::downgrade(self);
        let (channel, space) = (
            context.channel_id.clone(),
            self.voice.borrow().active_space.clone(),
        );
        name.connect_clicked(move |_| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            {
                let mut state = app.state.borrow_mut();
                if state.channel.as_deref() == Some(&channel) && state.direct.is_none() {
                    return;
                }
                let space = space.clone().or_else(|| state.space.clone());
                app.navigate(&mut state, space, Some(channel.clone()));
            }
            app.later_render();
        });
        title.append(&name);
        let status = label(
            match phase {
                Phase::Joining(_) => "Connecting…",
                Phase::Reconnecting(_) => "Reconnecting…",
                Phase::Failed(_) => "Disconnected",
                _ => "",
            },
            &["small", "call-status"],
        );
        status.set_hexpand(true);
        status.set_xalign(1.0);
        title.append(&status);
        let clock = label(
            timer.as_deref().unwrap_or_default(),
            &["small", "call-timer"],
        );
        title.append(&clock);
        *self.voice_timer.borrow_mut() = Some(clock);
        card.append(&title);
        card.append(&label(&context.space_name, &["muted", "small"]));
        let me = self.voice.borrow().self_id.clone();
        for person in people {
            let row = self.occupant(person, true, 22);
            if person.id == me {
                card.append(&row);
                continue;
            }
            // Others' audio controls: what you hear from them.
            let button: gtk::Button =
                crate::app::named("GtkButton", &format!("{} audio", person.name));
            button.add_css_class("flat");
            button.add_css_class("voice-person-button");
            button.set_child(Some(&row));
            button.set_tooltip_text(Some("User volume"));
            let (weak, id) = (Rc::downgrade(self), person.id.clone());
            button.connect_clicked(move |button| {
                if let Some(app) = weak.upgrade() {
                    app.participant_audio(&id, button);
                }
            });
            card.append(&button);
        }
        if let Some(error) = error {
            let text = label(&error, &["error", "small"]);
            text.set_wrap(true);
            card.append(&text);
        }
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        buttons.set_margin_top(4);
        let audio = self.voice.borrow().state.audio;
        let toggle = |icon: Icon, off: Icon, active: bool, name: &str, on_name: &str| {
            let button: gtk::Button =
                crate::app::named("GtkButton", if active { on_name } else { name });
            let image = self.images.icon_image(
                if active { off } else { icon },
                if active { "#e5735a" } else { "#f3f4f5" },
                16,
            );
            image.set_halign(gtk::Align::Center);
            image.set_valign(gtk::Align::Center);
            button.set_child(Some(&image));
            button.set_tooltip_text(Some(if active { on_name } else { name }));
            button.add_css_class("call-toggle");
            if active {
                button.add_css_class("active");
            }
            button
        };
        if failed {
            let again = gtk::Button::with_label("Try again");
            again.add_css_class("primary");
            let weak = Rc::downgrade(self);
            let channel = context.channel_id.clone();
            again.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.voice.borrow_mut().error = None;
                    app.join_voice(&channel);
                }
            });
            buttons.append(&again);
        } else {
            let mute = toggle(Icon::Mic, Icon::MicOff, audio.muted, "Mute", "Unmute");
            let weak = Rc::downgrade(self);
            mute.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.toggle_mute();
                }
            });
            buttons.append(&mute);
            let deafen = toggle(
                Icon::Headphones,
                Icon::HeadphoneOff,
                audio.deafened,
                "Deafen",
                "Undeafen",
            );
            let weak = Rc::downgrade(self);
            deafen.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.toggle_deafen();
                }
            });
            buttons.append(&deafen);
        }
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        buttons.append(&spacer);
        let joining = matches!(phase, Phase::Joining(_));
        let leave: gtk::Button = crate::app::named(
            "GtkButton",
            if failed {
                "Dismiss"
            } else if joining {
                "Cancel"
            } else {
                "Leave voice"
            },
        );
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        if !failed {
            line.append(&self.images.icon_image(Icon::PhoneOff, "#f3f4f5", 15));
        }
        line.append(&label(
            if failed {
                "Dismiss"
            } else if joining {
                "Cancel"
            } else {
                "Leave"
            },
            &["strong"],
        ));
        leave.set_child(Some(&line));
        leave.add_css_class(if failed { "flat" } else { "destructive-action" });
        let weak = Rc::downgrade(self);
        leave.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.leave_voice();
            }
        });
        buttons.append(&leave);
        card.append(&buttons);
    }

    /// A participant's volume and Mute for me, as the desktop's call list.
    fn participant_audio(self: &Rc<Self>, id: &str, anchor: &gtk::Button) {
        let playback = self.voice.borrow().playback(id);
        let popover = gtk::Popover::new();
        popover.add_css_class("menu");
        let body = gtk::Box::new(gtk::Orientation::Vertical, 6);
        body.set_margin_start(6);
        body.set_margin_end(6);
        body.set_margin_top(6);
        body.set_margin_bottom(6);
        body.set_size_request(220, -1);
        body.append(&label("User volume", &["small", "strong"]));
        let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 200.0, 1.0);
        volume.set_digits(0);
        volume.set_draw_value(true);
        volume.set_value_pos(gtk::PositionType::Right);
        volume.set_value(f64::from(playback.gain_percent));
        body.append(&volume);
        let mute = crate::app::named::<gtk::CheckButton>("GtkCheckButton", "Mute for me");
        mute.set_label(Some("Mute for me"));
        mute.set_active(playback.muted);
        body.append(&mute);
        body.append(&label("Only changes what you hear.", &["muted", "small"]));
        popover.set_child(Some(&body));
        let apply = {
            let (weak, id) = (Rc::downgrade(self), id.to_owned());
            let (volume, mute) = (volume.clone(), mute.clone());
            move || {
                if let Some(app) = weak.upgrade() {
                    let mut playback = app.voice.borrow().playback(&id);
                    playback.gain_percent = volume.value().round() as u16;
                    playback.muted = mute.is_active();
                    app.voice
                        .borrow_mut()
                        .set_participant_playback(&id, playback);
                }
            }
        };
        let weak = Rc::downgrade(self);
        let on_volume = apply.clone();
        volume.connect_value_changed(move |scale| {
            on_volume();
            if let Some(app) = weak.upgrade() {
                app.effects
                    .borrow_mut()
                    .slider(scale.value() as f32 / 200.0);
            }
        });
        let weak = Rc::downgrade(self);
        mute.connect_toggled(move |check| {
            apply();
            if let Some(app) = weak.upgrade() {
                app.effects.borrow().toggle(!check.is_active());
            }
        });
        super::popup(&popover, anchor.upcast_ref());
        self.effects.borrow().toggle(true);
    }

    /// Connects the header's voice button and the account bar's toggles.
    pub(super) fn connect_voice(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.ui.voice_join.connect_clicked(move |_| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let (channel, action) = {
                let state = app.state.borrow();
                let Some(channel) = state.channel.clone() else {
                    return;
                };
                let action = app.action(&state, &channel);
                (channel, action)
            };
            match action {
                Action::Leave => app.leave_voice(),
                Action::Join { enabled: true, .. } => app.join_voice(&channel),
                _ => {}
            }
        });
        let weak = Rc::downgrade(self);
        self.ui.mic.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.toggle_mute();
            }
        });
        for (button, input) in [
            (&self.ui.input_options, true),
            (&self.ui.output_options, false),
        ] {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |button| {
                if let Some(app) = weak.upgrade() {
                    app.device_menu(input, button);
                }
            });
        }
        let weak = Rc::downgrade(self);
        self.ui.deafen.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.toggle_deafen();
            }
        });
    }
}
