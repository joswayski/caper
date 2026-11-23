//! Opening invite links in this app (`caper_client::app_link`), with the
//! handoff file in eframe's storage directory.

use caper_client::app_link::{hand_off_at, listen_at};
pub use caper_client::app_link::{launch_code, register};
use std::path::PathBuf;
use std::sync::mpsc;

fn handoff_path() -> Option<PathBuf> {
    eframe::storage_dir("Caper").map(|directory| directory.join("link-handoff"))
}

/// Gives `code` to an already-running Caper. False when none accepted it.
pub fn hand_off(code: &str) -> bool {
    allow_running_app_to_focus();
    handoff_path().is_some_and(|path| hand_off_at(&path, code))
}

/// Windows lets only the process the browser just started take the
/// foreground, so the running Caper's Focus would only flash its taskbar
/// button. This launch passes that right on before handing off the code.
#[cfg(target_os = "windows")]
#[expect(unsafe_code)]
fn allow_running_app_to_focus() {
    use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    // SAFETY: takes no pointers; failing only leaves focus as it was.
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
}

#[cfg(not(target_os = "windows"))]
fn allow_running_app_to_focus() {}

/// Receives codes handed off by later launches; `wake` runs after each one so
/// the UI can repaint and pick it up.
pub fn listen(wake: impl Fn() + Send + 'static) -> Option<mpsc::Receiver<String>> {
    listen_at(&handoff_path()?, wake)
}
