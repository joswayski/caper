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
    handoff_path().is_some_and(|path| hand_off_at(&path, code))
}

/// Receives codes handed off by later launches; `wake` runs after each one so
/// the UI can repaint and pick it up.
pub fn listen(wake: impl Fn() + Send + 'static) -> Option<mpsc::Receiver<String>> {
    listen_at(&handoff_path()?, wake)
}
