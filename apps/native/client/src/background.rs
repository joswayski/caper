//! Desktop apps keep running when their window closes, so a voice call and
//! notifications carry on: Windows hides to its tray icon, and Linux
//! minimizes, because many desktops (GNOME) have no tray to find a hidden app
//! in. macOS hides the window as Mac apps do (CaperAppKitApp.swift). This
//! holds the per-device setting and the notices, so both Rust clients say the
//! same things.

use crate::state::{CallContext, Phase};

/// The saved setting, `"true"` or `"false"`; absent means on.
pub const KEY: &str = "keep-running-when-closed-v1";
/// Set once the first close has said where Caper went.
pub const NOTICE_KEY: &str = "keep-running-notice-v1";

pub const SETTING_TITLE: &str = "Keep Caper running when the window closes";
#[cfg(not(target_os = "linux"))]
pub const SETTING_DETAIL: &str = "Closing the window hides Caper in the system tray, so calls and notifications keep going. Off: closing the window quits Caper.";
#[cfg(target_os = "linux")]
pub const SETTING_DETAIL: &str = "Closing the window minimizes Caper to your taskbar or dock, so calls and notifications keep going. Off: closing the window quits Caper.";

/// Whether the saved value keeps Caper running.
pub fn enabled(saved: Option<&str>) -> bool {
    saved != Some("false")
}

/// The call that goes on while the window is away: joining, connected or
/// reconnecting.
pub fn ongoing(phase: &Phase) -> Option<&CallContext> {
    match phase {
        Phase::Joining(call) | Phase::Connected(call) | Phase::Reconnecting(call) => Some(call),
        Phase::Idle | Phase::Failed(_) => None,
    }
}

/// What the window says as it goes, as a notification's title and body:
/// every time during a call, otherwise only the first time (`noticed` is
/// whether that has happened).
pub fn notice(call: Option<&CallContext>, noticed: bool) -> Option<(String, String)> {
    #[cfg(not(target_os = "linux"))]
    const CALL: &str =
        "Caper is in the system tray. Right-click its icon to mute, deafen or leave the call.";
    #[cfg(target_os = "linux")]
    const CALL: &str =
        "Caper is minimized. Open it from your taskbar or dock to mute, deafen or leave the call.";
    #[cfg(not(target_os = "linux"))]
    const RUNNING: &str = "Caper is in the system tray, so you still get messages. Right-click its icon to quit, or turn this off in Settings.";
    #[cfg(target_os = "linux")]
    const RUNNING: &str = "Caper is minimized, so you still get messages. Choose Quit Caper in the account menu (Ctrl+Q) to quit, or turn this off in Settings.";
    match call {
        Some(call) => Some((
            format!("You’re still in the call in {}", call.channel_name),
            CALL.into(),
        )),
        None if !noticed => Some(("Caper is still running".into(), RUNNING.into())),
        None => None,
    }
}

/// Shows a notice as a system notification; `clicked` runs, on another
/// thread, when it is clicked. Windows shows its notices from the tray icon.
#[cfg(target_os = "linux")]
pub fn show(title: String, body: String, clicked: impl Fn() + Send + 'static) {
    let _ = std::thread::Builder::new()
        .name("caper-background-notice".into())
        .spawn(move || {
            let mut notification = notify_rust::Notification::new();
            notification
                .appname("Caper")
                .summary(&title)
                .body(&body)
                .action("default", "Open Caper")
                .timeout(10_000);
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async {
                let Ok(handle) = notification.show_async().await else {
                    return;
                };
                handle
                    .wait_for_action_async(|action| {
                        if action.is_default_action() {
                            clicked();
                        }
                    })
                    .await;
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call() -> CallContext {
        CallContext {
            channel_id: "c1".into(),
            channel_name: "standup".into(),
            space_name: "Team".into(),
        }
    }

    #[test]
    fn on_unless_turned_off() {
        assert!(enabled(None));
        assert!(enabled(Some("true")));
        assert!(!enabled(Some("false")));
    }

    #[test]
    fn a_failed_call_is_not_one_that_keeps_going() {
        assert!(ongoing(&Phase::Reconnecting(call())).is_some());
        assert!(ongoing(&Phase::Joining(call())).is_some());
        assert!(ongoing(&Phase::Failed(call())).is_none());
        assert!(ongoing(&Phase::Idle).is_none());
    }

    #[test]
    fn a_call_is_announced_every_time_and_running_only_once() {
        let call = call();
        let (title, _) = notice(Some(&call), true).unwrap();
        assert!(title.contains("standup"));
        assert!(notice(None, false).is_some());
        assert!(notice(None, true).is_none());
    }
}
