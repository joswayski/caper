use crate::media_gateway::{CATCHUP_DEADLINE, Connection, ConnectionAttempt, Failure};
use crate::model::{
    AttachmentUpdate, Author, Message, Presence, ReactionUpdate, VoiceOccupant, sequence,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};
use std::thread;
use std::time::{Duration, Instant};

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
    Reactions {
        generation: u64,
        channel: String,
        update: ReactionUpdate,
    },
    Attachments {
        generation: u64,
        channel: String,
        update: AttachmentUpdate,
    },
    /// Ephemeral media-worker progress for a processing attachment.
    AttachmentProgress {
        generation: u64,
        channel: String,
        attachment: String,
        percent: u8,
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
        session_started_at: Option<u64>,
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

struct ChatStream {
    connection: Connection,
    subscribed: BTreeSet<String>,
    cursor: String,
    ready: bool,
    media_revisions: BTreeMap<String, u64>,
    presence: Option<Vec<Presence>>,
}

impl ChatStream {
    fn new(connection: Connection, cursor: &str) -> Self {
        Self {
            connection,
            subscribed: BTreeSet::new(),
            cursor: cursor.into(),
            ready: false,
            media_revisions: BTreeMap::new(),
            presence: None,
        }
    }
}

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
    stop: &Arc<AtomicBool>,
    activity: &Mutex<Instant>,
) -> Result<(), Failure> {
    let mut active = ChatStream::new(Connection::open(url, token.as_deref(), stop)?, cursor);
    let mut candidate: Option<ChatStream> = None;
    let mut opening: Option<ConnectionAttempt> = None;
    let mut migrating = false;
    let mut retry_at = Instant::now();
    let subscription = uuid::Uuid::new_v4().to_string();
    let presence_subscription = presence.map(|_| uuid::Uuid::new_v4().to_string());
    let mut media_subscriptions: BTreeMap<_, _> = media
        .channels
        .iter()
        .take(MAX_MEDIA_CHANNELS)
        .map(|channel| (uuid::Uuid::new_v4().to_string(), (channel, None::<u64>)))
        .collect();
    loop {
        if stop.load(Ordering::Relaxed) {
            active.connection.close();
            if let Some(candidate) = &mut candidate {
                candidate.connection.close();
            }
            return Ok(());
        }
        if migrating && candidate.is_none() && opening.is_none() && Instant::now() >= retry_at {
            opening = Some(ConnectionAttempt::start(url, token.as_deref()));
        }
        if let Some(result) = opening.as_ref().and_then(ConnectionAttempt::take) {
            opening = None;
            match result {
                Ok(connection) => candidate = Some(ChatStream::new(connection, cursor)),
                Err(_) => retry_at = Instant::now() + Duration::from_millis(250),
            }
        }
        for replacement in [false, true] {
            let stream = if replacement {
                let Some(stream) = &mut candidate else {
                    continue;
                };
                stream
            } else {
                &mut active
            };
            let result = (|| {
                if replacement && stream.connection.opened.elapsed() >= CATCHUP_DEADLINE {
                    return Err(Failure::Retry("replacement did not catch up".into()));
                }
                let Some(value) = stream.connection.read(activity)? else {
                    return Ok(false);
                };
                receive_frame(
                    stream,
                    replacement,
                    value,
                    &subscription,
                    presence_subscription.as_deref(),
                    generation,
                    channel,
                    cursor,
                    presence,
                    media.epoch,
                    &mut media_subscriptions,
                    events,
                )
            })();
            match result {
                Ok(true) => migrating = true,
                Ok(false) => {}
                Err(_) if replacement => {
                    if let Some(mut failed) = candidate.take() {
                        failed.connection.close();
                    }
                    retry_at = Instant::now() + Duration::from_millis(250);
                }
                Err(error) => return Err(error),
            }
        }
        if candidate.as_ref().is_some_and(|stream| {
            stream.ready
                && stream.cursor == *cursor
                && stream.subscribed.contains(&subscription)
                && presence_subscription
                    .as_ref()
                    .is_none_or(|id| stream.subscribed.contains(id) && stream.presence.is_some())
                && media_subscriptions.iter().all(|(id, (_, revision))| {
                    stream.subscribed.contains(id)
                        && stream
                            .media_revisions
                            .get(id)
                            .is_some_and(|seen| revision.is_none_or(|applied| *seen >= applied))
                })
        }) {
            let mut old = std::mem::replace(&mut active, candidate.take().unwrap());
            if let (Some((space, _)), Some(members)) = (presence, active.presence.take()) {
                let _ = events.send(GatewayEvent::Presence {
                    generation,
                    space: space.clone(),
                    members,
                });
            }
            old.connection.close();
            migrating = false;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn receive_frame(
    stream: &mut ChatStream,
    replacement: bool,
    value: Value,
    subscription: &str,
    presence_subscription: Option<&str>,
    generation: u64,
    channel: &str,
    cursor: &mut String,
    presence: Option<&(String, Vec<String>)>,
    media_epoch: u64,
    media_subscriptions: &mut BTreeMap<String, (&MediaChannel, Option<u64>)>,
    events: &Sender<GatewayEvent>,
) -> Result<bool, Failure> {
    match value["type"].as_str() {
        Some("hello") if !stream.connection.hello => {
            stream.connection.hello = true;
            stream.cursor = cursor.clone();
            stream.connection.send(json!({"type":"subscribe", "id":subscription, "kind":"chat", "channelId":channel, "after":cursor}))?;
            if let (Some((space, users)), Some(id)) = (presence, presence_subscription) {
                stream.connection.send(json!({"type":"subscribe", "id":id, "kind":"presence", "spaceId":space, "userIds":users}))?;
            }
            for (id, (channel, _)) in media_subscriptions.iter() {
                let mut frame = json!({"type":"subscribe", "id":id, "kind":"media"});
                if !channel.demo {
                    frame["channelId"] = json!(channel.id);
                }
                stream.connection.send(frame)?;
            }
        }
        Some("heartbeat") => {}
        Some("subscribed") => {
            if let Some(id) = value["id"].as_str() {
                stream.subscribed.insert(id.into());
            }
            if !replacement && value["id"] == subscription {
                let _ = events.send(GatewayEvent::Status {
                    generation,
                    channel: channel.into(),
                    online: true,
                    detail: "Live".into(),
                });
            }
        }
        Some("event") if value["id"] == subscription => {
            let event = &value["event"];
            match event["type"].as_str() {
                Some("message.created") => {
                    let message: Message = serde_json::from_value(event["message"].clone())
                        .map_err(|_| {
                            Failure::Retry("The gateway returned an invalid message.".into())
                        })?;
                    if message.channel_id != channel || event["seq"].as_str() != Some(&message.seq)
                    {
                        return Err(Failure::Retry(
                            "The gateway mixed channel data; resyncing.".into(),
                        ));
                    }
                    let previous = sequence(cursor).map_err(Failure::Retry)?;
                    let next = sequence(&message.seq).map_err(Failure::Retry)?;
                    let position = sequence(&stream.cursor).map_err(Failure::Retry)?;
                    if next > position.saturating_add(1) || next > previous.saturating_add(1) {
                        if !replacement {
                            let _ = events.send(GatewayEvent::Resync {
                                generation,
                                channel: channel.into(),
                            });
                        }
                        return Err(Failure::Retry("Non-contiguous gateway replay.".into()));
                    }
                    if next > position {
                        stream.cursor = message.seq.clone();
                    }
                    if next <= previous {
                        return Ok(false);
                    }
                    *cursor = message.seq.clone();
                    let _ = events.send(GatewayEvent::Message {
                        generation,
                        channel: channel.into(),
                        message: Box::new(message),
                    });
                }
                Some("message.reactions") => {
                    let update: ReactionUpdate =
                        serde_json::from_value(event.clone()).map_err(|_| {
                            Failure::Retry("The gateway returned invalid reactions.".into())
                        })?;
                    if update.channel_id != channel
                        || update.kind != "message.reactions"
                        || update.schema_version != 1
                        || update.message_id.is_empty()
                        || update.reactions.iter().any(|reaction| {
                            reaction.emoji.is_empty()
                                || reaction.author_ids.iter().any(String::is_empty)
                        })
                    {
                        return Err(Failure::Retry(
                            "The gateway returned invalid reactions.".into(),
                        ));
                    }
                    let previous = sequence(cursor).map_err(Failure::Retry)?;
                    let next = sequence(&update.seq).map_err(Failure::Retry)?;
                    let position = sequence(&stream.cursor).map_err(Failure::Retry)?;
                    if next > position.saturating_add(1) || next > previous.saturating_add(1) {
                        if !replacement {
                            let _ = events.send(GatewayEvent::Resync {
                                generation,
                                channel: channel.into(),
                            });
                        }
                        return Err(Failure::Retry("Non-contiguous gateway replay.".into()));
                    }
                    if next > position {
                        stream.cursor = update.seq.clone();
                    }
                    if next <= previous {
                        return Ok(false);
                    }
                    *cursor = update.seq.clone();
                    let _ = events.send(GatewayEvent::Reactions {
                        generation,
                        channel: channel.into(),
                        update,
                    });
                }
                Some("message.attachments") => {
                    let update: AttachmentUpdate =
                        serde_json::from_value(event.clone()).map_err(|_| {
                            Failure::Retry("The gateway returned invalid attachments.".into())
                        })?;
                    if update.channel_id != channel
                        || update.kind != "message.attachments"
                        || update.schema_version != 1
                        || update.message_id.is_empty()
                    {
                        return Err(Failure::Retry(
                            "The gateway returned invalid attachments.".into(),
                        ));
                    }
                    let previous = sequence(cursor).map_err(Failure::Retry)?;
                    let next = sequence(&update.seq).map_err(Failure::Retry)?;
                    let position = sequence(&stream.cursor).map_err(Failure::Retry)?;
                    if next > position.saturating_add(1) || next > previous.saturating_add(1) {
                        if !replacement {
                            let _ = events.send(GatewayEvent::Resync {
                                generation,
                                channel: channel.into(),
                            });
                        }
                        return Err(Failure::Retry("Non-contiguous gateway replay.".into()));
                    }
                    if next > position {
                        stream.cursor = update.seq.clone();
                    }
                    if next <= previous {
                        return Ok(false);
                    }
                    *cursor = update.seq.clone();
                    let _ = events.send(GatewayEvent::Attachments {
                        generation,
                        channel: channel.into(),
                        update,
                    });
                }
                // Ephemeral like typing: a malformed frame is dropped, never
                // a reason to reconnect.
                Some("attachment.progress") => {
                    if replacement || event["channelId"].as_str() != Some(channel) {
                        return Ok(false);
                    }
                    let attachment = event["attachmentId"].as_str().filter(|id| !id.is_empty());
                    let percent = event["percent"]
                        .as_f64()
                        .filter(|percent| percent.is_finite());
                    if let (Some(attachment), Some(percent)) = (attachment, percent) {
                        let _ = events.send(GatewayEvent::AttachmentProgress {
                            generation,
                            channel: channel.into(),
                            attachment: attachment.into(),
                            percent: percent.clamp(0.0, 100.0).round() as u8,
                        });
                    }
                }
                Some("resync_required") => {
                    if !replacement {
                        let _ = events.send(GatewayEvent::Resync {
                            generation,
                            channel: channel.into(),
                        });
                    }
                    return Err(Failure::Retry("Gateway replay requires resync.".into()));
                }
                Some("ready") => {
                    let ready = event["cursor"]
                        .as_str()
                        .ok_or_else(|| Failure::Retry("Invalid gateway cursor".into()))?;
                    sequence(ready).map_err(Failure::Retry)?;
                    if ready != stream.cursor {
                        return Err(Failure::Retry(
                            "The gateway checkpoint did not match replay.".into(),
                        ));
                    }
                    stream.ready = true;
                }
                Some("typing.updated") => {
                    if replacement {
                        return Ok(false);
                    }
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
                let members: Vec<Presence> = serde_json::from_value(event["members"].clone())
                    .map_err(|_| Failure::Retry("Invalid member presence.".into()))?;
                if replacement {
                    stream.presence = Some(members);
                } else {
                    let _ = events.send(GatewayEvent::Presence {
                        generation,
                        space: space.clone(),
                        members,
                    });
                }
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
            stream
                .media_revisions
                .entry(value["id"].as_str().unwrap().into())
                .and_modify(|position| *position = (*position).max(revision))
                .or_insert(revision);
            if previous.is_some_and(|old| revision <= old) {
                return Ok(false);
            }
            let participants = serde_json::from_value(event["participants"].clone())
                .map_err(|_| Failure::Retry("Invalid voice roster.".into()))?;
            *previous = Some(revision);
            let _ = events.send(GatewayEvent::VoiceRoster {
                generation: media_epoch,
                channel: channel.id.clone(),
                participants,
                session_started_at: event["sessionStartedAt"].as_u64(),
            });
        }
        Some("error")
            if value["id"]
                .as_str()
                .is_some_and(|id| media_subscriptions.contains_key(id)) =>
        {
            if replacement {
                return Err(Failure::Retry("Replacement subscription failed.".into()));
            }
            // Remove this logical subscription before accepting another event;
            // a denied spectator channel must not poison the active chat.
            let (channel, _) = media_subscriptions
                .remove(value["id"].as_str().unwrap())
                .unwrap();
            let _ = events.send(GatewayEvent::VoiceUnavailable {
                generation: media_epoch,
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
            if !replacement {
                let _ = events.send(GatewayEvent::Resync {
                    generation,
                    channel: channel.into(),
                });
            }
            return Err(Failure::Retry(
                value["error"]
                    .as_str()
                    .unwrap_or("Live subscription failed.")
                    .into(),
            ));
        }
        Some("migrating") => {
            if replacement {
                return Err(Failure::Retry("Replacement is draining.".into()));
            }
            return Ok(true);
        }
        _ => {}
    }
    Ok(false)
}

#[cfg(test)]
#[allow(clippy::result_large_err)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use tungstenite::Message as WsMessage;
    use tungstenite::handshake::server::{ErrorResponse, Request, Response};

    #[test]
    fn mixed_handoffs_preserve_delivery_after_failed_and_stale_candidates() {
        type Socket = tungstenite::WebSocket<std::net::TcpStream>;
        fn send(socket: &mut Socket, frame: Value) {
            socket
                .send(WsMessage::Text(frame.to_string().into()))
                .unwrap();
        }
        fn accept(listener: &TcpListener, after: &str) -> (Socket, BTreeMap<String, String>) {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut socket =
                tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
                    assert_eq!(request.headers()["authorization"], "Bearer account-secret");
                    assert!(request.uri().query().is_none());
                    Ok(response)
                })
                .unwrap();
            send(&mut socket, json!({"type":"hello"}));
            let mut ids = BTreeMap::new();
            for _ in 0..3 {
                let frame: Value =
                    serde_json::from_str(&socket.read().unwrap().into_text().unwrap()).unwrap();
                assert!(frame.get("token").is_none());
                if frame["kind"] == "chat" {
                    assert_eq!(frame["after"], after);
                }
                let kind = frame["kind"].as_str().unwrap().to_owned();
                let id = frame["id"].as_str().unwrap().to_owned();
                send(&mut socket, json!({"type":"subscribed", "id":id}));
                ids.insert(kind, id);
            }
            (socket, ids)
        }
        fn ready(socket: &mut Socket, id: &str, cursor: &str) {
            send(
                socket,
                json!({"type":"event", "id":id, "event":{"type":"ready", "cursor":cursor}}),
            );
        }
        fn message(socket: &mut Socket, id: &str, seq: &str) {
            send(
                socket,
                json!({"type":"event", "id":id, "event":{
                    "type":"message.created", "seq":seq, "message":{
                        "id":format!("message-{seq}"), "channelId":"text", "seq":seq,
                        "clientMessageId":format!("client-{seq}"), "createdAt":"2026-10-03T00:00:00Z",
                        "author":{"id":"author", "name":"Author", "isGuest":false},
                        "content":{"version":1,"type":"text","text":seq}
                    }
                }}),
            );
        }
        fn reactions(socket: &mut Socket, id: &str, seq: &str) {
            send(
                socket,
                json!({"type":"event", "id":id, "event":{
                    "type":"message.reactions", "schemaVersion":1, "channelId":"text",
                    "seq":seq, "messageId":"message-38",
                    "reactions":[{"emoji":"👍", "authorIds":["author"]}]
                }}),
            );
        }
        fn attachments(socket: &mut Socket, id: &str, seq: &str) {
            send(
                socket,
                json!({"type":"event", "id":id, "event":{
                    "type":"message.attachments", "schemaVersion":1, "channelId":"text",
                    "seq":seq, "messageId":"message-41",
                    "attachments":[{"id":"asset", "kind":"video", "contentType":"video/mp4",
                        "name":"clip.mp4", "size":9, "status":"ready", "animated":true}, "junk"]
                }}),
            );
        }
        fn progress(socket: &mut Socket, id: &str, percent: Value) {
            send(
                socket,
                json!({"type":"event", "id":id, "event":{
                    "type":"attachment.progress", "channelId":"text", "messageId":"message-41",
                    "attachmentId":"asset", "percent":percent
                }}),
            );
        }
        fn roster(socket: &mut Socket, id: &str, revision: u64) {
            send(
                socket,
                json!({"type":"event", "id":id, "event":{
                    "type":"snapshot", "revision":revision,
                    "participants":[{"id":"speaker", "name":format!("revision-{revision}"), "muted":false, "deafened":false}]
                }}),
            );
        }
        fn presence(socket: &mut Socket, id: &str) {
            send(
                socket,
                json!({"type":"event", "id":id, "event":{"type":"snapshot", "members":[{"userId":"author", "status":"online"}]}}),
            );
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (advance, advanced) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut old, ids) = accept(&listener, "37");
            ready(&mut old, &ids["chat"], "37");
            roster(&mut old, &ids["media"], 7);
            presence(&mut old, &ids["presence"]);
            advanced.recv_timeout(Duration::from_secs(5)).unwrap();
            send(&mut old, json!({"type":"migrating"}));
            let (stream, _) = listener.accept().unwrap();
            let _: Result<_, _> = tungstenite::accept_hdr(stream, |_: &Request, _: Response| {
                let mut response = ErrorResponse::new(None);
                *response.status_mut() = tungstenite::http::StatusCode::SERVICE_UNAVAILABLE;
                Err(response)
            });
            message(&mut old, &ids["chat"], "38");
            advanced.recv_timeout(Duration::from_secs(5)).unwrap();
            let (mut replacement, next_ids) = accept(&listener, "38");
            assert_eq!(ids, next_ids, "logical subscriptions survive the handoff");
            ready(&mut replacement, &ids["chat"], "38");
            roster(&mut replacement, &ids["media"], 6);
            presence(&mut replacement, &ids["presence"]);
            reactions(&mut old, &ids["chat"], "39");
            roster(&mut old, &ids["media"], 8);
            advanced.recv_timeout(Duration::from_secs(5)).unwrap();
            reactions(&mut replacement, &ids["chat"], "39");
            roster(&mut replacement, &ids["media"], 8);
            assert!(
                matches!(old.read(), Ok(WsMessage::Close(_))),
                "old closes only after replacement catches up"
            );
            message(&mut replacement, &ids["chat"], "40");
            advanced.recv_timeout(Duration::from_secs(5)).unwrap();
            send(&mut replacement, json!({"type":"migrating"}));
            let (mut last, last_ids) = accept(&listener, "40");
            assert_eq!(ids, last_ids);
            ready(&mut last, &ids["chat"], "40");
            roster(&mut last, &ids["media"], 8);
            presence(&mut last, &ids["presence"]);
            assert!(matches!(replacement.read(), Ok(WsMessage::Close(_))));
            message(&mut last, &ids["chat"], "41");
            progress(&mut last, &ids["chat"], json!("not a number"));
            progress(&mut last, &ids["chat"], json!(140.4));
            attachments(&mut last, &ids["chat"], "42");
            assert!(
                matches!(last.read(), Ok(WsMessage::Close(_))),
                "stop closes promoted socket"
            );
        });
        let (events, incoming) = mpsc::channel();
        let control = spawn(
            &url::Url::parse(&format!("http://{address}")).unwrap(),
            Some("account-secret".into()),
            4,
            "text".into(),
            "37".into(),
            Some(("space".into(), vec!["author".into()])),
            MediaWatch {
                epoch: 91,
                channels: vec![MediaChannel {
                    id: "voice".into(),
                    demo: false,
                }],
            },
            events,
        );
        // Ignore the initial Connecting status, then require uninterrupted live state.
        while !matches!(
            incoming.recv_timeout(Duration::from_secs(5)).unwrap(),
            GatewayEvent::Status { online: true, .. }
        ) {}
        advance.send(()).unwrap();
        let mut messages = vec![];
        let mut reactions = vec![];
        let mut rosters = vec![];
        let mut percents = vec![];
        loop {
            match incoming.recv_timeout(Duration::from_secs(5)).unwrap() {
                GatewayEvent::Message { message, .. } => {
                    let seq = message.seq.clone();
                    messages.push(seq.clone());
                    if seq == "38" || seq == "40" {
                        advance.send(()).unwrap();
                    }
                }
                GatewayEvent::VoiceRoster { participants, .. } => {
                    let name = participants[0].name.clone();
                    if name == "revision-8" {
                        advance.send(()).unwrap();
                    }
                    rosters.push(name);
                }
                GatewayEvent::Reactions { update, .. } => reactions.push(update.seq),
                GatewayEvent::AttachmentProgress {
                    attachment,
                    percent,
                    ..
                } => percents.push((attachment, percent)),
                GatewayEvent::Attachments { update, .. } => {
                    assert_eq!(update.seq, "42");
                    assert_eq!(update.attachments.len(), 1, "malformed entries are skipped");
                    assert!(update.attachments[0].animated);
                    break;
                }
                GatewayEvent::Presence { .. } | GatewayEvent::Status { online: true, .. } => {}
                other => panic!("handoff must not disconnect/reset/resync: {other:?}"),
            }
        }
        assert_eq!(messages, ["38", "40", "41"]);
        assert_eq!(reactions, ["39"], "candidate replay is deduplicated");
        assert_eq!(rosters, ["revision-7", "revision-8"]);
        assert_eq!(
            percents,
            [("asset".to_owned(), 100)],
            "malformed progress is dropped without reconnecting"
        );
        control.stop();
        server.join().unwrap();
    }

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
                    session_started_at: _,
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
