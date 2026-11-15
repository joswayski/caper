//! Persistent pane widths; window clamping never overwrites the preference.
use crate::overlay::Rect;
use crate::settings::preferences::Preferences;
use crate::{App, Dirty, EV_FOCUS, EV_SCREEN};
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const HELP: &str = "Drag to resize. Arrow keys to adjust. Double-click to reset.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Sidebar,
    Members,
    Thread,
}

impl Pane {
    pub const ALL: [Self; 3] = [Self::Sidebar, Self::Members, Self::Thread];

    pub fn name(self) -> &'static str {
        match self {
            Self::Sidebar => "Channel sidebar width",
            Self::Members => "Member list width",
            Self::Thread => "Thread width",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar-width-v1",
            Self::Members => "members-width-v1",
            Self::Thread => "thread-width-v1",
        }
    }

    pub fn min(self) -> f32 {
        if self == Self::Thread { 300.0 } else { 220.0 }
    }

    fn max(self) -> f32 {
        if self == Self::Thread { 480.0 } else { 440.0 }
    }

    fn default(self) -> f32 {
        match self {
            Self::Sidebar => crate::theme::SIDEBAR_W,
            Self::Members => crate::theme::MEMBERS_W,
            Self::Thread => crate::thread::THREAD_W,
        }
    }

    fn direction(self) -> f32 {
        if self == Self::Sidebar { 1.0 } else { -1.0 }
    }
}

pub struct State {
    widths: [f32; 3],
    pub focus: Option<Pane>,
    drag: Option<(Pane, f32, f32)>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            widths: Pane::ALL.map(Pane::default),
            focus: None,
            drag: None,
        }
    }
}

impl State {
    pub fn restore(&mut self, prefs: &Preferences) {
        for p in Pane::ALL {
            if let Some(width) = prefs.get(p.key()).and_then(|s| s.parse().ok()) {
                self.set(p, width);
            }
        }
    }

    pub fn width(&self, p: Pane) -> f32 {
        self.widths[p as usize]
    }

    fn set(&mut self, p: Pane, width: f32) {
        if width.is_finite() {
            self.widths[p as usize] = width.clamp(p.min(), p.max());
        }
    }

    fn save(&self, prefs: &mut Preferences) {
        for p in Pane::ALL {
            prefs.set(p.key(), self.width(p).to_string());
        }
    }
}

impl App {
    pub fn sidebar_w(&self) -> f32 {
        if self.is_narrow() {
            return (self.w - crate::theme::RAIL_W).max(0.0);
        }
        self.panes
            .width(Pane::Sidebar)
            .min(self.pane_max(Pane::Sidebar))
    }

    pub fn member_width(&self) -> f32 {
        if self.is_narrow() {
            return (self.w - 64.0).clamp(0.0, 280.0);
        }
        self.panes
            .width(Pane::Members)
            .min(self.pane_max(Pane::Members))
    }

    pub fn pane_max(&self, p: Pane) -> f32 {
        match p {
            Pane::Sidebar => (self.w - 380.0).clamp(220.0, 440.0),
            Pane::Members => {
                let stage = self.w - crate::theme::RAIL_W - self.sidebar_w();
                (stage - if stage >= 540.0 { 320.0 } else { 64.0 }).clamp(220.0, 440.0)
            }
            Pane::Thread => 480.0,
        }
    }

    pub fn pane_value(&self, p: Pane) -> f32 {
        match p {
            Pane::Sidebar => self.sidebar_w(),
            Pane::Members => self.member_width(),
            Pane::Thread => self.thread_w(),
        }
    }

    pub fn pane_rect(&self, p: Pane) -> Option<Rect> {
        if self.is_narrow()
            || self.signin.is_some()
            || self.dialog.is_some()
            || self.settings.is_some()
            || self.viewer.is_some()
            || self.attachment_editor.is_some()
            || self.pins.is_some()
        {
            return None;
        }
        let x = match p {
            Pane::Sidebar => self.list_x(),
            Pane::Members if self.members_w() > 0.0 => self.members_x(),
            Pane::Thread if self.thread.is_some() => self.thread_x(),
            _ => return None,
        };
        let y = if p == Pane::Members {
            self.members_y()
        } else {
            0.0
        };
        Some((x - 3.0, y, 7.0, self.h - y))
    }

    pub fn pane_at(&self, x: f32, y: f32) -> Option<Pane> {
        Pane::ALL.into_iter().find(|&p| {
            self.pane_rect(p)
                .is_some_and(|(l, t, w, h)| x >= l && x < l + w && y >= t && y < t + h)
        })
    }

    pub fn focus_pane(&mut self, p: Pane) {
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        self.set_member_search_focus(false);
        self.set_kb(None);
        self.panes.focus = Some(p);
        self.panes_changed();
        self.events |= EV_FOCUS;
    }

    pub fn clear_pane_focus(&mut self) {
        if self.panes.focus.take().is_some() {
            self.dirty.sidebar = true;
            self.dirty.members = true;
            self.dirty.thread = true;
            self.events |= EV_FOCUS;
            self.need = true;
        }
    }

    fn panes_changed(&mut self) {
        self.dirty = Dirty::all();
        self.events |= EV_SCREEN;
        self.need = true;
    }

    fn save_panes(&mut self) {
        if let Some(live) = &mut self.live {
            self.panes.save(&mut live.preferences);
        }
    }

    pub fn set_pane_width(&mut self, p: Pane, width: f32, save: bool) {
        self.panes.set(p, width.min(self.pane_max(p)));
        self.panes_changed();
        if save {
            self.save_panes();
        }
    }

    pub fn pane_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        shift: bool,
    ) -> Option<LRESULT> {
        if self
            .panes
            .focus
            .is_some_and(|p| self.pane_rect(p).is_none())
        {
            self.clear_pane_focus();
        }
        let hit = self.pane_at(x, y);
        match msg {
            WM_LBUTTONDOWN | WM_LBUTTONDBLCLK if hit.is_some() => {
                let p = hit?;
                self.stop_autoscroll();
                self.focus_pane(p);
                if msg == WM_LBUTTONDBLCLK {
                    self.set_pane_width(p, p.default(), true);
                } else {
                    self.panes.drag = Some((p, x, self.pane_value(p)));
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                }
            }
            WM_MOUSEMOVE if self.panes.drag.is_some() => {
                let (p, start, width) = self.panes.drag?;
                self.set_pane_width(p, width + (x - start) * p.direction(), false);
            }
            WM_LBUTTONUP | WM_CAPTURECHANGED | WM_KILLFOCUS | WM_CANCELMODE
                if self.panes.drag.is_some() =>
            {
                self.panes.drag = None;
                self.save_panes();
                if msg != WM_CAPTURECHANGED {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                }
                if msg != WM_LBUTTONUP {
                    return None;
                }
            }
            WM_SETCURSOR if hit.is_some() || self.panes.drag.is_some() => {
                unsafe {
                    SetCursor(LoadCursorW(None, IDC_HAND).ok());
                }
                return Some(LRESULT(1));
            }
            WM_KEYDOWN if self.panes.focus.is_some() => {
                let p = self.panes.focus?;
                let value = self.pane_value(p);
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_LEFT => self.set_pane_width(p, value - 10.0 * p.direction(), true),
                    VK_RIGHT => self.set_pane_width(p, value + 10.0 * p.direction(), true),
                    VK_HOME => self.set_pane_width(p, p.min(), true),
                    VK_END => self.set_pane_width(p, self.pane_max(p), true),
                    VK_TAB => {
                        let mut parts: Vec<_> = Pane::ALL
                            .into_iter()
                            .filter(|&p| self.pane_rect(p).is_some())
                            .collect();
                        if shift {
                            parts.reverse();
                        }
                        let next = parts
                            .iter()
                            .position(|q| *q == p)
                            .and_then(|i| parts.get(i + 1))
                            .copied();
                        if let Some(p) = next {
                            self.focus_pane(p);
                        } else {
                            self.panes.focus = None;
                            self.set_composer_focus(true);
                            self.panes_changed();
                        }
                    }
                    VK_ESCAPE => {
                        self.panes.drag = None;
                        self.panes.focus = None;
                        self.save_panes();
                        unsafe {
                            let _ = ReleaseCapture();
                        }
                        self.set_composer_focus(true);
                        self.panes_changed();
                    }
                    _ => return None,
                }
            }
            WM_KEYDOWN
                if wp.0 == VK_TAB.0 as usize
                    && self.composer.focused
                    && !shift
                    && !self.is_narrow() =>
            {
                self.focus_pane(Pane::Sidebar);
            }
            WM_LBUTTONDOWN => {
                self.clear_pane_focus();
                return None;
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_widths_survive_relaunch_and_bad_saved_values_are_bounded() {
        let root = std::env::temp_dir().join(format!("caper-panes-{}", uuid::Uuid::new_v4()));
        let mut prefs = Preferences::load(Some(root.clone()));
        let mut state = State::default();
        for (p, width) in [
            (Pane::Sidebar, 327.5),
            (Pane::Members, 391.0),
            (Pane::Thread, 456.0),
        ] {
            state.set(p, width);
        }
        state.save(&mut prefs);
        let mut restored = State::default();
        restored.restore(&Preferences::load(Some(root.clone())));
        assert_eq!(restored.widths, [327.5, 391.0, 456.0]);
        prefs.set("sidebar-width-v1", "NaN".into());
        prefs.set("members-width-v1", "9999".into());
        prefs.set("thread-width-v1", "-1".into());
        restored.restore(&prefs);
        assert_eq!(restored.widths, [327.5, 440.0, 300.0]);
        std::fs::remove_dir_all(root).unwrap();
    }
}
