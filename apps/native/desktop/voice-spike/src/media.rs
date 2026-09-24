#![allow(dead_code)]

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
    factory: PeerConnectionFactory,
    peer: PeerConnection,
    token: String,
    self_id: String,
    microphone: MediaStreamTrack,
    subscriptions: Arc<Mutex<BTreeMap<String, String>>>,
    remote_tracks: Arc<Mutex<BTreeMap<String, MediaStreamTrack>>>,
    state_sequence: u64,
    muted: bool,
    deafened: bool,
    mute_before_deafen: bool,
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
}

struct LocalAudio {
    factory: PeerConnectionFactory,
    peer: PeerConnection,
    sender: RtpSender,
    microphone: MediaStreamTrack,
    silence: MediaStreamTrack,
    on_microphone: bool,
}

impl LocalAudio {
    fn silence(&self) {
        self.microphone.set_enabled(false);
        self.factory.set_adm_recording_enabled(false);
        self.factory.set_adm_playout_enabled(false);
        self.peer.close();
    }
}

impl JoinControl {
    pub fn new() -> Self {
        let (stop, _) = watch::channel(false);
        Self {
            stop,
            local: Arc::new(Mutex::new(None)),
            audio: Arc::new(Mutex::new((false, false))),
            activated: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.stop.send_replace(true);
        self.activated.store(false, Ordering::Release);
        if let Ok(local) = self.local.lock()
            && let Some(local) = local.as_ref()
        {
            local.silence();
        }
    }

    pub fn is_cancelled(&self) -> bool {
        *self.stop.borrow()
    }

    pub fn set_local_audio(&self, muted: bool, deafened: bool) -> Result<(), String> {
        self.update_local_audio(Some((muted, deafened)))
    }

    pub fn enforce_local_audio(&self) -> Result<(), String> {
        self.update_local_audio(None)
    }

    fn update_local_audio(&self, intent: Option<(bool, bool)>) -> Result<(), String> {
        // Serialize intent reads with both device application and intent writes.
        // A worker must never copy old intent, wait behind a UI mute, and then
        // overwrite that newer mute while enforcing its stale copy.
        let mut local = self.local.lock().map_err(|_| "local audio unavailable")?;
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
            if active && !muted {
                // Swap to the device only after this exact attempt is ready.
                if !local.on_microphone {
                    local
                        .sender
                        .set_track(Some(local.microphone.clone()))
                        .map_err(|error| error.to_string())?;
                    local.on_microphone = true;
                }
                local.microphone.set_enabled(true);
                local.factory.set_adm_recording_enabled(true);
            } else {
                // Muting stops physical capture, but a zero-PCM source keeps
                // the published audio MID alive for remote subscription.
                local.microphone.set_enabled(false);
                local.factory.set_adm_recording_enabled(false);
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
        Ok(())
    }

    pub fn activate(&self) -> Result<(), String> {
        if !self.is_cancelled() {
            self.activated.store(true, Ordering::Release);
            self.enforce_local_audio()?;
        }
        Ok(())
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
        };
        // Create and negotiate with closed local devices. Only the current
        // desktop attempt may activate them after gateway and roster readiness.
        factory.set_adm_playout_enabled(false);
        factory.set_adm_recording_enabled(false);
        if input_guid.is_some_and(|guid| !factory.set_recording_device_by_guid(guid)) {
            return Err("selected microphone is unavailable".into());
        }
        if output_guid.is_some_and(|guid| !factory.set_playout_device_by_guid(guid)) {
            return Err("selected speaker is unavailable".into());
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
        let remote_tracks = Arc::new(Mutex::new(BTreeMap::new()));
        let callback_subscriptions = subscriptions.clone();
        let callback_tracks = remote_tracks.clone();
        peer.on_track(Some(Box::new(move |event| {
            let Some(mid) = event.transceiver.mid() else {
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
            if let Some(track_id) = track_id
                && let Ok(mut tracks) = callback_tracks.lock()
            {
                tracks.insert(track_id, event.track);
            }
        })));

        let audio = factory.create_device_audio_track("caper-microphone");
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
        if let Ok(mut local) = control.local.lock() {
            *local = Some(LocalAudio {
                factory: factory.clone(),
                peer: peer.clone(),
                sender: transceiver.sender(),
                microphone: audio.clone().into(),
                silence: silence.into(),
                on_microphone: false,
            });
            if *control.stop.borrow() {
                local.as_ref().unwrap().silence();
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
            factory,
            peer,
            token: joined.token,
            self_id: joined.id,
            microphone: audio.into(),
            subscriptions,
            remote_tracks,
            state_sequence: 0,
            muted,
            deafened,
            mute_before_deafen: muted,
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
        self.factory
            .set_recording_device_by_guid(guid)
            .then_some(())
            .ok_or_else(|| "selected microphone is unavailable".into())
    }

    pub fn select_output(&self, guid: &str) -> Result<(), VoiceError> {
        self.factory
            .set_playout_device_by_guid(guid)
            .then_some(())
            .ok_or_else(|| "selected speaker is unavailable".into())
    }

    pub async fn set_muted(&mut self, muted: bool) -> Result<(), VoiceError> {
        if self.deafened && !muted {
            return Ok(());
        }
        self.muted = muted;
        self.local_control.enforce_local_audio()?;
        self.sync_state().await
    }

    pub async fn set_deafened(&mut self, deafened: bool) -> Result<(), VoiceError> {
        if deafened == self.deafened {
            return Ok(());
        }
        if deafened {
            self.mute_before_deafen = self.muted;
            self.muted = true;
        } else {
            self.muted = self.mute_before_deafen;
        }
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
            .create_offer(OfferOptions {
                ice_restart: true,
                ..OfferOptions::default()
            })
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
        if let Some(track) = self
            .remote_tracks
            .lock()
            .map_err(|_| "remote voice state unavailable")?
            .remove(track_id)
        {
            track.set_enabled(false);
        }
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
        let tracks = self
            .remote_tracks
            .lock()
            .map_err(|_| "remote voice state unavailable")?;
        let track = tracks
            .get(track_id)
            .ok_or_else(|| "remote audio track is unavailable".to_owned())?;
        track.set_enabled(!muted);
        Ok(())
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
        *control.local.lock().unwrap() = Some(LocalAudio {
            factory: factory.clone(),
            peer: sender.clone(),
            sender: transceiver.sender(),
            microphone: track.clone().into(),
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
                assert!(
                    Instant::now() < deadline,
                    "No two-way RTP within 10s: A sent/received {}/{}, B sent/received {}/{}",
                    a.sent_bytes,
                    a.received_bytes,
                    b.sent_bytes,
                    b.received_bytes
                );
                tokio::time::sleep(Duration::from_millis(500)).await;
            };
            eprintln!(
                "silent SFU: both transport-connected, published and subscribed; A sent/received {}/{}, B sent/received {}/{} bytes (isolated virtual devices)",
                first_stats.sent_bytes,
                first_stats.received_bytes,
                second_stats.sent_bytes,
                second_stats.received_bytes
            );
            first.leave();
            second.leave();
            assert!(first_control.is_cancelled() && second_control.is_cancelled());
        };
        tokio::time::timeout(Duration::from_secs(55), work)
            .await
            .expect("silent SFU smoke exceeded 55s; local Drop closed media");
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
            factory,
            peer: peer.clone(),
            token: "media-token".into(),
            self_id: "self".into(),
            microphone: audio.clone(),
            subscriptions: Arc::new(Mutex::new(BTreeMap::new())),
            remote_tracks: Arc::new(Mutex::new(BTreeMap::new())),
            state_sequence: 0,
            muted: false,
            deafened: false,
            mute_before_deafen: false,
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
            .remote_tracks
            .lock()
            .unwrap()
            .insert("departed".into(), receiver.clone());
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
            factory: factory.clone(),
            peer,
            token: "local-test-token".into(),
            self_id: "self".into(),
            microphone: factory.create_device_audio_track("local-test-mic").into(),
            subscriptions: Arc::new(Mutex::new(BTreeMap::new())),
            remote_tracks: Arc::new(Mutex::new(BTreeMap::new())),
            state_sequence: 0,
            muted: false,
            deafened: false,
            mute_before_deafen: false,
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
