//! Real WebRTC peers (str0m, standing in for browsers) exchange Opus frames
//! through the SFU over loopback UDP, signalled via its HTTP API exactly as
//! the Caper API drives Cloudflare.

use caper_sfu::{AppState, Credentials, engine, router, turn::Turn};
use serde_json::{Value, json};
use std::{
    net::UdpSocket,
    time::{Duration, Instant},
};
use str0m::{
    Candidate, Event, Input, Output, Rtc,
    change::{SdpAnswer, SdpOffer},
    media::{Direction, Frequency, MediaKind, MediaTime, Mid},
    net::{Protocol, Receive},
};

const SECRET: &str = "test-secret";

struct Sfu {
    base: String,
    client: reqwest::Client,
}

impl Sfu {
    async fn start() -> Self {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let public = socket.local_addr().unwrap();
        let (commands, receive) = tokio::sync::mpsc::channel(64);
        tokio::spawn(engine::run(
            engine::Sfu::new(public, 100, Instant::now()),
            socket,
            receive,
        ));
        let credentials = Credentials {
            app_id: "app".into(),
            app_secret: SECRET.into(),
            turn_key_id: "key".into(),
            turn_api_token: "turn-token".into(),
        };
        let turn = Turn::new(
            Some("coturn".into()),
            vec![],
            vec!["turn:127.0.0.1:3478".into()],
        );
        let app = router(AppState::new(commands, credentials, turn));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            base,
            client: reqwest::Client::new(),
        }
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut request = self
            .client
            .request(method, format!("{}/apps/app/{path}", self.base))
            .bearer_auth(SECRET);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request.send().await.unwrap();
        let status = response.status().as_u16();
        let text = response.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }
}

struct Peer {
    rtc: Rtc,
    socket: UdpSocket,
    received: usize,
}

impl Peer {
    fn new() -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        socket.set_nonblocking(true).unwrap();
        let mut rtc = Rtc::new(Instant::now());
        rtc.add_local_candidate(Candidate::host(socket.local_addr().unwrap(), "udp").unwrap());
        Self {
            rtc,
            socket,
            received: 0,
        }
    }

    fn step(&mut self) {
        let mut buf = [0; 2000];
        while let Ok((n, source)) = self.socket.recv_from(&mut buf) {
            let receive = Receive {
                proto: Protocol::Udp,
                source,
                destination: self.socket.local_addr().unwrap(),
                contents: buf[..n].try_into().unwrap(),
            };
            self.rtc
                .handle_input(Input::Receive(Instant::now(), receive))
                .unwrap();
        }
        self.rtc
            .handle_input(Input::Timeout(Instant::now()))
            .unwrap();
        loop {
            match self.rtc.poll_output().unwrap() {
                Output::Timeout(_) => break,
                Output::Transmit(t) => {
                    let _ = self.socket.send_to(&t.contents, t.destination);
                }
                Output::Event(Event::MediaData(_)) => self.received += 1,
                Output::Event(_) => {}
            }
        }
    }
}

fn publish_offer(peer: &mut Peer) -> (Mid, String, str0m::change::SdpPendingOffer) {
    let mut change = peer.rtc.sdp_api();
    let mid = change.add_media(MediaKind::Audio, Direction::SendOnly, None, None, None);
    let (offer, pending) = change.apply().unwrap();
    (mid, offer.to_sdp_string(), pending)
}

/// Writes one 20 ms Opus-sized frame once the sender's track is negotiated.
fn send_frame(peer: &mut Peer, mid: Mid, frame: u64) -> bool {
    let Some(writer) = peer.rtc.writer(mid) else {
        return false;
    };
    let Some(pt) = writer.payload_params().next().map(|p| p.pt()) else {
        return false;
    };
    let time = MediaTime::from_millis(frame * 20).rebase(Frequency::FORTY_EIGHT_KHZ);
    writer
        .write(pt, Instant::now(), time, vec![0xf8, 0xff, 0xfe])
        .is_ok()
}

#[tokio::test]
async fn audio_published_into_one_session_reaches_a_puller() {
    let sfu = Sfu::start().await;

    // Publisher: create a session, then publish with an offer.
    let mut sender = Peer::new();
    let (status, created) = sfu.call(reqwest::Method::POST, "sessions/new", None).await;
    assert_eq!(status, 201);
    let sender_session = created["sessionId"].as_str().unwrap().to_owned();
    let (mid, offer, pending) = publish_offer(&mut sender);
    let (status, published) = sfu
        .call(
            reqwest::Method::POST,
            &format!("sessions/{sender_session}/tracks/new"),
            Some(json!({
                "sessionDescription": {"type": "offer", "sdp": offer},
                "tracks": [{"location": "local", "mid": mid.to_string(), "trackName": "mic", "kind": "audio"}],
            })),
        )
        .await;
    assert_eq!(status, 200, "{published}");
    assert_eq!(published["tracks"][0]["mid"], mid.to_string());
    assert_eq!(published["sessionDescription"]["type"], "answer");
    let answer =
        SdpAnswer::from_sdp_string(published["sessionDescription"]["sdp"].as_str().unwrap())
            .unwrap();
    sender.rtc.sdp_api().accept_answer(pending, answer).unwrap();

    // Listener: a fresh session pulls the track; the SFU offers.
    let mut listener = Peer::new();
    let (_, created) = sfu.call(reqwest::Method::POST, "sessions/new", None).await;
    let listener_session = created["sessionId"].as_str().unwrap().to_owned();
    let (status, pulled) = sfu
        .call(
            reqwest::Method::POST,
            &format!("sessions/{listener_session}/tracks/new"),
            Some(json!({"tracks": [
                {"location": "remote", "sessionId": sender_session, "trackName": "mic"},
                {"location": "remote", "sessionId": sender_session, "trackName": "gone"},
            ]})),
        )
        .await;
    assert_eq!(status, 200, "{pulled}");
    assert_eq!(pulled["requiresImmediateRenegotiation"], true);
    assert!(pulled["tracks"][0]["mid"].is_string());
    assert_eq!(pulled["tracks"][1]["errorCode"], "not_found_track_error");
    let offer =
        SdpOffer::from_sdp_string(pulled["sessionDescription"]["sdp"].as_str().unwrap()).unwrap();
    let answer = listener.rtc.sdp_api().accept_offer(offer).unwrap();
    let (status, _) = sfu
        .call(
            reqwest::Method::PUT,
            &format!("sessions/{listener_session}/renegotiate"),
            Some(json!({"sessionDescription": {"type": "answer", "sdp": answer.to_sdp_string()}})),
        )
        .await;
    assert_eq!(status, 200);

    // Send 20 ms Opus-sized frames until the listener has decoded some.
    let deadline = Instant::now() + Duration::from_secs(10);
    let started = Instant::now();
    let mut next_frame = Instant::now();
    let mut frame = 0u64;
    while listener.received < 25 && Instant::now() < deadline {
        sender.step();
        listener.step();
        if Instant::now() >= next_frame {
            next_frame += Duration::from_millis(20);
            if send_frame(&mut sender, mid, frame) {
                frame += 1;
            }
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert!(
        listener.received >= 25,
        "listener received {} frames in {:?}",
        listener.received,
        started.elapsed()
    );

    // Discovery lists both MIDs; closing the pull stops forwarding.
    let (_, tracks) = sfu
        .call(
            reqwest::Method::GET,
            &format!("sessions/{listener_session}"),
            None,
        )
        .await;
    let pulled_mid = pulled["tracks"][0]["mid"].as_str().unwrap();
    assert_eq!(tracks["tracks"][0]["mid"], pulled_mid);
    assert_eq!(tracks["tracks"][0]["location"], "remote");
    let (status, closed) = sfu
        .call(
            reqwest::Method::PUT,
            &format!("sessions/{listener_session}/tracks/close"),
            Some(json!({"tracks": [{"mid": pulled_mid}], "force": true})),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(closed["tracks"][0]["mid"], pulled_mid);
    let before = listener.received;
    let until = Instant::now() + Duration::from_millis(500);
    while Instant::now() < until {
        sender.step();
        listener.step();
        if send_frame(&mut sender, mid, frame) {
            frame += 1;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Frames already in flight may still land; nothing new is forwarded.
    assert!(
        listener.received - before <= 3,
        "{} frames after close",
        listener.received - before
    );
}

#[tokio::test]
async fn requests_need_the_app_secret_and_known_sessions() {
    let sfu = Sfu::start().await;
    let response = sfu
        .client
        .post(format!("{}/apps/app/sessions/new", sfu.base))
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 401);
    let (status, error) = sfu
        .call(
            reqwest::Method::POST,
            "sessions/missing/tracks/new",
            Some(json!({"tracks": [{"location": "remote", "sessionId": "x", "trackName": "y"}]})),
        )
        .await;
    assert_eq!(status, 404);
    assert_eq!(error["errorCode"], "not_found_session_error");
    // Cleanup of an ended session is idempotent.
    let (status, closed) = sfu
        .call(
            reqwest::Method::PUT,
            "sessions/missing/tracks/close",
            Some(json!({"tracks": [{"mid": "1"}], "force": true})),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(closed["tracks"][0]["mid"], "1");
}

#[tokio::test]
async fn a_pull_into_an_unnegotiated_session_still_offers_when_every_source_is_gone() {
    let sfu = Sfu::start().await;
    let (_, created) = sfu.call(reqwest::Method::POST, "sessions/new", None).await;
    let session = created["sessionId"].as_str().unwrap();
    let (status, pulled) = sfu
        .call(
            reqwest::Method::POST,
            &format!("sessions/{session}/tracks/new"),
            Some(json!({"tracks": [{"location": "remote", "sessionId": "nobody", "trackName": "mic"}]})),
        )
        .await;
    assert_eq!(status, 200);
    assert_eq!(pulled["requiresImmediateRenegotiation"], true);
    assert_eq!(pulled["sessionDescription"]["type"], "offer");
    assert!(pulled["tracks"][0]["mid"].is_null());
}

#[tokio::test]
async fn turn_credentials_use_the_cloudflare_shape() {
    let sfu = Sfu::start().await;
    let response = sfu
        .client
        .post(format!(
            "{}/turn/keys/key/credentials/generate-ice-servers",
            sfu.base
        ))
        .bearer_auth("turn-token")
        .json(&json!({"ttl": 172_800}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let value: Value = response.json().await.unwrap();
    assert_eq!(value["iceServers"][0]["urls"][0], "turn:127.0.0.1:3478");
    assert!(value["iceServers"][0]["credential"].is_string());
    let revoked = sfu
        .client
        .post(format!("{}/turn/keys/key/credentials/abc/revoke", sfu.base))
        .bearer_auth("turn-token")
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), 204);
}
