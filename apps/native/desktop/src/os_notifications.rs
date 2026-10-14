//! Account notifications have their own lifetime, independent of channel navigation.
use crate::media_gateway::{Connection, Failure};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Alert {
    /// `mention.user`, `feedback`, `space.invitation`, … Older servers
    /// may omit it.
    pub kind: Option<String>,
    /// Absent for invitations and message requests.
    #[serde(default)]
    pub message_id: Option<String>,
    #[cfg(target_os = "windows")]
    pub notification_id: Option<String>,
    /// Present when the message is a thread reply (every kind).
    #[serde(default)]
    pub thread_root_id: Option<String>,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub sender_avatar_id: Option<i32>,
    pub attachment_url: Option<String>,
    pub attachment_content_type: Option<String>,
    pub attachment_size: Option<u64>,
    pub conversation_id: Option<String>,
    pub space_id: Option<String>,
    pub channel_id: Option<String>,
}

impl Alert {
    pub fn invitation(&self) -> bool {
        matches!(
            self.kind.as_deref(),
            Some("space.invitation" | "channel.invitation")
        )
    }
    pub fn conversation(&self) -> Option<&str> {
        if self.invitation() || self.kind.as_deref() == Some("direct.request") {
            return None;
        }
        self.conversation_id
            .as_deref()
            .or(self.channel_id.as_deref())
    }
    /// Where a click goes: `feedback` first, opening the conversation as
    /// the Caper team at the report, in its thread; then invitations; then a
    /// feedback team member's notification from a space or DM missing from
    /// their lists (`known` false), visited in the reply's thread if it was
    /// one; then the conversation as before.
    pub fn route(&self, feedback_team: bool, known: bool) -> Option<Route> {
        let visit = |thread: Option<String>| {
            let message = self.message_id.clone()?;
            Some(Route::Visit(crate::feedback::VisitTarget {
                channel: self
                    .conversation_id
                    .clone()
                    .or_else(|| self.channel_id.clone())?,
                thread,
                message,
                reporter: None,
            }))
        };
        if self.kind.as_deref() == Some("feedback") {
            return visit(
                self.thread_root_id
                    .clone()
                    .or_else(|| self.message_id.clone()),
            );
        }
        if self.invitation() {
            return Some(Route::Invitation {
                space: self.space_id.clone()?,
                channel: self.channel_id.clone(),
            });
        }
        if feedback_team
            && !known
            && self.kind.as_deref() != Some("direct.request")
            && let Some(route) = visit(self.thread_root_id.clone())
        {
            return Some(route);
        }
        if let Some(id) = &self.conversation_id {
            return Some(Route::Direct(id.clone()));
        }
        Some(Route::Channel {
            space: self.space_id.clone()?,
            channel: self.channel_id.clone()?,
        })
    }

    pub fn fresh(&self) -> bool {
        DateTime::parse_from_rfc3339(&self.created_at).is_ok_and(|date| {
            (-30..=120).contains(&(Utc::now() - date.with_timezone(&Utc)).num_seconds())
        })
    }
}

/// What clicking a notification opens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    Direct(String),
    Channel {
        space: String,
        channel: String,
    },
    /// A space (or private channel) invitation to answer.
    Invitation {
        space: String,
        channel: Option<String>,
    },
    Visit(crate::feedback::VisitTarget),
}

pub struct Control {
    stop: Arc<AtomicBool>,
    activity: Arc<Mutex<Instant>>,
}
impl Control {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
    pub fn activity(&self) {
        if let Ok(mut time) = self.activity.lock() {
            *time = Instant::now();
        }
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The account feed: notifications, and the `profiles` subscription's live
/// profile and space changes (`profiles`), on one socket for the sign-in.
pub fn watch(
    base: &url::Url,
    token: String,
    receive: impl Fn(Alert, Arc<AtomicBool>) + Send + 'static,
    profiles: impl Fn(crate::gateway::ProfileEvent) + Send + 'static,
) -> Control {
    let stop = Arc::new(AtomicBool::new(false));
    let activity = Arc::new(Mutex::new(Instant::now()));
    let control = Control {
        stop: stop.clone(),
        activity: activity.clone(),
    };
    let mut url = base.join("api/chat/events").expect("constant gateway path");
    url.set_scheme(if base.scheme() == "https" {
        "wss"
    } else {
        "ws"
    })
    .expect("WebSocket scheme");
    std::thread::spawn(move || {
        let mut cursor: Option<u64> = None;
        while !stop.load(Ordering::Relaxed) {
            let result = (|| {
                let mut connection = Connection::open(&url, Some(&token), &stop)?;
                while !stop.load(Ordering::Relaxed) {
                    let Some(frame) = connection.read(&activity)? else {
                        continue;
                    };
                    match frame["type"].as_str() {
                        Some("hello") => {
                            connection.hello = true;
                            let mut subscription = json!({"type":"subscribe","id":"notifications","kind":"notifications"});
                            if let Some(cursor) = cursor {
                                subscription["after"] = json!(cursor.to_string());
                            }
                            connection.send(subscription)?;
                            // Again on every socket: nothing is replayed.
                            connection.send(crate::gateway::profiles_subscription())?;
                        }
                        Some("event") if frame["id"] == crate::gateway::PROFILES => {
                            if let Some(event) = crate::gateway::profiles_event(&frame) {
                                profiles(event);
                            }
                        }
                        // An older gateway without `profiles`: live updates are
                        // unavailable on this socket; notifications carry on.
                        Some("error") if frame["id"] == crate::gateway::PROFILES => {}
                        Some("event") if frame["id"] == "notifications" => {
                            let event = &frame["event"];
                            let ready = event["type"] == "ready";
                            let next = event[if ready { "cursor" } else { "seq" }]
                                .as_str()
                                .and_then(|seq| seq.parse::<u64>().ok())
                                .ok_or_else(|| {
                                    Failure::Retry("invalid notification cursor".into())
                                })?;
                            if !ready
                                && cursor.is_none_or(|old| next > old)
                                && event["type"] == "notification.created"
                            {
                                let alert = serde_json::from_value(event.clone())
                                    .map_err(|_| Failure::Retry("invalid notification".into()))?;
                                receive(alert, stop.clone());
                            }
                            cursor = Some(cursor.unwrap_or(0).max(next));
                        }
                        Some("migrating") => {
                            connection.close();
                            break;
                        }
                        Some("error") if frame["id"] == "notifications" => {
                            return Err(if matches!(frame["status"].as_u64(), Some(401 | 403)) {
                                Failure::Denied("session ended".into())
                            } else {
                                Failure::Retry("notifications unavailable".into())
                            });
                        }
                        _ => {}
                    }
                }
                Ok::<(), Failure>(())
            })();
            if matches!(result, Err(Failure::Denied(_))) {
                break;
            }
            for _ in 0..5 {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
    control
}

fn avatar(index: Option<i32>, directory: &std::path::Path) -> Option<std::path::PathBuf> {
    let index = crate::caper_avatar_index(index)? as usize;
    let tree = resvg::usvg::Tree::from_data(
        crate::avatar_images::SVG[index],
        &resvg::usvg::Options::default(),
    )
    .ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(128, 128)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(
            128.0 / tree.size().width(),
            128.0 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );
    let path = directory.join("sender.png");
    pixmap.save_png(&path).ok()?;
    Some(path)
}

fn attachment(alert: &Alert, directory: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::io::{Cursor, Read};
    let url = url::Url::parse(alert.attachment_url.as_deref()?).ok()?;
    let size = alert.attachment_size?;
    let content_type = alert.attachment_content_type.as_deref()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || !["/assets/", "/original/", "/preview/"]
            .iter()
            .any(|prefix| {
                url.path().strip_prefix(prefix).is_some_and(|id| {
                    !id.is_empty()
                        && id.len() <= 64
                        && id.bytes().all(|b| b.is_ascii_alphanumeric())
                })
            })
        || !(1..=50_000_000).contains(&size)
        || !matches!(
            content_type,
            "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif"
        )
    {
        return None;
    }
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let response = client.get(url).send().ok()?;
    if response.status() != reqwest::StatusCode::OK
        || response
            .content_length()
            .is_some_and(|length| length > size)
    {
        return None;
    }
    let mut bytes = Vec::new();
    response.take(size + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 != size {
        return None;
    }
    let image = if content_type != "image/avif" {
        let mut reader = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .ok()?;
        let mut limits = image::Limits::default();
        // The inline thumbnail decoder's budget: a 24 MP photo needs ~96 MB
        // decoded. The preview is downscaled to 1024 px below.
        limits.max_alloc = Some(512 * 1024 * 1024);
        limits.max_image_width = Some(16_384);
        limits.max_image_height = Some(16_384);
        reader.limits(limits);
        let mut decoder = reader.into_decoder().ok()?;
        let orientation = image::ImageDecoder::orientation(&mut decoder).ok()?;
        let mut image = image::DynamicImage::from_decoder(decoder).ok()?;
        image.apply_orientation(orientation);
        image.thumbnail(1024, 1024)
    } else {
        let original = directory.join("original");
        std::fs::write(&original, bytes).ok()?;
        let image = (|| {
            let binary = crate::ffmpeg::binary()?;
            let probe = crate::ffmpeg::probe(binary, &original).ok()?;
            crate::ffmpeg::poster(binary, &original, &probe, 1024)
        })();
        let _ = std::fs::remove_file(original);
        image::DynamicImage::ImageRgb8(image?)
    };
    let output = directory.join("upload.png");
    image.save(&output).ok()?;
    Some(output)
}

pub fn show(
    alert: Alert,
    stopped: Arc<AtomicBool>,
    suppressed: impl Fn() -> bool,
    clicked: impl Fn() + Send + 'static,
) -> Result<(), String> {
    if stopped.load(Ordering::Relaxed) || suppressed() {
        return Ok(());
    }
    let directory =
        std::env::temp_dir().join(format!("caper-notification-{}", uuid::Uuid::new_v4()));
    #[cfg(unix)]
    let created = {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&directory)
    };
    #[cfg(not(unix))]
    let created = std::fs::create_dir(&directory);
    created.map_err(|_| "Couldn’t prepare a notification.".to_owned())?;
    let image = avatar(alert.sender_avatar_id, &directory);
    let media = attachment(&alert, &directory);
    let result = if stopped.load(Ordering::Relaxed) || suppressed() {
        Ok(())
    } else {
        platform_show(&alert, image.as_deref(), media.as_deref(), stopped, clicked)
    };
    // Keep artwork and activation handlers until dismissal, expiry or logout.
    let _ = std::fs::remove_dir_all(directory);
    result
}

#[cfg(target_os = "linux")]
fn platform_show(
    alert: &Alert,
    image: Option<&std::path::Path>,
    media: Option<&std::path::Path>,
    stopped: Arc<AtomicBool>,
    clicked: impl Fn() + Send + 'static,
) -> Result<(), String> {
    let mut notification = notify_rust::Notification::new();
    notification
        .appname("Caper")
        .summary(&alert.title)
        .body(
            &alert
                .body
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;"),
        )
        .action("default", "Open Caper")
        .timeout(10_000);
    if let Some(path) = image.and_then(|path| path.to_str()) {
        notification.icon(path);
    }
    if let Some(path) = media.or(image).and_then(|path| path.to_str()) {
        notification.image_path(path);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Couldn’t prepare desktop notifications.".to_owned())?;
    runtime.block_on(async {
        let handle = notification.show_async().await.map_err(|_| {
            "Desktop notifications are unavailable. Check your notification service.".to_owned()
        })?;
        tokio::select! {
            _ = handle.wait_for_action_async(|action| {
                if action.is_default_action() && !stopped.load(Ordering::Relaxed) { clicked(); }
            }) => {},
            _ = async {
                let start = Instant::now();
                while !stopped.load(Ordering::Relaxed) && start.elapsed() < Duration::from_secs(86_400) {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            } => {},
        }
        handle.close_async().await;
        Ok(())
    })
}

#[cfg(target_os = "windows")]
fn platform_show(
    alert: &Alert,
    image: Option<&std::path::Path>,
    media: Option<&std::path::Path>,
    stopped: Arc<AtomicBool>,
    clicked: impl Fn() + Send + 'static,
) -> Result<(), String> {
    use winrt_toast_reborn::{
        Image, Toast, ToastManager,
        content::image::{ImageHintCrop, ImagePlacement},
        register,
    };
    const APP_ID: &str = "chat.caper.desktop";
    register(APP_ID, "Caper", None)
        .map_err(|_| "Couldn’t register desktop notifications.".to_owned())?;
    let mut toast = Toast::new();
    let tag = if let Some(id) = &alert.notification_id {
        let prefix = if alert.kind.as_deref() == Some("direct.request") {
            "request"
        } else {
            "invitation"
        };
        format!("{prefix}:{id}")
    } else {
        alert.message_id.clone().unwrap_or_default()
    };
    toast.text1(&alert.title).text2(&alert.body).tag(&tag);
    if let Some(image) = image.and_then(|path| Image::new_local(path).ok()) {
        toast.image(
            1,
            image
                .with_placement(ImagePlacement::AppLogoOverride)
                .with_hint_crop(ImageHintCrop::Circle),
        );
    }
    if let Some(media) = media.and_then(|path| Image::new_local(path).ok()) {
        toast.image(2, media.with_placement(ImagePlacement::Hero));
    }
    let activated = Arc::new(AtomicBool::new(false));
    let done = activated.clone();
    let cancelled = stopped.clone();
    let manager = ToastManager::new(APP_ID).on_activated(None, move |_| {
        done.store(true, Ordering::Relaxed);
        if !cancelled.load(Ordering::Relaxed) {
            clicked();
        }
    });
    manager.show(&toast).map_err(|_| {
        "Couldn’t show desktop notification. Check Windows notification settings.".to_owned()
    })?;
    let start = Instant::now();
    while !stopped.load(Ordering::Relaxed)
        && !activated.load(Ordering::Relaxed)
        && start.elapsed() < Duration::from_secs(86_400)
    {
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = manager.remove(&tag);
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn platform_show(
    _: &Alert,
    _: Option<&std::path::Path>,
    _: Option<&std::path::Path>,
    _: Arc<AtomicBool>,
    _: impl Fn() + Send + 'static,
) -> Result<(), String> {
    Err("Use the native Mac app for desktop notifications.".into())
}

#[cfg(test)]
#[allow(clippy::result_large_err)] // Tungstenite's handshake callback uses an unboxed HTTP response.
mod tests {
    use super::*;
    use std::{net::TcpListener, sync::mpsc, thread};
    use tungstenite::{
        Message,
        handshake::server::{Request, Response},
    };

    fn alert(value: serde_json::Value) -> Alert {
        let mut event = json!({
            "type": "notification.created", "seq": "1", "messageId": "msg000000002",
            "title": "Alice · Feedback", "body": "@feedback help", "sender": "Alice",
            "senderId": "user00000001", "createdAt": "2026-10-11T19:00:00Z"
        });
        for (key, value) in value.as_object().unwrap() {
            event[key] = value.clone();
        }
        serde_json::from_value(event).unwrap()
    }

    #[test]
    fn clicks_route_feedback_to_a_visit_and_the_rest_as_before() {
        use crate::feedback::VisitTarget;
        let visit = |channel: &str, message: &str, thread: &str| {
            Some(Route::Visit(VisitTarget {
                channel: channel.into(),
                message: message.into(),
                thread: Some(thread.into()),
                reporter: None,
            }))
        };
        let top = alert(json!({"kind": "feedback", "spaceId": "space0000001",
            "channelId": "chan00000001", "conversationTitle": "#plans"}));
        assert_eq!(
            top.route(true, true),
            visit("chan00000001", "msg000000002", "msg000000002"),
            "a top-level report opens its own thread, even in a known space"
        );
        let reply = alert(json!({"kind": "feedback", "conversationId": "dm0000000001",
            "threadRootId": "msg000000001"}));
        assert_eq!(
            reply.route(true, false),
            visit("dm0000000001", "msg000000002", "msg000000001")
        );
        let mention = alert(json!({"kind": "mention.user", "spaceId": "space0000001",
            "channelId": "chan00000001"}));
        assert_eq!(
            mention.route(true, true),
            Some(Route::Channel {
                space: "space0000001".into(),
                channel: "chan00000001".into()
            })
        );
        assert_eq!(
            mention.route(false, false),
            Some(Route::Channel {
                space: "space0000001".into(),
                channel: "chan00000001".into()
            }),
            "only the team visits"
        );
        assert_eq!(
            mention.route(true, false),
            Some(Route::Visit(VisitTarget {
                channel: "chan00000001".into(),
                message: "msg000000002".into(),
                thread: None,
                reporter: None,
            })),
            "a space the team member isn't in opens as a visit"
        );
        let threaded = alert(
            json!({"kind": "mention.user", "conversationId": "dm0000000002",
            "threadRootId": "msg000000001"}),
        );
        assert_eq!(
            threaded.route(true, false),
            visit("dm0000000002", "msg000000002", "msg000000001")
        );
        assert_eq!(
            threaded.route(true, true),
            Some(Route::Direct("dm0000000002".into()))
        );
        assert_eq!(threaded.thread_root_id.as_deref(), Some("msg000000001"));
        let old = alert(json!({"conversationId": "dm0000000003"}));
        assert_eq!(old.kind, None, "older servers omit the kind");
        assert_eq!(
            old.route(false, true),
            Some(Route::Direct("dm0000000003".into()))
        );
        assert_eq!(alert(json!({"kind": "feedback"})).route(true, true), None);
        // Main's kinds keep their routes, even for the team outside the space.
        let invitation = alert(json!({"kind": "space.invitation", "spaceId": "space0000009"}));
        assert_eq!(
            invitation.route(true, false),
            Some(Route::Invitation {
                space: "space0000009".into(),
                channel: None
            })
        );
        let request = alert(json!({"kind": "direct.request", "conversationId": "dm0000000009"}));
        assert_eq!(
            request.route(true, false),
            Some(Route::Direct("dm0000000009".into())),
            "a message request is never a visit"
        );
        assert_eq!(request.conversation(), None, "never suppressed as visible");
    }

    #[test]
    fn invitation_is_not_a_message_or_a_focused_conversation() {
        let alert: Alert = serde_json::from_value(json!({
            "kind":"channel.invitation", "notificationId":"91", "title":"You’re invited!",
            "body":"Maya invited you to #planning in Studio.", "createdAt":Utc::now().to_rfc3339(),
            "spaceId":"space0000001", "channelId":"channel00001"
        }))
        .unwrap();
        assert!(alert.invitation());
        assert_eq!(alert.conversation(), None);
        assert!(alert.fresh());
        assert_eq!(alert.channel_id.as_deref(), Some("channel00001"));
        let request: Alert = serde_json::from_value(json!({
            "kind":"direct.request", "notificationId":"92", "title":"Message request",
            "body":"Maya wants to message you.", "createdAt":Utc::now().to_rfc3339(),
            "conversationId":"direct000001"
        }))
        .unwrap();
        assert!(!request.invitation());
        assert_eq!(request.conversation(), None);
        assert_eq!(request.conversation_id.as_deref(), Some("direct000001"));
    }

    #[test]
    fn account_feed_subscribes_to_profiles_each_socket_and_survives_an_old_gateway() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let mut kinds = Vec::new();
            for round in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut socket = tungstenite::accept(stream).unwrap();
                let send = |socket: &mut tungstenite::WebSocket<_>, value: serde_json::Value| {
                    socket
                        .send(Message::Text(value.to_string().into()))
                        .unwrap();
                };
                send(&mut socket, json!({"type":"hello","serverTime":0}));
                let mut frames = Vec::new();
                for _ in 0..2 {
                    let frame: serde_json::Value =
                        serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                    frames.push(frame);
                }
                assert_eq!(
                    frames[1],
                    json!({"type":"subscribe","id":"profiles","kind":"profiles"}),
                    "exactly the profiles subscription, after notifications"
                );
                kinds.push(frames[0]["kind"].clone());
                let notifications = frames[0]["id"].clone();
                if round == 0 {
                    // A current gateway: live profile and space changes, then
                    // a migration to another socket.
                    send(&mut socket, json!({"type":"subscribed","id":"profiles"}));
                    send(
                        &mut socket,
                        json!({"type":"event","id":"profiles","event":{
                        "type":"profile.updated","person":{"id":"u1","username":"maya",
                        "displayName":"Maya","avatarId":3,"avatarAsset":"pic"}}}),
                    );
                    send(
                        &mut socket,
                        json!({"type":"event","id":"profiles","event":{
                        "type":"space.updated","space":{"id":"s1","name":"Friends","ownerId":"u1"}}}),
                    );
                    send(&mut socket, json!({"type":"migrating"}));
                } else {
                    // An older gateway: `profiles` is unknown; notifications
                    // carry on over the same socket.
                    send(
                        &mut socket,
                        json!({"type":"error","id":"profiles","status":400,
                        "error":"unknown subscription kind"}),
                    );
                    send(
                        &mut socket,
                        json!({"type":"event","id":notifications,"event":{"type":"ready","cursor":"1"}}),
                    );
                    send(
                        &mut socket,
                        json!({"type":"event","id":notifications,"event":{
                        "type":"notification.created","seq":"2","messageId":"message00000001",
                        "title":"Test sender","body":"after the error",
                        "createdAt":Utc::now().to_rfc3339(),"conversationId":"dm0000000001"}}),
                    );
                    let _ = socket.read();
                }
            }
            kinds
        });
        let (alerts, received_alerts) = mpsc::channel();
        let (profiles, received_profiles) = mpsc::channel();
        let control = watch(
            &url::Url::parse(&format!("http://{address}/")).unwrap(),
            "test-account".into(),
            move |alert, stop| {
                stop.store(true, Ordering::Relaxed);
                alerts.send(alert).unwrap();
            },
            move |event| profiles.send(event).unwrap(),
        );
        let person = received_profiles
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(
            matches!(&person, crate::gateway::ProfileEvent::Person(person)
                if person.id == "u1" && person.avatar_asset.as_deref() == Some("pic"))
        );
        let space = received_profiles
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert!(
            matches!(&space, crate::gateway::ProfileEvent::Space(space) if space.name == "Friends")
        );
        let alert = received_alerts
            .recv_timeout(Duration::from_secs(5))
            .unwrap();
        assert_eq!(alert.body, "after the error", "the error tore nothing down");
        assert!(received_profiles.try_recv().is_err());
        control.stop();
        let kinds = server.join().unwrap();
        assert_eq!(kinds, [json!("notifications"), json!("notifications")]);
    }

    #[test]
    fn account_feed_resumes_sparse_ids_without_duplicate_alerts_or_a_ui_loop() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for (after, sequences) in [
                (None, vec![("9007199254741001", "first")]),
                (
                    Some("9007199254741013"),
                    vec![
                        ("9007199254741001", "duplicate"),
                        ("9007199254741057", "second"),
                    ],
                ),
            ] {
                let (stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut socket =
                    tungstenite::accept_hdr(stream, |request: &Request, response: Response| {
                        assert_eq!(request.uri().path(), "/api/chat/events");
                        assert!(request.uri().query().is_none());
                        assert_eq!(request.headers()["authorization"], "Bearer test-account");
                        Ok(response)
                    })
                    .unwrap();
                socket
                    .send(Message::Text(
                        json!({"type":"hello","serverTime":0}).to_string().into(),
                    ))
                    .unwrap();
                let subscription: serde_json::Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                assert_eq!(subscription["kind"], "notifications");
                assert_eq!(subscription["after"].as_str(), after);
                // Read the profiles subscription too: unread data would turn
                // this server's close into a reset that loses the frames below.
                let profiles: serde_json::Value =
                    serde_json::from_str(socket.read().unwrap().to_text().unwrap()).unwrap();
                assert_eq!(profiles["kind"], "profiles");
                let id = subscription["id"].clone();
                socket.send(Message::Text(json!({"type":"event","id":id,"event":{"type":"ready","cursor":after.unwrap_or("700")}}).to_string().into())).unwrap();
                for (seq, body) in sequences {
                    socket.send(Message::Text(json!({"type":"event","id":id,"event":{
                        "type":"notification.created","seq":seq,"messageId":"message00000001",
                        "title":"Test sender","body":body,"senderAvatarId":799,
                        "createdAt":Utc::now().to_rfc3339(),"conversationId":"dm0000000001"
                    }}).to_string().into())).unwrap();
                }
                if after.is_none() {
                    // Suppressed rows also advance the ready checkpoint.
                    socket.send(Message::Text(json!({"type":"event","id":id,"event":{"type":"ready","cursor":"9007199254741013"}}).to_string().into())).unwrap();
                    socket
                        .send(Message::Text(
                            json!({"type":"migrating"}).to_string().into(),
                        ))
                        .unwrap();
                } else {
                    // The caller stops after the second alert; no UI frame is required.
                    let _ = socket.read();
                }
            }
        });
        let (send, received) = mpsc::channel();
        let control = watch(
            &url::Url::parse(&format!("http://{address}/")).unwrap(),
            "test-account".into(),
            move |alert, stop| {
                if alert.body == "second" {
                    stop.store(true, Ordering::Relaxed);
                }
                send.send(alert).unwrap();
            },
            |_| {},
        );
        let first = received.recv_timeout(Duration::from_secs(5)).unwrap();
        let second = received.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(
            (first.body.as_str(), second.body.as_str()),
            ("first", "second")
        );
        assert_eq!(second.sender_avatar_id, Some(799));
        assert_eq!(second.conversation(), Some("dm0000000001"));
        assert!(received.recv_timeout(Duration::from_millis(200)).is_err());
        control.stop();
        server.join().unwrap();
    }
}
