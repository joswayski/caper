//! Badge updates must keep running when a minimized window stops painting.
//! No notification is created: Windows uses its badge API; Linux emits the
//! Unity launcher protocol (only desktops implementing that protocol show it).
use crate::api::Api;
use eframe::egui;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Default)]
struct State {
    token: Option<String>,
    revision: u64,
    stopped: bool,
}

pub struct Badge {
    shared: Arc<(Mutex<State>, Condvar)>,
    context: egui::Context,
}

impl Badge {
    pub fn new(api: Api, context: egui::Context) -> Self {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker = shared.clone();
        let repaint = context.clone();
        std::thread::spawn(move || {
            let (lock, wake) = &*worker;
            let mut state = lock.lock().unwrap();
            loop {
                if state.stopped {
                    break;
                }
                if let Some(token) = state.token.clone() {
                    let revision = state.revision;
                    drop(state);
                    let result = api.unread_spaces(&token);
                    state = lock.lock().unwrap();
                    // Account changes and read acknowledgements invalidate in-flight polls.
                    if state.revision != revision {
                        continue;
                    }
                    match result {
                        Ok(unread) => {
                            if let Some(count) = unread.badge_count {
                                publish(&repaint, count);
                            }
                        }
                        Err(error) if error.status == Some(reqwest::StatusCode::UNAUTHORIZED) => {
                            publish(&repaint, 0);
                        }
                        Err(_) => {} // Keep the last known count during an outage.
                    }
                    state = wake.wait_timeout(state, Duration::from_secs(15)).unwrap().0;
                } else {
                    state = wake.wait(state).unwrap();
                }
            }
        });
        Self { shared, context }
    }

    pub fn account(&self, token: Option<String>) {
        let mut state = self.shared.0.lock().unwrap();
        state.revision = state.revision.wrapping_add(1);
        state.token = token;
        publish(&self.context, 0);
        self.shared.1.notify_one();
    }

    pub fn refresh(&self) {
        let mut state = self.shared.0.lock().unwrap();
        state.revision = state.revision.wrapping_add(1);
        self.shared.1.notify_one();
    }
}

impl Drop for Badge {
    fn drop(&mut self) {
        let mut state = self.shared.0.lock().unwrap();
        state.stopped = true;
        state.revision = state.revision.wrapping_add(1);
        publish(&self.context, 0);
        self.shared.1.notify_one();
    }
}

fn publish(context: &egui::Context, count: u64) {
    context.send_viewport_cmd(egui::ViewportCommand::Title(if count == 0 {
        "Caper".into()
    } else {
        format!("({count}) Caper")
    }));
    context.request_repaint();
    // Unsupported shell/settings, missing session bus, unpackaged Windows
    // execution, etc. leave the title fallback intact.
    let _ = platform(count);
}

#[cfg(target_os = "linux")]
fn platform(count: u64) -> Result<(), Box<dyn std::error::Error>> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    use zbus::zvariant::Value;
    // Keep the sender alive: launchers associate entries with its bus name.
    static BUS: OnceLock<Mutex<Option<zbus::blocking::Connection>>> = OnceLock::new();
    let mut bus = BUS.get_or_init(|| Mutex::new(None)).lock().unwrap();
    if bus.is_none() {
        *bus = Some(zbus::blocking::Connection::session()?);
    }
    let properties = HashMap::from([
        ("count", Value::I64(count.min(i64::MAX as u64) as i64)),
        ("count-visible", Value::Bool(count > 0)),
    ]);
    // build.sh installs /usr/share/applications/caper.desktop; Wayland app_id is caper.
    let result = bus.as_ref().unwrap().emit_signal(
        None::<&str>,
        "/com/canonical/Unity/LauncherEntry",
        "com.canonical.Unity.LauncherEntry",
        "Update",
        &("application://caper.desktop", properties),
    );
    if result.is_err() {
        *bus = None;
    }
    result?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn platform(count: u64) -> Result<(), Box<dyn std::error::Error>> {
    use windows::{
        Data::Xml::Dom::XmlDocument,
        UI::Notifications::{BadgeNotification, BadgeUpdateManager},
        core::HSTRING,
    };
    const APP_ID: &str = "chat.caper.desktop";
    winrt_toast_reborn::register(APP_ID, "Caper", None)?;
    let updater =
        BadgeUpdateManager::CreateBadgeUpdaterForApplicationWithId(&HSTRING::from(APP_ID))?;
    if count == 0 {
        updater.Clear()?;
    } else {
        let xml = XmlDocument::new()?;
        // Windows renders numeric badges above 99 as 99+.
        xml.LoadXml(&HSTRING::from(format!(
            "<badge value=\"{}\"/>",
            count.min(100)
        )))?;
        updater.Update(&BadgeNotification::CreateBadgeNotification(&xml)?)?;
    }
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn platform(_: u64) -> Result<(), Box<dyn std::error::Error>> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc,
        time::Instant,
    };

    #[test]
    fn badge_counts_are_optional_unsigned_and_not_estimated_from_channel_sequences() {
        for (extra, expected) in [
            ("", None),
            (",\"badgeCount\":0", Some(0)),
            (",\"badgeCount\":18446744073709551615", Some(u64::MAX)),
        ] {
            let unread: crate::model::UnreadSpaces =
                serde_json::from_str(&format!("{{\"spaces\":[]{extra}}}")).unwrap();
            assert_eq!(unread.badge_count, expected);
        }
        assert!(
            serde_json::from_str::<crate::model::UnreadSpaces>("{\"spaces\":[],\"badgeCount\":-1}")
                .is_err()
        );
    }

    #[test]
    fn badge_refreshes_without_ui_frames_and_rejects_old_account_responses() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = Api::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let (blocked, received) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let server = std::thread::spawn(move || {
            for (i, count) in [7, 99, 2].into_iter().enumerate() {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                reader.read_line(&mut request).unwrap();
                assert_eq!(request, "GET /api/spaces/unread HTTP/1.1\r\n");
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {
                        break;
                    }
                }
                if i == 1 {
                    blocked.send(()).unwrap();
                    released.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                let body = format!("{{\"spaces\":[],\"badgeCount\":{count}}}");
                write!(reader.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let context = egui::Context::default();
        let (repaints, repaint_rx) = mpsc::channel();
        context.set_request_repaint_callback(move |_| {
            let _ = repaints.send(());
        });
        let badge = Badge::new(api, context.clone());
        badge.account(Some("old-account".into()));
        // No egui frame is needed for the authoritative fetch or platform update.
        let mut titles = Vec::new();
        let mut wait_for = |wanted: &str| {
            let until = Instant::now() + Duration::from_secs(5);
            loop {
                let output = context.run(egui::RawInput::default(), |_| {});
                titles.extend(output.viewport_output.values().flat_map(|viewport| {
                    viewport
                        .commands
                        .iter()
                        .filter_map(|command| match command {
                            egui::ViewportCommand::Title(title) => Some(title.clone()),
                            _ => None,
                        })
                }));
                if titles.last().is_some_and(|title| title == wanted) {
                    break;
                }
                assert!(Instant::now() < until, "missing {wanted}: {titles:?}");
                let _ = repaint_rx.recv_timeout(Duration::from_millis(20));
            }
        };
        wait_for("(7) Caper");
        badge.refresh();
        received.recv_timeout(Duration::from_secs(5)).unwrap();
        badge.account(Some("new-account".into()));
        release.send(()).unwrap();
        wait_for("(2) Caper");
        badge.account(None);
        wait_for("Caper");
        assert!(!titles.iter().any(|title| title == "(99) Caper"));
        server.join().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "run under dbus-run-session; validates the wire protocol, not a desktop shell"]
    fn launcher_badge_emits_numeric_count_and_clear_for_packaged_identity() {
        let connection = zbus::blocking::Connection::session().unwrap();
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("com.canonical.Unity.LauncherEntry")
            .unwrap()
            .member("Update")
            .unwrap()
            .build();
        let mut messages =
            zbus::blocking::MessageIterator::for_match_rule(rule, &connection, Some(2)).unwrap();
        let (sent, received) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for _ in 0..2 {
                let message = messages.next().unwrap().unwrap();
                let body: (
                    String,
                    std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
                ) = message.body().deserialize().unwrap();
                sent.send(body).unwrap();
            }
        });
        for count in [42, 0] {
            platform(count).unwrap();
            let (identity, properties) = received.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(identity, "application://caper.desktop");
            assert_eq!(
                i64::try_from(properties.get("count").unwrap()).unwrap(),
                count as i64
            );
            assert_eq!(
                bool::try_from(properties.get("count-visible").unwrap()).unwrap(),
                count > 0
            );
        }
        reader.join().unwrap();
    }
}
