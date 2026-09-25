use crate::model::{Author, Message, Presence, VoiceOccupant, sequence};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};
use std::thread;
use std::time::{Duration, Instant};
use tungstenite::{Message as WsMessage, client::IntoClientRequest, stream::MaybeTlsStream};

#[derive(Clone, Debug)]
pub struct MediaChannel {
    pub id: String,
    pub demo: bool,
}

#[derive(Default)]
pub struct MediaWatch {
    pub epoch: u64,
    pub channels: Vec<MediaChannel>,
}

pub const MAX_MEDIA_CHANNELS: usize = 24;

#[derive(Debug)]
pub enum GatewayEvent {
    Status {
        generation: u64,
        channel: String,
        online: bool,
        detail: String,
    },
    Message {
        generation: u64,
        channel: String,
        message: Box<Message>,
    },
    Typing {
        generation: u64,
        channel: String,
        author: Author,
        typing: bool,
        revision: String,
    },
    Presence {
        generation: u64,
        space: String,
        members: Vec<Presence>,
    },
    VoiceRoster {
        generation: u64,
        channel: String,
        participants: Vec<VoiceOccupant>,
    },
    VoiceUnavailable {
        generation: u64,
        channel: String,
        revoked: bool,
    },
    VoiceReset {
        generation: u64,
    },
    Resync {
        generation: u64,
        channel: String,
    },
    AccessDenied {
        generation: u64,
        channel: String,
        detail: String,
    },
}

pub struct GatewayControl {
    stop: Arc<AtomicBool>,
    activity: Arc<Mutex<Instant>>,
}

impl GatewayControl {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn activity(&self) {
        if let Ok(mut activity) = self.activity.lock() {
            *activity = Instant::now();
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn(
    base: &url::Url,
    account_token: Option<String>,
    generation: u64,
    channel: String,
    cursor: String,
    presence: Option<(String, Vec<String>)>,
    media: MediaWatch,
    events: Sender<GatewayEvent>,
) -> GatewayControl {
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let activity = Arc::new(Mutex::new(Instant::now()));
    let thread_activity = activity.clone();
    let mut url = base.join("api/chat/events").expect("constant gateway path");
    url.set_scheme(if base.scheme() == "https" {
        "wss"
    } else {
        "ws"
    })
    .expect("compatible scheme");
    thread::spawn(move || {
        run(
            url,
            account_token,
            generation,
            channel,
            cursor,
            presence,
            media,
            events,
            stopped,
            thread_activity,
        )
    });
    GatewayControl { stop, activity }
}

#[derive(Debug)]
enum Failure {
    Retry(String),
    Denied(String),
}

#[derive(Clone, Copy)]
struct Timing {
    heartbeat: Duration,
    watchdog: Duration,
    poll: Duration,
}

const TIMING: Timing = Timing {
    heartbeat: Duration::from_secs(10),
    watchdog: Duration::from_secs(30),
    poll: Duration::from_millis(200),
};

#[allow(clippy::too_many_arguments)]
fn run(
    url: url::Url,
    token: Option<String>,
    generation: u64,
    channel: String,
    mut cursor: String,
    presence: Option<(String, Vec<String>)>,
    media: MediaWatch,
    events: Sender<GatewayEvent>,
    stop: Arc<AtomicBool>,
    activity: Arc<Mutex<Instant>>,
) {
    let mut attempt = 0_u32;
    while !stop.load(Ordering::Relaxed) {
        let _ = events.send(GatewayEvent::Status {
            generation,
            channel: channel.clone(),
            online: false,
            detail: if attempt == 0 {
                "Connecting…"
            } else {
                "Reconnecting…"
            }
            .into(),
        });
        let result = connect_once(
            &url,
            &token,
            generation,
            &channel,
            &mut cursor,
            presence.as_ref(),
            &media,
            &events,
            &stop,
            &activity,
            TIMING,
        );
        let _ = events.send(GatewayEvent::VoiceReset {
            generation: media.epoch,
        });
        match result {
            Ok(()) if stop.load(Ordering::Relaxed) => break,
            Ok(()) => {}
            Err(Failure::Denied(detail)) => {
                let _ = events.send(GatewayEvent::AccessDenied {
                    generation,
                    channel: channel.clone(),
                    detail,
                });
                break;
            }
            Err(Failure::Retry(detail)) => {
                let _ = events.send(GatewayEvent::Status {
                    generation,
                    channel: channel.clone(),
                    online: false,
                    detail,
                });
            }
        }
        attempt = attempt.saturating_add(1);
        let delay = Duration::from_millis((250_u64 << attempt.min(4)).min(5_000));
        for _ in 0..delay.as_millis().div_ceil(100) {
            if stop.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn connect_once(
    url: &url::Url,
    token: &Option<String>,
    generation: u64,
    channel: &str,
    cursor: &mut String,
    presence: Option<&(String, Vec<String>)>,
    media: &MediaWatch,
    events: &Sender<GatewayEvent>,
    stop: &AtomicBool,
    activity: &Mutex<Instant>,
    timing: Timing,
) -> Result<(), Failure> {
    // Account credentials are carried only by the Authorization header. The
    // URL remains capability-free so proxies, histories and diagnostics cannot
    // accidentally retain a token.
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| Failure::Retry("Invalid gateway endpoint".into()))?;
    if let Some(token) = token {
        let mut authorization =
            tungstenite::http::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| Failure::Denied("Invalid account credential".into()))?;
        authorization.set_sensitive(true);
        request
            .headers_mut()
            .insert(tungstenite::http::header::AUTHORIZATION, authorization);
    }
    let (mut socket, _) = tungstenite::connect(request).map_err(|error| match error {
        tungstenite::Error::Http(response)
            if matches!(response.status().as_u16(), 401 | 403 | 404) =>
        {
            Failure::Denied("Your channel access or account session has expired.".into())
        }
        _ => Failure::Retry("Live messages are offline.".into()),
    })?;
    set_timeout(socket.get_mut(), timing.poll)
        .map_err(|_| Failure::Retry("Could not configure the live connection.".into()))?;
    let subscription = uuid::Uuid::new_v4().to_string();
    let presence_subscription = presence.map(|_| uuid::Uuid::new_v4().to_string());
    let mut media_subscriptions: std::collections::BTreeMap<_, _> = media
        .channels
        .iter()
        .take(MAX_MEDIA_CHANNELS)
        .map(|channel| (uuid::Uuid::new_v4().to_string(), (channel, None::<u64>)))
        .collect();
    let mut subscribed = false;
    let mut last_server = Instant::now();
    let mut last_heartbeat = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            let _ = socket.close(None);
            return Ok(());
        }
        if subscribed && last_heartbeat.elapsed() >= timing.heartbeat {
            let activity_age = activity.lock().map_or(86_400_000, |at| {
                at.elapsed().as_millis().min(86_400_000) as u64
            });
            socket
                .send(WsMessage::Text(
                    json!({"type":"heartbeat","activityAgeMs":activity_age})
                        .to_string()
                        .into(),
                ))
                .map_err(|_| Failure::Retry("Could not heartbeat the gateway.".into()))?;
            last_heartbeat = Instant::now();
        }
        if last_server.elapsed() > timing.watchdog {
            return Err(Failure::Retry("Live messages timed out.".into()));
        }
        let frame = match socket.read() {
            Ok(frame) => {
                last_server = Instant::now();
                frame
            }
            Err(tungstenite::Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                continue;
            }
            Err(_) => return Err(Failure::Retry("Live messages disconnected.".into())),
        };
        let WsMessage::Text(text) = frame else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| Failure::Retry("The gateway returned an invalid frame.".into()))?;
        match value["type"].as_str() {
            Some("hello") if !subscribed => {
                socket
                    .send(WsMessage::Text(
                        json!({
                            "type":"subscribe", "id":subscription, "kind":"chat",
                            "channelId":channel, "after":cursor
                        })
                        .to_string()
                        .into(),
                    ))
                    .map_err(|_| Failure::Retry("Could not subscribe to live messages.".into()))?;
                subscribed = true;
                if let (Some((space, users)), Some(id)) = (presence, &presence_subscription) {
                    socket
                        .send(WsMessage::Text(
                            json!({
                                "type":"subscribe", "id":id, "kind":"presence",
                                "spaceId":space, "userIds":users
                            })
                            .to_string()
                            .into(),
                        ))
                        .map_err(|_| {
                            Failure::Retry("Could not subscribe to member presence.".into())
                        })?;
                }
                for (id, (channel, _)) in &media_subscriptions {
                    let mut frame = json!({"type":"subscribe", "id":id, "kind":"media"});
                    if !channel.demo {
                        frame["channelId"] = json!(channel.id);
                    }
                    socket
                        .send(WsMessage::Text(frame.to_string().into()))
                        .map_err(|_| {
                            Failure::Retry("Could not subscribe to voice presence.".into())
                        })?;
                }
            }
            Some("heartbeat") => {}
            Some("subscribed") if value["id"] == subscription => {
                let _ = events.send(GatewayEvent::Status {
                    generation,
                    channel: channel.into(),
                    online: true,
                    detail: "Live".into(),
                });
            }
            Some("event") if value["id"] == subscription => {
                let event = &value["event"];
                match event["type"].as_str() {
                    Some("message.created") => {
                        let message: Message = serde_json::from_value(event["message"].clone())
                            .map_err(|_| {
                                Failure::Retry("The gateway returned an invalid message.".into())
                            })?;
                        if message.channel_id != channel
                            || event["seq"].as_str() != Some(&message.seq)
                        {
                            return Err(Failure::Retry(
                                "The gateway mixed channel data; resyncing.".into(),
                            ));
                        }
                        let previous = sequence(cursor).map_err(Failure::Retry)?;
                        let next = sequence(&message.seq).map_err(Failure::Retry)?;
                        if next > previous + 1 {
                            let _ = events.send(GatewayEvent::Resync {
                                generation,
                                channel: channel.into(),
                            });
                            return Ok(());
                        }
                        if next <= previous {
                            continue;
                        }
                        *cursor = message.seq.clone();
                        let _ = events.send(GatewayEvent::Message {
                            generation,
                            channel: channel.into(),
                            message: Box::new(message),
                        });
                    }
                    Some("resync_required") => {
                        let _ = events.send(GatewayEvent::Resync {
                            generation,
                            channel: channel.into(),
                        });
                        return Ok(());
                    }
                    Some("ready") => {
                        let ready = event["cursor"]
                            .as_str()
                            .ok_or_else(|| Failure::Retry("Invalid gateway cursor".into()))?;
                        sequence(ready).map_err(Failure::Retry)?;
                        if ready != cursor {
                            return Err(Failure::Retry(
                                "The gateway checkpoint did not match replay.".into(),
                            ));
                        }
                        *cursor = ready.into();
                    }
                    Some("typing.updated") => {
                        let author: Author = serde_json::from_value(event["author"].clone())
                            .map_err(|_| Failure::Retry("Invalid typing author.".into()))?;
                        let revision = event["revision"]
                            .as_str()
                            .ok_or_else(|| Failure::Retry("Invalid typing revision.".into()))?;
                        sequence(revision).map_err(Failure::Retry)?;
                        let _ = events.send(GatewayEvent::Typing {
                            generation,
                            channel: channel.into(),
                            author,
                            typing: event["typing"].as_bool().unwrap_or(false),
                            revision: revision.into(),
                        });
                    }
                    _ => {
                        return Err(Failure::Retry(
                            "The gateway returned an invalid event.".into(),
                        ));
                    }
                }
            }
            Some("event")
                if presence_subscription
                    .as_ref()
                    .is_some_and(|id| value["id"] == *id) =>
            {
                let event = &value["event"];
                if event["type"] == "snapshot"
                    && let Some((space, _)) = presence
                {
                    let members: Vec<Presence> =
                        serde_json::from_value(event["members"].clone())
                            .map_err(|_| Failure::Retry("Invalid member presence.".into()))?;
                    let _ = events.send(GatewayEvent::Presence {
                        generation,
                        space: space.clone(),
                        members,
                    });
                }
            }
            Some("event")
                if value["id"]
                    .as_str()
                    .is_some_and(|id| media_subscriptions.contains_key(id)) =>
            {
                let (channel, previous) = media_subscriptions
                    .get_mut(value["id"].as_str().unwrap())
                    .unwrap();
                let event = &value["event"];
                let revision = event["revision"]
                    .as_u64()
                    .filter(|_| event["type"] == "snapshot")
                    .ok_or_else(|| Failure::Retry("Invalid voice roster.".into()))?;
                if previous.is_some_and(|old| revision <= old) {
                    continue;
                }
                let participants = serde_json::from_value(event["participants"].clone())
                    .map_err(|_| Failure::Retry("Invalid voice roster.".into()))?;
                *previous = Some(revision);
                let _ = events.send(GatewayEvent::VoiceRoster {
                    generation: media.epoch,
                    channel: channel.id.clone(),
                    participants,
                });
            }
            Some("error")
                if value["id"]
                    .as_str()
                    .is_some_and(|id| media_subscriptions.contains_key(id)) =>
            {
                // Remove this logical subscription before accepting another event;
                // a denied spectator channel must not poison the active chat.
                let (channel, _) = media_subscriptions
                    .remove(value["id"].as_str().unwrap())
                    .unwrap();
                let _ = events.send(GatewayEvent::VoiceUnavailable {
                    generation: media.epoch,
                    channel: channel.id.clone(),
                    revoked: value["status"]
                        .as_u64()
                        .is_some_and(|status| matches!(status, 401 | 403 | 404)),
                });
            }
            Some("error") if value["id"] == subscription => {
                if value["status"]
                    .as_u64()
                    .is_some_and(|status| matches!(status, 401 | 403 | 404))
                {
                    return Err(Failure::Denied(
                        value["error"]
                            .as_str()
                            .unwrap_or("Channel access denied.")
                            .into(),
                    ));
                }
                let _ = events.send(GatewayEvent::Resync {
                    generation,
                    channel: channel.into(),
                });
                return Err(Failure::Retry(
                    value["error"]
                        .as_str()
                        .unwrap_or("Live subscription failed.")
                        .into(),
                ));
            }
            Some("migrating") => return Ok(()),
            _ => {}
        }
    }
}

fn set_timeout(
    stream: &mut MaybeTlsStream<std::net::TcpStream>,
    timeout: Duration,
) -> std::io::Result<()> {
    match stream {
        MaybeTlsStream::Plain(stream) => stream.set_read_timeout(Some(timeout)),
        MaybeTlsStream::Rustls(stream) => stream.get_mut().set_read_timeout(Some(timeout)),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};

    #[test]
    fn local_gateway_uses_auth_header_resumes_cursor_does_not_echo_and_stops() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket =
                tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
                    assert_eq!(request.uri().path(), "/api/chat/events");
                    assert!(
                        request.uri().query().is_none(),
                        "credentials and cursor stay out of URL"
                    );
                    assert_eq!(request.headers()["authorization"], "Bearer account-secret");
                    assert!(request.headers().get("x-caper-chat-token").is_none());
                    Ok(response)
                })
                .unwrap();
            socket
                .send(WsMessage::Text(
                    json!({"type":"hello","idleTimeoutSeconds":600,"serverTime":0})
                        .to_string()
                        .into(),
                ))
                .unwrap();
            let WsMessage::Text(subscribe) = socket.read().unwrap() else {
                panic!("expected subscription")
            };
            let subscribe: Value = serde_json::from_str(&subscribe).unwrap();
            assert_eq!(subscribe["after"], "37");
            socket
                .send(WsMessage::Text(
                    json!({"type":"heartbeat"}).to_string().into(),
                ))
                .unwrap();
            socket
                .get_mut()
                .set_read_timeout(Some(Duration::from_millis(120)))
                .unwrap();
            assert!(
                matches!(socket.read(), Err(tungstenite::Error::Io(error)) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)),
                "server heartbeat receipt must not trigger an immediate client heartbeat"
            );
            socket
                .get_mut()
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            assert!(
                matches!(socket.read(), Ok(WsMessage::Close(_))),
                "stop must promptly close the socket"
            );
        });
        let (events, _) = mpsc::channel();
        let base = url::Url::parse(&format!("http://{address}/")).unwrap();
        let control = spawn(
            &base,
            Some("account-secret".into()),
            4,
            "channel-id".into(),
            "37".into(),
            None,
            MediaWatch::default(),
            events,
        );
        thread::sleep(Duration::from_millis(250));
        control.stop();
        server.join().unwrap();
    }

    #[test]
    fn spectator_channels_share_one_authenticated_socket_and_reject_stale_or_revoked_events() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (release, released) = mpsc::channel();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut socket =
                tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
                    assert_eq!(
                        request.headers()["authorization"],
                        "Bearer spectator-account"
                    );
                    assert!(request.uri().query().is_none());
                    Ok(response)
                })
                .unwrap();
            socket
                .send(WsMessage::Text(json!({"type":"hello"}).to_string().into()))
                .unwrap();
            let mut subscriptions = std::collections::BTreeMap::new();
            let mut chat = String::new();
            for _ in 0..25 {
                let frame: Value =
                    serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
                assert_eq!(frame["type"], "subscribe");
                assert!(
                    frame.get("token").is_none(),
                    "Spectators must not carry participant capabilities"
                );
                if frame["kind"] == "chat" {
                    chat = frame["id"].as_str().unwrap().to_owned();
                } else {
                    assert_eq!(frame["kind"], "media");
                    subscriptions.insert(
                        frame["channelId"].as_str().unwrap_or("demo").to_owned(),
                        frame["id"].clone(),
                    );
                }
            }
            assert_eq!(subscriptions.len(), 24);
            assert!(subscriptions.contains_key("demo"));
            assert!(!subscriptions.contains_key("channel24"));
            let snapshot = |id: &Value, revision, name| {
                json!({"type":"event","id":id,"event":{
                    "type":"snapshot", "revision":revision, "participants":[{"id":"speaker", "name":name, "muted":false, "deafened":true}]
                }})
            };
            for frame in [
                snapshot(&subscriptions["channel1"], 7, "current"),
                snapshot(&subscriptions["channel1"], 6, "old"),
                json!({"type":"error", "id":subscriptions["channel1"], "status":403}),
                snapshot(&subscriptions["channel1"], 8, "revoked"),
                snapshot(&subscriptions["demo"], 0, "public"),
                json!({"type":"subscribed", "id":chat}),
            ] {
                socket
                    .send(WsMessage::Text(frame.to_string().into()))
                    .unwrap();
            }
            released.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(
                matches!(socket.read(), Ok(WsMessage::Close(_))),
                "No extra subscriptions past the cap"
            );
        });
        let (events, incoming) = mpsc::channel();
        let control = spawn(
            &url::Url::parse(&format!("http://{address}")).unwrap(),
            Some("spectator-account".into()),
            4,
            "text-channel".into(),
            "0".into(),
            None,
            MediaWatch {
                epoch: 91,
                channels: (0..26)
                    .map(|index| MediaChannel {
                        id: format!("channel{index}"),
                        demo: index == 0,
                    })
                    .collect(),
            },
            events,
        );
        let mut rosters = vec![];
        let mut denied = vec![];
        loop {
            match incoming.recv_timeout(Duration::from_secs(5)).unwrap() {
                GatewayEvent::VoiceRoster {
                    generation,
                    channel,
                    participants,
                } => {
                    assert_eq!(generation, 91);
                    rosters.push((channel, participants[0].name.clone()));
                }
                GatewayEvent::VoiceUnavailable {
                    generation,
                    channel,
                    revoked,
                } => {
                    assert_eq!(generation, 91);
                    assert!(revoked);
                    denied.push(channel);
                }
                GatewayEvent::Status { online: true, .. } => break,
                _ => {}
            }
        }
        assert_eq!(
            rosters,
            vec![
                ("channel1".into(), "current".into()),
                ("channel0".into(), "public".into())
            ]
        );
        assert_eq!(denied, vec!["channel1"]);
        control.stop();
        release.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn denied_upgrade_is_terminal() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _: Result<_, _> = tungstenite::accept_hdr(stream, |_: &Request, _: Response| {
                let mut denied = ErrorResponse::new(Some("expired".into()));
                *denied.status_mut() = tungstenite::http::StatusCode::UNAUTHORIZED;
                Err(denied)
            });
        });
        let (events, incoming) = mpsc::channel();
        let base = url::Url::parse(&format!("http://{address}/")).unwrap();
        let _control = spawn(
            &base,
            Some("revoked".into()),
            9,
            "private".into(),
            "0".into(),
            None,
            MediaWatch::default(),
            events,
        );
        let denied =
            (0..3).find_map(
                |_| match incoming.recv_timeout(Duration::from_secs(1)).ok()? {
                    GatewayEvent::AccessDenied {
                        generation,
                        channel,
                        ..
                    } => Some((generation, channel)),
                    _ => None,
                },
            );
        assert_eq!(denied, Some((9, "private".into())));
        server.join().unwrap();
    }
}
