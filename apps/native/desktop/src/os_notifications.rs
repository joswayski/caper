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
    #[cfg(target_os = "windows")]
    pub message_id: String,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub sender_avatar_id: Option<i32>,
    pub conversation_id: Option<String>,
    pub space_id: Option<String>,
    pub channel_id: Option<String>,
}

impl Alert {
    pub fn conversation(&self) -> Option<&str> {
        self.conversation_id
            .as_deref()
            .or(self.channel_id.as_deref())
    }
    pub fn fresh(&self) -> bool {
        DateTime::parse_from_rfc3339(&self.created_at).is_ok_and(|date| {
            (-30..=120).contains(&(Utc::now() - date.with_timezone(&Utc)).num_seconds())
        })
    }
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

pub fn watch(
    base: &url::Url,
    token: String,
    receive: impl Fn(Alert, Arc<AtomicBool>) + Send + 'static,
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
                        }
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

pub fn show(
    alert: Alert,
    stopped: Arc<AtomicBool>,
    clicked: impl Fn() + Send + 'static,
) -> Result<(), String> {
    if stopped.load(Ordering::Relaxed) {
        return Ok(());
    }
    let directory =
        std::env::temp_dir().join(format!("caper-notification-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).map_err(|_| "Couldn’t prepare a notification.".to_owned())?;
    let image = avatar(alert.sender_avatar_id, &directory);
    let result = platform_show(&alert, image.as_deref(), stopped, clicked);
    // Keep artwork and activation handlers until dismissal, expiry or logout.
    let _ = std::fs::remove_dir_all(directory);
    result
}

#[cfg(target_os = "linux")]
fn platform_show(
    alert: &Alert,
    image: Option<&std::path::Path>,
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
        notification.icon(path).image_path(path);
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
    toast
        .text1(&alert.title)
        .text2(&alert.body)
        .tag(&alert.message_id);
    if let Some(image) = image.and_then(|path| Image::new_local(path).ok()) {
        toast.image(
            1,
            image
                .with_placement(ImagePlacement::AppLogoOverride)
                .with_hint_crop(ImageHintCrop::Circle),
        );
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
    let _ = manager.remove(&alert.message_id);
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn platform_show(
    _: &Alert,
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
