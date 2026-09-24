//! Explicit, local-only microphone comparison. No HTTP, SFU or persistent PCM.

use super::select_device;
use libwebrtc::audio_frame::AudioFrame;
use libwebrtc::audio_source::{AudioSourceOptions, native::NativeAudioSource};
use libwebrtc::audio_stream::native::NativeAudioStream;
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
    finish_recording: Arc<AtomicBool>,
    playback_generation: Arc<AtomicU64>,
}

struct TestDevices {
    input: PeerConnectionFactory,
    output: PeerConnectionFactory,
    sender: Option<PeerConnection>,
    receiver: Option<PeerConnection>,
    playback: Option<NativeAudioSource>,
    input_acquired: bool,
    output_acquired: bool,
    released: bool,
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
                playback: None,
                input_acquired: false,
                output_acquired: false,
                released: false,
            })),
            stopped: Arc::new(AtomicBool::new(false)),
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
        self.finish_recording.store(true, Ordering::Release);
        if let Ok(mut devices) = self.devices.lock() {
            self.playback_generation.fetch_add(1, Ordering::AcqRel);
            if devices.released {
                return;
            }
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
}

pub struct MicTest {
    control: MicTestControl,
    sender: RtpSender,
    microphone: MediaStreamTrack,
    stream: NativeAudioStream,
    playback: NativeAudioSource,
    output_guid: Option<String>,
    sample: Option<MicSample>,
}

impl MicTest {
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
        let stream = NativeAudioStream::new(track, RATE as i32, 1);
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
        })
    }

    /// Record at most 30 seconds. `finish_recording` can end it earlier.
    pub async fn record(&mut self, strength: u8) -> Result<&MicSample, String> {
        if strength > 100 {
            return Err("voice processing strength must be 0–100".into());
        }
        if self.sample.is_some() {
            return Err("recording already completed".into());
        }
        {
            let devices = self
                .control
                .devices
                .lock()
                .map_err(|_| "microphone test unavailable")?;
            self.control.ensure_active()?;
            self.microphone.set_enabled(true);
            devices.input.set_adm_recording_enabled(true);
        }
        let started = Instant::now();
        let mut last_frame = started;
        let mut natural = Vec::new();
        while started.elapsed() < Duration::from_secs(30) {
            self.control.ensure_active()?;
            if self.control.finish_recording.load(Ordering::Acquire) {
                break;
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
                    natural.extend_from_slice(frame.data.as_ref());
                    last_frame = Instant::now();
                }
                None => {
                    self.control.cancel();
                    return Err("microphone stopped delivering audio".into());
                }
            }
        }
        self.microphone.set_enabled(false);
        {
            let devices = self
                .control
                .devices
                .lock()
                .map_err(|_| "microphone test unavailable")?;
            self.control.ensure_active()?;
            devices.input.set_adm_recording_enabled(false);
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
        let enhanced = process_voice(&natural, strength);
        self.sample = Some(MicSample {
            natural,
            enhanced,
            sample_rate: RATE,
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
        self.control.cancel();
    }
}

// Local comparison only. Strength zero is bit-exact passthrough. At nonzero
// strength: 75 Hz HPF, 180 Hz warmth, 3 kHz presence, soft compression,
// makeup and a -2 dB peak limiter. This approximates the web node chain;
// browser DynamicsCompressor transfer curves are not standardized.
fn process_voice(input: &[i16], strength: u8) -> Vec<i16> {
    if strength == 0 {
        return input.to_vec();
    }
    let amount = f32::from(strength) / 100.0;
    let dt = 1.0 / RATE as f32;
    let hp_alpha = 1.0 / (1.0 + 2.0 * std::f32::consts::PI * 75.0 * amount * dt);
    let low_alpha = 2.0 * std::f32::consts::PI * 180.0 * dt;
    let presence_alpha = 2.0 * std::f32::consts::PI * 3_000.0 * dt;
    let warmth = 10.0_f32.powf(2.0 * amount / 20.0) - 1.0;
    let presence = 10.0_f32.powf(1.5 * amount / 20.0) - 1.0;
    let makeup = 1.35_f32.powf(amount);
    let mut prev = 0.0;
    let mut high = 0.0;
    let mut low = 0.0;
    let mut upper = 0.0;
    let mut envelope = 0.0;
    input
        .iter()
        .map(|sample| {
            let value = f32::from(*sample) / 32768.0;
            high = hp_alpha * (high + value - prev);
            prev = value;
            low += low_alpha * (high - low);
            upper += presence_alpha * (high - upper);
            let shaped = high + low * warmth + (upper - low) * presence;
            let level = shaped.abs();
            let speed = if level > envelope { 0.008 } else { 0.18 };
            envelope += (1.0 - (-dt / speed).exp()) * (level - envelope);
            let threshold = 10.0_f32.powf(-24.0 / 20.0);
            let ratio = 1.0 + 2.0 * amount;
            let compressed = if envelope > threshold {
                shaped * (threshold / envelope).powf(1.0 - 1.0 / ratio)
            } else {
                shaped
            } * makeup;
            let limit = 10.0_f32.powf(-2.0 / 20.0);
            (compressed.clamp(-limit, limit) * 32768.0) as i16
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processing_zero_is_exact_and_enhanced_has_bounded_output() {
        let input = [0, 1_000, -5_000, 28_000, -32_000];
        assert_eq!(process_voice(&input, 0), input);
        let enhanced = process_voice(&input, 100);
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
        let sample = test.record(25).await.unwrap();
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
        test.play_prepared(false, 100, replay).await.unwrap();
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
