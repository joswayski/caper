use crate::{media, media_gateway, state};
use eframe::egui;
use media::{JoinControl, MediaApi, NativeSession, Participant, Snapshot, VoiceError};
use state::{CallContext, CallState, Phase};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use url::Url;

pub struct Voice {
    pub state: CallState,
    pub participants: Vec<Participant>,
    pub self_id: String,
    pub error: Option<String>,
    pub inputs: Vec<(String, String)>,
    pub outputs: Vec<(String, String)>,
    pub input: Option<String>,
    pub output: Option<String>,
    base: Url,
    control: Option<JoinControl>,
    commands: Option<Sender<Operation>>,
    events: Sender<Report>,
    incoming: Receiver<Report>,
    repaint: egui::Context,
    active_space: Option<String>,
}

enum Operation {
    Mute(bool),
    Deafen(bool),
    Input(String),
    Output(String),
}

enum Report {
    Connected(u64, String, Vec<(String, String)>, Vec<(String, String)>),
    Roster(u64, Snapshot),
    Gateway(u64, media_gateway::Event),
    Failed(u64, VoiceError),
    Changed(u64, Result<(), VoiceError>),
}

impl Voice {
    pub fn new(base: Url, repaint: egui::Context) -> Self {
        let (events, incoming) = mpsc::channel();
        Self {
            state: CallState::default(),
            participants: vec![],
            self_id: String::new(),
            error: None,
            inputs: vec![],
            outputs: vec![],
            input: None,
            output: None,
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
        if let Err(error) =
            control.set_local_audio(self.state.audio.muted, self.state.audio.deafened)
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
        let muted = self.state.audio.muted;
        let deafened = self.state.audio.deafened;
        let input = self.input.clone();
        let output = self.output.clone();
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
                muted,
                deafened,
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
                            Operation::Input(guid) => session.select_input(&guid),
                            Operation::Output(guid) => session.select_output(&guid),
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
            }
            gateway.stop();
            session.leave();
        });
    }

    pub fn leave(&mut self) {
        if let Some(control) = self.control.take() {
            control.cancel();
        }
        self.commands = None;
        self.active_space = None;
        self.participants.clear();
        self.self_id.clear();
        self.state.leave_now();
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
            VoiceOperation::Input(guid) => {
                self.input = Some(guid.clone());
                Operation::Input(guid)
            }
            VoiceOperation::Output(guid) => {
                self.output = Some(guid.clone());
                Operation::Output(guid)
            }
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
                    self.participants = snapshot.participants
                }
                Report::Gateway(g, media_gateway::Event::Snapshot { snapshot, .. })
                    if g == self.state.generation =>
                {
                    self.participants = snapshot.participants
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
                _ => {}
            }
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
        voice.command(VoiceOperation::Mute(false));
        assert!(!voice.state.audio.muted);
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
