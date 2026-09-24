//! Explicit, local-only microphone comparison. No HTTP, SFU or persistent PCM.

#[cfg(test)]
use super::input_processing::VoiceProcessor;
use super::input_processing::{InputProcessing, ProcessedVoice};
use super::select_device;
use libwebrtc::audio_frame::AudioFrame;
use libwebrtc::audio_source::{AudioSourceOptions, native::NativeAudioSource};
use libwebrtc::audio_stream::native::{NativeAudioStream, NativeAudioStreamOptions};
use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection::{
    AnswerOptions, OfferOptions, PeerConnection, PeerConnectionState,
};
use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use libwebrtc::peer_connection_factory::{
    ContinualGatheringPolicy, PeerConnectionFactory, RtcConfiguration,
};
use libwebrtc::rtp_sender::RtpSender;
use libwebrtc::rtp_transceiver::{RtpTransceiverDirection, RtpTransceiverInit};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

const RATE: u32 = 48_000;
const FRAME: usize = 480;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackToken(u64);

#[derive(Clone)]
pub struct MicTestControl {
    devices: Arc<Mutex<TestDevices>>,
    stopped: Arc<AtomicBool>,
    live_enabled: Arc<AtomicBool>,
    finish_recording: Arc<AtomicBool>,
    playback_generation: Arc<AtomicU64>,
}

struct TestDevices {
    input: PeerConnectionFactory,
    output: PeerConnectionFactory,
    sender: Option<PeerConnection>,
    receiver: Option<PeerConnection>,
    capture_track: Option<MediaStreamTrack>,
    playback: Option<NativeAudioSource>,
    input_acquired: bool,
    output_acquired: bool,
    released: bool,
    capture_epoch: u64,
    bound_epoch: Option<u64>,
    previously_live: bool,
    fresh_on_reopen: bool,
}

impl MicTestControl {
    /// Create the stop handle synchronously, before the worker begins setup.
    /// Constructing it does not acquire an ADM or start capture.
    pub fn new() -> Self {
        let input = PeerConnectionFactory::default();
        input.set_adm_recording_enabled(false);
        input.set_adm_playout_enabled(false);
        let output = PeerConnectionFactory::default();
        output.set_adm_recording_enabled(false);
        output.set_adm_playout_enabled(false);
        Self {
            devices: Arc::new(Mutex::new(TestDevices {
                input,
                output,
                sender: None,
                receiver: None,
                capture_track: None,
                playback: None,
                input_acquired: false,
                output_acquired: false,
                released: false,
                capture_epoch: 0,
                bound_epoch: None,
                previously_live: false,
                fresh_on_reopen: false,
            })),
            stopped: Arc::new(AtomicBool::new(false)),
            live_enabled: Arc::new(AtomicBool::new(false)),
            finish_recording: Arc::new(AtomicBool::new(false)),
            playback_generation: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn finish_recording(&self) {
        self.finish_recording.store(true, Ordering::Release);
    }

    /// Call synchronously at the explicit Play action, before enqueueing the
    /// worker command. Stop invalidates even a play that has not been polled.
    pub fn prepare_playback(&self) -> PlaybackToken {
        PlaybackToken(self.playback_generation.fetch_add(1, Ordering::AcqRel) + 1)
    }

    /// Stop output synchronously without discarding the recording or peers.
    /// A later explicit `play` starts a new generation of the same sample.
    pub fn stop_playback(&self) {
        if let Ok(devices) = self.devices.lock() {
            self.playback_generation.fetch_add(1, Ordering::AcqRel);
            if let Some(source) = &devices.playback {
                source.clear_buffer();
            }
            if !devices.released {
                devices.output.set_adm_playout_enabled(false);
            }
        }
    }

    /// The caller may invoke this from the UI thread during any await.
    pub fn cancel(&self) {
        self.stopped.store(true, Ordering::Release);
        self.live_enabled.store(false, Ordering::Release);
        self.finish_recording.store(true, Ordering::Release);
        if let Ok(mut devices) = self.devices.lock() {
            devices.capture_epoch = devices.capture_epoch.wrapping_add(1);
            devices.bound_epoch = None;
            self.playback_generation.fetch_add(1, Ordering::AcqRel);
            if devices.released {
                return;
            }
            if let Some(track) = &devices.capture_track {
                track.set_enabled(false);
            }
            devices.input.stop_recording();
            devices.input.set_adm_recording_enabled(false);
            devices.output.set_adm_playout_enabled(false);
            if let Some(source) = &devices.playback {
                source.clear_buffer();
            }
            if let Some(peer) = devices.sender.take() {
                peer.close();
            }
            if let Some(peer) = devices.receiver.take() {
                peer.close();
            }
            if devices.input_acquired {
                devices.input.release_platform_adm();
            }
            if devices.output_acquired {
                devices.output.release_platform_adm();
            }
            devices.released = true;
        }
    }

    fn ensure_active(&self) -> Result<(), String> {
        if self.stopped.load(Ordering::Acquire) {
            Err("microphone test was stopped".into())
        } else {
            Ok(())
        }
    }

    fn playback_current(&self, generation: u64) -> Result<bool, String> {
        self.ensure_active()?;
        Ok(self.playback_generation.load(Ordering::Acquire) == generation)
    }

    fn change_live_gate(&self, devices: &mut TestDevices, enabled: bool) {
        if self.live_enabled.swap(enabled, Ordering::AcqRel) != enabled {
            devices.capture_epoch = devices.capture_epoch.wrapping_add(1);
            if !enabled || devices.previously_live {
                devices.bound_epoch = None;
            }
        }
    }

    /// Published microphone capture requires a new private peer on reopen;
    /// a local comparison recording may restart its own selected input.
    pub(crate) fn require_fresh_on_reopen(&self) -> Result<(), String> {
        self.devices
            .lock()
            .map_err(|_| "local capture unavailable")?
            .fresh_on_reopen = true;
        Ok(())
    }

    pub(crate) fn set_live_recording(&self, enabled: bool) -> Result<(), String> {
        let mut devices = self
            .devices
            .lock()
            .map_err(|_| "local capture unavailable")?;
        if devices.released || self.stopped.load(Ordering::Acquire) {
            return Ok(());
        }
        if !enabled {
            self.change_live_gate(&mut devices, false);
            if let Some(track) = &devices.capture_track {
                track.set_enabled(false);
            }
            devices.input.stop_recording();
            devices.input.set_adm_recording_enabled(false);
            return Ok(());
        }
        if self.live_enabled.load(Ordering::Acquire) {
            return Ok(());
        }
        self.change_live_gate(&mut devices, true);
        if devices.previously_live && devices.fresh_on_reopen {
            // Do not reopen the previous APM/encoder/decoder generation. The
            // live task must bind a fresh pair of private peers first.
            return Ok(());
        }
        let track = devices
            .capture_track
            .as_ref()
            .ok_or("local capture is not ready")?
            .clone();
        devices.input.set_adm_recording_enabled(true);
        if !devices.input.init_recording() || !devices.input.start_recording() {
            devices.input.stop_recording();
            devices.input.set_adm_recording_enabled(false);
            return Err("microphone could not start capture".into());
        }
        track.set_enabled(true);
        devices.bound_epoch = Some(devices.capture_epoch);
        devices.previously_live = true;
        Ok(())
    }

    pub(crate) fn live_recording_enabled(&self) -> bool {
        self.live_enabled.load(Ordering::Acquire) && !self.stopped.load(Ordering::Acquire)
    }

    pub(crate) fn live_epoch(&self) -> Option<u64> {
        let devices = self.devices.lock().ok()?;
        (self.live_recording_enabled()
            && !devices.released
            && devices.bound_epoch == Some(devices.capture_epoch))
        .then_some(devices.capture_epoch)
    }

    pub(crate) fn requested_epoch(&self) -> Option<u64> {
        let devices = self.devices.lock().ok()?;
        (self.live_recording_enabled() && !devices.released).then_some(devices.capture_epoch)
    }

    /// Close the entire old private capture path and construct a fresh ADM.
    /// The old receiver/decoder is never attached to the new stream.
    pub(crate) fn retire_capture(&self, epoch: u64) -> Result<(), String> {
        let mut devices = self
            .devices
            .lock()
            .map_err(|_| "local capture unavailable")?;
        if devices.released
            || self.stopped.load(Ordering::Acquire)
            || devices.capture_epoch != epoch
        {
            return Err("microphone capture changed".into());
        }
        devices.bound_epoch = None;
        if let Some(track) = devices.capture_track.take() {
            track.set_enabled(false);
        }
        devices.input.stop_recording();
        devices.input.set_adm_recording_enabled(false);
        if let Some(sender) = devices.sender.take() {
            sender.close();
        }
        if let Some(receiver) = devices.receiver.take() {
            receiver.close();
        }
        if devices.input_acquired {
            devices.input.release_platform_adm();
            devices.input_acquired = false;
        }
        let input = PeerConnectionFactory::default();
        input.set_adm_recording_enabled(false);
        input.set_adm_playout_enabled(false);
        devices.input = input;
        if devices.output_acquired {
            devices.output.release_platform_adm();
            devices.output_acquired = false;
        }
        let output = PeerConnectionFactory::default();
        output.set_adm_recording_enabled(false);
        output.set_adm_playout_enabled(false);
        devices.output = output;
        Ok(())
    }

    pub(crate) fn bind_capture(&self, epoch: u64) -> Result<bool, String> {
        let mut devices = self
            .devices
            .lock()
            .map_err(|_| "local capture unavailable")?;
        if devices.released
            || self.stopped.load(Ordering::Acquire)
            || devices.capture_epoch != epoch
            || !self.live_enabled.load(Ordering::Acquire)
        {
            return Ok(false);
        }
        let track = devices
            .capture_track
            .as_ref()
            .ok_or("local capture is not ready")?
            .clone();
        devices.input.set_adm_recording_enabled(true);
        if !devices.input.init_recording() || !devices.input.start_recording() {
            devices.input.stop_recording();
            devices.input.set_adm_recording_enabled(false);
            return Err("microphone could not restart capture".into());
        }
        track.set_enabled(true);
        devices.bound_epoch = Some(epoch);
        Ok(true)
    }

    /// The zero-buffer source writes synchronously while holding the same
    /// device gate used by mute, route selection and cancel. A stale frame
    /// cannot pass a Boolean false→true transition from a different epoch.
    pub(crate) fn publish_if_current(
        &self,
        source: &NativeAudioSource,
        frame: &AudioFrame<'_>,
        epoch: u64,
    ) -> Result<bool, String> {
        let devices = self
            .devices
            .lock()
            .map_err(|_| "local capture unavailable")?;
        if !self.live_recording_enabled()
            || devices.released
            || devices.bound_epoch != Some(epoch)
            || devices.capture_epoch != epoch
        {
            return Ok(false);
        }
        source
            .capture_frame_direct(frame)
            .map_err(|error| error.to_string())?;
        Ok(true)
    }

    pub(crate) fn select_default_input(&self) -> Result<(), String> {
        self.route_input(
            |factory| factory.select_default_recording_device(),
            "system default microphone is unavailable",
        )
    }

    pub(crate) fn select_input(&self, guid: &str) -> Result<(), String> {
        self.route_input(
            |factory| select_device(factory, guid, true),
            "selected microphone is unavailable",
        )
    }

    fn route_input(
        &self,
        select: impl FnOnce(&PeerConnectionFactory) -> bool,
        error: &'static str,
    ) -> Result<(), String> {
        let mut devices = self
            .devices
            .lock()
            .map_err(|_| "local capture unavailable")?;
        if devices.released || self.stopped.load(Ordering::Acquire) {
            return Err("voice call was stopped".into());
        }
        let was_live = self.live_enabled.load(Ordering::Acquire);
        self.change_live_gate(&mut devices, false);
        if !was_live {
            devices.capture_epoch = devices.capture_epoch.wrapping_add(1);
            devices.bound_epoch = None;
        }
        if let Some(track) = &devices.capture_track {
            track.set_enabled(false);
        }
        devices.input.stop_recording();
        devices.input.set_adm_recording_enabled(false);
        if !select(&devices.input) {
            return Err(error.into());
        }
        if was_live {
            self.change_live_gate(&mut devices, true);
            if !devices.fresh_on_reopen {
                devices.input.set_adm_recording_enabled(true);
                if !devices.input.init_recording() || !devices.input.start_recording() {
                    devices.input.stop_recording();
                    devices.input.set_adm_recording_enabled(false);
                    return Err("microphone could not restart capture".into());
                }
                if let Some(track) = &devices.capture_track {
                    track.set_enabled(true);
                }
                devices.bound_epoch = Some(devices.capture_epoch);
            }
        }
        Ok(())
    }

    fn register_peers(
        &self,
        sender: &PeerConnection,
        receiver: &PeerConnection,
    ) -> Result<(), String> {
        let mut devices = self
            .devices
            .lock()
            .map_err(|_| "microphone test unavailable")?;
        if self.ensure_active().is_err() || devices.released {
            drop(devices);
            sender.close();
            receiver.close();
            return Err("microphone test was stopped".into());
        }
        devices.sender = Some(sender.clone());
        devices.receiver = Some(receiver.clone());
        Ok(())
    }
}

pub struct MicSample {
    /// Captured locally at 48 kHz mono. Never sent to an API or logged.
    pub natural: Vec<i16>,
    pub enhanced: Vec<i16>,
    pub sample_rate: u32,
    #[cfg(test)]
    pub(crate) raw_fixture: Vec<i16>,
}

pub struct MicTest {
    control: MicTestControl,
    sender: RtpSender,
    microphone: MediaStreamTrack,
    stream: NativeAudioStream,
    playback: NativeAudioSource,
    output_guid: Option<String>,
    sample: Option<MicSample>,
    cancel_on_drop: bool,
}

impl MicTest {
    pub(crate) async fn next_live_frame(&mut self) -> Option<AudioFrame<'static>> {
        self.stream.next_frame().await
    }

    pub(crate) fn reset_live_stream(&mut self) {
        let track = self.stream.track();
        self.stream.close();
        self.stream = NativeAudioStream::with_options(
            track,
            RATE as i32,
            1,
            NativeAudioStreamOptions {
                queue_size_frames: Some(8),
            },
        );
    }

    /// The caller has already fenced publication. Detach this private
    /// decoder before negotiating a new sender/receiver and input ADM.
    pub(crate) fn retire_for_reconnect(&mut self) {
        self.stream.close();
        self.cancel_on_drop = false;
    }

    /// Replace the actual private capture transport. A new ADM, device
    /// track, sender, receiver and decoder must all be ready before unmuting.
    pub(crate) async fn reconnect_live(
        &mut self,
        epoch: u64,
        input_guid: Option<&str>,
    ) -> Result<bool, String> {
        self.retire_for_reconnect();
        self.control.retire_capture(epoch)?;
        let mut replacement = Self::start(self.control.clone(), input_guid, None).await?;
        if self.control.requested_epoch() != Some(epoch) {
            replacement.retire_for_reconnect();
            *self = replacement;
            return Ok(false);
        }
        *self = replacement;
        self.control.bind_capture(epoch)
    }

    /// An explicit user action only: acquires a private ADM and connects two
    /// local WebRTC peers. The output remains disabled until `play` is called.
    pub async fn start(
        control: MicTestControl,
        input_guid: Option<&str>,
        output_guid: Option<&str>,
    ) -> Result<Self, String> {
        let (input, output) = {
            let mut devices = control
                .devices
                .lock()
                .map_err(|_| "microphone test unavailable")?;
            control.ensure_active()?;
            if !devices.input.acquire_platform_adm() {
                return Err("microphone is unavailable".into());
            }
            devices.input_acquired = true;
            (devices.input.clone(), devices.output.clone())
        };
        let result = Self::connect(input, output, control.clone(), input_guid, output_guid).await;
        if result.is_err() {
            control.cancel();
        }
        result
    }

    async fn connect(
        input: PeerConnectionFactory,
        output: PeerConnectionFactory,
        control: MicTestControl,
        input_guid: Option<&str>,
        output_guid: Option<&str>,
    ) -> Result<Self, String> {
        if input_guid.is_some_and(|id| !select_device(&input, id, true)) {
            return Err("selected microphone is unavailable".into());
        }
        let mut config = RtcConfiguration::default();
        config.continual_gathering_policy = ContinualGatheringPolicy::GatherOnce;
        let sender = input
            .create_peer_connection(config.clone())
            .map_err(|error| error.to_string())?;
        let receiver = match output.create_peer_connection(config) {
            Ok(receiver) => receiver,
            Err(error) => {
                sender.close();
                return Err(error.to_string());
            }
        };
        control.register_peers(&sender, &receiver)?;
        let microphone = input.create_device_audio_track("caper-local-mic-test");
        microphone.set_enabled(false);
        {
            let mut devices = control
                .devices
                .lock()
                .map_err(|_| "local capture unavailable")?;
            control.ensure_active()?;
            devices.capture_track = Some(microphone.clone().into());
        }
        let transceiver = sender
            .add_transceiver(
                microphone.clone().into(),
                RtpTransceiverInit {
                    direction: RtpTransceiverDirection::SendOnly,
                    stream_ids: vec!["local-mic-test".into()],
                    send_encodings: vec![],
                },
            )
            .map_err(|error| error.to_string())?;
        let (tracks_tx, mut tracks_rx) = mpsc::unbounded_channel();
        receiver.on_track(Some(Box::new(move |event| {
            if let MediaStreamTrack::Audio(track) = event.track {
                let _ = tracks_tx.send(track);
            }
        })));
        let (to_receiver, mut receiver_candidates) = mpsc::unbounded_channel();
        let (to_sender, mut sender_candidates) = mpsc::unbounded_channel();
        sender.on_ice_candidate(Some(Box::new(move |candidate| {
            let _ = to_receiver.send(candidate);
        })));
        receiver.on_ice_candidate(Some(Box::new(move |candidate| {
            let _ = to_sender.send(candidate);
        })));
        let offer = sender
            .create_offer(OfferOptions::default())
            .await
            .map_err(|error| error.to_string())?;
        sender
            .set_local_description(offer.clone())
            .await
            .map_err(|error| error.to_string())?;
        receiver
            .set_remote_description(offer)
            .await
            .map_err(|error| error.to_string())?;
        let answer = receiver
            .create_answer(AnswerOptions::default())
            .await
            .map_err(|error| error.to_string())?;
        receiver
            .set_local_description(answer.clone())
            .await
            .map_err(|error| error.to_string())?;
        sender
            .set_remote_description(answer)
            .await
            .map_err(|error| error.to_string())?;
        let connected = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                control.ensure_active()?;
                if sender.connection_state() == PeerConnectionState::Connected && receiver.connection_state() == PeerConnectionState::Connected {
                    return Ok::<_, String>(());
                }
                tokio::select! {
                    Some(candidate) = sender_candidates.recv() => sender.add_ice_candidate(candidate).await.map_err(|error| error.to_string())?,
                    Some(candidate) = receiver_candidates.recv() => receiver.add_ice_candidate(candidate).await.map_err(|error| error.to_string())?,
                    _ = tokio::time::sleep(Duration::from_millis(20)) => {},
                }
            }
        }).await.map_err(|_| "local microphone test connection timed out".to_owned())?;
        connected?;
        let track = tokio::time::timeout(Duration::from_secs(2), tracks_rx.recv())
            .await
            .map_err(|_| "local microphone test track timed out".to_owned())?
            .ok_or_else(|| "local microphone test track ended".to_owned())?;
        let stream = NativeAudioStream::with_options(
            track,
            RATE as i32,
            1,
            NativeAudioStreamOptions {
                queue_size_frames: Some(8),
            },
        );
        let playback = NativeAudioSource::new(AudioSourceOptions::default(), RATE, 1, 0);
        {
            let mut devices = control
                .devices
                .lock()
                .map_err(|_| "microphone test unavailable")?;
            control.ensure_active()?;
            devices.playback = Some(playback.clone());
        }
        Ok(Self {
            control,
            sender: transceiver.sender(),
            microphone: microphone.into(),
            stream,
            playback,
            output_guid: output_guid.map(str::to_owned),
            sample: None,
            cancel_on_drop: true,
        })
    }

    /// Record at most 30 seconds. `finish_recording` can end it earlier.
    pub async fn record(&mut self, strength: u8) -> Result<&MicSample, String> {
        self.record_with_processing(100, strength).await
    }

    /// Natural replay uses gain + denoise, and enhanced additionally applies
    /// the live contour. Raw capture is not retained after this operation.
    pub async fn record_with_processing(
        &mut self,
        gain_percent: u16,
        strength: u8,
    ) -> Result<&MicSample, String> {
        if gain_percent > 200 || strength > 100 {
            return Err("input gain must be 0–200 and processing strength 0–100".into());
        }
        if self.sample.is_some() {
            return Err("recording already completed".into());
        }
        self.control.set_live_recording(true)?;
        let started = Instant::now();
        let mut last_frame = started;
        let mut natural = Vec::new();
        let mut recorded_epoch = None;
        while started.elapsed() < Duration::from_secs(30) {
            self.control.ensure_active()?;
            if self.control.finish_recording.load(Ordering::Acquire) {
                break;
            }
            let epoch = self.control.live_epoch();
            if recorded_epoch != epoch {
                self.reset_live_stream();
                natural.clear();
                recorded_epoch = epoch;
                last_frame = Instant::now();
            }
            if epoch.is_none() {
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            }
            if last_frame.elapsed() >= Duration::from_secs(2) {
                self.control.cancel();
                return Err("microphone stopped delivering audio".into());
            }
            let frame = tokio::select! {
                frame = self.stream.next_frame() => frame,
                _ = tokio::time::sleep(Duration::from_millis(20)) => continue,
            };
            match frame {
                Some(frame) => {
                    if self.control.live_epoch() != epoch {
                        continue;
                    }
                    natural.extend_from_slice(frame.data.as_ref());
                    last_frame = Instant::now();
                }
                None => {
                    self.control.cancel();
                    return Err("microphone stopped delivering audio".into());
                }
            }
        }
        self.control.set_live_recording(false)?;
        {
            let devices = self
                .control
                .devices
                .lock()
                .map_err(|_| "microphone test unavailable")?;
            self.control.ensure_active()?;
            let playback_track = devices
                .input
                .create_audio_track("caper-local-sample", self.playback.clone());
            if let Err(error) = self.sender.set_track(Some(playback_track.into())) {
                drop(devices);
                self.control.cancel();
                return Err(error.to_string());
            }
        }
        if natural.is_empty() {
            self.control.cancel();
            return Err("no microphone audio was recorded".into());
        }
        #[cfg(test)]
        let raw_fixture = natural.clone();
        let (natural, enhanced) = tokio::task::spawn_blocking(move || {
            let mut processor = ProcessedVoice::new();
            let mut clean = Vec::with_capacity(natural.len());
            let mut enhanced = Vec::with_capacity(natural.len());
            for chunk in natural.chunks(FRAME) {
                let mut frame = [0; FRAME];
                frame[..chunk.len()].copy_from_slice(chunk);
                let (raw, shaped) = processor.process(
                    &frame,
                    InputProcessing {
                        gain_percent,
                        strength,
                    },
                )?;
                clean.extend_from_slice(&raw[..chunk.len()]);
                enhanced.extend_from_slice(&shaped[..chunk.len()]);
            }
            Ok::<_, String>((clean, enhanced))
        })
        .await
        .map_err(|_| "microphone processor stopped")??;
        self.control.ensure_active()?;
        self.sample = Some(MicSample {
            natural,
            enhanced,
            sample_rate: RATE,
            #[cfg(test)]
            raw_fixture,
        });
        Ok(self.sample.as_ref().unwrap())
    }

    /// Convenience for direct callers; UI commands must prepare at click time
    /// and pass that token to `play_prepared` instead.
    pub async fn play(&self, enhanced: bool, volume_percent: u16) -> Result<(), String> {
        self.play_prepared(enhanced, volume_percent, self.control.prepare_playback())
            .await
    }

    /// Plays the selected sample locally through the chosen output. No network
    /// service sees the PCM. A stopped queued play returns without opening ADM.
    pub async fn play_prepared(
        &self,
        enhanced: bool,
        volume_percent: u16,
        token: PlaybackToken,
    ) -> Result<(), String> {
        if volume_percent > 200 {
            return Err("output gain must be 0–200 percent".into());
        }
        let sample = self.sample.as_ref().ok_or("record a sample first")?;
        {
            let mut devices = self
                .control
                .devices
                .lock()
                .map_err(|_| "microphone test unavailable")?;
            self.control.ensure_active()?;
            if self.control.playback_generation.load(Ordering::Acquire) != token.0 {
                return Ok(());
            }
            if !devices.output_acquired {
                if !devices.output.acquire_platform_adm() {
                    return Err("speaker is unavailable".into());
                }
                devices.output_acquired = true;
            }
            if self
                .output_guid
                .as_deref()
                .is_some_and(|id| !select_device(&devices.output, id, false))
            {
                return Err("selected speaker is unavailable".into());
            }
            self.playback.clear_buffer();
            devices.output.set_adm_playout_enabled(true);
        }
        let generation = token.0;
        let pcm = if enhanced {
            &sample.enhanced
        } else {
            &sample.natural
        };
        let result = async {
            let mut tick = tokio::time::interval(Duration::from_millis(10));
            for chunk in pcm.chunks(FRAME) {
                if !self.control.playback_current(generation)? {
                    return Ok(());
                }
                tick.tick().await;
                if !self.control.playback_current(generation)? {
                    return Ok(());
                }
                let data: Vec<_> = chunk
                    .iter()
                    .map(|value| {
                        (f32::from(*value) * f32::from(volume_percent) / 100.0)
                            .clamp(f32::from(i16::MIN), f32::from(i16::MAX))
                            as i16
                    })
                    .chain(std::iter::repeat(0))
                    .take(FRAME)
                    .collect();
                let frame = AudioFrame {
                    data: data.into(),
                    sample_rate: RATE,
                    num_channels: 1,
                    samples_per_channel: FRAME as u32,
                };
                self.playback
                    .capture_frame(&frame)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            self.control.playback_current(generation).map(|_| ())
        }
        .await;
        if let Ok(devices) = self.control.devices.lock()
            && !devices.released
            && self.control.playback_generation.load(Ordering::Acquire) == generation
        {
            devices.output.set_adm_playout_enabled(false);
        }
        if result.is_err() {
            self.control.cancel();
        }
        result
    }

    pub fn stop(&self) {
        self.control.cancel();
    }
}

impl Drop for MicTest {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            self.control.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn late_processed_frame_cannot_cross_mute_reopen_or_cancel_epoch() {
        use std::sync::Barrier;

        let control = MicTestControl::new();
        let source = NativeAudioSource::new(AudioSourceOptions::default(), RATE, 1, 0);
        {
            let mut devices = control.devices.lock().unwrap();
            control.change_live_gate(&mut devices, true);
            devices.bound_epoch = Some(devices.capture_epoch);
        }
        let old_epoch = control.live_epoch().unwrap();
        let ready = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let worker = {
            let (control, source, ready, release) = (
                control.clone(),
                source.clone(),
                ready.clone(),
                release.clone(),
            );
            std::thread::spawn(move || {
                // Captured under epoch 1; processing remains blocked while the
                // UI synchronously mutes and reopens the same microphone.
                ready.wait();
                release.wait();
                let frame = AudioFrame {
                    data: vec![7300; FRAME].into(),
                    sample_rate: RATE,
                    num_channels: 1,
                    samples_per_channel: FRAME as u32,
                };
                control
                    .publish_if_current(&source, &frame, old_epoch)
                    .unwrap()
            })
        };
        ready.wait();
        {
            let mut devices = control.devices.lock().unwrap();
            control.change_live_gate(&mut devices, false);
            control.change_live_gate(&mut devices, true);
            devices.bound_epoch = Some(devices.capture_epoch);
        }
        release.wait();
        assert!(
            !worker.join().unwrap(),
            "old PCM must not publish after reopen"
        );
        let fresh_epoch = control.live_epoch().unwrap();
        let fresh = AudioFrame {
            data: vec![2900; FRAME].into(),
            sample_rate: RATE,
            num_channels: 1,
            samples_per_channel: FRAME as u32,
        };
        assert!(
            control
                .publish_if_current(&source, &fresh, fresh_epoch)
                .unwrap()
        );
        control.cancel();
        assert!(
            !control
                .publish_if_current(&source, &fresh, fresh_epoch)
                .unwrap()
        );
    }

    #[test]
    fn zero_gain_fences_inflight_old_gain_and_positive_gain_waits_for_new_peer() {
        use super::super::JoinControl;
        use std::sync::Barrier;

        let route = JoinControl::new();
        let control = MicTestControl::new();
        control.require_fresh_on_reopen().unwrap();
        *route.capture.lock().unwrap() = Some(control.clone());
        {
            let mut devices = control.devices.lock().unwrap();
            control.change_live_gate(&mut devices, true);
            devices.bound_epoch = Some(devices.capture_epoch);
            devices.previously_live = true;
        }
        route.set_input_processing(175, 30).unwrap();
        let old_epoch = control.live_epoch().unwrap();
        let source = NativeAudioSource::new(AudioSourceOptions::default(), RATE, 1, 0);
        let old_frame = AudioFrame {
            data: vec![8_100; FRAME].into(),
            sample_rate: RATE,
            num_channels: 1,
            samples_per_channel: FRAME as u32,
        };
        let ready = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let delayed = {
            let (control, source, ready, release) = (
                control.clone(),
                source.clone(),
                ready.clone(),
                release.clone(),
            );
            std::thread::spawn(move || {
                ready.wait();
                release.wait();
                control
                    .publish_if_current(&source, &old_frame, old_epoch)
                    .unwrap()
            })
        };
        ready.wait();
        route.set_input_processing(0, 30).unwrap();
        assert_eq!(control.live_epoch(), None);
        route.set_input_processing(125, 30).unwrap();
        control.set_live_recording(true).unwrap();
        assert_eq!(
            control.live_epoch(),
            None,
            "positive gain cannot reopen the old peer"
        );
        release.wait();
        assert!(
            !delayed.join().unwrap(),
            "old high-gain PCM crossed zero→positive"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires isolated private PulseAudio null input and output"]
    async fn delayed_old_decoder_source_cannot_enter_reopened_capture_peer() {
        assert_eq!(
            std::env::var("PULSE_SERVER").unwrap(),
            "unix:/tmp/caper-voice-silent-parity/native"
        );
        let control = MicTestControl::new();
        let mut test = MicTest::start(control.clone(), None, None).await.unwrap();
        control.require_fresh_on_reopen().unwrap();
        let old_factory = control.devices.lock().unwrap().input.clone();
        let old_source = NativeAudioSource::new(AudioSourceOptions::default(), RATE, 1, 0);
        let old_track =
            old_factory.create_audio_track("private-old-generation", old_source.clone());
        test.sender.set_track(Some(old_track.into())).unwrap();
        control.set_live_recording(true).unwrap();
        let old_epoch = control.live_epoch().unwrap();
        let old_frame = AudioFrame {
            data: vec![7_300; FRAME].into(),
            sample_rate: RATE,
            num_channels: 1,
            samples_per_channel: FRAME as u32,
        };
        for _ in 0..35 {
            old_source.capture_frame_direct(&old_frame).unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let frame = test.next_live_frame().await.unwrap();
                if frame.data.iter().any(|value| *value > 1_000) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        // Leave old PCM queued in the decoder, then reopen without reading it.
        for _ in 0..12 {
            old_source.capture_frame_direct(&old_frame).unwrap();
        }
        control.set_live_recording(false).unwrap();
        control.set_live_recording(true).unwrap();
        let requested = control.requested_epoch().unwrap();
        assert_ne!(requested, old_epoch);
        assert_eq!(control.live_epoch(), None);
        assert!(
            !control
                .publish_if_current(&old_source, &old_frame, old_epoch)
                .unwrap()
        );
        assert!(test.reconnect_live(requested, None).await.unwrap());
        // Delayed old-source PCM after the new receiver is bound must be
        // inaudible; the new source below must still produce decoded PCM.
        for _ in 0..25 {
            old_source.capture_frame_direct(&old_frame).unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        for _ in 0..8 {
            let Ok(Some(frame)) =
                tokio::time::timeout(Duration::from_millis(30), test.next_live_frame()).await
            else {
                continue; // silence suppression may emit no RTP at all
            };
            assert!(
                frame.data.iter().all(|value| value.unsigned_abs() < 500),
                "old decoder PCM reached the replacement receiver"
            );
        }
        let fresh_factory = control.devices.lock().unwrap().input.clone();
        let fresh_source = NativeAudioSource::new(AudioSourceOptions::default(), RATE, 1, 0);
        let fresh_track =
            fresh_factory.create_audio_track("private-new-generation", fresh_source.clone());
        test.sender.set_track(Some(fresh_track.into())).unwrap();
        let fresh_frame = AudioFrame {
            data: vec![-5_100; FRAME].into(),
            sample_rate: RATE,
            num_channels: 1,
            samples_per_channel: FRAME as u32,
        };
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                fresh_source.capture_frame_direct(&fresh_frame).unwrap();
                tokio::time::sleep(Duration::from_millis(10)).await;
                if let Ok(Some(frame)) =
                    tokio::time::timeout(Duration::from_millis(10), test.next_live_frame()).await
                    && frame.data.iter().any(|value| *value < -1_000)
                {
                    break;
                }
            }
        })
        .await
        .expect("fresh capture did not reach the new decoded track");
    }

    #[test]
    fn processing_zero_is_exact_and_enhanced_has_bounded_output() {
        let input = [0, 1_000, -5_000, 28_000, -32_000];
        let process = |gain_percent, strength| {
            VoiceProcessor::default().process(
                &input,
                InputProcessing {
                    gain_percent,
                    strength,
                },
            )
        };
        assert_eq!(process(100, 0), input);
        assert_eq!(process(175, 0), [0, 1750, -8750, 32767, -32768]);
        assert_eq!(process(0, 80), [0; 5]);
        let enhanced = process(100, 100);
        assert_ne!(enhanced, input);
        assert!(
            enhanced
                .iter()
                .all(|sample| sample.unsigned_abs() <= 29_220)
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires isolated private PulseAudio null input and output"]
    async fn private_virtual_microphone_capture_and_local_playback() {
        assert_eq!(
            std::env::var("PULSE_SERVER").unwrap(),
            "unix:/tmp/caper-voice-silent-parity/native"
        );
        let control = MicTestControl::new();
        let mut test = MicTest::start(control.clone(), None, None).await.unwrap();
        assert!(
            !control
                .devices
                .lock()
                .unwrap()
                .input
                .adm_recording_enabled()
        );
        let stop = control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            stop.finish_recording();
        });
        let (recorded, reset) = tokio::join!(test.record(25), async {
            tokio::time::sleep(Duration::from_millis(100)).await;
            control.select_default_input()
        });
        reset.unwrap();
        let sample = recorded.unwrap();
        assert!(sample.natural.len() >= RATE as usize / 10);
        assert_eq!(sample.enhanced.len(), sample.natural.len());
        assert_eq!(sample.sample_rate, RATE);
        assert!(
            !control
                .devices
                .lock()
                .unwrap()
                .input
                .adm_recording_enabled()
        );
        let sink_inputs = || {
            std::process::Command::new("pactl")
                .args(["list", "short", "sink-inputs"])
                .output()
                .unwrap()
                .stdout
        };
        let queued = control.prepare_playback();
        control.stop_playback();
        test.play_prepared(false, 100, queued).await.unwrap();
        assert!(!control.devices.lock().unwrap().output_acquired);
        assert!(
            sink_inputs().is_empty(),
            "stopped queued play opened output"
        );
        let replay = control.prepare_playback();
        let (played, reset) = tokio::join!(test.play_prepared(false, 100, replay), async {
            tokio::time::sleep(Duration::from_millis(70)).await;
            control
                .devices
                .lock()
                .unwrap()
                .output
                .select_default_playout_device()
        });
        played.unwrap();
        assert!(reset, "in-call system-default output reset failed");
        assert!(
            sink_inputs().is_empty(),
            "private sink was not idle after playback"
        );
        // Exercise the output path with an intentionally synthetic fixture.
        // The private null sink cannot play to hardware. The second local
        // peer must decode nonzero playback PCM after the source swap.
        test.sample.as_mut().unwrap().enhanced = vec![8_000; FRAME * 30];
        let mut observed = NativeAudioStream::new(test.stream.track(), RATE as i32, 1);
        let (played, received, output_open) = tokio::join!(
            test.play(true, 100),
            async {
                tokio::time::timeout(Duration::from_secs(2), async {
                    loop {
                        if observed
                            .next_frame()
                            .await
                            .unwrap()
                            .data
                            .iter()
                            .any(|value| *value != 0)
                        {
                            break true;
                        }
                    }
                })
                .await
                .unwrap_or(false)
            },
            async {
                tokio::time::sleep(Duration::from_millis(70)).await;
                !sink_inputs().is_empty()
            }
        );
        played.unwrap();
        assert!(
            received,
            "synthetic playback did not reach local output peer"
        );
        assert!(
            output_open,
            "virtual output sink was never opened for playback"
        );
        test.sample.as_mut().unwrap().enhanced = vec![8_000; FRAME * 100];
        let stop = control.clone();
        let (interrupted, replayed) = tokio::join!(test.play(true, 100), async {
            tokio::time::sleep(Duration::from_millis(60)).await;
            stop.stop_playback();
            assert!(
                sink_inputs().is_empty(),
                "stop must close virtual output now"
            );
            assert!(!stop.stopped.load(Ordering::Acquire));
            assert!(stop.devices.lock().unwrap().sender.is_some());
            test.play(false, 100).await
        });
        assert!(interrupted.is_ok(), "stopped generation is benign");
        replayed.unwrap();
        assert_eq!(test.sample.as_ref().unwrap().enhanced.len(), FRAME * 100);
        test.play(true, 100).await.unwrap();
        let stop = control.clone();
        let (interrupted, ()) = tokio::join!(test.play(true, 100), async {
            tokio::time::sleep(Duration::from_millis(60)).await;
            stop.cancel();
        });
        assert!(interrupted.is_err(), "stop must interrupt local playback");
        assert!(
            sink_inputs().is_empty(),
            "stop must close the virtual output"
        );
        assert!(test.play(true, 100).await.is_err());
    }

    #[tokio::test]
    async fn stopped_before_setup_never_acquires_a_device() {
        let control = MicTestControl::new();
        control.cancel();
        assert!(MicTest::start(control, None, None).await.is_err());
    }

    #[test]
    fn cancelled_setup_cannot_publish_created_peers() {
        let control = MicTestControl::new();
        let (sender, receiver) = {
            let devices = control.devices.lock().unwrap();
            (
                devices
                    .input
                    .create_peer_connection(RtcConfiguration::default())
                    .unwrap(),
                devices
                    .output
                    .create_peer_connection(RtcConfiguration::default())
                    .unwrap(),
            )
        };
        control.cancel();
        assert!(control.register_peers(&sender, &receiver).is_err());
        let devices = control.devices.lock().unwrap();
        assert!(devices.sender.is_none() && devices.receiver.is_none());
        assert_eq!(sender.connection_state(), PeerConnectionState::Closed);
        assert_eq!(receiver.connection_state(), PeerConnectionState::Closed);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires isolated private PulseAudio null input and output"]
    async fn stop_during_local_setup_does_not_activate_capture() {
        assert_eq!(
            std::env::var("PULSE_SERVER").unwrap(),
            "unix:/tmp/caper-voice-silent-parity/native"
        );
        let control = MicTestControl::new();
        let worker = tokio::spawn(MicTest::start(control.clone(), None, None));
        tokio::time::timeout(Duration::from_secs(2), async {
            while control.devices.lock().unwrap().sender.is_none() {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        control.cancel();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), worker)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert!(control.devices.lock().unwrap().released);
    }
}
