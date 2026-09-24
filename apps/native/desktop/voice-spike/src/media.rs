#![allow(dead_code)]

#[path = "dpdfnet.rs"]
mod dpdfnet;
#[path = "dpdfnet_dsp.rs"]
mod dpdfnet_dsp;
#[path = "input_processing.rs"]
mod input_processing;
#[path = "mic_test.rs"]
pub mod mic_test;

#[cfg(all(test, target_os = "linux"))]
#[path = "sfu_smoke.rs"]
mod sfu_smoke;

pub use self::input_processing::AudioProcessingDiagnostics;
#[cfg(test)]
use self::input_processing::VoiceProcessor;
use self::input_processing::{InputProcessing, ProcessedVoice};
use self::mic_test::{MicTest, MicTestControl};
use libwebrtc::MediaType;
use libwebrtc::audio_frame::AudioFrame;
use libwebrtc::audio_source::{AudioSourceOptions, native::NativeAudioSource};
use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection::{
    AnswerOptions, OfferOptions, PeerConnection, PeerConnectionState,
};
use libwebrtc::peer_connection_factory::native::PeerConnectionFactoryExt;
use libwebrtc::peer_connection_factory::{
    ContinualGatheringPolicy, IceServer as RtcIceServer, PeerConnectionFactory, RtcConfiguration,
};
use libwebrtc::rtp_sender::RtpSender;
use libwebrtc::rtp_transceiver::{RtpTransceiverDirection, RtpTransceiverInit};
use libwebrtc::session_description::{SdpType, SessionDescription};
use libwebrtc::stats::{IceCandidateType, RtcStats};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue};
use reqwest::redirect::Policy;
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;
use url::Url;

const CONTROL_TIMEOUT: Duration = Duration::from_secs(25);
const ICE_TIMEOUT: Duration = Duration::from_secs(12);
#[cfg(test)]
const GATHER_TIMEOUT: Duration = Duration::from_secs(25);
const OPERATIONS: &[&str] = &[
    "join",
    "state",
    "leave",
    "snapshot",
    "publish",
    "subscribe",
    "negotiate",
    "close",
    "turn",
    "restart-ice",
    "restart-ice-ack",
];

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IceServer {
    pub urls: Value,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub credential: Option<String>,
}

impl IceServer {
    fn rtc(&self) -> Result<RtcIceServer, String> {
        let urls = match &self.urls {
            Value::String(url) => vec![url.clone()],
            Value::Array(urls) => urls
                .iter()
                .map(|url| {
                    url.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| "invalid ICE URL".to_owned())
                })
                .collect::<Result<_, _>>()?,
            _ => return Err("invalid ICE URL list".into()),
        };
        Ok(RtcIceServer {
            urls,
            username: self.username.clone().unwrap_or_default(),
            password: self.credential.clone().unwrap_or_default(),
        })
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnGeneration {
    pub generation: String,
    pub refresh_after_ms: u64,
    pub expires_in_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinResponse {
    pub token: String,
    pub id: String,
    pub ice_servers: Vec<IceServer>,
    pub turn: Option<TurnGeneration>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurnResponse {
    ice_servers: Vec<IceServer>,
    turn: TurnGeneration,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub kind: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub id: String,
    pub name: String,
    pub country_code: Option<String>,
    pub muted: bool,
    pub deafened: bool,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Snapshot {
    pub participants: Vec<Participant>,
    #[serde(default)]
    pub revision: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagnostics {
    pub received_bytes: u64,
    pub sent_bytes: u64,
    pub receive_bitrate: f64,
    pub send_bitrate: f64,
    pub packets_lost: i64,
    pub max_jitter_ms: f64,
    pub round_trip_ms: f64,
    pub route: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioDevices {
    pub inputs: Vec<AudioDevice>,
    pub outputs: Vec<AudioDevice>,
}

/// Enumerate using a separate factory/ADM. Never changes an active call's
/// capture, output selection, or playout gate; None means platform default.
pub fn enumerate_audio_devices() -> Result<AudioDevices, String> {
    let factory = PeerConnectionFactory::default();
    factory.set_adm_recording_enabled(false);
    factory.set_adm_playout_enabled(false);
    if !factory.acquire_platform_adm() {
        return Err("native audio devices are unavailable".into());
    }
    let devices = AudioDevices {
        inputs: (0..factory.recording_devices().max(0) as u16)
            .map(|index| AudioDevice {
                id: factory.recording_device_guid(index),
                name: factory.recording_device_name(index),
            })
            .collect(),
        outputs: (0..factory.playout_devices().max(0) as u16)
            .map(|index| AudioDevice {
                id: factory.playout_device_guid(index),
                name: factory.playout_device_name(index),
            })
            .collect(),
    };
    factory.release_platform_adm();
    Ok(devices)
}

fn select_device(factory: &PeerConnectionFactory, guid: &str, input: bool) -> bool {
    if guid.is_empty() {
        return false;
    }
    // The pinned SDK silently falls back to index 0 for an unknown GUID.
    // Validate first so a stale preference cannot select a different device.
    let count = if input {
        factory.recording_devices()
    } else {
        factory.playout_devices()
    };
    let found = (0..count.max(0) as u16).any(|index| {
        (if input {
            factory.recording_device_guid(index)
        } else {
            factory.playout_device_guid(index)
        }) == guid
    });
    found
        && if input {
            factory.set_recording_device_by_guid(guid)
        } else {
            factory.set_playout_device_by_guid(guid)
        }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Sdp {
    #[serde(rename = "type")]
    pub kind: String,
    pub sdp: String,
}

impl Sdp {
    fn parse(&self, expected: SdpType) -> Result<SessionDescription, String> {
        if self.kind != expected.to_string() {
            return Err(format!("unexpected SDP type {}", self.kind));
        }
        SessionDescription::parse(&self.sdp, expected).map_err(|error| error.to_string())
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignalingResponse {
    session_description: Option<Sdp>,
    #[serde(default)]
    tracks: Vec<Mid>,
    #[serde(default)]
    requires_immediate_renegotiation: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct Mid {
    mid: String,
}

#[derive(Clone)]
pub struct MediaApi {
    root: Url,
    account_token: Option<String>,
    client: Client,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaError {
    pub operation: Option<String>,
    pub status: Option<u16>,
    pub code: Option<String>,
    pub error_id: Option<String>,
    pub detail: String,
}

impl MediaError {
    fn local(detail: impl Into<String>) -> Self {
        Self {
            operation: None,
            status: None,
            code: None,
            error_id: None,
            detail: detail.into(),
        }
    }

    pub fn denied(&self) -> bool {
        matches!(self.status, Some(401 | 403 | 404))
            && !matches!(self.code.as_deref(), Some("ice_restart_retry"))
    }

    pub fn retryable(&self) -> bool {
        self.code.as_deref() != Some("ice_restart_invalid")
            && (self.status.is_none()
                || self.status == Some(408)
                || matches!(self.status, Some(429 | 500..=599))
                || matches!(
                    self.code.as_deref(),
                    Some("ice_restart_pending" | "ice_restart_retry")
                ))
    }
}

impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.detail)
    }
}

#[derive(Debug)]
pub enum VoiceError {
    Media(MediaError),
    Local(String),
}

impl VoiceError {
    pub fn terminal(&self) -> bool {
        match self {
            Self::Media(error) => !error.retryable(),
            Self::Local(_) => true,
        }
    }
}

impl From<MediaError> for VoiceError {
    fn from(error: MediaError) -> Self {
        Self::Media(error)
    }
}

impl From<String> for VoiceError {
    fn from(detail: String) -> Self {
        Self::Local(detail)
    }
}

impl From<&str> for VoiceError {
    fn from(detail: &str) -> Self {
        Self::Local(detail.into())
    }
}

impl std::fmt::Display for VoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Media(error) => error.fmt(f),
            Self::Local(detail) => detail.fmt(f),
        }
    }
}

impl MediaApi {
    pub fn new(
        base: &Url,
        channel_id: Option<&str>,
        account_token: Option<String>,
    ) -> Result<Self, String> {
        if !base.username().is_empty() || base.password().is_some() {
            return Err("Caper API URLs cannot contain credentials".into());
        }
        let loopback = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
        if base.scheme() != "https" && !(base.scheme() == "http" && loopback) {
            return Err("media API requires HTTPS outside loopback".into());
        }
        if channel_id.is_some_and(|channel| {
            channel.len() != 12 || !channel.bytes().all(|byte| byte.is_ascii_alphanumeric())
        }) {
            return Err("invalid media channel".into());
        }
        let path = channel_id.map_or_else(
            || "api/media/".to_owned(),
            |channel| format!("api/channels/{channel}/media/"),
        );
        let root = base.join(&path).map_err(|_| "invalid media API URL")?;
        let client = Client::builder()
            .timeout(CONTROL_TIMEOUT)
            .redirect(Policy::none())
            .user_agent("Caper-Desktop-Voice/0.1")
            .build()
            .map_err(|_| "could not initialize media networking")?;
        Ok(Self {
            root,
            account_token,
            client,
        })
    }

    async fn post<T: for<'de> Deserialize<'de>>(
        &self,
        operation: &str,
        media_token: Option<&str>,
        body: Value,
    ) -> Result<T, MediaError> {
        self.send(operation, media_token, body)
            .await?
            .json()
            .await
            .map_err(|_| MediaError::local("media service returned invalid JSON"))
    }

    async fn send(
        &self,
        operation: &str,
        media_token: Option<&str>,
        body: Value,
    ) -> Result<Response, MediaError> {
        if !OPERATIONS.contains(&operation) {
            return Err(MediaError::local("unsupported media operation"));
        }
        let mut headers = HeaderMap::new();
        if let Some(token) = &self.account_token {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {token}"))
                    .map_err(|_| MediaError::local("invalid account credential"))?,
            );
        }
        if let Some(token) = media_token {
            headers.insert(
                "x-caper-media-token",
                HeaderValue::from_str(token)
                    .map_err(|_| MediaError::local("invalid media capability"))?,
            );
        }
        let response = self
            .client
            .post(
                self.root
                    .join(operation)
                    .map_err(|_| MediaError::local("invalid media operation"))?,
            )
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(|_| MediaError::local("media request failed or timed out"))?;
        if !response.status().is_success() {
            let status = response.status();
            let error_id = response
                .headers()
                .get("x-caper-error-id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let body = response.json::<Value>().await.unwrap_or(Value::Null);
            return Err(MediaError {
                operation: Some(operation.into()),
                status: Some(status.as_u16()),
                code: body["code"].as_str().map(str::to_owned),
                error_id,
                detail: body["error"]
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("media service returned {status}")),
            });
        }
        Ok(response)
    }

    async fn post_empty(
        &self,
        operation: &str,
        media_token: &str,
        body: Value,
    ) -> Result<(), MediaError> {
        self.send(operation, Some(media_token), body).await?;
        Ok(())
    }
}

pub struct NativeSession {
    api: MediaApi,
    local_control: JoinControl,
    silence_task: tokio::task::JoinHandle<()>,
    live_task: tokio::task::JoinHandle<()>,
    factory: PeerConnectionFactory,
    peer: PeerConnection,
    token: String,
    self_id: String,
    microphone: MediaStreamTrack,
    subscriptions: Arc<Mutex<BTreeMap<String, String>>>,
    state_sequence: u64,
    muted: bool,
    deafened: bool,
    turn: Option<TurnGeneration>,
    restart_sequence: u64,
    pending_restart: Option<PendingRestart>,
    previous_stats: Option<(Instant, u64, u64)>,
}

struct PendingRestart {
    generation: TurnGeneration,
    sequence: u64,
    offer: Sdp,
    answer_applied: bool,
}

impl PendingRestart {
    async fn offer(&self, api: &MediaApi, token: &str) -> Result<SignalingResponse, MediaError> {
        api.post(
            "restart-ice",
            Some(token),
            json!({
                "generation":self.generation.generation,
                "sequence":self.sequence,
                "sessionDescription":self.offer,
            }),
        )
        .await
    }

    async fn ack(&self, api: &MediaApi, token: &str) -> Result<(), MediaError> {
        api.post_empty(
            "restart-ice-ack",
            token,
            json!({
                "generation":self.generation.generation,
                "sequence":self.sequence,
            }),
        )
        .await
    }
}

struct SetupGuard {
    api: MediaApi,
    factory: PeerConnectionFactory,
    peer: Option<PeerConnection>,
    token: String,
    armed: bool,
    silence_task: Option<tokio::task::JoinHandle<()>>,
    live_task: Option<tokio::task::JoinHandle<()>>,
    capture: Option<MicTestControl>,
}

impl Drop for SetupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.factory.set_adm_recording_enabled(false);
        self.factory.set_adm_playout_enabled(false);
        if let Some(task) = self.silence_task.take() {
            task.abort();
        }
        if let Some(task) = self.live_task.take() {
            task.abort();
        }
        if let Some(capture) = self.capture.take() {
            capture.cancel();
        }
        if let Some(peer) = &self.peer {
            peer.close();
        }
        self.factory.release_platform_adm();
        detached_leave(self.api.clone(), self.token.clone());
    }
}

fn detached_leave(api: MediaApi, token: String) {
    std::thread::spawn(move || {
        if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            let _ = runtime.block_on(api.post_empty("leave", &token, json!({})));
        }
    });
}

#[derive(Clone)]
pub struct JoinControl {
    stop: watch::Sender<bool>,
    local: Arc<Mutex<Option<LocalAudio>>>,
    audio: Arc<Mutex<(bool, bool)>>,
    activated: Arc<AtomicBool>,
    testing: Arc<AtomicBool>,
    capture: Arc<Mutex<Option<MicTestControl>>>,
    device_intent: Arc<Mutex<DeviceIntent>>,
    input_processing: Arc<Mutex<InputProcessing>>,
    processing_diagnostics: Arc<Mutex<Option<AudioProcessingDiagnostics>>>,
    playback: Arc<Mutex<PlaybackState>>,
}

#[derive(Default, Debug, PartialEq, Eq)]
struct DeviceIntent {
    // Outer None means no UI override yet; inner None selects system default.
    input: Option<Option<String>>,
    output: Option<Option<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackPlayback {
    pub gain_percent: u16,
    pub muted: bool,
}

struct PlaybackState {
    master_percent: u16,
    per_track: BTreeMap<String, TrackPlayback>,
    per_participant: BTreeMap<String, TrackPlayback>,
    track_owners: BTreeMap<String, String>,
    tracks: BTreeMap<String, MediaStreamTrack>,
    roster_reconciled: bool,
    silenced: bool,
}

impl PlaybackState {
    fn effective_volume(&self, id: &str) -> f64 {
        let track = self.per_track.get(id);
        let participant = self
            .track_owners
            .get(id)
            .and_then(|owner| self.per_participant.get(owner));
        if self.silenced
            || track.is_some_and(|preference| preference.muted)
            || participant.is_some_and(|preference| preference.muted)
        {
            0.0
        } else {
            let percent = participant
                .or(track)
                .map_or(100, |value| value.gain_percent);
            f64::from(self.master_percent) * f64::from(percent) / 10_000.0
        }
    }

    fn apply(&self, id: &str, track: &MediaStreamTrack) {
        let volume = self.effective_volume(id);
        if let MediaStreamTrack::Audio(audio) = track {
            // This is WebRTC's per-source software volume, not system volume.
            audio.set_volume(volume);
        }
        track.set_enabled(volume > 0.0);
    }
}

struct LocalAudio {
    factory: PeerConnectionFactory,
    peer: PeerConnection,
    sender: RtpSender,
    microphone: MediaStreamTrack,
    capture: MicTestControl,
    source: NativeAudioSource,
    silence: MediaStreamTrack,
    on_microphone: bool,
}

impl LocalAudio {
    fn silence(&self) {
        self.microphone.set_enabled(false);
        self.source.clear_buffer();
        self.capture.cancel();
        self.factory.set_adm_recording_enabled(false);
        self.factory.set_adm_playout_enabled(false);
        self.peer.close();
    }
}

#[cfg(test)]
async fn publish_processed_input(
    source: &NativeAudioSource,
    processor: &mut VoiceProcessor,
    frame: &AudioFrame<'_>,
    settings: InputProcessing,
) -> Result<(), String> {
    let data = processor.process(frame.data.as_ref(), settings);
    source
        .capture_frame(&AudioFrame {
            data: data.into(),
            sample_rate: 48_000,
            num_channels: 1,
            samples_per_channel: frame.samples_per_channel,
        })
        .await
        .map_err(|error| error.to_string())
}

async fn publish_denoised_input(
    source: &NativeAudioSource,
    processor: ProcessedVoice,
    frame: &AudioFrame<'_>,
    settings: InputProcessing,
    capture: &MicTestControl,
    epoch: u64,
) -> Result<ProcessedVoice, String> {
    let pcm = frame.data.as_ref().to_vec();
    let started = Instant::now();
    let (processor, processed) = tokio::task::spawn_blocking(move || {
        let mut processor = processor;
        let processed = processor
            .process(&pcm, settings)
            .map(|(_, enhanced)| enhanced);
        (processor, processed)
    })
    .await
    .map_err(|_| "microphone processing worker stopped".to_owned())?;
    let mut processor = processor;
    processor.observe_wall_time(started.elapsed());
    let data = processed?;
    capture.publish_if_current(
        source,
        &AudioFrame {
            data: data.into(),
            sample_rate: 48_000,
            num_channels: 1,
            samples_per_channel: frame.samples_per_channel,
        },
        epoch,
    )?;
    Ok(processor)
}

impl JoinControl {
    pub fn new() -> Self {
        let (stop, _) = watch::channel(false);
        Self {
            stop,
            local: Arc::new(Mutex::new(None)),
            audio: Arc::new(Mutex::new((false, false))),
            activated: Arc::new(AtomicBool::new(false)),
            testing: Arc::new(AtomicBool::new(false)),
            capture: Arc::new(Mutex::new(None)),
            device_intent: Arc::new(Mutex::new(DeviceIntent::default())),
            input_processing: Arc::new(Mutex::new(InputProcessing::default())),
            processing_diagnostics: Arc::new(Mutex::new(None)),
            playback: Arc::new(Mutex::new(PlaybackState {
                master_percent: 100,
                per_track: BTreeMap::new(),
                per_participant: BTreeMap::new(),
                track_owners: BTreeMap::new(),
                tracks: BTreeMap::new(),
                roster_reconciled: false,
                silenced: true,
            })),
        }
    }

    pub fn cancel(&self) {
        self.stop.send_replace(true);
        self.activated.store(false, Ordering::Release);
        self.testing.store(false, Ordering::Release);
        if let Ok(capture) = self.capture.lock()
            && let Some(capture) = capture.as_ref()
        {
            capture.cancel();
        }
        if let Ok(local) = self.local.lock()
            && let Some(local) = local.as_ref()
        {
            local.silence();
        }
        if let Ok(mut playback) = self.playback.lock() {
            playback.silenced = true;
            for (id, track) in &playback.tracks {
                playback.apply(id, track);
            }
        }
    }

    /// Sanitized actual denoiser state, updated only by the local PCM worker.
    /// None means capture processing has not initialized yet.
    pub fn audio_processing_diagnostics(&self) -> Option<AudioProcessingDiagnostics> {
        self.processing_diagnostics
            .lock()
            .ok()
            .and_then(|state| *state)
    }

    pub fn is_cancelled(&self) -> bool {
        *self.stop.borrow()
    }

    pub fn set_input_processing(&self, gain_percent: u16, strength: u8) -> Result<(), String> {
        if gain_percent > 200 || strength > 100 {
            return Err("input gain must be 0–200 and processing strength 0–100".into());
        }
        let local = self.local.lock().map_err(|_| "local audio unavailable")?;
        let mut processing = self
            .input_processing
            .lock()
            .map_err(|_| "input processing unavailable")?;
        let was_zero = processing.gain_percent == 0;
        if gain_percent == 0 && !was_zero {
            if let Some(local) = local.as_ref() {
                local.microphone.set_enabled(false);
                local.capture.set_live_recording(false)?;
                local.source.clear_buffer();
            } else if let Some(capture) = self
                .capture
                .lock()
                .map_err(|_| "local capture unavailable")?
                .as_ref()
            {
                capture.set_live_recording(false)?;
            }
        }
        *processing = InputProcessing {
            gain_percent,
            strength,
        };
        drop(processing);
        drop(local);
        if was_zero != (gain_percent == 0) {
            self.enforce_local_audio()?;
        }
        Ok(())
    }

    pub fn select_input(&self, guid: &str) -> Result<(), String> {
        self.select_input_route(Some(guid))
    }

    pub fn select_default_input(&self) -> Result<(), String> {
        self.select_input_route(None)
    }

    fn select_input_route(&self, guid: Option<&str>) -> Result<(), String> {
        if self.is_cancelled() {
            return Err("voice call was stopped".into());
        }
        let result = {
            let mut intent = self
                .device_intent
                .lock()
                .map_err(|_| "device intent unavailable")?;
            let mut local = self.local.lock().map_err(|_| "local audio unavailable")?;
            let capture = self
                .capture
                .lock()
                .map_err(|_| "local capture unavailable")?;
            let was_enabled = local
                .as_ref()
                .is_some_and(|audio| audio.microphone.enabled());
            if let Some(audio) = local.as_mut() {
                audio.microphone.set_enabled(false);
                audio.source.clear_buffer();
            }
            let result = capture.as_ref().map(|capture| match guid {
                Some(guid) => capture.select_input(guid),
                None => capture.select_default_input(),
            });
            if let Some(audio) = local.as_mut() {
                audio.source.clear_buffer();
                if was_enabled && result.as_ref().is_none_or(Result::is_ok) && !self.is_cancelled()
                {
                    audio.microphone.set_enabled(true);
                }
            }
            if result.as_ref().is_none_or(Result::is_ok) {
                intent.input = Some(guid.map(str::to_owned));
            }
            result.unwrap_or(Ok(()))
        };
        if result.is_err() {
            self.cancel();
        }
        result
    }

    pub fn select_output(&self, guid: &str) -> Result<(), String> {
        self.select_output_route(Some(guid))
    }

    pub fn select_default_output(&self) -> Result<(), String> {
        self.select_output_route(None)
    }

    fn select_output_route(&self, guid: Option<&str>) -> Result<(), String> {
        if self.is_cancelled() {
            return Err("voice call was stopped".into());
        }
        let result = {
            let mut intent = self
                .device_intent
                .lock()
                .map_err(|_| "device intent unavailable")?;
            let local = self.local.lock().map_err(|_| "local audio unavailable")?;
            let result = local.as_ref().map(|local| {
                if match guid {
                    Some(guid) => select_device(&local.factory, guid, false),
                    None => local.factory.select_default_playout_device(),
                } {
                    Ok(())
                } else {
                    local.factory.set_adm_playout_enabled(false);
                    Err("selected speaker is unavailable".into())
                }
            });
            if result.as_ref().is_none_or(Result::is_ok) {
                intent.output = Some(guid.map(str::to_owned));
            }
            result.unwrap_or(Ok(()))
        };
        if result.is_err() {
            self.cancel();
        }
        result
    }

    pub fn set_local_audio(&self, muted: bool, deafened: bool) -> Result<(), String> {
        self.update_local_audio(Some((muted, deafened)), None)
    }

    pub fn enforce_local_audio(&self) -> Result<(), String> {
        self.update_local_audio(None, None)
    }

    fn update_local_audio(
        &self,
        intent: Option<(bool, bool)>,
        mic_test: Option<bool>,
    ) -> Result<(), String> {
        // Serialize intent reads with both device application and intent writes.
        // A worker must never copy old intent, wait behind a UI mute, and then
        // overwrite that newer mute while enforcing its stale copy.
        let mut local = self.local.lock().map_err(|_| "local audio unavailable")?;
        if let Some(testing) = mic_test {
            self.testing.store(testing, Ordering::Release);
        }
        let mut audio = self
            .audio
            .lock()
            .map_err(|_| "local audio intent unavailable")?;
        if let Some(intent) = intent {
            *audio = intent;
        }
        let (muted, deafened) = *audio;
        if let Some(local) = local.as_mut() {
            let active = self.activated.load(Ordering::Acquire) && !self.is_cancelled();
            let gain_enabled = self
                .input_processing
                .lock()
                .map_err(|_| "input processing unavailable")?
                .gain_percent
                > 0;
            if active && !muted && gain_enabled && !self.testing.load(Ordering::Acquire) {
                // Swap to the device only after this exact attempt is ready.
                if !local.on_microphone {
                    local.source.clear_buffer();
                    local
                        .sender
                        .set_track(Some(local.microphone.clone()))
                        .map_err(|error| error.to_string())?;
                    local.on_microphone = true;
                }
                local.capture.set_live_recording(true)?;
                local.microphone.set_enabled(true);
            } else {
                // Muting stops physical capture, but a zero-PCM source keeps
                // the published audio MID alive for remote subscription.
                local.microphone.set_enabled(false);
                local.capture.set_live_recording(false)?;
                local.source.clear_buffer();
                if !self.is_cancelled() && local.on_microphone {
                    local
                        .sender
                        .set_track(Some(local.silence.clone()))
                        .map_err(|error| error.to_string())?;
                    local.on_microphone = false;
                }
            }
            local.factory.set_adm_playout_enabled(active && !deafened);
        }
        let mut playback = self.playback.lock().map_err(|_| "playback unavailable")?;
        playback.silenced =
            self.is_cancelled() || !self.activated.load(Ordering::Acquire) || deafened;
        for (id, track) in &playback.tracks {
            playback.apply(id, track);
        }
        Ok(())
    }

    pub fn set_playback_preferences(
        &self,
        master_percent: u16,
        per_track: &BTreeMap<String, TrackPlayback>,
    ) -> Result<(), String> {
        if master_percent > 200 || per_track.values().any(|value| value.gain_percent > 200) {
            return Err("playback gain must be between 0 and 200 percent".into());
        }
        let mut playback = self.playback.lock().map_err(|_| "playback unavailable")?;
        playback.master_percent = master_percent;
        playback.per_track = per_track.clone();
        for (id, track) in &playback.tracks {
            playback.apply(id, track);
        }
        Ok(())
    }

    pub fn set_participant_playback_preferences(
        &self,
        master_percent: u16,
        per_participant: &BTreeMap<String, TrackPlayback>,
    ) -> Result<(), String> {
        if master_percent > 200
            || per_participant
                .values()
                .any(|value| value.gain_percent > 200)
        {
            return Err("playback gain must be between 0 and 200 percent".into());
        }
        let mut playback = self.playback.lock().map_err(|_| "playback unavailable")?;
        playback.master_percent = master_percent;
        playback.per_participant = per_participant.clone();
        for (id, track) in &playback.tracks {
            playback.apply(id, track);
        }
        Ok(())
    }

    fn reconcile_participants(&self, snapshot: &Snapshot) -> Result<(), String> {
        let mut playback = self.playback.lock().map_err(|_| "playback unavailable")?;
        let owners: BTreeMap<String, String> = snapshot
            .participants
            .iter()
            .flat_map(|participant| {
                participant
                    .tracks
                    .iter()
                    .map(move |track| (track.id.clone(), participant.id.clone()))
            })
            .collect();
        let previous = std::mem::take(&mut playback.track_owners);
        playback.tracks.retain(|id, track| {
            if owners.contains_key(id) && owners.get(id) == previous.get(id) {
                return true;
            }
            track.set_enabled(false);
            if let MediaStreamTrack::Audio(audio) = track {
                audio.set_volume(0.0);
            }
            false
        });
        playback.track_owners = owners;
        playback.roster_reconciled = true;
        for (id, track) in &playback.tracks {
            playback.apply(id, track);
        }
        Ok(())
    }

    fn add_remote_track(&self, id: String, track: MediaStreamTrack) {
        if let Ok(mut playback) = self.playback.lock() {
            if self.is_cancelled()
                || (playback.roster_reconciled && !playback.track_owners.contains_key(&id))
            {
                track.set_enabled(false);
                if let MediaStreamTrack::Audio(audio) = &track {
                    audio.set_volume(0.0);
                }
                return;
            }
            playback.apply(&id, &track);
            playback.tracks.insert(id, track);
        } else {
            track.set_enabled(false);
        }
    }

    fn remove_remote_track(&self, id: &str) {
        if let Ok(mut playback) = self.playback.lock()
            && let Some(track) = playback.tracks.remove(id)
        {
            track.set_enabled(false);
            if let MediaStreamTrack::Audio(audio) = track {
                audio.set_volume(0.0);
            }
        }
    }

    fn set_remote_muted(&self, id: &str, muted: bool) -> Result<(), String> {
        let mut playback = self.playback.lock().map_err(|_| "playback unavailable")?;
        let preference = playback
            .per_track
            .entry(id.to_owned())
            .or_insert(TrackPlayback {
                gain_percent: 100,
                muted: false,
            });
        preference.muted = muted;
        if let Some(track) = playback.tracks.get(id) {
            playback.apply(id, track);
        }
        Ok(())
    }

    pub fn activate(&self) -> Result<(), String> {
        if !self.is_cancelled() {
            self.activated.store(true, Ordering::Release);
            self.enforce_local_audio()?;
        }
        Ok(())
    }

    /// Detach device publication during an explicit local microphone test.
    /// Muting/deafening remain owned by normal intent, restored on completion.
    pub fn suspend_for_mic_test(&self) -> Result<(), String> {
        self.update_local_audio(None, Some(true))
    }

    pub fn finish_mic_test(&self) -> Result<(), String> {
        self.update_local_audio(None, Some(false))
    }

    async fn cancelled(&self) {
        let mut receive = self.stop.subscribe();
        while !*receive.borrow() {
            if receive.changed().await.is_err() {
                break;
            }
        }
    }
}

impl NativeSession {
    pub async fn join(
        api: MediaApi,
        name: &str,
        muted: bool,
        deafened: bool,
        input_guid: Option<&str>,
        output_guid: Option<&str>,
        control: &JoinControl,
    ) -> Result<Self, VoiceError> {
        tokio::select! {
            biased;
            () = control.cancelled() => Err(VoiceError::Local("voice join cancelled".into())),
            result = Self::join_inner(api, name, muted, deafened, input_guid, output_guid, control) => result,
        }
    }

    async fn join_inner(
        api: MediaApi,
        name: &str,
        muted: bool,
        deafened: bool,
        input_guid: Option<&str>,
        output_guid: Option<&str>,
        control: &JoinControl,
    ) -> Result<Self, VoiceError> {
        let joined: JoinResponse = api
            .post(
                "join",
                None,
                json!({"name":name,"muted":muted,"deafened":deafened}),
            )
            .await
            .map_err(VoiceError::Media)?;
        Self::from_join(
            api,
            joined,
            muted,
            deafened,
            input_guid,
            output_guid,
            control,
        )
        .await
    }

    // Keep the issued capability available to callers that must await cleanup
    // even if device/signaling setup fails. Normal joins retain SetupGuard.
    async fn from_join(
        api: MediaApi,
        joined: JoinResponse,
        muted: bool,
        deafened: bool,
        input_guid: Option<&str>,
        output_guid: Option<&str>,
        control: &JoinControl,
    ) -> Result<Self, VoiceError> {
        let factory = PeerConnectionFactory::default();
        if !factory.acquire_platform_adm() {
            detached_leave(api.clone(), joined.token);
            return Err("native audio devices are unavailable".into());
        }
        let mut guard = SetupGuard {
            api: api.clone(),
            factory: factory.clone(),
            peer: None,
            token: joined.token.clone(),
            armed: true,
            silence_task: None,
            live_task: None,
            capture: None,
        };
        // Create and negotiate with closed local devices. Only the current
        // desktop attempt may activate them after gateway and roster readiness.
        factory.set_adm_playout_enabled(false);
        factory.set_adm_recording_enabled(false);
        let capture = MicTestControl::new();
        guard.capture = Some(capture.clone());
        let mut live_microphone = MicTest::start(capture.clone(), None, None).await?;
        capture.require_fresh_on_reopen()?;
        {
            let intent = control
                .device_intent
                .lock()
                .map_err(|_| "device intent unavailable")?;
            let input = intent
                .input
                .as_ref()
                .map(|route| route.as_deref())
                .unwrap_or(input_guid);
            match input {
                Some(guid) => capture.select_input(guid)?,
                None if intent.input.is_some() => capture.select_default_input()?,
                None => {}
            }
            let output = intent
                .output
                .as_ref()
                .map(|route| route.as_deref())
                .unwrap_or(output_guid);
            match output {
                Some(guid) if !select_device(&factory, guid, false) => {
                    return Err("selected speaker is unavailable".into());
                }
                None if intent.output.is_some() && !factory.select_default_playout_device() => {
                    return Err("system default speaker is unavailable".into());
                }
                _ => {}
            }
            let mut current = control
                .capture
                .lock()
                .map_err(|_| "local capture unavailable")?;
            if control.is_cancelled() {
                return Err("voice join cancelled".into());
            }
            *current = Some(capture.clone());
        }
        let mut config = RtcConfiguration::default();
        // The wrapper defaults to GatherContinually, which never transitions to
        // Complete. GatherOnce keeps local ICE completion observable for tests;
        // production signaling sends the pending local SDP immediately, as web does.
        config.continual_gathering_policy = ContinualGatheringPolicy::GatherOnce;
        config.ice_servers = joined
            .ice_servers
            .iter()
            .map(IceServer::rtc)
            .collect::<Result<_, _>>()?;
        let peer = factory
            .create_peer_connection(config)
            .map_err(|error| error.to_string())?;
        guard.peer = Some(peer.clone());
        let subscriptions = Arc::new(Mutex::new(BTreeMap::<String, String>::new()));
        let callback_subscriptions = subscriptions.clone();
        let callback_control = control.clone();
        peer.on_track(Some(Box::new(move |event| {
            let Some(mid) = event.transceiver.mid() else {
                event.track.set_enabled(false);
                return;
            };
            let track_id = callback_subscriptions
                .lock()
                .ok()
                .and_then(|subscriptions| {
                    subscriptions
                        .iter()
                        .find_map(|(track, assigned)| (assigned == &mid).then(|| track.clone()))
                });
            if let Some(track_id) = track_id {
                callback_control.add_remote_track(track_id, event.track);
            } else {
                event.track.set_enabled(false);
            }
        })));

        let processed_source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
        let audio = factory.create_audio_track("caper-microphone", processed_source.clone());
        audio.set_enabled(false);
        let silence_source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
        let silence = factory.create_audio_track("caper-silence", silence_source.clone());
        factory.set_adm_recording_enabled(false);
        let transceiver = peer
            .add_transceiver(
                silence.clone().into(),
                RtpTransceiverInit {
                    direction: RtpTransceiverDirection::SendOnly,
                    stream_ids: vec!["caper-audio".into()],
                    send_encodings: vec![],
                },
            )
            .map_err(|error| error.to_string())?;
        let opus = factory
            .get_rtp_sender_capabilities(MediaType::Audio)
            .codecs
            .into_iter()
            .filter(|codec| codec.mime_type.eq_ignore_ascii_case("audio/opus"))
            .collect();
        transceiver
            .set_codec_preferences(opus)
            .map_err(|error| error.to_string())?;
        let mut stopped = control.stop.subscribe();
        guard.silence_task = Some(tokio::spawn(async move {
            let zero = AudioFrame::new(48_000, 1, 480);
            let mut tick = tokio::time::interval(Duration::from_millis(10));
            loop {
                tokio::select! {
                    biased;
                    _ = stopped.changed() => { if *stopped.borrow() { break; } },
                    _ = tick.tick() => {
                        if silence_source.capture_frame(&zero).await.is_err() { break; }
                    }
                }
            }
        }));
        let mut capture_stopped = control.stop.subscribe();
        let processing = control.input_processing.clone();
        let processed = processed_source.clone();
        let capture_gate = capture.clone();
        let capture_control = control.clone();
        let diagnostics = control.processing_diagnostics.clone();
        let initial_input_route = input_guid.map(str::to_owned);
        guard.live_task = Some(tokio::spawn(async move {
            let mut processor = None;
            let mut active_epoch = None;
            loop {
                // A request to resume an already used ADM is not a readable
                // generation. Retire its sender, receiver, decoder and input
                // factory before accepting another frame; no decoded packet
                // from the previous peer can enter the new stream.
                if let Some(requested) = capture_gate.requested_epoch()
                    && capture_gate.live_epoch().is_none()
                {
                    let input_route = match capture_control.device_intent.lock() {
                        Ok(intent) => intent.input.clone(),
                        Err(_) => break,
                    };
                    let selected_input = input_route.unwrap_or_else(|| initial_input_route.clone());
                    let rebound = tokio::select! {
                        biased;
                        _ = capture_stopped.changed() => break,
                        result = live_microphone.reconnect_live(requested, selected_input.as_deref()) => result,
                    };
                    if !matches!(rebound, Ok(true)) {
                        if rebound.is_err() && capture_gate.requested_epoch() == Some(requested) {
                            break;
                        }
                        continue;
                    }
                }
                let epoch = capture_gate.live_epoch();
                if active_epoch != epoch {
                    live_microphone.reset_live_stream();
                    processor = None; // old recurrent state/OLA must not reach a reopened mic
                    active_epoch = epoch;
                }
                if epoch.is_none() {
                    tokio::select! {
                        biased;
                        _ = capture_stopped.changed() => break,
                        _ = tokio::time::sleep(Duration::from_millis(20)) => continue,
                    }
                }
                if epoch.is_some() && processor.is_none() {
                    processor = tokio::task::spawn_blocking(ProcessedVoice::new).await.ok();
                    if processor.is_none() {
                        break;
                    }
                    if let Ok(mut snapshot) = diagnostics.lock() {
                        *snapshot = processor.as_ref().map(ProcessedVoice::diagnostics);
                    }
                    if capture_gate.live_epoch() != epoch {
                        continue;
                    }
                }
                let frame = tokio::select! {
                    biased;
                    _ = capture_stopped.changed() => break,
                    _ = tokio::time::sleep(Duration::from_millis(20)) => continue,
                    frame = live_microphone.next_live_frame() => frame,
                };
                let Some(frame) = frame else {
                    break;
                };
                if capture_gate.live_epoch() != epoch || epoch.is_none() {
                    continue;
                }
                let settings = match processing.lock() {
                    Ok(settings) => *settings,
                    Err(_) => break,
                };
                if capture_gate.live_epoch() != epoch {
                    continue;
                }
                let result = publish_denoised_input(
                    &processed,
                    processor.take().unwrap(),
                    &frame,
                    settings,
                    &capture_gate,
                    epoch.unwrap(),
                )
                .await;
                processor = match result {
                    Ok(next) => Some(next),
                    Err(_) => break,
                };
                if let Ok(mut snapshot) = diagnostics.lock() {
                    *snapshot = processor.as_ref().map(ProcessedVoice::diagnostics);
                }
            }
            capture_gate.cancel();
            if !*capture_stopped.borrow() {
                // A failed private capture peer cannot leave a seemingly live
                // publication behind while the user expects a microphone.
                capture_control.cancel();
            }
        }));
        {
            let intent = control
                .device_intent
                .lock()
                .map_err(|_| "device intent unavailable")?;
            let input = intent
                .input
                .as_ref()
                .map(|route| route.as_deref())
                .unwrap_or(input_guid);
            if let Some(guid) = input {
                capture.select_input(guid)?;
            } else if intent.input.is_some() {
                capture.select_default_input()?;
            }
            let output = intent
                .output
                .as_ref()
                .map(|route| route.as_deref())
                .unwrap_or(output_guid);
            if let Some(guid) = output {
                if !select_device(&factory, guid, false) {
                    return Err("selected speaker is unavailable".into());
                }
            } else if intent.output.is_some() && !factory.select_default_playout_device() {
                return Err("system default speaker is unavailable".into());
            }
            if let Ok(mut local) = control.local.lock() {
                *local = Some(LocalAudio {
                    factory: factory.clone(),
                    peer: peer.clone(),
                    sender: transceiver.sender(),
                    microphone: audio.clone().into(),
                    capture: capture.clone(),
                    source: processed_source,
                    silence: silence.into(),
                    on_microphone: false,
                });
                if *control.stop.borrow() {
                    local.as_ref().unwrap().silence();
                }
            }
        }
        let offer = peer
            .create_offer(OfferOptions::default())
            .await
            .map_err(|error| error.to_string())?;
        peer.set_local_description(offer)
            .await
            .map_err(|error| error.to_string())?;
        let mid = transceiver
            .mid()
            .ok_or_else(|| "native WebRTC did not assign a media identifier".to_owned())?;
        let local = local_sdp(&peer)?;
        let response: SignalingResponse = api
            .post(
                "publish",
                Some(&joined.token),
                json!({"kind":"microphone","mid":mid,"sessionDescription":local}),
            )
            .await
            .map_err(VoiceError::Media)?;
        let answer = response
            .session_description
            .ok_or_else(|| "media service did not answer publication".to_owned())?;
        peer.set_remote_description(answer.parse(SdpType::Answer)?)
            .await
            .map_err(|error| error.to_string())?;
        wait_for_connection(&peer).await?;
        guard.armed = false;
        Ok(Self {
            api,
            local_control: control.clone(),
            silence_task: guard.silence_task.take().unwrap(),
            live_task: guard.live_task.take().unwrap(),
            factory,
            peer,
            token: joined.token,
            self_id: joined.id,
            microphone: audio.into(),
            subscriptions,
            state_sequence: 0,
            muted,
            deafened,
            turn: joined.turn,
            restart_sequence: 1,
            pending_restart: None,
            previous_stats: None,
        })
    }

    pub fn input_devices(&self) -> Vec<(String, String)> {
        (0..self.factory.recording_devices().max(0) as u16)
            .map(|index| {
                (
                    self.factory.recording_device_guid(index),
                    self.factory.recording_device_name(index),
                )
            })
            .collect()
    }

    pub fn output_devices(&self) -> Vec<(String, String)> {
        (0..self.factory.playout_devices().max(0) as u16)
            .map(|index| {
                (
                    self.factory.playout_device_guid(index),
                    self.factory.playout_device_name(index),
                )
            })
            .collect()
    }

    pub fn set_input_processing(&self, gain_percent: u16, strength: u8) -> Result<(), VoiceError> {
        self.local_control
            .set_input_processing(gain_percent, strength)
            .map_err(VoiceError::Local)
    }

    pub fn select_default_input(&self) -> Result<(), VoiceError> {
        self.local_control
            .select_default_input()
            .map_err(VoiceError::Local)
    }

    pub fn select_default_output(&self) -> Result<(), VoiceError> {
        self.local_control
            .select_default_output()
            .map_err(VoiceError::Local)
    }

    pub fn set_playback_preferences(
        &self,
        master_percent: u16,
        per_track: &BTreeMap<String, TrackPlayback>,
    ) -> Result<(), VoiceError> {
        self.local_control
            .set_playback_preferences(master_percent, per_track)
            .map_err(VoiceError::Local)
    }

    pub fn media_token(&self) -> &str {
        &self.token
    }

    pub fn self_id(&self) -> &str {
        &self.self_id
    }

    pub fn transport_failed(&self) -> bool {
        matches!(
            self.peer.connection_state(),
            PeerConnectionState::Failed | PeerConnectionState::Closed
        )
    }

    pub fn turn_refresh_delay(&self) -> Option<Duration> {
        self.turn
            .as_ref()
            .map(|turn| Duration::from_millis(turn.refresh_after_ms.max(1_000)))
    }

    pub fn select_input(&self, guid: &str) -> Result<(), VoiceError> {
        self.local_control
            .select_input(guid)
            .map_err(VoiceError::Local)
    }

    pub fn select_output(&self, guid: &str) -> Result<(), VoiceError> {
        self.local_control
            .select_output(guid)
            .map_err(VoiceError::Local)
    }

    pub async fn set_muted(&mut self, muted: bool) -> Result<(), VoiceError> {
        if !muted {
            self.deafened = false;
        }
        self.muted = muted;
        self.local_control.enforce_local_audio()?;
        self.sync_state().await
    }

    pub async fn set_deafened(&mut self, deafened: bool) -> Result<(), VoiceError> {
        if deafened == self.deafened {
            return Ok(());
        }
        self.muted = deafened;
        self.deafened = deafened;
        self.local_control.enforce_local_audio()?;
        self.sync_state().await
    }

    pub async fn renew_turn(&mut self) -> Result<Duration, VoiceError> {
        if self.pending_restart.is_some() {
            return self.finish_restart().await;
        }
        let current = self
            .turn
            .as_ref()
            .ok_or_else(|| "media service omitted TURN renewal metadata".to_owned())?
            .generation
            .clone();
        let credentials: TurnResponse = self
            .api
            .post("turn", Some(&self.token), json!({"generation":current}))
            .await
            .map_err(|error| self.fail_closed(error))?;
        if credentials.turn.generation == current {
            let delay = Duration::from_millis(credentials.turn.refresh_after_ms.max(1_000));
            self.turn = Some(credentials.turn);
            return Ok(delay);
        }

        let mut config = RtcConfiguration::default();
        config.continual_gathering_policy = ContinualGatheringPolicy::GatherOnce;
        config.ice_servers = credentials
            .ice_servers
            .iter()
            .map(IceServer::rtc)
            .collect::<Result<_, _>>()?;
        self.peer
            .set_configuration(config)
            .map_err(|error| error.to_string())?;
        let offer = self
            .peer
            .create_offer(restart_offer_options(&self.peer))
            .await
            .map_err(|error| error.to_string())?;
        self.peer
            .set_local_description(offer)
            .await
            .map_err(|error| error.to_string())?;
        self.pending_restart = Some(PendingRestart {
            generation: credentials.turn,
            sequence: self.restart_sequence,
            offer: local_sdp(&self.peer)?,
            answer_applied: false,
        });
        self.finish_restart().await
    }

    async fn finish_restart(&mut self) -> Result<Duration, VoiceError> {
        let pending = self
            .pending_restart
            .as_ref()
            .ok_or("missing pending ICE restart")?;
        if !pending.answer_applied {
            // Retain the exact SDP bytes and sequence on timeout/lost answer.
            let response = pending
                .offer(&self.api, &self.token)
                .await
                .map_err(|error| self.fail_closed(error))?;
            let answer = response
                .session_description
                .ok_or_else(|| "media service did not answer ICE restart".to_owned())?;
            self.peer
                .set_remote_description(answer.parse(SdpType::Answer)?)
                .await
                .map_err(|error| error.to_string())?;
            self.pending_restart.as_mut().unwrap().answer_applied = true;
        }
        // ACK is idempotent and retried independently, never by re-offering.
        self.pending_restart
            .as_ref()
            .unwrap()
            .ack(&self.api, &self.token)
            .await
            .map_err(|error| self.fail_closed(error))?;
        let generation = self.pending_restart.take().unwrap().generation;
        self.restart_sequence += 1;
        let delay = Duration::from_millis(generation.refresh_after_ms.max(1_000));
        self.turn = Some(generation);
        Ok(delay)
    }

    fn fail_closed(&mut self, error: MediaError) -> VoiceError {
        if !error.retryable() && !self.token.is_empty() {
            self.close_local();
            detached_leave(self.api.clone(), std::mem::take(&mut self.token));
        }
        VoiceError::Media(error)
    }

    async fn sync_state(&mut self) -> Result<(), VoiceError> {
        self.state_sequence += 1;
        self.api
            .post_empty(
                "state",
                &self.token,
                json!({
                    "muted":self.muted,
                    "deafened":self.deafened,
                    "sequence":self.state_sequence
                }),
            )
            .await
            .map_err(|error| self.fail_closed(error))
    }

    pub async fn snapshot(&mut self) -> Result<Snapshot, VoiceError> {
        self.snapshot_with_allowed(None).await
    }

    #[cfg(test)]
    async fn snapshot_owned(&mut self, allowed: &[&str]) -> Result<Snapshot, VoiceError> {
        self.snapshot_with_allowed(Some(allowed)).await
    }

    async fn snapshot_with_allowed(
        &mut self,
        allowed: Option<&[&str]>,
    ) -> Result<Snapshot, VoiceError> {
        let snapshot: Snapshot = self
            .api
            .post("snapshot", Some(&self.token), json!({}))
            .await
            .map_err(|error| self.fail_closed(error))?;
        if allowed.is_some_and(|allowed| {
            snapshot.participants.iter().any(|participant| {
                participant.id != self.self_id && !allowed.contains(&participant.id.as_str())
            })
        }) {
            return Err(VoiceError::Local(
                "unrelated voice participant present; aborting isolated test".into(),
            ));
        }
        // Install participant intent before subscribe can attach replacement
        // tracks; the UI receives Report::Roster only after reconciliation.
        self.local_control.reconcile_participants(&snapshot)?;
        let available: BTreeMap<_, _> = snapshot
            .participants
            .iter()
            .filter(|participant| participant.id != self.self_id)
            .flat_map(|participant| participant.tracks.iter())
            .map(|track| (track.id.clone(), track.kind.clone()))
            .collect();
        let subscribed: Vec<_> = self
            .subscriptions
            .lock()
            .map_err(|_| "voice subscription state unavailable")?
            .keys()
            .cloned()
            .collect();
        for track in subscribed {
            if !available.contains_key(&track) {
                self.unsubscribe(&track).await?;
            }
        }
        for (track, kind) in available {
            if kind == "microphone"
                && !self
                    .subscriptions
                    .lock()
                    .map_err(|_| "voice subscription state unavailable")?
                    .contains_key(&track)
            {
                self.subscribe(&track).await?;
            }
        }
        Ok(snapshot)
    }

    async fn subscribe(&mut self, track_id: &str) -> Result<(), VoiceError> {
        let response: SignalingResponse = match self
            .api
            .post("subscribe", Some(&self.token), json!({"trackId":track_id}))
            .await
        {
            Ok(response) => response,
            Err(error)
                if error.status == Some(404) && error.code.as_deref() == Some("track_gone") =>
            {
                return Ok(());
            }
            Err(error) => return Err(self.fail_closed(error)),
        };
        let mid = response
            .tracks
            .first()
            .map(|track| track.mid.clone())
            .ok_or_else(|| "media service omitted subscription identifier".to_owned())?;
        self.subscriptions
            .lock()
            .map_err(|_| "voice subscription state unavailable")?
            .insert(track_id.into(), mid);
        if let Some(offer) = response.session_description {
            self.peer
                .set_remote_description(offer.parse(SdpType::Offer)?)
                .await
                .map_err(|error| error.to_string())?;
            let answer = self
                .peer
                .create_answer(AnswerOptions::default())
                .await
                .map_err(|error| error.to_string())?;
            self.peer
                .set_local_description(answer)
                .await
                .map_err(|error| error.to_string())?;
            self.api
                .post_empty(
                    "negotiate",
                    &self.token,
                    json!({"sessionDescription":local_sdp(&self.peer)?}),
                )
                .await
                .map_err(|error| self.fail_closed(error))?;
        } else if response.requires_immediate_renegotiation {
            return Err("media service requested negotiation without an offer".into());
        }
        Ok(())
    }

    async fn unsubscribe(&mut self, track_id: &str) -> Result<(), VoiceError> {
        let mid = self
            .subscriptions
            .lock()
            .map_err(|_| "voice subscription state unavailable")?
            .get(track_id)
            .cloned();
        self.local_control.remove_remote_track(track_id);
        if let Some(mid) = mid {
            self.api
                .post_empty("close", &self.token, json!({"mid":mid}))
                .await
                .map_err(|error| self.fail_closed(error))?;
            self.subscriptions
                .lock()
                .map_err(|_| "voice subscription state unavailable")?
                .remove(track_id);
        }
        Ok(())
    }

    pub fn set_remote_muted(&self, track_id: &str, muted: bool) -> Result<(), VoiceError> {
        self.local_control
            .set_remote_muted(track_id, muted)
            .map_err(VoiceError::Local)
    }

    pub async fn diagnostics(&mut self) -> Result<Diagnostics, VoiceError> {
        let report = self
            .peer
            .get_stats()
            .await
            .map_err(|error| error.to_string())?;
        let mut diagnostics = Diagnostics::default();
        let mut selected_local = None;
        let mut local_candidates = BTreeMap::new();
        for stat in report {
            match stat {
                RtcStats::InboundRtp(stat) => {
                    diagnostics.received_bytes += stat.inbound.bytes_received;
                    diagnostics.packets_lost += stat.received.packets_lost;
                    diagnostics.max_jitter_ms = diagnostics
                        .max_jitter_ms
                        .max(stat.received.jitter * 1_000.0);
                }
                RtcStats::OutboundRtp(stat) => {
                    diagnostics.sent_bytes += stat.sent.bytes_sent;
                }
                RtcStats::CandidatePair(stat) if stat.candidate_pair.nominated => {
                    selected_local = Some(stat.candidate_pair.local_candidate_id);
                    diagnostics.round_trip_ms = diagnostics
                        .round_trip_ms
                        .max(stat.candidate_pair.current_round_trip_time * 1_000.0);
                }
                RtcStats::LocalCandidate(stat) => {
                    local_candidates.insert(stat.rtc.id, stat.local_candidate.candidate_type);
                }
                _ => {}
            }
        }
        diagnostics.route = selected_local
            .and_then(|id| local_candidates.remove(&id))
            .flatten()
            .map_or("unknown", |kind| {
                if kind == IceCandidateType::Relay {
                    "relay"
                } else {
                    "direct"
                }
            });
        let now = Instant::now();
        if let Some((sampled, received, sent)) = self.previous_stats {
            let seconds = now.duration_since(sampled).as_secs_f64();
            if seconds > 0.0 {
                diagnostics.receive_bitrate =
                    diagnostics.received_bytes.saturating_sub(received) as f64 * 8.0 / seconds;
                diagnostics.send_bitrate =
                    diagnostics.sent_bytes.saturating_sub(sent) as f64 * 8.0 / seconds;
            }
        }
        self.previous_stats = Some((now, diagnostics.received_bytes, diagnostics.sent_bytes));
        Ok(diagnostics)
    }

    pub fn leave(mut self) {
        self.close_local();
        detached_leave(self.api.clone(), self.token.clone());
        self.token.clear();
    }

    fn close_local(&mut self) {
        self.local_control.cancel();
        self.silence_task.abort();
        self.live_task.abort();
        self.microphone.set_enabled(false);
        self.factory.set_adm_recording_enabled(false);
        self.factory.set_adm_playout_enabled(false);
        self.peer.close();
        self.factory.release_platform_adm();
    }
}

impl Drop for NativeSession {
    fn drop(&mut self) {
        if !self.token.is_empty() {
            self.close_local();
            detached_leave(self.api.clone(), std::mem::take(&mut self.token));
        }
    }
}

fn restart_offer_options(peer: &PeerConnection) -> OfferOptions {
    // The binding maps false to legacy offerToReceiveAudio=0, which removes
    // receive directions. Keep existing receivers, but do not add an unused
    // receive MID when this participant only publishes.
    OfferOptions {
        ice_restart: true,
        offer_to_receive_audio: peer.transceivers().iter().any(|transceiver| {
            matches!(
                transceiver.direction(),
                RtpTransceiverDirection::RecvOnly | RtpTransceiverDirection::SendRecv
            )
        }),
        ..OfferOptions::default()
    }
}

fn local_sdp(peer: &PeerConnection) -> Result<Sdp, String> {
    let description = peer
        .local_description()
        .ok_or_else(|| "native WebRTC did not produce a local description".to_owned())?;
    Ok(Sdp {
        kind: description.sdp_type().to_string(),
        sdp: description.to_string(),
    })
}

#[cfg(test)]
async fn wait_for_ice(peer: &PeerConnection) -> Result<(), String> {
    let deadline = Instant::now() + GATHER_TIMEOUT;
    while peer.ice_gathering_state() != libwebrtc::peer_connection::IceGatheringState::Complete {
        if Instant::now() >= deadline {
            return Err("native WebRTC ICE gathering timed out".into());
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Ok(())
}

async fn wait_for_connection(peer: &PeerConnection) -> Result<(), String> {
    let deadline = Instant::now() + ICE_TIMEOUT;
    loop {
        match peer.connection_state() {
            PeerConnectionState::Connected => return Ok(()),
            PeerConnectionState::Failed | PeerConnectionState::Closed => {
                return Err("native WebRTC transport failed".into());
            }
            _ if Instant::now() >= deadline => {
                return Err("native WebRTC connection timed out".into());
            }
            _ => tokio::time::sleep(Duration::from_millis(20)).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn participant_intent_precedes_replacement_track_and_cancel_silences_late_callbacks() {
        let factory = PeerConnectionFactory::default();
        let source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
        let first: MediaStreamTrack = factory.create_audio_track("first", source.clone()).into();
        let replacement: MediaStreamTrack =
            factory.create_audio_track("replacement", source).into();
        let control = JoinControl::new();
        let mut per_participant = BTreeMap::new();
        per_participant.insert(
            "p1".into(),
            TrackPlayback {
                gain_percent: 75,
                muted: true,
            },
        );
        control
            .set_participant_playback_preferences(180, &per_participant)
            .unwrap();
        let snapshot = Snapshot {
            participants: vec![Participant {
                id: "p1".into(),
                name: "Test fixture".into(),
                country_code: None,
                muted: false,
                deafened: false,
                tracks: vec![Track {
                    id: "old".into(),
                    kind: "microphone".into(),
                }],
            }],
            revision: None,
        };
        control.reconcile_participants(&snapshot).unwrap();
        control.activate().unwrap();
        control.add_remote_track("old".into(), first.clone());
        assert!(!first.enabled());
        let mut next = snapshot;
        next.participants[0].tracks[0].id = "new".into();
        control.reconcile_participants(&next).unwrap();
        assert!(
            !first.enabled(),
            "departed track must stay silent before HTTP close"
        );
        assert!(!control.playback.lock().unwrap().tracks.contains_key("old"));
        // The old MID can still resolve in the callback while HTTP close is
        // pending. Even after the participant unmutes, the old track may not
        // be resurrected at default gain or retained as an active track.
        per_participant.get_mut("p1").unwrap().muted = false;
        control
            .set_participant_playback_preferences(180, &per_participant)
            .unwrap();
        let departed: MediaStreamTrack = factory.create_device_audio_track("departed").into();
        control.add_remote_track("old".into(), departed.clone());
        assert!(
            !departed.enabled(),
            "late departed callback must stay silent"
        );
        assert!(!control.playback.lock().unwrap().tracks.contains_key("old"));
        per_participant.get_mut("p1").unwrap().muted = true;
        control
            .set_participant_playback_preferences(180, &per_participant)
            .unwrap();
        control.add_remote_track("new".into(), replacement.clone());
        assert!(
            !replacement.enabled(),
            "new track must inherit mute before UI roster update"
        );
        per_participant.get_mut("p1").unwrap().muted = false;
        control
            .set_participant_playback_preferences(180, &per_participant)
            .unwrap();
        assert!(replacement.enabled());
        assert_eq!(
            control.playback.lock().unwrap().effective_volume("new"),
            1.35
        );
        control.set_local_audio(false, true).unwrap();
        assert!(!replacement.enabled(), "deafen silences immediately");
        control.cancel();
        control.set_local_audio(false, false).unwrap();
        assert!(
            !replacement.enabled(),
            "post-cancel intent cannot reopen playback"
        );
        let late: MediaStreamTrack = factory.create_device_audio_track("late").into();
        control.add_remote_track("new".into(), late.clone());
        assert!(
            !late.enabled(),
            "late callback after cancel must remain silent"
        );
        assert!(
            control
                .set_participant_playback_preferences(201, &per_participant)
                .is_err()
        );
        assert_eq!(control.playback.lock().unwrap().master_percent, 180);
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "isolated private PulseAudio null device required"]
    fn prejoin_enumeration_does_not_touch_existing_factory_or_start_capture() {
        assert!(
            std::env::var("PULSE_SERVER")
                .is_ok_and(|server| server.starts_with("unix:/tmp/caper-voice-silent-"))
        );
        let active = PeerConnectionFactory::default();
        assert!(active.acquire_platform_adm());
        active.set_adm_recording_enabled(false);
        active.set_adm_playout_enabled(false);
        let reference_count = active.platform_adm_ref_count();
        let devices = enumerate_audio_devices().unwrap();
        assert!(devices.inputs.iter().any(|device| {
            device
                .name
                .to_ascii_lowercase()
                .contains("caper_silent_sink")
        }));
        assert!(devices.outputs.iter().any(|device| {
            device
                .name
                .to_ascii_lowercase()
                .contains("caper_silent_sink")
        }));
        assert!(!select_device(&active, "not-a-device", true));
        assert!(!select_device(&active, "not-a-device", false));
        assert_eq!(active.platform_adm_ref_count(), reference_count);
        assert!(!active.adm_recording_enabled());
        assert!(!active.adm_playout_enabled());
        active.release_platform_adm();
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "requires isolated private PulseAudio server with two null sinks"]
    async fn default_device_reset_routes_to_new_system_default() {
        assert_eq!(
            std::env::var("PULSE_SERVER").as_deref(),
            Ok("unix:/tmp/caper-voice-silent-parity/native")
        );
        let pactl = |args: &[&str]| {
            let result = std::process::Command::new("pactl")
                .args(args)
                .output()
                .unwrap();
            assert!(result.status.success(), "private PulseAudio command failed");
            String::from_utf8(result.stdout).unwrap()
        };
        let devices = enumerate_audio_devices().unwrap();
        assert!(
            devices
                .inputs
                .iter()
                .any(|d| d.name.contains("caper_silent_sink"))
        );
        let capture_control = MicTestControl::new();
        let mut capture = MicTest::start(capture_control.clone(), None, None)
            .await
            .unwrap();
        let route = JoinControl::new();
        *route.capture.lock().unwrap() = Some(capture_control.clone());
        capture_control.set_live_recording(true).unwrap();
        let first_index = pactl(&["list", "short", "sources"])
            .lines()
            .find(|line| line.contains("caper_silent_sink.monitor"))
            .unwrap()
            .split('\t')
            .next()
            .unwrap()
            .to_owned();
        assert!(
            pactl(&["list", "short", "source-outputs"])
                .lines()
                .any(|line| line.split('\t').nth(1) == Some(first_index.as_str()))
        );
        let _ = pactl(&["set-default-source", "caper_second_sink.monitor"]);
        let _ = pactl(&["set-default-sink", "caper_second_sink"]);
        route.select_default_input().unwrap();
        assert!(
            enumerate_audio_devices()
                .unwrap()
                .inputs
                .iter()
                .any(|d| d.name.contains("caper_second_sink")),
            "system default recording device did not refresh"
        );
        capture_control.set_live_recording(false).unwrap();
        let fixture = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i",
                "flite=text=Caper private voice verification testing natural microphone capture with several syllables and changing cadence on an isolated null device",
                "-t", "5", "-af", "apad", "-f", "s16le", "-ar", "48000", "-ac", "1", "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(fixture.status.success());
        let mut playback = std::process::Command::new("pacat")
            .env("PULSE_SERVER", "unix:/tmp/caper-voice-silent-parity/native")
            .args([
                "--playback",
                "--device=caper_second_sink",
                "--raw",
                "--rate=48000",
                "--channels=1",
                "--format=s16le",
            ])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut writer = playback.stdin.take().unwrap();
        let feeder = std::thread::spawn(move || {
            use std::io::Write;
            // Give ADM time to start its monitor source before the synthetic
            // speech is injected into the otherwise silent private sink.
            std::thread::sleep(Duration::from_millis(350));
            writer.write_all(&fixture.stdout).unwrap();
        });
        let (recorded, sources) = tokio::join!(capture.record_with_processing(175, 0), async {
            tokio::time::sleep(Duration::from_secs(3)).await;
            let sources = pactl(&["list", "short", "source-outputs"]);
            capture_control.finish_recording();
            sources
        });
        let recorded = recorded.unwrap();
        assert_eq!(
            recorded.enhanced, recorded.natural,
            "strength zero is exactly natural gain + denoise without contour"
        );
        let captured = recorded.raw_fixture.clone();
        let peak = captured.iter().map(|v| v.unsigned_abs()).max().unwrap_or(0);
        feeder.join().unwrap();
        assert!(playback.wait().unwrap().success());
        assert!(
            peak > 500,
            "private speech fixture did not reach decoded capture"
        );
        assert!(
            recorded.natural.iter().any(|sample| *sample != 0),
            "denoiser suppressed every speech sample"
        );
        // Feed the actual decoded ADM sample into the same processed source
        // used for publication, then inspect a second local peer's decoded PCM.
        let publisher_factory = PeerConnectionFactory::default();
        let listener_factory = PeerConnectionFactory::default();
        let mut config = RtcConfiguration::default();
        config.continual_gathering_policy = ContinualGatheringPolicy::GatherOnce;
        let publisher = publisher_factory
            .create_peer_connection(config.clone())
            .unwrap();
        let listener = listener_factory.create_peer_connection(config).unwrap();
        let (track_tx, mut track_rx) = tokio::sync::mpsc::unbounded_channel();
        listener.on_track(Some(Box::new(move |event| {
            let _ = track_tx.send(event.track);
        })));
        let source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
        let audio = publisher_factory.create_audio_track("private-processed-input", source.clone());
        let transceiver = publisher
            .add_transceiver(
                audio.clone().into(),
                RtpTransceiverInit {
                    direction: RtpTransceiverDirection::SendOnly,
                    stream_ids: vec!["private-test".into()],
                    send_encodings: vec![],
                },
            )
            .unwrap();
        let offer = publisher
            .create_offer(OfferOptions::default())
            .await
            .unwrap();
        publisher.set_local_description(offer).await.unwrap();
        listener
            .set_remote_description(
                local_sdp(&publisher)
                    .unwrap()
                    .parse(SdpType::Offer)
                    .unwrap(),
            )
            .await
            .unwrap();
        let answer = listener
            .create_answer(AnswerOptions::default())
            .await
            .unwrap();
        listener.set_local_description(answer).await.unwrap();
        wait_for_ice(&listener).await.unwrap();
        publisher
            .set_remote_description(
                local_sdp(&listener)
                    .unwrap()
                    .parse(SdpType::Answer)
                    .unwrap(),
            )
            .await
            .unwrap();
        wait_for_connection(&publisher).await.unwrap();
        let MediaStreamTrack::Audio(track) =
            tokio::time::timeout(Duration::from_secs(2), track_rx.recv())
                .await
                .unwrap()
                .unwrap()
        else {
            panic!("local audio track");
        };
        let mut received =
            libwebrtc::audio_stream::native::NativeAudioStream::new(track, 48_000, 1);
        let mut processor = ProcessedVoice::new();
        let speech = captured
            .chunks_exact(480)
            .position(|frame| frame.iter().any(|v| v.unsigned_abs() > 500))
            .unwrap();
        let mut reference_processor = ProcessedVoice::new();
        let mut processed_peak = 0;
        for frame in captured[speech * 480..].chunks_exact(480).take(100) {
            let (_, enhanced) = reference_processor
                .process(
                    frame,
                    InputProcessing {
                        gain_percent: 175,
                        strength: 0,
                    },
                )
                .unwrap();
            processed_peak =
                processed_peak.max(enhanced.iter().map(|v| v.unsigned_abs()).max().unwrap());
        }
        assert!(
            processed_peak > 500,
            "speech-like fixture did not survive native denoise"
        );
        route.set_input_processing(175, 0).unwrap();
        capture_control.set_live_recording(true).unwrap();
        let epoch = capture_control.live_epoch().unwrap();
        for frame in captured[speech * 480..].chunks_exact(480).take(100) {
            let input = AudioFrame {
                data: frame.to_vec().into(),
                sample_rate: 48_000,
                num_channels: 1,
                samples_per_channel: 480,
            };
            let settings = *route.input_processing.lock().unwrap();
            processor = publish_denoised_input(
                &source,
                processor,
                &input,
                settings,
                &capture_control,
                epoch,
            )
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut outbound_peak = 0;
        let deadline = Instant::now() + Duration::from_secs(4);
        while Instant::now() < deadline && outbound_peak <= 500 {
            let frame = tokio::time::timeout(Duration::from_secs(2), received.next_frame())
                .await
                .unwrap()
                .unwrap();
            outbound_peak = outbound_peak.max(
                frame
                    .data
                    .iter()
                    .map(|v| v.unsigned_abs())
                    .max()
                    .unwrap_or(0),
            );
        }
        assert!(
            outbound_peak > 500,
            "captured speech was lost before local processed publication"
        );
        let silence_source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
        let silence =
            publisher_factory.create_audio_track("private-muted-silence", silence_source.clone());
        *route.local.lock().unwrap() = Some(LocalAudio {
            factory: publisher_factory.clone(),
            peer: publisher.clone(),
            sender: transceiver.sender(),
            microphone: audio.into(),
            capture: capture_control.clone(),
            source: source.clone(),
            silence: silence.into(),
            on_microphone: true,
        });
        route.set_local_audio(true, false).unwrap();
        assert!(!capture_control.live_recording_enabled());
        let zero = AudioFrame::new(48_000, 1, 480);
        for _ in 0..80 {
            silence_source.capture_frame(&zero).await.unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut quiet = false;
        for _ in 0..100 {
            let frame = tokio::time::timeout(Duration::from_secs(2), received.next_frame())
                .await
                .unwrap()
                .unwrap();
            if frame
                .data
                .iter()
                .all(|v| v.unsigned_abs() < outbound_peak / 4)
            {
                quiet = true;
            }
        }
        assert!(quiet, "mute must silence the local published receiver");
        publisher.close();
        listener.close();
        let second_index = pactl(&["list", "short", "sources"])
            .lines()
            .find(|line| line.contains("caper_second_sink.monitor"))
            .unwrap()
            .split('\t')
            .next()
            .unwrap()
            .to_owned();
        assert!(
            sources
                .lines()
                .any(|line| line.split('\t').nth(1) == Some(second_index.as_str())),
            "reset did not route recording to new system default; source outputs: {sources:?}"
        );
        assert!(
            pactl(&["list", "short", "source-outputs"]).is_empty(),
            "stopped microphone test kept device capture open"
        );
        capture_control.set_live_recording(true).unwrap();
        assert!(route.select_input("missing-private-device").is_err());
        assert!(route.is_cancelled());
        assert!(pactl(&["list", "short", "source-outputs"]).is_empty());
        capture_control.cancel();
        let _ = pactl(&["set-default-source", "caper_silent_sink.monitor"]);
        let _ = pactl(&["set-default-sink", "caper_silent_sink"]);
    }

    #[test]
    fn blocked_join_device_intent_is_synchronous_and_cancelled_intent_stays_closed() {
        let control = JoinControl::new();
        control.select_input("prejoin-mic").unwrap();
        control.select_output("prejoin-speaker").unwrap();
        assert_eq!(
            control.device_intent.lock().unwrap().input,
            Some(Some("prejoin-mic".into()))
        );
        assert_eq!(
            control.device_intent.lock().unwrap().output,
            Some(Some("prejoin-speaker".into()))
        );
        control.select_default_input().unwrap();
        control.select_default_output().unwrap();
        assert_eq!(
            *control.device_intent.lock().unwrap(),
            DeviceIntent {
                input: Some(None),
                output: Some(None)
            }
        );
        control.cancel();
        assert!(control.select_input("stale-mic").is_err());
        assert_eq!(
            *control.device_intent.lock().unwrap(),
            DeviceIntent {
                input: Some(None),
                output: Some(None)
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    #[ignore = "isolated private PulseAudio null device required; no public SFU"]
    async fn local_muted_adm_packet_diagnostic() {
        assert_eq!(std::env::var("CAPER_SILENT_ADM_TEST").as_deref(), Ok("1"));
        assert!(
            std::env::var("PULSE_SERVER")
                .is_ok_and(|server| server.starts_with("unix:/tmp/caper-voice-silent-"))
        );
        let factory = PeerConnectionFactory::default();
        assert!(factory.acquire_platform_adm());
        let index = (0..factory.recording_devices().max(0) as u16)
            .find(|index| {
                factory
                    .recording_device_name(*index)
                    .to_ascii_lowercase()
                    .contains("caper_silent_sink")
            })
            .expect("private null source unavailable");
        assert!(factory.set_recording_device_by_guid(&factory.recording_device_guid(index)));
        factory.set_adm_recording_enabled(false);
        factory.set_adm_playout_enabled(false);
        let mut config = RtcConfiguration::default();
        config.continual_gathering_policy = ContinualGatheringPolicy::GatherOnce;
        let sender = factory.create_peer_connection(config.clone()).unwrap();
        let receiver = factory.create_peer_connection(config).unwrap();
        let track = factory.create_device_audio_track("silent-local");
        track.set_enabled(false);
        let transceiver = sender
            .add_transceiver(
                track.clone().into(),
                RtpTransceiverInit {
                    direction: RtpTransceiverDirection::SendOnly,
                    stream_ids: vec!["silent-local".into()],
                    send_encodings: vec![],
                },
            )
            .unwrap();
        let (to_sender, mut sender_candidates) = tokio::sync::mpsc::unbounded_channel();
        let (to_receiver, mut receiver_candidates) = tokio::sync::mpsc::unbounded_channel();
        sender.on_ice_candidate(Some(Box::new(move |candidate| {
            let _ = to_receiver.send(candidate);
        })));
        receiver.on_ice_candidate(Some(Box::new(move |candidate| {
            let _ = to_sender.send(candidate);
        })));
        let offer = sender.create_offer(OfferOptions::default()).await.unwrap();
        sender.set_local_description(offer.clone()).await.unwrap();
        receiver.set_remote_description(offer).await.unwrap();
        let answer = receiver
            .create_answer(AnswerOptions::default())
            .await
            .unwrap();
        receiver
            .set_local_description(answer.clone())
            .await
            .unwrap();
        sender.set_remote_description(answer).await.unwrap();
        let connected = tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                if sender.connection_state() == PeerConnectionState::Connected { break; }
                tokio::select! {
                    Some(candidate) = sender_candidates.recv() => sender.add_ice_candidate(candidate).await.unwrap(),
                    Some(candidate) = receiver_candidates.recv() => receiver.add_ice_candidate(candidate).await.unwrap(),
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {},
                }
            }
        }).await;
        assert!(
            connected.is_ok(),
            "isolated local peer pair did not connect"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
        let disabled = outbound_bytes(&sender).await;
        factory.set_adm_recording_enabled(true);
        tokio::time::sleep(Duration::from_secs(2)).await;
        let enabled = outbound_bytes(&sender).await;
        track.set_enabled(true);
        tokio::time::sleep(Duration::from_secs(2)).await;
        let active = outbound_bytes(&sender).await;
        track.set_enabled(false);
        factory.set_adm_recording_enabled(false);
        let silence_source = libwebrtc::audio_source::native::NativeAudioSource::new(
            libwebrtc::audio_source::AudioSourceOptions::default(),
            48_000,
            1,
            0,
        );
        let silent = factory.create_audio_track("local-zero-pcm", silence_source.clone());
        transceiver
            .sender()
            .set_track(Some(silent.clone().into()))
            .unwrap();
        let zero = libwebrtc::audio_frame::AudioFrame::new(48_000, 1, 480);
        for _ in 0..200 {
            silence_source.capture_frame(&zero).await.unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let synthetic = outbound_bytes(&sender).await;
        assert_eq!(disabled, 0);
        assert_eq!(enabled, 0);
        assert!(active > enabled);
        assert!(synthetic > active);
        let control = JoinControl::new();
        let capture = MicTestControl::new();
        let _live = MicTest::start(capture.clone(), None, None).await.unwrap();
        *control.local.lock().unwrap() = Some(LocalAudio {
            factory: factory.clone(),
            peer: sender.clone(),
            sender: transceiver.sender(),
            microphone: track.clone().into(),
            capture,
            source: silence_source,
            silence: silent.into(),
            on_microphone: false,
        });
        control.set_local_audio(false, false).unwrap();
        assert!(
            !track.enabled(),
            "pre-ready device capture must stay closed"
        );
        control.activate().unwrap();
        assert!(
            track.enabled(),
            "ready unmuted peer must enable device capture"
        );
        control.suspend_for_mic_test().unwrap();
        assert!(
            !track.enabled(),
            "local test detaches publication immediately"
        );
        control.set_local_audio(true, false).unwrap();
        control.finish_mic_test().unwrap();
        assert!(
            !track.enabled(),
            "test completion must preserve newer mute intent"
        );
        control.set_local_audio(false, false).unwrap();
        assert!(
            track.enabled(),
            "later unmute reopens only the current call"
        );
        control.set_local_audio(true, false).unwrap();
        assert!(!track.enabled(), "muting must stop device capture");
        control.cancel();
        control.set_local_audio(false, false).unwrap();
        control.activate().unwrap();
        assert!(
            !track.enabled(),
            "cancelled peer must not reopen device capture"
        );
        receiver.close();
        factory.release_platform_adm();
        eprintln!(
            "local outbound RTP bytes: ADM disabled+track disabled={disabled}, ADM enabled+track disabled={enabled}, virtual input+track enabled={active}, synthetic zero PCM={synthetic}"
        );
    }

    #[cfg(target_os = "linux")]
    async fn outbound_bytes(peer: &PeerConnection) -> u64 {
        peer.get_stats()
            .await
            .unwrap()
            .into_iter()
            .filter_map(|stat| match stat {
                RtcStats::OutboundRtp(stat) => Some(stat.sent.bytes_sent),
                _ => None,
            })
            .sum()
    }

    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "opt-in public General smoke; requires isolated silent PulseAudio devices"]
    async fn silent_public_sfu_two_native_sessions() {
        assert_eq!(std::env::var("CAPER_SILENT_SFU_SMOKE").as_deref(), Ok("1"));
        assert!(
            std::env::var("PULSE_SERVER")
                .is_ok_and(|server| server.starts_with("unix:/tmp/caper-voice-silent-"))
        );
        let factory = PeerConnectionFactory::default();
        assert!(factory.acquire_platform_adm());
        let input = (0..factory.recording_devices().max(0) as u16)
            .find(|index| {
                factory
                    .recording_device_name(*index)
                    .to_ascii_lowercase()
                    .contains("caper_silent_sink")
            })
            .map(|index| factory.recording_device_guid(index))
            .expect("isolated silent input unavailable; abort without joining");
        let output = (0..factory.playout_devices().max(0) as u16)
            .find(|index| {
                factory
                    .playout_device_name(*index)
                    .to_ascii_lowercase()
                    .contains("caper_silent_sink")
            })
            .map(|index| factory.playout_device_guid(index))
            .expect("isolated null output unavailable; abort without joining");
        factory.release_platform_adm();

        let work = async {
            let base = Url::parse("https://caper.chat/").unwrap();
            let api = MediaApi::new(&base, None, None).unwrap();
            let first_control = JoinControl::new();
            first_control.set_local_audio(false, false).unwrap();
            let mut first = NativeSession::join(
                api.clone(),
                "Caper Silent Verification A",
                false,
                false,
                Some(&input),
                Some(&output),
                &first_control,
            )
            .await
            .expect("first silent join/publish/transport");
            first
                .snapshot_owned(&[])
                .await
                .expect("abort before subscribing to any unrelated participant");
            first_control
                .activate()
                .expect("activate isolated virtual source");
            let second_control = JoinControl::new();
            second_control.set_local_audio(false, false).unwrap();
            let mut second = NativeSession::join(
                api,
                "Caper Silent Verification B",
                false,
                false,
                Some(&input),
                Some(&output),
                &second_control,
            )
            .await
            .expect("second silent join/publish/transport");
            second
                .snapshot_owned(&[&first.self_id])
                .await
                .expect("abort before subscribing to any unrelated participant");
            second_control
                .activate()
                .expect("activate isolated virtual source");
            let a = first
                .snapshot_owned(&[&second.self_id])
                .await
                .expect("first subscriber must reconcile once");
            let b = second
                .snapshot_owned(&[&first.self_id])
                .await
                .expect("second subscriber must reconcile once");
            let subscribed = a
                .participants
                .iter()
                .any(|p| p.id == second.self_id && !p.tracks.is_empty())
                && b.participants
                    .iter()
                    .any(|p| p.id == first.self_id && !p.tracks.is_empty())
                && !first.subscriptions.lock().unwrap().is_empty()
                && !second.subscriptions.lock().unwrap().is_empty();
            assert!(subscribed, "both isolated peers must subscribe");
            // A successful offer/answer exchange is not proof of media delivery.
            // Wait for packets in BOTH directions, checking the owned-only roster
            // on each sample. Never retry a failed join or subscription here.
            let deadline = Instant::now() + Duration::from_secs(10);
            let (first_stats, second_stats) = loop {
                first
                    .snapshot_owned(&[&second.self_id])
                    .await
                    .expect("owned-only roster A");
                second
                    .snapshot_owned(&[&first.self_id])
                    .await
                    .expect("owned-only roster B");
                let a = first
                    .diagnostics()
                    .await
                    .expect("first aggregate RTC stats");
                let b = second
                    .diagnostics()
                    .await
                    .expect("second aggregate RTC stats");
                if a.sent_bytes > 0
                    && a.received_bytes > 0
                    && b.sent_bytes > 0
                    && b.received_bytes > 0
                {
                    break (a, b);
                }
                if Instant::now() >= deadline {
                    break (a, b);
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            };
            eprintln!(
                "silent SFU: both transport-connected, published and subscribed; A sent/received {}/{}, B sent/received {}/{} bytes (isolated virtual devices)",
                first_stats.sent_bytes,
                first_stats.received_bytes,
                second_stats.sent_bytes,
                second_stats.received_bytes
            );
            // Log only directions/counts, never SDP, identifiers or capabilities.
            for session in [&first, &second] {
                eprintln!(
                    "negotiated directions: {:?}; received tracks: {}",
                    session
                        .peer
                        .transceivers()
                        .iter()
                        .map(|t| t.current_direction())
                        .collect::<Vec<_>>(),
                    session.local_control.playback.lock().unwrap().tracks.len()
                );
            }
            first.close_local();
            second.close_local();
            // A failed RTP assertion must not terminate the test process before
            // the remote cleanup requests complete. Local media is already shut.
            let (first_leave, second_leave) = tokio::join!(
                first.api.post_empty("leave", &first.token, json!({})),
                second.api.post_empty("leave", &second.token, json!({}))
            );
            first_leave.expect("remote leave A");
            first.token.clear();
            second_leave.expect("remote leave B");
            second.token.clear();
            assert!(first_control.is_cancelled() && second_control.is_cancelled());
            assert!(
                first_stats.sent_bytes > 0
                    && first_stats.received_bytes > 0
                    && second_stats.sent_bytes > 0
                    && second_stats.received_bytes > 0,
                "No two-way RTP within 10s (aggregate counters above)"
            );
        };
        tokio::time::timeout(Duration::from_secs(55), work)
            .await
            .expect("silent SFU smoke exceeded 55s; local Drop closed media");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscription_answer_receives_rtp_on_existing_transport() {
        // No platform ADM: both factories use synthetic audio only. Model the
        // SFU adding a send-only MID after our send-only publication connects.
        let client_factory = PeerConnectionFactory::default();
        let server_factory = PeerConnectionFactory::default();
        let mut config = RtcConfiguration::default();
        config.continual_gathering_policy = ContinualGatheringPolicy::GatherOnce;
        let client = client_factory
            .create_peer_connection(config.clone())
            .unwrap();
        let server = server_factory.create_peer_connection(config).unwrap();
        let (published_tx, published_rx) = std::sync::mpsc::channel();
        server.on_track(Some(Box::new(move |event| {
            let _ = published_tx.send(event.track);
        })));
        let control = JoinControl::new();
        control.activate().unwrap();
        let (remote_tx, remote_rx) = std::sync::mpsc::channel();
        let callback_control = control.clone();
        client.on_track(Some(Box::new(move |event| {
            callback_control.add_remote_track("remote-microphone".into(), event.track.clone());
            remote_tx.send(event.track).unwrap();
        })));
        let source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
        let publication = client_factory.create_audio_track("publication", source.clone());
        client
            .add_transceiver(
                publication.into(),
                RtpTransceiverInit {
                    direction: RtpTransceiverDirection::SendOnly,
                    stream_ids: vec!["publisher".into()],
                    send_encodings: vec![],
                },
            )
            .unwrap();
        let offer = client.create_offer(OfferOptions::default()).await.unwrap();
        client.set_local_description(offer).await.unwrap();
        assert!(!restart_offer_options(&client).offer_to_receive_audio);
        // As with the SFU, only the answerer sends gathered candidates.
        server
            .set_remote_description(local_sdp(&client).unwrap().parse(SdpType::Offer).unwrap())
            .await
            .unwrap();
        let answer = server
            .create_answer(AnswerOptions::default())
            .await
            .unwrap();
        server.set_local_description(answer).await.unwrap();
        wait_for_ice(&server).await.unwrap();
        client
            .set_remote_description(local_sdp(&server).unwrap().parse(SdpType::Answer).unwrap())
            .await
            .unwrap();
        wait_for_connection(&client).await.unwrap();

        let published_track = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(track) = published_rx.try_recv() {
                    break track;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let MediaStreamTrack::Audio(published_track) = published_track else {
            panic!("audio publication")
        };
        let mut decoded =
            libwebrtc::audio_stream::native::NativeAudioStream::new(published_track, 48_000, 1);
        let raw = AudioFrame {
            data: (0..480)
                .map(|n| (3_000.0 * (std::f64::consts::TAU * n as f64 / 48.0).sin()) as i16)
                .collect(),
            sample_rate: 48_000,
            num_channels: 1,
            samples_per_channel: 480,
        };
        let mut processor = VoiceProcessor::default();
        control.set_input_processing(175, 0).unwrap();
        for _ in 0..80 {
            let settings = *control.input_processing.lock().unwrap();
            publish_processed_input(&source, &mut processor, &raw, settings)
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut peak = 0u16;
        for _ in 0..10 {
            let frame = tokio::time::timeout(Duration::from_secs(2), decoded.next_frame())
                .await
                .unwrap()
                .unwrap();
            peak = peak.max(
                frame
                    .data
                    .iter()
                    .map(|sample| sample.unsigned_abs())
                    .max()
                    .unwrap_or(0),
            );
        }
        assert!(
            peak > 1_000,
            "processed live source must reach a decoded local peer"
        );
        control.set_input_processing(0, 90).unwrap();
        for _ in 0..80 {
            let settings = *control.input_processing.lock().unwrap();
            publish_processed_input(&source, &mut processor, &raw, settings)
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let mut quiet = u16::MAX;
        for _ in 0..20 {
            let frame = tokio::time::timeout(Duration::from_secs(2), decoded.next_frame())
                .await
                .unwrap()
                .unwrap();
            quiet = quiet.min(
                frame
                    .data
                    .iter()
                    .map(|sample| sample.unsigned_abs())
                    .max()
                    .unwrap_or(0),
            );
        }
        assert!(
            quiet < peak / 4,
            "zero input gain must silence locally decoded publication"
        );

        let remote_source = NativeAudioSource::new(AudioSourceOptions::default(), 48_000, 1, 0);
        let remote = server_factory.create_audio_track("subscription", remote_source.clone());
        let sender = server
            .add_transceiver(
                remote.into(),
                RtpTransceiverInit {
                    direction: RtpTransceiverDirection::SendOnly,
                    stream_ids: vec!["remote-publisher".into()],
                    send_encodings: vec![],
                },
            )
            .unwrap();
        control
            .set_playback_preferences(
                180,
                &BTreeMap::from([(
                    "remote-microphone".into(),
                    TrackPlayback {
                        gain_percent: 75,
                        muted: false,
                    },
                )]),
            )
            .unwrap();
        let offer = server
            .create_offer(OfferOptions {
                offer_to_receive_audio: true,
                ..OfferOptions::default()
            })
            .await
            .unwrap();
        server.set_local_description(offer).await.unwrap();
        client
            .set_remote_description(local_sdp(&server).unwrap().parse(SdpType::Offer).unwrap())
            .await
            .unwrap();
        let answer = client
            .create_answer(AnswerOptions::default())
            .await
            .unwrap();
        client.set_local_description(answer).await.unwrap();
        server
            .set_remote_description(local_sdp(&client).unwrap().parse(SdpType::Answer).unwrap())
            .await
            .unwrap();
        assert_eq!(
            sender.current_direction(),
            Some(RtpTransceiverDirection::SendOnly)
        );

        let remote_track = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(track) = remote_rx.try_recv() {
                    break track;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("native remote audio callback");
        let MediaStreamTrack::Audio(audio) = remote_track else {
            panic!("expected audio track")
        };
        assert!(
            (audio.volume() - 1.35).abs() < 0.001,
            "WebRTC remote source must receive combined software gain"
        );
        control.set_remote_muted("remote-microphone", true).unwrap();
        assert_eq!(audio.volume(), 0.0);
        assert!(!audio.enabled());
        control
            .set_remote_muted("remote-microphone", false)
            .unwrap();
        assert!((audio.volume() - 1.35).abs() < 0.001);

        let zero = AudioFrame::new(48_000, 1, 480);
        let work = async {
            loop {
                remote_source.capture_frame(&zero).await.unwrap();
                let received: u64 = client
                    .get_stats()
                    .await
                    .unwrap()
                    .into_iter()
                    .filter_map(|stat| match stat {
                        RtcStats::InboundRtp(stat) => Some(stat.inbound.bytes_received),
                        _ => None,
                    })
                    .sum();
                if received > 0 {
                    return received;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        let received = tokio::time::timeout(Duration::from_secs(5), work).await;
        let restart = client
            .create_offer(restart_offer_options(&client))
            .await
            .unwrap();
        client.set_local_description(restart).await.unwrap();
        server
            .set_remote_description(local_sdp(&client).unwrap().parse(SdpType::Offer).unwrap())
            .await
            .unwrap();
        let answer = server
            .create_answer(AnswerOptions::default())
            .await
            .unwrap();
        server.set_local_description(answer).await.unwrap();
        wait_for_ice(&server).await.unwrap();
        client
            .set_remote_description(local_sdp(&server).unwrap().parse(SdpType::Answer).unwrap())
            .await
            .unwrap();
        let receives_after_restart = client.transceivers().iter().any(|transceiver| {
            transceiver.current_direction() == Some(RtpTransceiverDirection::RecvOnly)
        });
        let directions_after_restart: Vec<_> = client
            .transceivers()
            .iter()
            .map(|t| t.current_direction())
            .collect();
        let receiving_again = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                remote_source.capture_frame(&zero).await.unwrap();
                let total: u64 = client
                    .get_stats()
                    .await
                    .unwrap()
                    .into_iter()
                    .filter_map(|stat| match stat {
                        RtcStats::InboundRtp(stat) => Some(stat.inbound.bytes_received),
                        _ => None,
                    })
                    .sum();
                if total > received.as_ref().copied().unwrap_or(0) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        client.close();
        server.close();
        assert!(
            received.is_ok(),
            "subscription must receive RTP, not merely connect"
        );
        assert!(
            receives_after_restart,
            "ICE renewal must retain the subscribed receive direction"
        );
        assert_eq!(
            directions_after_restart,
            vec![
                Some(RtpTransceiverDirection::SendOnly),
                Some(RtpTransceiverDirection::RecvOnly)
            ]
        );
        assert!(
            receiving_again.is_ok(),
            "ICE renewal must resume received RTP"
        );
    }

    #[tokio::test]
    async fn initial_offer_serializes_pending_local_not_missing_current() {
        let factory = PeerConnectionFactory::default();
        let mut config = RtcConfiguration::default();
        config.continual_gathering_policy = ContinualGatheringPolicy::GatherOnce;
        let peer = factory.create_peer_connection(config).unwrap();
        let _channel = peer
            .create_data_channel("probe", Default::default())
            .unwrap();
        peer.add_transceiver_for_media(
            MediaType::Audio,
            RtpTransceiverInit {
                direction: RtpTransceiverDirection::RecvOnly,
                stream_ids: vec![],
                send_encodings: vec![],
            },
        )
        .unwrap();
        let offer = peer.create_offer(OfferOptions::default()).await.unwrap();
        peer.set_local_description(offer).await.unwrap();
        assert!(peer.current_local_description().is_none());
        wait_for_ice(&peer).await.unwrap();
        assert_eq!(
            peer.ice_gathering_state(),
            libwebrtc::peer_connection::IceGatheringState::Complete
        );
        let sent = local_sdp(&peer).unwrap();
        assert_eq!(sent.kind, "offer");
        assert!(sent.sdp.contains("a=ice-ufrag:"));
        assert!(
            sent.sdp.contains("a=candidate:"),
            "gathered offer must contain a usable candidate"
        );
        peer.close();
    }

    #[tokio::test]
    async fn cancellation_interrupts_silent_join_before_native_capture() {
        use std::sync::mpsc;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (seen, request_seen) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = receive(&mut stream);
            seen.send(request).unwrap();
            thread::sleep(Duration::from_millis(300));
        });
        let base = Url::parse(&format!("http://{address}/")).unwrap();
        let api = MediaApi::new(&base, None, None).unwrap();
        let control = JoinControl::new();
        let cancelling = control.clone();
        thread::spawn(move || {
            let request = request_seen.recv_timeout(Duration::from_secs(2)).unwrap();
            assert!(request.starts_with("POST /api/media/join "));
            cancelling.set_input_processing(175, 85).unwrap();
            assert_eq!(
                cancelling.input_processing.lock().unwrap().gain_percent,
                175
            );
            assert_eq!(cancelling.input_processing.lock().unwrap().strength, 85);
            cancelling.select_input("pending-microphone").unwrap();
            cancelling.select_default_output().unwrap();
            assert_eq!(
                cancelling.device_intent.lock().unwrap().input,
                Some(Some("pending-microphone".into()))
            );
            assert_eq!(cancelling.device_intent.lock().unwrap().output, Some(None));
            cancelling.cancel();
        });
        let started = Instant::now();
        let result = NativeSession::join(api, "Guest", false, false, None, None, &control).await;
        assert!(
            matches!(result, Err(VoiceError::Local(ref detail)) if detail == "voice join cancelled")
        );
        assert!(started.elapsed() < Duration::from_millis(250));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn media_denial_is_distinct_from_provider_retry_even_with_same_http_status() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for (status, body) in [
                (
                    "403 Forbidden",
                    r#"{"error":"membership revoked","code":"access_denied"}"#,
                ),
                (
                    "403 Forbidden",
                    r#"{"error":"provider failed","code":"ice_restart_retry"}"#,
                ),
                (
                    "503 Service Unavailable",
                    r#"{"error":"provider offline","code":"ice_restart_retry"}"#,
                ),
                (
                    "502 Bad Gateway",
                    r#"{"error":"invalid answer","code":"ice_restart_invalid"}"#,
                ),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = receive(&mut stream);
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nx-caper-error-id: 11111111-2222-3333-4444-555555555555\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
            }
        });
        let base = Url::parse(&format!("http://{address}/")).unwrap();
        let api = MediaApi::new(&base, Some("chan00000001"), Some("account".into())).unwrap();
        let revoked = api
            .post::<Value>("snapshot", Some("media"), json!({}))
            .await
            .unwrap_err();
        assert_eq!(
            (revoked.status, revoked.code.as_deref()),
            (Some(403), Some("access_denied"))
        );
        assert_eq!(revoked.operation.as_deref(), Some("snapshot"));
        assert_eq!(
            revoked.error_id.as_deref(),
            Some("11111111-2222-3333-4444-555555555555")
        );
        assert!(revoked.denied());
        assert!(!revoked.retryable());
        let provider = api
            .post::<Value>("restart-ice", Some("media"), json!({}))
            .await
            .unwrap_err();
        assert_eq!(provider.status, Some(403));
        assert!(!provider.denied());
        assert!(provider.retryable());
        let unavailable = api
            .post::<Value>("restart-ice", Some("media"), json!({}))
            .await
            .unwrap_err();
        assert_eq!(unavailable.status, Some(503));
        assert!(unavailable.retryable());
        let invalid = api
            .post::<Value>("restart-ice", Some("media"), json!({}))
            .await
            .unwrap_err();
        assert!(!invalid.retryable());
        assert!(VoiceError::Media(invalid).terminal());
        assert!(VoiceError::Local("invalid SDP".into()).terminal());
        server.join().unwrap();
    }

    #[test]
    fn reapplying_audio_holds_the_device_gate_before_reading_current_intent() {
        let control = JoinControl::new();
        let mut audio = control.audio.lock().unwrap();
        let worker_control = control.clone();
        let worker = thread::spawn(move || worker_control.enforce_local_audio());
        let deadline = Instant::now() + Duration::from_secs(2);
        let serialized = loop {
            if matches!(
                control.local.try_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ) {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            thread::sleep(Duration::from_millis(1));
        };
        *audio = (true, false);
        drop(audio);
        worker.join().unwrap().unwrap();
        assert!(
            serialized,
            "Reapplication must not read intent before acquiring the device gate"
        );
        assert_eq!(*control.audio.lock().unwrap(), (true, false));
    }

    #[tokio::test]
    async fn native_mute_and_deafen_match_web_state_contract() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for (sequence, muted, deafened) in [
                (1, true, false),
                (2, true, true),
                (3, false, false),
                (4, true, true),
                (5, false, false),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let request = receive(&mut stream);
                assert!(request.starts_with("POST /api/media/state "));
                assert!(request.contains(&format!("\"sequence\":{sequence}")));
                assert!(request.contains(&format!("\"muted\":{muted}")));
                assert!(request.contains(&format!("\"deafened\":{deafened}")));
                stream.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
        });
        let factory = PeerConnectionFactory::default();
        let api = MediaApi::new(
            &Url::parse(&format!("http://{address}/")).unwrap(),
            None,
            None,
        )
        .unwrap();
        let mut session = NativeSession {
            api,
            local_control: JoinControl::new(),
            silence_task: tokio::spawn(async {}),
            live_task: tokio::spawn(async {}),
            peer: factory
                .create_peer_connection(RtcConfiguration::default())
                .unwrap(),
            microphone: factory.create_device_audio_track("test-mic").into(),
            factory,
            token: "test-token".into(),
            self_id: "self".into(),
            subscriptions: Arc::new(Mutex::new(BTreeMap::new())),
            state_sequence: 0,
            muted: false,
            deafened: false,
            turn: None,
            restart_sequence: 1,
            pending_restart: None,
            previous_stats: None,
        };
        session.set_muted(true).await.unwrap();
        session.set_deafened(true).await.unwrap();
        session.set_deafened(false).await.unwrap();
        session.set_deafened(true).await.unwrap();
        session.set_muted(false).await.unwrap();
        assert!(!session.muted && !session.deafened);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn departed_track_and_close_retry_preserve_session_until_revoked() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for (operation, status, body) in [
                (
                    "subscribe",
                    "404 Not Found",
                    r#"{"code":"track_gone","error":"track unavailable"}"#,
                ),
                (
                    "close",
                    "503 Service Unavailable",
                    r#"{"error":"temporary"}"#,
                ),
                ("close", "204 No Content", ""),
                ("snapshot", "403 Forbidden", r#"{"error":"access revoked"}"#),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let request = receive(&mut stream);
                assert!(request.starts_with(&format!(
                    "POST /api/channels/chan00000001/media/{operation} "
                )));
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
            }
        });
        let api = MediaApi::new(
            &Url::parse(&format!("http://{address}/")).unwrap(),
            Some("chan00000001"),
            Some("account".into()),
        )
        .unwrap();
        let factory = PeerConnectionFactory::default();
        let peer = factory
            .create_peer_connection(RtcConfiguration::default())
            .unwrap();
        let audio: MediaStreamTrack = factory.create_device_audio_track("test-mic").into();
        audio.set_enabled(true);
        let receiver: MediaStreamTrack = factory.create_device_audio_track("test-receiver").into();
        receiver.set_enabled(true);
        let mut session = NativeSession {
            api,
            local_control: JoinControl::new(),
            silence_task: tokio::spawn(async {}),
            live_task: tokio::spawn(async {}),
            factory,
            peer: peer.clone(),
            token: "media-token".into(),
            self_id: "self".into(),
            microphone: audio.clone(),
            subscriptions: Arc::new(Mutex::new(BTreeMap::new())),
            state_sequence: 0,
            muted: false,
            deafened: false,
            turn: None,
            restart_sequence: 1,
            pending_restart: None,
            previous_stats: None,
        };
        session.subscribe("not-ready").await.unwrap();
        assert_eq!(session.token, "media-token");
        assert!(audio.enabled());
        assert_ne!(peer.connection_state(), PeerConnectionState::Closed);
        assert!(session.subscriptions.lock().unwrap().is_empty());
        session
            .subscriptions
            .lock()
            .unwrap()
            .insert("departed".into(), "remote-mid".into());
        session
            .local_control
            .add_remote_track("departed".into(), receiver.clone());
        assert!(session.unsubscribe("departed").await.is_err());
        assert!(
            !receiver.enabled(),
            "Local playback must stop even when remote close fails"
        );
        assert_eq!(
            session
                .subscriptions
                .lock()
                .unwrap()
                .get("departed")
                .map(String::as_str),
            Some("remote-mid")
        );
        session.unsubscribe("departed").await.unwrap();
        assert!(session.subscriptions.lock().unwrap().is_empty());
        let error = session.snapshot().await.unwrap_err();
        assert!(
            matches!(error, VoiceError::Media(MediaError { status: Some(403), ref detail, .. }) if detail == "access revoked")
        );
        assert!(session.token.is_empty());
        assert!(!audio.enabled());
        assert_eq!(peer.connection_state(), PeerConnectionState::Closed);
        server.join().unwrap();
    }

    #[tokio::test]
    async fn isolated_snapshot_rejects_unowned_tracks_before_subscribing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = receive(&mut stream);
            assert!(request.starts_with("POST /api/media/snapshot "));
            let body = r#"{"participants":[{"id":"self","name":"A","muted":false,"deafened":false,"tracks":[]},{"id":"stranger","name":"Another person","muted":false,"deafened":false,"tracks":[{"id":"foreign-track","kind":"microphone"}]}]}"#;
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
        });
        let api = MediaApi::new(
            &Url::parse(&format!("http://{address}/")).unwrap(),
            None,
            None,
        )
        .unwrap();
        let factory = PeerConnectionFactory::default();
        let peer = factory
            .create_peer_connection(RtcConfiguration::default())
            .unwrap();
        let mut session = NativeSession {
            api,
            local_control: JoinControl::new(),
            silence_task: tokio::spawn(async {}),
            live_task: tokio::spawn(async {}),
            factory: factory.clone(),
            peer,
            token: "local-test-token".into(),
            self_id: "self".into(),
            microphone: factory.create_device_audio_track("local-test-mic").into(),
            subscriptions: Arc::new(Mutex::new(BTreeMap::new())),
            state_sequence: 0,
            muted: false,
            deafened: false,
            turn: None,
            restart_sequence: 1,
            pending_restart: None,
            previous_stats: None,
        };
        let error = session.snapshot_owned(&[]).await.unwrap_err();
        assert!(
            matches!(error, VoiceError::Local(ref detail) if detail.contains("unrelated voice participant"))
        );
        assert!(session.subscriptions.lock().unwrap().is_empty());
        server.join().unwrap();
    }

    #[tokio::test]
    async fn restart_retries_identical_offer_then_retries_ack_without_another_offer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in [
                ("503 Service Unavailable", r#"{"code":"ice_restart_retry"}"#),
                (
                    "200 OK",
                    r#"{"sessionDescription":{"type":"answer","sdp":"answer-bytes"}}"#,
                ),
                ("503 Service Unavailable", r#"{"code":"ice_restart_retry"}"#),
                ("204 No Content", ""),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                requests.push(receive(&mut stream));
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
            }
            requests
        });
        let base = Url::parse(&format!("http://{address}/")).unwrap();
        let api = MediaApi::new(&base, None, None).unwrap();
        let mut pending = PendingRestart {
            generation: TurnGeneration {
                generation: "turn-two".into(),
                refresh_after_ms: 1000,
                expires_in_ms: 5000,
            },
            sequence: 7,
            offer: Sdp {
                kind: "offer".into(),
                sdp: "exact-ufrag-and-candidates".into(),
            },
            answer_applied: false,
        };
        assert!(
            pending
                .offer(&api, "media-token")
                .await
                .unwrap_err()
                .retryable()
        );
        let answer = pending
            .offer(&api, "media-token")
            .await
            .unwrap()
            .session_description
            .unwrap();
        assert_eq!(answer.sdp, "answer-bytes");
        pending.answer_applied = true;
        assert!(
            pending
                .ack(&api, "media-token")
                .await
                .unwrap_err()
                .retryable()
        );
        pending.ack(&api, "media-token").await.unwrap();
        let requests = server.join().unwrap();
        assert_eq!(
            requests[0].split("\r\n\r\n").nth(1),
            requests[1].split("\r\n\r\n").nth(1)
        );
        assert!(requests[0].contains("exact-ufrag-and-candidates"));
        assert!(requests[2].starts_with("POST /api/media/restart-ice-ack "));
        assert!(requests[3].starts_with("POST /api/media/restart-ice-ack "));
        assert_eq!(
            requests[2].split("\r\n\r\n").nth(1),
            requests[3].split("\r\n\r\n").nth(1)
        );
        assert!(requests[2].contains("\"sequence\":7"));
    }

    fn receive(stream: &mut std::net::TcpStream) -> String {
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            request.extend_from_slice(&buffer[..read]);
            let Some(headers) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
            let header = String::from_utf8_lossy(&request[..headers + 4]);
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap_or_default();
            if request.len() >= headers + 4 + length {
                return String::from_utf8(request).unwrap();
            }
        }
    }

    #[tokio::test]
    async fn account_and_media_capabilities_are_separate_headers_never_urls() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                requests.push(receive(&mut stream));
                stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                    )
                    .unwrap();
            }
            requests
        });
        let base = Url::parse(&format!("http://{address}/")).unwrap();
        let api =
            MediaApi::new(&base, Some("chan00000001"), Some("account-secret".into())).unwrap();
        let _: Value = api.post("join", None, json!({})).await.unwrap();
        let _: Value = api
            .post("snapshot", Some("media-secret"), json!({}))
            .await
            .unwrap();
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /api/channels/chan00000001/media/join "));
        assert!(requests[1].starts_with("POST /api/channels/chan00000001/media/snapshot "));
        assert!(requests.iter().all(|request| {
            request
                .to_ascii_lowercase()
                .contains("authorization: bearer account-secret")
        }));
        assert!(
            !requests[0]
                .to_ascii_lowercase()
                .contains("x-caper-media-token")
        );
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("x-caper-media-token: media-secret")
        );
        assert!(
            requests
                .iter()
                .all(|request| !request.lines().next().unwrap().contains("secret"))
        );
    }

    #[tokio::test]
    async fn public_general_uses_only_the_media_capability_header() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = receive(&mut stream);
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                )
                .unwrap();
            request
        });
        let base = Url::parse(&format!("http://{address}/")).unwrap();
        let api = MediaApi::new(&base, None, None).unwrap();
        let _: Value = api
            .post("snapshot", Some("public-media-secret"), json!({}))
            .await
            .unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /api/media/snapshot "));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("x-caper-media-token: public-media-secret")
        );
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        assert!(!request.lines().next().unwrap().contains("secret"));
    }
}
