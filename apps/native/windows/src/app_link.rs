//! `caper://invite/<code>` links (`caper_client::app_link`), as in the egui
//! client: this executable is the registered handler, and a link clicked
//! while Caper is open is handed to that window through egui's handoff
//! file, so whichever Caper is running receives it. A code opens Join with
//! invite link with the link filled in and its preview loading, once
//! signed in.
use crate::overlay::Dialog;
use crate::{App, EV_SCREEN};
use std::path::PathBuf;
use std::sync::mpsc;
use windows::Win32::UI::WindowsAndMessaging::{
    IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
};

/// egui's handoff file (eframe's storage directory, `%APPDATA%\Caper\data`).
fn handoff_path() -> Option<PathBuf> {
    let base = PathBuf::from(std::env::var_os("APPDATA")?);
    Some(base.join("Caper").join("data").join("link-handoff"))
}

/// The invite code this launch was opened with, if any.
pub fn launch_code() -> Option<String> {
    caper_client::app_link::launch_code(&std::env::args().collect::<Vec<_>>())
}

/// Gives `code` to an already-running Caper. False when none accepted it.
pub fn hand_off(code: &str) -> bool {
    handoff_path().is_some_and(|path| caper_client::app_link::hand_off_at(&path, code))
}

/// Points `caper://` at this executable. Release builds only, as in egui:
/// a development build must not take over the installed app's links.
pub fn register() {
    if !cfg!(debug_assertions)
        && let Err(error) = caper_client::app_link::register()
    {
        eprintln!("Caper could not register caper:// links: {error}");
    }
}

/// Receives codes from later launches; each one wakes the window.
pub fn listen(wake: impl Fn() + Send + 'static) -> Option<mpsc::Receiver<String>> {
    caper_client::app_link::listen_at(&handoff_path()?, wake)
}

impl App {
    /// Codes handed off since the last wake come to the front, then wait
    /// for sign-in (egui's `poll_app_links`).
    pub(crate) fn poll_app_links(&mut self) {
        let mut arrived = false;
        if let Some(links) = &self.app_links {
            while let Ok(code) = links.try_recv() {
                self.pending_invite = Some(code);
                arrived = true;
            }
        }
        if arrived {
            unsafe {
                if IsIconic(self.hwnd).as_bool() {
                    let _ = ShowWindow(self.hwnd, SW_RESTORE);
                }
                let _ = SetForegroundWindow(self.hwnd);
            }
        }
        if self.signin.is_some() {
            return;
        }
        let Some(live) = &mut self.live else { return };
        let Some(code) = self.pending_invite.take() else {
            return;
        };
        let mut d = Dialog::join_invite();
        if let Some(base) = crate::live::endpoint().and_then(|u| url::Url::parse(&u).ok()) {
            let link = caper_client::invites::invite_url(&base, &code);
            d.field.set_text(link.encode_utf16().collect());
        }
        d.busy = true;
        d.request = live.preview_invite(code);
        self.open_dialog(d);
        self.events |= EV_SCREEN;
    }
}
