//! The notification-area (tray) icon that holds Caper while it keeps running
//! with its window closed (`caper_client::background`). Shown while that
//! setting is on: a click opens the window, and its menu offers Open Caper,
//! Mute, Deafen and Leave call during a call, and Quit Caper. The notices
//! that say where Caper went are its balloons, which Windows shows as toasts.

use caper_client::background;
use windows::Win32::Foundation::{HWND, LPARAM, POINT};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_RESPECT_QUIET_TIME, NIIF_USER,
    NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_BALLOONUSERCLICK, NIN_SELECT, NINF_KEY,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, GetSystemMetrics, HICON, IMAGE_ICON,
    IsIconic, IsWindowVisible, LR_DEFAULTCOLOR, LoadImageW, MF_CHECKED, MF_SEPARATOR, MF_STRING,
    PostMessageW, RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON, SW_HIDE, SW_RESTORE, SW_SHOW,
    SetForegroundWindow, ShowWindow, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    WM_APP, WM_CONTEXTMENU, WM_NULL,
};
use windows::core::{HSTRING, PCWSTR, w};

/// The icon's messages to the window; the event is `LOWORD(lParam)`.
pub const WM_TRAY: u32 = WM_APP + 40;
const ID: u32 = 1;

/// A tray menu choice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Open,
    Mute,
    Deafen,
    Leave,
    Quit,
}

const CHOICES: [Choice; 5] = [
    Choice::Open,
    Choice::Mute,
    Choice::Deafen,
    Choice::Leave,
    Choice::Quit,
];

/// The call the menu controls, as the menu shows it.
pub struct Call {
    pub muted: bool,
    pub deafened: bool,
}

pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    /// Explorer's broadcast when the taskbar (re)starts, after which the
    /// icon must be added again.
    pub taskbar_created: u32,
    shown: bool,
    tip: String,
}

impl Tray {
    pub fn new(hwnd: HWND) -> Self {
        // SAFETY: plain Win32 calls; a missing icon leaves the default one.
        unsafe {
            let instance =
                windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
            // build.rs embeds Caper's icon as resource 1, at the tray's size.
            let icon = LoadImageW(
                Some(instance.into()),
                PCWSTR(1 as _),
                IMAGE_ICON,
                GetSystemMetrics(SM_CXSMICON),
                GetSystemMetrics(SM_CYSMICON),
                LR_DEFAULTCOLOR,
            )
            .map(|h| HICON(h.0))
            .unwrap_or_default();
            Self {
                hwnd,
                icon,
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                shown: false,
                tip: String::new(),
            }
        }
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: ID,
            ..Default::default()
        }
    }

    /// Shows the icon with `tip`; false when the taskbar has no tray (it may
    /// not be running), so the window must not hide.
    pub fn show(&mut self, tip: &str) -> bool {
        if self.shown && self.tip == tip {
            return true;
        }
        self.tip = tip.to_owned();
        let mut d = self.data();
        d.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        d.uCallbackMessage = WM_TRAY;
        d.hIcon = self.icon;
        copy(&mut d.szTip, tip);
        d.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: `d` outlives each call.
        unsafe {
            // Already there after a setting change: just retitle it.
            if self.shown && Shell_NotifyIconW(NIM_MODIFY, &d).as_bool() {
                return true;
            }
            self.shown = Shell_NotifyIconW(NIM_ADD, &d).as_bool();
            if self.shown {
                let _ = Shell_NotifyIconW(NIM_SETVERSION, &d);
            }
        }
        self.shown
    }

    /// The taskbar restarted: whatever was shown is gone and comes back.
    pub fn taskbar_restarted(&mut self) {
        if std::mem::take(&mut self.shown) {
            let tip = std::mem::take(&mut self.tip);
            self.show(&tip);
        }
    }

    pub fn hide(&mut self) {
        if std::mem::take(&mut self.shown) {
            // SAFETY: `d` outlives the call.
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &self.data());
            }
        }
    }

    /// A balloon from the icon (a toast on Windows 10 and 11); a click on it
    /// arrives as `NIN_BALLOONUSERCLICK`.
    pub fn notice(&self, title: &str, body: &str) {
        if !self.shown {
            return;
        }
        let mut d = self.data();
        d.uFlags = NIF_INFO;
        copy(&mut d.szInfoTitle, title);
        copy(&mut d.szInfo, body);
        d.dwInfoFlags = NIIF_USER | NIIF_RESPECT_QUIET_TIME;
        d.hBalloonIcon = self.icon;
        // SAFETY: `d` outlives the call.
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &d);
        }
    }
}

/// The icon's menu at the pointer, for `hwnd`; what was chosen, if anything.
/// It runs a modal loop, so the app must not be borrowed meanwhile.
pub fn menu(hwnd: HWND, call: Option<Call>) -> Option<Choice> {
    // SAFETY: the menu lives until DestroyMenu below.
    unsafe {
        let menu = CreatePopupMenu().ok()?;
        let item = |flags, choice: Choice, text: &str| {
            let id = CHOICES.iter().position(|c| *c == choice).unwrap_or(0) + 1;
            let _ = AppendMenuW(menu, flags, id, &HSTRING::from(text));
        };
        item(MF_STRING, Choice::Open, "Open Caper");
        if let Some(call) = call {
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            item(
                if call.muted { MF_CHECKED } else { MF_STRING },
                Choice::Mute,
                "Mute",
            );
            item(
                if call.deafened { MF_CHECKED } else { MF_STRING },
                Choice::Deafen,
                "Deafen",
            );
            item(MF_STRING, Choice::Leave, "Leave call");
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        item(MF_STRING, Choice::Quit, "Quit Caper");
        let mut at = POINT::default();
        let _ = GetCursorPos(&mut at);
        // Without this the menu stays open after a click elsewhere.
        let _ = SetForegroundWindow(hwnd);
        let chosen = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
            at.x,
            at.y,
            None,
            hwnd,
            None,
        )
        .0 as usize;
        let _ = PostMessageW(Some(hwnd), WM_NULL, Default::default(), Default::default());
        let _ = DestroyMenu(menu);
        chosen.checked_sub(1).and_then(|i| CHOICES.get(i).copied())
    }
}

impl crate::App {
    /// Whether closing the window keeps Caper running: the setting, while
    /// connected (fixture and bench runs quit as before).
    pub(crate) fn keep_running(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| background::enabled(l.preferences.get(background::KEY)))
    }

    fn background_call(&self) -> Option<&caper_client::state::CallContext> {
        background::ongoing(&self.live.as_ref()?.voice.state.phase)
    }

    /// The icon's tooltip, which also says when a call is going on.
    fn tray_tip(&self) -> String {
        match self.background_call() {
            Some(call) => format!("Caper · In the call in {}", call.channel_name),
            None => "Caper".into(),
        }
    }

    /// Shows the icon while keeping running is on, as up to date as the
    /// call; hides it otherwise.
    pub(crate) fn sync_tray(&mut self) {
        if self.keep_running() {
            let tip = self.tray_tip();
            self.tray.show(&tip);
        } else {
            self.tray.hide();
        }
    }

    /// The window's close: hides it to the tray while keeping running is
    /// on, and says where Caper went. False when Caper should quit instead,
    /// including when there is no tray to come back from.
    pub(crate) fn close_to_tray(&mut self) -> bool {
        if !self.keep_running() {
            return false;
        }
        let tip = self.tray_tip();
        if !self.tray.show(&tip) {
            return false;
        }
        // SAFETY: hides our own window.
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        let call = self.background_call().cloned();
        let Some(live) = &mut self.live else {
            return true;
        };
        let noticed = live.preferences.get(background::NOTICE_KEY) == Some("true");
        if let Some((title, body)) = background::notice(call.as_ref(), noticed) {
            self.tray.notice(&title, &body);
            if call.is_none() {
                live.preferences.set(background::NOTICE_KEY, "true".into());
            }
        }
        true
    }

    /// Brings the window back from the tray, the taskbar or behind others.
    /// A hidden window keeps its maximized state, which `SW_RESTORE` would
    /// undo.
    pub(crate) fn show_window(&mut self) {
        // SAFETY: plain calls on our own window.
        unsafe {
            if !IsWindowVisible(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_SHOW);
            }
            if IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(self.hwnd);
        }
        self.need = true;
    }

    /// What the menu shows about the call, while there is one.
    pub(crate) fn tray_call(&self) -> Option<Call> {
        self.background_call()?;
        let audio = self.live.as_ref()?.voice.state.audio;
        Some(Call {
            muted: audio.muted,
            deafened: audio.deafened,
        })
    }

    pub(crate) fn tray_choice(&mut self, choice: Choice) {
        match choice {
            Choice::Open => return self.show_window(),
            Choice::Quit => {
                self.quit = true;
                return;
            }
            _ => {}
        }
        let Some(live) = &mut self.live else { return };
        match choice {
            Choice::Mute => live.toggle_mute(),
            Choice::Deafen => live.toggle_deafen(),
            Choice::Leave => live.leave_voice(),
            Choice::Open | Choice::Quit => {}
        }
        self.voice_changed();
    }
}

/// What a message from the icon asks for (`lp` is its `lParam`), running
/// the menu for a right-click. Called outside the app borrow.
pub fn event(hwnd: HWND, lp: LPARAM) -> Option<Choice> {
    match (lp.0 & 0xffff) as u32 {
        NIN_SELECT | NIN_BALLOONUSERCLICK => Some(Choice::Open),
        e if e == NIN_SELECT | NINF_KEY => Some(Choice::Open),
        WM_CONTEXTMENU => menu(hwnd, crate::with_app(|a| a.tray_call()).flatten()),
        _ => None,
    }
}

/// Copies `text` into a fixed, NUL-terminated field, cut to fit.
fn copy<const N: usize>(field: &mut [u16; N], text: &str) {
    let mut end = 0;
    for (slot, unit) in field[..N - 1].iter_mut().zip(text.encode_utf16()) {
        *slot = unit;
        end += 1;
    }
    field[end] = 0;
}

#[cfg(test)]
mod tests {
    #[test]
    fn long_text_is_cut_and_terminated() {
        let mut field = [1u16; 4];
        super::copy(&mut field, "abcdef");
        assert_eq!(field, [97, 98, 99, 0]);
        super::copy(&mut field, "a");
        assert_eq!(&field[..2], &[97, 0]);
    }
}
