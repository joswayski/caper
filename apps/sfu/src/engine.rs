//! The media engine: every provider session is one `str0m` [`Rtc`] (one browser
//! `RTCPeerConnection`), multiplexed over a single UDP port. Audio frames
//! published into one session are forwarded unchanged to every session that
//! pulled that track. One task owns all state; HTTP handlers send it commands.

use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    net::SocketAddr,
    time::{Duration, Instant},
};
use str0m::{
    Candidate, Event, Input, Output, Rtc,
    change::{SdpAnswer, SdpOffer, SdpPendingOffer},
    media::{Direction, MediaData, MediaKind, Mid},
    net::{Protocol, Receive},
};
use tokio::{
    net::UdpSocket,
    sync::{mpsc, oneshot},
};

/// A session that never receives a packet is dropped after this long. The
/// Caper API only offers an unconnected prepared session for 8 seconds.
const UNCONNECTED_TTL: Duration = Duration::from_secs(30);
/// A connected session whose browser stops sending (ICE consent checks every
/// few seconds, even without media) is dropped after this long. Browsers give
/// a `disconnected` connection 10 seconds before rejoining.
const IDLE_TTL: Duration = Duration::from_secs(30);
const HOUSEKEEPING: Duration = Duration::from_secs(1);
const MAX_TRACKS_PER_REQUEST: usize = 64;

#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub code: &'static str,
    pub description: String,
}

impl ApiError {
    fn new(status: u16, code: &'static str, description: impl Into<String>) -> Self {
        Self {
            status,
            code,
            description: description.into(),
        }
    }
    fn session() -> Self {
        Self::new(404, "not_found_session_error", "session not found")
    }
    fn invalid(description: impl Into<String>) -> Self {
        Self::new(400, "invalid_request_error", description)
    }
    fn sdp(error: &impl std::fmt::Display) -> Self {
        Self::new(400, "invalid_sdp_error", error.to_string())
    }
}

pub type Reply<T> = oneshot::Sender<Result<T, ApiError>>;

/// One requested track in `tracks/new`, in Cloudflare Realtime's shape.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackRequest {
    pub location: String,
    pub mid: Option<String>,
    pub session_id: Option<String>,
    pub track_name: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Description {
    #[serde(rename = "type")]
    pub kind: String,
    pub sdp: String,
}

pub enum Command {
    Create {
        offer: Option<String>,
        reply: Reply<Value>,
    },
    Get {
        session: String,
        reply: Reply<Value>,
    },
    Tracks {
        session: String,
        offer: Option<Description>,
        tracks: Vec<TrackRequest>,
        reply: Reply<Value>,
    },
    Renegotiate {
        session: String,
        answer: Description,
        reply: Reply<Value>,
    },
    Close {
        session: String,
        mids: Vec<String>,
        reply: Reply<Value>,
    },
    Stats {
        reply: oneshot::Sender<Stats>,
    },
}

#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct Stats {
    pub sessions: usize,
    pub connected: usize,
    pub published: usize,
    pub forwards: usize,
}

struct Remote {
    source: String,
    name: String,
}

struct Session {
    rtc: Rtc,
    created: Instant,
    last_received: Option<Instant>,
    deadline: Instant,
    pending: Option<SdpPendingOffer>,
    negotiated: bool,
    /// Published tracks: the browser's MID to its track name.
    local: BTreeMap<String, String>,
    /// Pulled tracks: this session's MID to the source.
    remote: BTreeMap<String, Remote>,
}

pub struct Sfu {
    public: SocketAddr,
    max_sessions: usize,
    sessions: HashMap<String, Session>,
    /// (source session, track name) to its subscribers' (session, MID).
    forwards: HashMap<(String, String), Vec<(String, String)>>,
    /// Remote address to session, a cache in front of `Rtc::accepts`.
    addresses: HashMap<SocketAddr, String>,
    dirty: HashSet<String>,
    housekeeping: Instant,
}

impl Sfu {
    #[must_use]
    pub fn new(public: SocketAddr, max_sessions: usize, now: Instant) -> Self {
        str0m::crypto::from_feature_flags().install_process_default();
        Self {
            public,
            max_sessions,
            sessions: HashMap::new(),
            forwards: HashMap::new(),
            addresses: HashMap::new(),
            dirty: HashSet::new(),
            housekeeping: now + HOUSEKEEPING,
        }
    }

    fn rtc(&self, now: Instant) -> Result<Rtc, ApiError> {
        let mut rtc = Rtc::builder()
            .set_ice_lite(true)
            .clear_codecs()
            .enable_opus(true, false)
            .build(now);
        let candidate = Candidate::host(self.public, "udp")
            .map_err(|_| ApiError::new(500, "internal_error", "invalid public address"))?;
        rtc.add_local_candidate(candidate);
        Ok(rtc)
    }

    pub fn command(&mut self, command: Command, now: Instant) {
        match command {
            Command::Create { offer, reply } => {
                let _ = reply.send(self.create(offer.as_deref(), now));
            }
            Command::Get { session, reply } => {
                let _ = reply.send(Ok(self.get(&session)));
            }
            Command::Tracks {
                session,
                offer,
                tracks,
                reply,
            } => {
                let _ = reply.send(self.tracks(&session, offer, &tracks));
            }
            Command::Renegotiate {
                session,
                answer,
                reply,
            } => {
                let _ = reply.send(self.renegotiate(&session, &answer));
            }
            Command::Close {
                session,
                mids,
                reply,
            } => {
                let _ = reply.send(Ok(self.close(&session, &mids)));
            }
            Command::Stats { reply } => {
                let _ = reply.send(self.stats());
            }
        }
    }

    #[must_use]
    pub fn stats(&self) -> Stats {
        Stats {
            sessions: self.sessions.len(),
            connected: self
                .sessions
                .values()
                .filter(|s| s.last_received.is_some())
                .count(),
            published: self.sessions.values().map(|s| s.local.len()).sum(),
            forwards: self.forwards.values().map(Vec::len).sum(),
        }
    }

    pub fn create(&mut self, offer: Option<&str>, now: Instant) -> Result<Value, ApiError> {
        if self.sessions.len() >= self.max_sessions {
            return Err(ApiError::new(503, "capacity_error", "session limit reached"));
        }
        let mut rtc = self.rtc(now)?;
        let answer = match offer {
            Some(sdp) => {
                let offer = SdpOffer::from_sdp_string(sdp).map_err(|e| ApiError::sdp(&e))?;
                Some(
                    rtc.sdp_api()
                        .accept_offer(offer)
                        .map_err(|e| ApiError::sdp(&e))?,
                )
            }
            None => None,
        };
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.sessions.insert(
            id.clone(),
            Session {
                rtc,
                created: now,
                last_received: None,
                deadline: now,
                pending: None,
                negotiated: answer.is_some(),
                local: BTreeMap::new(),
                remote: BTreeMap::new(),
            },
        );
        self.dirty.insert(id.clone());
        Ok(match answer {
            Some(answer) => json!({
                "sessionId": id,
                "sessionDescription": {"type": "answer", "sdp": answer.to_sdp_string()},
            }),
            None => json!({ "sessionId": id }),
        })
    }

    #[must_use]
    pub fn get(&self, session: &str) -> Value {
        // A session that already ended has nothing left to close.
        let Some(s) = self.sessions.get(session) else {
            return json!({ "tracks": [] });
        };
        let local = s.local.iter().map(|(mid, name)| {
            json!({"location": "local", "mid": mid, "trackName": name, "status": "active"})
        });
        let remote = s.remote.iter().map(|(mid, r)| {
            json!({"location": "remote", "mid": mid, "sessionId": r.source, "trackName": r.name, "status": "active"})
        });
        json!({ "tracks": local.chain(remote).collect::<Vec<_>>() })
    }

    pub fn tracks(
        &mut self,
        id: &str,
        offer: Option<Description>,
        tracks: &[TrackRequest],
    ) -> Result<Value, ApiError> {
        if tracks.len() > MAX_TRACKS_PER_REQUEST {
            return Err(ApiError::invalid("too many tracks"));
        }
        if !self.sessions.contains_key(id) {
            return Err(ApiError::session());
        }
        match offer {
            Some(offer) => self.push(id, &offer, tracks),
            None => self.pull(id, tracks),
        }
    }

    /// An offer from the browser: publication of local tracks, or an ICE
    /// restart when no tracks are listed.
    fn push(
        &mut self,
        id: &str,
        offer: &Description,
        tracks: &[TrackRequest],
    ) -> Result<Value, ApiError> {
        if offer.kind != "offer" {
            return Err(ApiError::invalid("sessionDescription must be an offer"));
        }
        let mut local = vec![];
        for track in tracks {
            match (track.location.as_str(), &track.mid, &track.track_name) {
                ("local", Some(mid), Some(name)) if !mid.is_empty() && !name.is_empty() => {
                    local.push((mid.clone(), name.clone()));
                }
                _ => {
                    return Err(ApiError::invalid(
                        "an offer can only publish local tracks with a mid and trackName",
                    ));
                }
            }
        }
        let s = self.sessions.get_mut(id).ok_or_else(ApiError::session)?;
        if s.pending.is_some() {
            return Err(ApiError::new(
                409,
                "negotiation_pending_error",
                "answer the pending offer first",
            ));
        }
        for (mid, name) in &local {
            if s.local.iter().any(|(m, n)| n == name && m != mid) {
                return Err(ApiError::invalid("trackName already published"));
            }
        }
        let sdp = SdpOffer::from_sdp_string(&offer.sdp).map_err(|e| ApiError::sdp(&e))?;
        let answer = s
            .rtc
            .sdp_api()
            .accept_offer(sdp)
            .map_err(|e| ApiError::sdp(&e))?;
        s.negotiated = true;
        let mut results = vec![];
        for (mid, name) in local {
            let known = s
                .rtc
                .media(Mid::from(mid.as_str()))
                .is_some_and(|m| m.kind() == MediaKind::Audio);
            if known {
                s.local.insert(mid.clone(), name.clone());
                results.push(json!({"mid": mid, "trackName": name, "sessionId": id}));
            } else {
                results.push(json!({
                    "mid": mid, "trackName": name, "sessionId": id,
                    "errorCode": "invalid_track_error",
                    "errorDescription": "mid is not an audio section of the offer",
                }));
            }
        }
        self.dirty.insert(id.to_owned());
        Ok(json!({
            "requiresImmediateRenegotiation": false,
            "tracks": results,
            "sessionDescription": {"type": "answer", "sdp": answer.to_sdp_string()},
        }))
    }

    /// Pulls of other sessions' published tracks: the SFU makes the offer.
    fn pull(&mut self, id: &str, tracks: &[TrackRequest]) -> Result<Value, ApiError> {
        if tracks.is_empty() {
            return Err(ApiError::invalid("no tracks"));
        }
        let mut wanted = vec![];
        for track in tracks {
            match (track.location.as_str(), &track.session_id, &track.track_name) {
                ("remote", Some(source), Some(name)) => {
                    let found = source != id
                        && self
                            .sessions
                            .get(source)
                            .is_some_and(|s| s.local.values().any(|n| n == name));
                    wanted.push((source.clone(), name.clone(), found));
                }
                _ => {
                    return Err(ApiError::invalid(
                        "without an offer, only remote tracks with a sessionId and trackName can be requested",
                    ));
                }
            }
        }
        let s = self.sessions.get_mut(id).ok_or_else(ApiError::session)?;
        if s.pending.is_some() {
            return Err(ApiError::new(
                409,
                "negotiation_pending_error",
                "answer the pending offer first",
            ));
        }
        let mut change = s.rtc.sdp_api();
        let mut results = vec![];
        let mut added = vec![];
        for (source, name, found) in wanted {
            if found {
                let mid = change
                    .add_media(
                        MediaKind::Audio,
                        Direction::SendOnly,
                        Some(source.clone()),
                        Some(name.clone()),
                        None,
                    )
                    .to_string();
                results.push(json!({"mid": mid, "sessionId": source, "trackName": name}));
                added.push((mid, source, name));
            } else {
                results.push(json!({
                    "sessionId": source, "trackName": name,
                    "errorCode": "not_found_track_error",
                    "errorDescription": "track not found",
                }));
            }
        }
        // Like Cloudflare, a session without a negotiated connection gets an
        // offer even when every source is gone, so the browser can connect.
        if added.is_empty() && !s.negotiated {
            change.add_media(MediaKind::Audio, Direction::Inactive, None, None, None);
        }
        let offer = change.apply();
        let description = offer.map(|(offer, pending)| {
            s.pending = Some(pending);
            json!({"type": "offer", "sdp": offer.to_sdp_string()})
        });
        for (mid, source, name) in added {
            s.remote.insert(
                mid.clone(),
                Remote {
                    source: source.clone(),
                    name: name.clone(),
                },
            );
            self.forwards
                .entry((source, name))
                .or_default()
                .push((id.to_owned(), mid));
        }
        self.dirty.insert(id.to_owned());
        let mut value = json!({
            "requiresImmediateRenegotiation": description.is_some(),
            "tracks": results,
        });
        if let Some(description) = description {
            value["sessionDescription"] = description;
        }
        Ok(value)
    }

    pub fn renegotiate(&mut self, id: &str, answer: &Description) -> Result<Value, ApiError> {
        if answer.kind != "answer" {
            return Err(ApiError::invalid("sessionDescription must be an answer"));
        }
        let s = self.sessions.get_mut(id).ok_or_else(ApiError::session)?;
        let Some(pending) = s.pending.take() else {
            return Err(ApiError::invalid("no pending offer"));
        };
        let sdp = SdpAnswer::from_sdp_string(&answer.sdp).map_err(|e| ApiError::sdp(&e))?;
        s.rtc
            .sdp_api()
            .accept_answer(pending, sdp)
            .map_err(|e| ApiError::sdp(&e))?;
        s.negotiated = true;
        self.dirty.insert(id.to_owned());
        Ok(json!({}))
    }

    /// Stops forwarding into or out of these MIDs without renegotiating, like
    /// Cloudflare's `force: true`. Closing an unknown MID is a no-op.
    pub fn close(&mut self, id: &str, mids: &[String]) -> Value {
        let mut closed = vec![];
        if let Some(s) = self.sessions.get_mut(id) {
            for mid in mids {
                if let Some(name) = s.local.remove(mid) {
                    self.forwards.remove(&(id.to_owned(), name));
                }
                if let Some(remote) = s.remote.remove(mid) {
                    let key = (remote.source, remote.name);
                    if let Some(list) = self.forwards.get_mut(&key) {
                        list.retain(|(session, m)| !(session == id && m == mid));
                        if list.is_empty() {
                            self.forwards.remove(&key);
                        }
                    }
                }
                closed.push(json!({ "mid": mid }));
            }
        } else {
            closed.extend(mids.iter().map(|mid| json!({ "mid": mid })));
        }
        json!({"requiresImmediateRenegotiation": false, "tracks": closed})
    }

    fn remove(&mut self, id: &str) {
        let Some(s) = self.sessions.remove(id) else {
            return;
        };
        for name in s.local.into_values() {
            self.forwards.remove(&(id.to_owned(), name));
        }
        for (mid, remote) in s.remote {
            let key = (remote.source, remote.name);
            if let Some(list) = self.forwards.get_mut(&key) {
                list.retain(|(session, m)| !(session == id && *m == mid));
                if list.is_empty() {
                    self.forwards.remove(&key);
                }
            }
        }
        self.addresses.retain(|_, session| session != id);
        self.dirty.remove(id);
        tracing::debug!(session = id, "session ended");
    }

    pub fn receive(&mut self, source: SocketAddr, packet: &[u8], now: Instant) {
        let Ok(contents) = packet.try_into() else {
            return;
        };
        let input = Input::Receive(
            now,
            Receive {
                proto: Protocol::Udp,
                source,
                destination: self.public,
                contents,
            },
        );
        let cached = self
            .addresses
            .get(&source)
            .filter(|id| {
                self.sessions
                    .get(*id)
                    .is_some_and(|s| s.rtc.accepts(&input))
            })
            .cloned();
        let id = cached.or_else(|| {
            let id = self
                .sessions
                .iter()
                .find(|(_, s)| s.rtc.accepts(&input))
                .map(|(id, _)| id.clone())?;
            self.addresses.insert(source, id.clone());
            Some(id)
        });
        let Some(id) = id else {
            return;
        };
        let s = self.sessions.get_mut(&id).expect("session exists");
        s.last_received = Some(now);
        if let Err(error) = s.rtc.handle_input(input) {
            tracing::debug!(session = id, %error, "session input failed");
            s.rtc.disconnect();
        }
        self.dirty.insert(id);
    }

    pub fn timeout(&mut self, now: Instant) {
        for (id, s) in &mut self.sessions {
            if s.deadline <= now {
                if let Err(error) = s.rtc.handle_input(Input::Timeout(now)) {
                    tracing::debug!(session = id, %error, "session timeout failed");
                    s.rtc.disconnect();
                }
                self.dirty.insert(id.clone());
            }
        }
        if self.housekeeping <= now {
            self.housekeeping = now + HOUSEKEEPING;
            let expired = self
                .sessions
                .iter()
                .filter(|(_, s)| {
                    !s.rtc.is_alive()
                        || match s.last_received {
                            Some(at) => now.duration_since(at) > IDLE_TTL,
                            None => now.duration_since(s.created) > UNCONNECTED_TTL,
                        }
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            for id in expired {
                self.remove(&id);
            }
        }
    }

    #[must_use]
    pub fn next_deadline(&self) -> Instant {
        self.sessions
            .values()
            .map(|s| s.deadline)
            .fold(self.housekeeping, Instant::min)
    }

    /// Polls every session with new input until it only waits for time,
    /// forwarding media as it goes. Returns datagrams to send.
    pub fn drive(&mut self, now: Instant, out: &mut Vec<(SocketAddr, Vec<u8>)>) {
        let mut queue = self.dirty.drain().collect::<VecDeque<_>>();
        let mut media = vec![];
        while let Some(id) = queue.pop_front() {
            let Some(s) = self.sessions.get_mut(&id) else {
                continue;
            };
            loop {
                if !s.rtc.is_alive() {
                    s.deadline = now + HOUSEKEEPING;
                    break;
                }
                match s.rtc.poll_output() {
                    Ok(Output::Timeout(at)) => {
                        s.deadline = at;
                        break;
                    }
                    Ok(Output::Transmit(t)) => out.push((t.destination, t.contents.to_vec())),
                    Ok(Output::Event(Event::MediaData(data))) => media.push(data),
                    Ok(Output::Event(_)) => {}
                    Err(error) => {
                        tracing::debug!(session = id, %error, "session output failed");
                        s.rtc.disconnect();
                    }
                }
            }
            for data in media.drain(..) {
                for target in self.forward(&id, &data) {
                    if !queue.contains(&target) {
                        queue.push_back(target);
                    }
                }
            }
        }
    }

    fn forward(&mut self, source: &str, data: &MediaData) -> Vec<String> {
        let Some(name) = self
            .sessions
            .get(source)
            .and_then(|s| s.local.get(&data.mid.to_string()))
        else {
            return vec![];
        };
        let Some(targets) = self.forwards.get(&(source.to_owned(), name.clone())) else {
            return vec![];
        };
        let mut written = vec![];
        for (target, mid) in targets {
            let Some(s) = self.sessions.get_mut(target) else {
                continue;
            };
            // None until the subscriber has answered the offer for this MID.
            let Some(writer) = s.rtc.writer(Mid::from(mid.as_str())) else {
                continue;
            };
            let Some(pt) = writer.match_params(data.params) else {
                continue;
            };
            if let Err(error) = writer.write(pt, data.network_time, data.time, data.data.clone()) {
                tracing::debug!(session = target, %error, "forward failed");
                continue;
            }
            written.push(target.clone());
        }
        written
    }
}

pub async fn run(mut sfu: Sfu, socket: UdpSocket, mut commands: mpsc::Receiver<Command>) {
    let mut buf = vec![0; 2000];
    let mut out = vec![];
    loop {
        let deadline = sfu.next_deadline();
        tokio::select! {
            command = commands.recv() => match command {
                Some(command) => sfu.command(command, Instant::now()),
                None => break,
            },
            received = socket.recv_from(&mut buf) => match received {
                Ok((n, source)) => sfu.receive(source, &buf[..n], Instant::now()),
                Err(error) => tracing::warn!(%error, "UDP receive failed"),
            },
            () = tokio::time::sleep_until(deadline.into()) => {}
        }
        let now = Instant::now();
        sfu.timeout(now);
        sfu.drive(now, &mut out);
        for (destination, packet) in out.drain(..) {
            if let Err(error) = socket.send_to(&packet, destination).await {
                tracing::debug!(%error, "UDP send failed");
            }
        }
    }
}
