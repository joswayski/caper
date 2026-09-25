use crate::{media, media_gateway, state};
use eframe::egui;
use media::mic_test::{MicTest, MicTestControl, PlaybackToken};
use media::{
    JoinControl, MediaApi, NativeSession, Participant, Snapshot, TrackPlayback, VoiceActivity,
    VoiceError,
};
use serde::{Deserialize, Serialize};
use state::{CallContext, CallState, Phase};
use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use url::Url;

/// Web keeps a voice lit this long after its last loud 32 ms window.
pub const SPEAKING_RELEASE: Duration = Duration::from_millis(180);

/// A muted voice is never shown speaking, whatever its last level.
pub fn speaking_at(last_loud: Option<Instant>, muted: bool, now: Instant) -> bool {
    !muted && last_loud.is_some_and(|loud| now.saturating_duration_since(loud) < SPEAKING_RELEASE)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    pub input: Option<String>,
    pub output: Option<String>,
    pub master_percent: u16,
    pub input_percent: u16,
    pub processing_strength: u8,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            input: None,
            output: None,
            master_percent: 100,
            input_percent: 100,
            processing_strength: 25,
        }
    }
}

impl Preferences {
    pub fn restore(json: &str) -> Self {
        let mut preferences: Self = serde_json::from_str(json).unwrap_or_default();
        preferences.master_percent = preferences.master_percent.min(200);
        preferences.input_percent = preferences.input_percent.min(200);
        preferences.processing_strength = preferences.processing_strength.min(100);
        preferences
    }
}

#[derive(Clone, Debug)]
pub enum MicrophoneState {
    Idle,
    Preparing,
    Recording(Instant),
    Ready(f32),
    Playing { seconds: f32, enhanced: bool },
}

pub struct Voice {
    pub state: CallState,
    pub participants: Vec<Participant>,
    pub self_id: String,
    pub error: Option<String>,
    pub diagnostics: Option<(media::Diagnostics, Instant)>,
    pub activity: VoiceActivity,
    pub inputs: Vec<(String, String)>,
    pub outputs: Vec<(String, String)>,
    pub preferences: Preferences,
    pub device_error: Option<String>,
    pub refreshing_devices: bool,
    pub microphone: MicrophoneState,
    pub microphone_error: Option<String>,
    pub active_space: Option<String>,
    microphone_generation: u64,
    microphone_control: Option<MicTestControl>,
    microphone_commands: Option<Sender<(bool, u16, PlaybackToken)>>,
    device_request: u64,
    participant_playback: BTreeMap<String, TrackPlayback>,
    base: Url,
    control: Option<JoinControl>,
    commands: Option<Sender<Operation>>,
    events: Sender<Report>,
    incoming: Receiver<Report>,
    repaint: egui::Context,
}

enum Operation {
    Mute(bool),
    Deafen(bool),
}

enum Report {
    Connected(u64, String, Vec<(String, String)>, Vec<(String, String)>),
    Roster(u64, Snapshot),
    Gateway(u64, media_gateway::Event),
    Failed(u64, VoiceError),
    Changed(u64, Result<(), VoiceError>),
    Diagnostics(u64, media::Diagnostics),
    Activity(u64, VoiceActivity),
    Devices(u64, Result<media::AudioDevices, String>),
    Microphone(u64, Result<MicrophoneState, String>),
}

impl Voice {
    pub fn audio_processing_report(&self) -> Option<serde_json::Value> {
        let stats = self.control.as_ref()?.audio_processing_diagnostics()?;
        Some(serde_json::json!({
            "engine": stats.engine,
            "processedHops": stats.processed_hops,
            "meanProcessingMs": if stats.processed_hops == 0 { 0.0 } else {
                stats.total_processing_us as f64 / stats.processed_hops as f64 / 1000.0
            },
            "maxProcessingMs": stats.max_processing_us as f64 / 1000.0,
            "hopBudgetMs": 10,
            "fallbackCount": stats.fallback_count,
        }))
    }

    pub fn new(base: Url, repaint: egui::Context) -> Self {
        let (events, incoming) = mpsc::channel();
        Self {
            state: CallState::default(),
            participants: vec![],
            self_id: String::new(),
            error: None,
            diagnostics: None,
            activity: VoiceActivity::default(),
            inputs: vec![],
            outputs: vec![],
            preferences: Preferences::default(),
            device_error: None,
            refreshing_devices: false,
            microphone: MicrophoneState::Idle,
            microphone_error: None,
            microphone_generation: 0,
            microphone_control: None,
            microphone_commands: None,
            device_request: 0,
            participant_playback: BTreeMap::new(),
            base,
            control: None,
            commands: None,
            events,
            incoming,
            repaint,
            active_space: None,
        }
    }

    pub fn join(
        &mut self,
        context: CallContext,
        space: Option<String>,
        token: Option<String>,
        name: String,
    ) {
        self.leave();
        let token = space.as_ref().and(token);
        let api = match MediaApi::new(
            &self.base,
            space.as_ref().map(|_| context.channel_id.as_str()),
            token.clone(),
        ) {
            Ok(api) => api,
            Err(detail) => {
                self.error = Some(detail);
                return;
            }
        };
        self.active_space = space.clone();
        let generation = self.state.join(context);
        self.error = None;
        let control = JoinControl::new();
        if let Err(error) = control
            .set_local_audio(self.state.audio.muted, self.state.audio.deafened)
            .and_then(|()| {
                control.set_participant_playback_preferences(
                    self.preferences.master_percent,
                    &BTreeMap::new(),
                )
            })
            .and_then(|()| {
                control.set_input_processing(
                    self.preferences.input_percent,
                    self.preferences.processing_strength,
                )
            })
        {
            self.leave();
            self.error = Some(error);
            return;
        }
        self.control = Some(control.clone());
        let (sender, receiver) = mpsc::channel();
        self.commands = Some(sender);
        let events = self.events.clone();
        let repaint = self.repaint.clone();
        let base = self.base.clone();
        let audio_intent = self.state.audio;
        let input = self.preferences.input.clone();
        let output = self.preferences.output.clone();
        let channel_id = space
            .as_ref()
            .map(|_| self.state.active_channel().unwrap().to_owned());
        std::thread::spawn(move || {
            let runtime = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    report(
                        &events,
                        &repaint,
                        Report::Failed(generation, VoiceError::Local(error.to_string())),
                    );
                    return;
                }
            };
            let session = runtime.block_on(NativeSession::join(
                api,
                &name,
                audio_intent,
                input.as_deref(),
                output.as_deref(),
                &control,
            ));
            let mut session = match session {
                Ok(session) => session,
                Err(error) => {
                    if !matches!(&error, VoiceError::Local(detail) if detail == "voice join cancelled")
                    {
                        report(&events, &repaint, Report::Failed(generation, error));
                    }
                    return;
                }
            };
            if control.is_cancelled() {
                session.leave();
                return;
            }
            let (gateway_events, gateway_incoming) = mpsc::channel();
            let gateway = media_gateway::spawn(
                &base,
                token,
                session.media_token().into(),
                channel_id,
                generation,
                gateway_events,
            );
            let mut ready = false;
            let mut gateway_ready = false;
            let mut roster_ready = false;
            let mut next_snapshot = Instant::now();
            let mut next_diagnostics = Instant::now();
            let mut activity = VoiceActivity::default();
            let mut next_turn = session
                .turn_refresh_delay()
                .map(|delay| Instant::now() + delay);
            while !control.is_cancelled() {
                while let Ok(event) = gateway_incoming.try_recv() {
                    if !ready {
                        match event {
                            media_gateway::Event::Online { .. } => gateway_ready = true,
                            media_gateway::Event::Offline { .. } => gateway_ready = false,
                            _ => {}
                        }
                    }
                    let denied = matches!(event, media_gateway::Event::AccessDenied { .. });
                    report(&events, &repaint, Report::Gateway(generation, event));
                    if denied {
                        control.cancel();
                        break;
                    }
                    next_snapshot = Instant::now();
                }
                if control.is_cancelled() {
                    break;
                }
                match receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(command) => {
                        let result = match command {
                            Operation::Mute(value) => runtime.block_on(session.set_muted(value)),
                            Operation::Deafen(value) => {
                                runtime.block_on(session.set_deafened(value))
                            }
                        };
                        let result = result.and_then(|()| {
                            control.enforce_local_audio().map_err(VoiceError::Local)
                        });
                        let terminal = result.as_ref().is_err_and(|error| error.terminal());
                        if terminal {
                            control.cancel();
                        }
                        report(&events, &repaint, Report::Changed(generation, result));
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if control.is_cancelled() {
                    break;
                }
                if session.transport_failed() {
                    control.cancel();
                    report(
                        &events,
                        &repaint,
                        Report::Failed(
                            generation,
                            VoiceError::Local(
                                "Voice transport ended. Join again to reconnect.".into(),
                            ),
                        ),
                    );
                    break;
                }
                if Instant::now() >= next_snapshot {
                    match runtime.block_on(session.snapshot()) {
                        Ok(snapshot) => {
                            roster_ready = true;
                            report(&events, &repaint, Report::Roster(generation, snapshot))
                        }
                        Err(error) => {
                            if error.terminal() {
                                control.cancel();
                            }
                            report(&events, &repaint, Report::Failed(generation, error));
                        }
                    }
                    next_snapshot = Instant::now() + Duration::from_secs(3);
                }
                if !control.is_cancelled() && next_turn.is_some_and(|next| Instant::now() >= next) {
                    match runtime.block_on(session.renew_turn()) {
                        Ok(delay) => next_turn = Some(Instant::now() + delay),
                        Err(error) => {
                            if error.terminal() {
                                control.cancel();
                            }
                            report(&events, &repaint, Report::Failed(generation, error));
                            next_turn = Some(Instant::now() + Duration::from_secs(5));
                        }
                    }
                }
                if !ready && roster_ready && gateway_ready && !control.is_cancelled() {
                    match control.activate() {
                        Ok(()) => {
                            ready = true;
                            report(
                                &events,
                                &repaint,
                                Report::Connected(
                                    generation,
                                    session.self_id().into(),
                                    session.input_devices(),
                                    session.output_devices(),
                                ),
                            );
                        }
                        Err(error) => {
                            control.cancel();
                            report(
                                &events,
                                &repaint,
                                Report::Failed(generation, VoiceError::Local(error)),
                            );
                        }
                    }
                }
                if ready && !control.is_cancelled() {
                    let current = control.voice_activity();
                    if current != activity {
                        activity = current.clone();
                        report(&events, &repaint, Report::Activity(generation, current));
                    }
                }
                if ready && !control.is_cancelled() && Instant::now() >= next_diagnostics {
                    if let Ok(Ok(stats)) = runtime.block_on(async {
                        tokio::time::timeout(Duration::from_secs(2), session.diagnostics()).await
                    }) {
                        report(&events, &repaint, Report::Diagnostics(generation, stats));
                    }
                    next_diagnostics = Instant::now() + Duration::from_secs(3);
                }
            }
            gateway.stop();
            session.leave();
        });
    }

    pub fn leave(&mut self) {
        if let Some(control) = self.control.take() {
            control.cancel();
        }
        self.stop_mic_test();
        self.commands = None;
        self.active_space = None;
        self.participants.clear();
        self.self_id.clear();
        self.diagnostics = None;
        self.activity = VoiceActivity::default();
        self.participant_playback.clear();
        self.state.leave_now();
    }

    pub fn start_mic_test(&mut self) {
        self.stop_mic_test();
        self.microphone_error = None;
        if let Some(control) = &self.control
            && let Err(error) = control.suspend_for_mic_test()
        {
            self.microphone_error = Some(error);
            return;
        }
        let control = MicTestControl::new();
        self.microphone_control = Some(control.clone());
        self.microphone = MicrophoneState::Preparing;
        let generation = self.microphone_generation;
        let (sender, receiver) = mpsc::channel();
        self.microphone_commands = Some(sender);
        let events = self.events.clone();
        let repaint = self.repaint.clone();
        let input = self.preferences.input.clone();
        let output = self.preferences.output.clone();
        let gain = self.preferences.input_percent;
        let strength = self.preferences.processing_strength;
        std::thread::spawn(move || {
            let send = |state| report(&events, &repaint, Report::Microphone(generation, state));
            let result = (|| -> Result<(), String> {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .map_err(|error| error.to_string())?;
                let mut test = runtime.block_on(MicTest::start(
                    control.clone(),
                    input.as_deref(),
                    output.as_deref(),
                ))?;
                send(Ok(MicrophoneState::Recording(Instant::now())));
                let sample = runtime.block_on(test.record_with_processing(gain, strength))?;
                let seconds = sample.natural.len() as f32 / sample.sample_rate as f32;
                send(Ok(MicrophoneState::Ready(seconds)));
                while let Ok((enhanced, volume, token)) = receiver.recv() {
                    runtime.block_on(test.play_prepared(enhanced, volume, token))?;
                    send(Ok(MicrophoneState::Ready(seconds)));
                }
                test.stop();
                Ok(())
            })();
            // Includes runtime/setup failures before MicTest owns cleanup.
            control.cancel();
            if let Err(error) = result {
                send(Err(error));
            }
        });
    }

    pub fn finish_mic_recording(&self) {
        if let Some(control) = &self.microphone_control {
            control.finish_recording();
        }
    }

    pub fn play_mic_sample(&mut self, enhanced: bool) {
        if let MicrophoneState::Ready(seconds) = self.microphone
            && let Some(sender) = &self.microphone_commands
            && let Some(control) = &self.microphone_control
            && sender
                .send((
                    enhanced,
                    self.preferences.master_percent,
                    control.prepare_playback(),
                ))
                .is_ok()
        {
            self.microphone = MicrophoneState::Playing { seconds, enhanced };
        }
    }

    pub fn stop_mic_playback(&self) {
        if let Some(control) = &self.microphone_control {
            control.stop_playback();
        }
        // Remain Playing until the worker acknowledges completion. Otherwise
        // the previous completion could overwrite a newly queued replay state.
    }

    pub fn stop_mic_test(&mut self) {
        self.microphone_generation += 1;
        if let Some(control) = self.microphone_control.take() {
            control.cancel();
            if let Some(call) = &self.control
                && let Err(error) = call.finish_mic_test()
            {
                call.cancel();
                self.microphone_error = Some(error);
            }
        }
        self.microphone_commands = None;
        self.microphone = MicrophoneState::Idle;
    }

    pub fn refresh_devices(&mut self) {
        if self.refreshing_devices {
            return;
        }
        self.refreshing_devices = true;
        self.device_error = None;
        self.device_request += 1;
        let request = self.device_request;
        let events = self.events.clone();
        let repaint = self.repaint.clone();
        std::thread::spawn(move || {
            report(
                &events,
                &repaint,
                Report::Devices(request, media::enumerate_audio_devices()),
            );
        });
    }

    /// Only this client's own call has levels; `muted` is the local mute for
    /// yourself and the roster's flag for others, as on web.
    pub fn speaking(&self, participant: &str, muted: bool, now: Instant) -> bool {
        let last_loud = if participant == self.self_id {
            self.activity.local
        } else {
            self.activity.participants.get(participant).copied()
        };
        speaking_at(last_loud, muted, now)
    }

    /// When the next lit voice releases, so the UI repaints on time.
    pub fn next_speaking_release(&self, now: Instant) -> Option<Duration> {
        self.activity
            .local
            .iter()
            .chain(self.activity.participants.values())
            .filter_map(|loud| (*loud + SPEAKING_RELEASE).checked_duration_since(now))
            .filter(|remaining| !remaining.is_zero())
            .min()
    }

    pub fn playback(&self, participant: &str) -> TrackPlayback {
        self.participant_playback
            .get(participant)
            .cloned()
            .unwrap_or(TrackPlayback {
                gain_percent: 100,
                muted: false,
            })
    }

    pub fn set_participant_playback(&mut self, participant: &str, playback: TrackPlayback) {
        if participant == self.self_id || !self.participants.iter().any(|p| p.id == participant) {
            return;
        }
        self.participant_playback.insert(
            participant.into(),
            TrackPlayback {
                gain_percent: playback.gain_percent.min(200),
                muted: playback.muted,
            },
        );
        self.apply_playback();
    }

    pub fn set_master_gain(&mut self, percent: u16) {
        self.preferences.master_percent = percent.min(200);
        self.apply_playback();
    }

    pub fn set_input_processing(&mut self, percent: u16, strength: u8) {
        self.preferences.input_percent = percent.min(200);
        self.preferences.processing_strength = strength.min(100);
        if let Some(control) = &self.control
            && let Err(error) = control.set_input_processing(
                self.preferences.input_percent,
                self.preferences.processing_strength,
            )
        {
            self.leave();
            self.error = Some(error);
        }
    }

    fn apply_playback(&mut self) {
        if let Some(control) = &self.control
            && let Err(error) = control.set_participant_playback_preferences(
                self.preferences.master_percent,
                &self.participant_playback,
            )
        {
            self.leave();
            self.error = Some(error);
        }
    }

    fn roster(&mut self, snapshot: Snapshot) {
        self.participants = snapshot.participants;
        self.apply_playback();
    }

    pub fn revoke_channel(&mut self, id: &str) {
        if self.state.active_channel() == Some(id) {
            self.leave();
        }
    }

    pub fn revoke_space(&mut self, id: &str) {
        if self.active_space.as_deref() == Some(id) {
            self.leave();
        }
    }

    fn select_device(&mut self, input: bool, guid: Option<String>) {
        if let Some(control) = &self.control {
            let result = match (input, guid.as_deref()) {
                (true, Some(guid)) => control.select_input(guid),
                (false, Some(guid)) => control.select_output(guid),
                (true, None) => control.select_default_input(),
                (false, None) => control.select_default_output(),
            };
            if let Err(error) = result {
                self.leave();
                self.device_error = Some(error);
                return;
            }
        }
        if input {
            self.preferences.input = guid;
        } else {
            self.preferences.output = guid;
        }
        self.device_error = None;
    }

    pub fn command(&mut self, operation: VoiceOperation) {
        let command = match operation {
            VoiceOperation::Mute(value) => {
                self.state.audio.set_muted(value);
                Operation::Mute(value)
            }
            VoiceOperation::Deafen(value) => {
                self.state.audio.set_deafened(value);
                Operation::Deafen(value)
            }
            VoiceOperation::Input(guid) => return self.select_device(true, Some(guid)),
            VoiceOperation::Output(guid) => return self.select_device(false, Some(guid)),
            VoiceOperation::DefaultInput => return self.select_device(true, None),
            VoiceOperation::DefaultOutput => return self.select_device(false, None),
        };
        if let Some(control) = &self.control
            && let Err(error) =
                control.set_local_audio(self.state.audio.muted, self.state.audio.deafened)
        {
            self.error = Some(error);
            self.leave();
            return;
        }
        if let Some(sender) = &self.commands {
            let _ = sender.send(command);
        }
    }

    pub fn receive(&mut self) {
        while let Ok(report) = self.incoming.try_recv() {
            match report {
                Report::Connected(g, id, inputs, outputs) if self.state.connected(g) => {
                    self.self_id = id;
                    self.inputs = inputs;
                    self.outputs = outputs;
                }
                Report::Roster(g, snapshot) if g == self.state.generation => {
                    self.roster(snapshot);
                }
                Report::Gateway(g, media_gateway::Event::Snapshot { snapshot, .. })
                    if g == self.state.generation =>
                {
                    self.roster(snapshot);
                }
                Report::Gateway(g, media_gateway::Event::AccessDenied { detail, .. })
                    if g == self.state.generation =>
                {
                    self.leave();
                    self.error = Some(detail);
                }
                Report::Gateway(g, media_gateway::Event::Offline { detail, .. })
                    if g == self.state.generation =>
                {
                    self.error = Some(detail)
                }
                Report::Gateway(g, media_gateway::Event::Online { .. })
                    if g == self.state.generation =>
                {
                    self.error = None
                }
                Report::Failed(g, error) if g == self.state.generation => {
                    if error.terminal() || matches!(self.state.phase, Phase::Joining(_)) {
                        self.leave();
                    }
                    self.error = Some(error.to_string());
                }
                Report::Changed(g, Err(error)) if g == self.state.generation => {
                    if error.terminal() {
                        self.leave();
                    }
                    self.error = Some(error.to_string());
                }
                Report::Diagnostics(g, stats) if g == self.state.generation => {
                    self.diagnostics = Some((stats, Instant::now()));
                }
                Report::Activity(g, activity) if g == self.state.generation => {
                    self.activity = activity;
                }
                Report::Microphone(g, state) if g == self.microphone_generation => match state {
                    Ok(state) => self.microphone = state,
                    Err(error) => {
                        self.stop_mic_test();
                        self.microphone_error = Some(error);
                    }
                },
                Report::Devices(request, result) if request == self.device_request => {
                    self.refreshing_devices = false;
                    match result {
                        Ok(devices) => {
                            self.inputs = devices
                                .inputs
                                .into_iter()
                                .map(|device| (device.id, device.name))
                                .collect();
                            self.outputs = devices
                                .outputs
                                .into_iter()
                                .map(|device| (device.id, device.name))
                                .collect();
                        }
                        Err(error) => self.device_error = Some(error),
                    }
                }
                _ => {}
            }
        }
        if let Some(release) = self.next_speaking_release(Instant::now()) {
            self.repaint.request_repaint_after(release);
        }
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        self.leave();
    }
}

pub enum VoiceOperation {
    Mute(bool),
    Deafen(bool),
    Input(String),
    Output(String),
    DefaultInput,
    DefaultOutput,
}

fn report(sender: &Sender<Report>, repaint: &egui::Context, event: Report) {
    let _ = sender.send(event);
    repaint.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn context(id: &str) -> CallContext {
        CallContext {
            channel_id: id.into(),
            channel_name: id.into(),
            space_name: "General".into(),
        }
    }

    #[test]
    fn preferences_preserve_asymmetric_device_ids_and_clamp_gain() {
        let preferences =
            Preferences::restore(r#"{"input":"mic-a","output":"speaker-b","master_percent":175}"#);
        assert_eq!(preferences.input.as_deref(), Some("mic-a"));
        assert_eq!(preferences.output.as_deref(), Some("speaker-b"));
        assert_eq!(preferences.master_percent, 175);
        assert_eq!(preferences.input_percent, 100);
        assert_eq!(preferences.processing_strength, 25);
        let live = Preferences::restore(r#"{"input_percent":163,"processing_strength":71}"#);
        assert_eq!(live.input_percent, 163);
        assert_eq!(live.processing_strength, 71);
        assert_eq!(
            Preferences::restore(&serde_json::to_string(&live).unwrap()),
            live
        );
        let clamped = Preferences::restore(r#"{"input_percent":201,"processing_strength":101}"#);
        assert_eq!(clamped.input_percent, 200);
        assert_eq!(clamped.processing_strength, 100);
        let zero = Preferences::restore(r#"{"input_percent":0,"processing_strength":0}"#);
        assert_eq!(zero.input_percent, 0);
        assert_eq!(zero.processing_strength, 0);
        assert_eq!(
            Preferences::restore(&serde_json::to_string(&preferences).unwrap()),
            preferences
        );
        assert_eq!(
            Preferences::restore(r#"{"master_percent":0}"#).master_percent,
            0
        );
        assert_eq!(
            Preferences::restore(r#"{"master_percent":201}"#).master_percent,
            200
        );
        assert_eq!(Preferences::restore("broken"), Preferences::default());
    }

    #[test]
    fn participant_preferences_survive_roster_changes_but_not_call_replacement() {
        let mut voice = Voice::new(
            Url::parse("http://127.0.0.1:9/").unwrap(),
            egui::Context::default(),
        );
        let snapshot: Snapshot = serde_json::from_str(r#"{"participants":[
            {"id":"self","name":"Me","muted":false,"deafened":false,"tracks":[{"id":"own","kind":"microphone"}]},
            {"id":"a","name":"A","muted":false,"deafened":false,"tracks":[{"id":"track-a","kind":"microphone"}]},
            {"id":"b","name":"B","muted":false,"deafened":false,"tracks":[{"id":"track-b","kind":"microphone"}]}
        ]}"#).unwrap();
        voice.self_id = "self".into();
        voice.roster(snapshot.clone());
        voice.set_master_gain(75);
        voice.set_participant_playback(
            "a",
            TrackPlayback {
                gain_percent: 135,
                muted: true,
            },
        );
        voice.set_participant_playback(
            "self",
            TrackPlayback {
                gain_percent: 0,
                muted: true,
            },
        );
        assert!(!voice.participant_playback.contains_key("self"));
        assert_eq!(voice.playback("a").gain_percent, 135);
        assert!(voice.playback("a").muted);
        assert_eq!(voice.playback("b").gain_percent, 100);
        assert!(!voice.playback("b").muted);
        let mut republished = snapshot;
        republished.participants[1].tracks[0].id = "replacement-a".into();
        voice.roster(republished);
        assert!(voice.playback("a").muted);
        assert_eq!(voice.playback("a").gain_percent, 135);
        let generation = voice.state.generation;
        voice.leave();
        voice
            .events
            .send(Report::Diagnostics(
                generation,
                media::Diagnostics {
                    received_bytes: 555,
                    ..Default::default()
                },
            ))
            .unwrap();
        voice.receive();
        assert!(voice.participant_playback.is_empty());
        assert!(
            voice.diagnostics.is_none(),
            "late statistics must not resurrect a ended call"
        );
        assert_eq!(voice.preferences.master_percent, 75);
    }

    #[test]
    fn mic_sample_commands_use_current_gain_and_discard_rejects_late_completion() {
        let mut voice = Voice::new(
            Url::parse("http://127.0.0.1:9/").unwrap(),
            egui::Context::default(),
        );
        let (sender, receiver) = mpsc::channel();
        voice.microphone_commands = Some(sender);
        voice.microphone_control = Some(MicTestControl::new());
        voice.microphone = MicrophoneState::Ready(2.7);
        voice.set_master_gain(143);
        voice.play_mic_sample(true);
        let (enhanced, volume, _) = receiver.try_recv().unwrap();
        assert!(enhanced);
        assert_eq!(volume, 143);
        voice.stop_mic_playback();
        voice.play_mic_sample(false);
        assert!(
            receiver.try_recv().is_err(),
            "do not queue overlapping playback"
        );
        let generation = voice.microphone_generation;
        voice.stop_mic_test();
        voice
            .events
            .send(Report::Microphone(
                generation,
                Ok(MicrophoneState::Ready(2.7)),
            ))
            .unwrap();
        voice
            .events
            .send(Report::Microphone(generation, Err("old error".into())))
            .unwrap();
        voice.receive();
        assert!(matches!(voice.microphone, MicrophoneState::Idle));
        assert!(voice.microphone_error.is_none());
        assert!(voice.microphone_commands.is_none());
    }

    #[test]
    fn speaking_follows_web_release_and_never_lights_muted_voices() {
        let loud = Instant::now();
        assert!(!speaking_at(None, false, loud));
        assert!(speaking_at(Some(loud), false, loud));
        assert!(speaking_at(
            Some(loud),
            false,
            loud + Duration::from_millis(179)
        ));
        assert!(!speaking_at(
            Some(loud),
            false,
            loud + Duration::from_millis(180)
        ));
        assert!(!speaking_at(Some(loud), true, loud), "muted is never lit");

        let mut voice = Voice::new(
            Url::parse("http://127.0.0.1:9/").unwrap(),
            egui::Context::default(),
        );
        voice.self_id = "self".into();
        voice.activity = VoiceActivity {
            local: Some(loud),
            participants: [("a".to_owned(), loud - Duration::from_millis(100))].into(),
        };
        assert!(voice.speaking("self", false, loud));
        assert!(!voice.speaking("self", true, loud), "local mute wins");
        assert!(voice.speaking("a", false, loud));
        assert!(!voice.speaking("a", true, loud), "their muted flag wins");
        assert!(!voice.speaking("b", false, loud));
        assert_eq!(
            voice.next_speaking_release(loud),
            Some(Duration::from_millis(80))
        );
        assert_eq!(
            voice.next_speaking_release(loud + Duration::from_millis(180)),
            None
        );
        let generation = voice.state.generation;
        voice.leave();
        voice
            .events
            .send(Report::Activity(
                generation,
                VoiceActivity {
                    local: Some(loud),
                    participants: BTreeMap::new(),
                },
            ))
            .unwrap();
        voice.receive();
        assert_eq!(voice.activity, VoiceActivity::default());
    }

    #[test]
    fn prejoin_controls_change_intent_without_starting_a_call() {
        let mut voice = Voice::new(
            Url::parse("https://caper.chat/").unwrap(),
            egui::Context::default(),
        );
        voice.command(VoiceOperation::Mute(true));
        voice.command(VoiceOperation::Deafen(true));
        voice.command(VoiceOperation::Deafen(false));
        assert!(voice.state.audio.muted);
        assert!(!voice.state.audio.deafened);
        voice.command(VoiceOperation::Mute(true));
        voice.command(VoiceOperation::Deafen(false));
        assert!(
            voice.state.audio.muted,
            "an idempotent undeafen preserves mute"
        );
        voice.command(VoiceOperation::Deafen(true));
        voice.command(VoiceOperation::Mute(false));
        assert!(!voice.state.audio.muted);
        assert!(!voice.state.audio.deafened);
        voice.command(VoiceOperation::Input("mic-b".into()));
        voice.command(VoiceOperation::Output("speaker-c".into()));
        voice.command(VoiceOperation::DefaultInput);
        assert!(voice.preferences.input.is_none());
        assert_eq!(voice.preferences.output.as_deref(), Some("speaker-c"));
        voice.command(VoiceOperation::DefaultOutput);
        assert!(voice.preferences.output.is_none());
        voice.set_input_processing(157, 63);
        assert_eq!(voice.preferences.input_percent, 157);
        assert_eq!(voice.preferences.processing_strength, 63);
        assert_eq!(voice.state.phase, Phase::Idle);
        assert!(voice.control.is_none());
        assert!(voice.commands.is_none());
    }

    #[test]
    fn actual_join_denial_fails_closed_and_browsing_never_cancels_the_other_call() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = Url::parse(&format!("http://{}/", server.local_addr().unwrap())).unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).unwrap();
            assert!(String::from_utf8_lossy(&bytes[..count]).starts_with("POST /api/media/join "));
            assert!(
                !String::from_utf8_lossy(&bytes[..count])
                    .to_ascii_lowercase()
                    .contains("authorization:")
            );
            stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: 50\r\nConnection: close\r\n\r\n{\"code\":\"access_denied\",\"error\":\"No voice access\"}").unwrap();
        });
        let mut voice = Voice::new(base, egui::Context::default());
        voice.join(
            context("general"),
            None,
            Some("account-not-for-public-media".into()),
            "Guest".into(),
        );
        voice.state.browse("planning".into());
        assert_eq!(voice.state.active_channel(), Some("general"));
        handle.join().unwrap();
        for _ in 0..100 {
            voice.receive();
            if voice.error.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(voice.state.phase, Phase::Idle);
        assert_eq!(voice.error.as_deref(), Some("No voice access"));
    }

    #[test]
    fn leave_during_real_join_request_cancels_and_rejects_late_completion() {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = Url::parse(&format!("http://{}/", server.local_addr().unwrap())).unwrap();
        let (started, received) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            let mut bytes = [0; 4096];
            let count = stream.read(&mut bytes).unwrap();
            assert!(String::from_utf8_lossy(&bytes[..count]).starts_with("POST /api/media/join "));
            started.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(200));
        });
        let mut voice = Voice::new(base, egui::Context::default());
        voice.join(context("general"), None, None, "Guest".into());
        received.recv_timeout(Duration::from_secs(3)).unwrap();
        let now = Instant::now();
        voice.leave();
        assert!(now.elapsed() < Duration::from_millis(100));
        handle.join().unwrap();
        voice.receive();
        assert_eq!(voice.state.phase, Phase::Idle);
        assert!(voice.error.is_none());
    }
}
