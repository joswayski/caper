//! Link preview players (YouTube, Vimeo, Twitch), as the egui client plays
//! them on Windows: after Play, WebView2 shows Caper's `play` page in a
//! child window kept exactly over the card's player box, with Caper's player
//! safety rules (`caper_web_player::engine`). Nothing else uses a web engine.
//!
//! One player at a time. The view hides (still playing) while its box is
//! not fully inside the visible history or a menu, dialog, picker, settings
//! or the media viewer is open; the card then shows its poster marked
//! "Playing", so the layout never moves. It goes away when another starts,
//! the conversation changes, its card has been out of the laid-out history
//! for [`FORGET_AFTER`], or the app closes. Without the WebView2 runtime, or
//! when it fails to start, Play opens the card's link in the browser.
//!
//! The history is composed below child windows (`CreateTargetForHwnd` with
//! `topmost` false), so the view shows above it.

use std::num::NonZeroIsize;
use std::rc::Rc;
use std::time::{Duration, Instant};

use caper_web_player::engine::{self as shared, wry};
use raw_window_handle::{
    HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

/// Rechecks a card that left the laid-out history.
pub const TIMER_WEB_PLAYER: usize = 31;
/// How long a playing card may stay out of the laid-out history (scrolled
/// far away, or its message gone) before its player closes; egui's 90
/// frames.
pub const FORGET_AFTER: Duration = Duration::from_millis(1500);

/// Which player block: its message, card and picture within the card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Key {
    pub message: String,
    pub card: usize,
    pub picture: usize,
}

/// Physical pixels in the window's client area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// `(left, top, width, height)` in DIPs at `scale`, edges rounded so
/// neighbouring boxes never gap.
pub fn physical((left, top, width, height): (f32, f32, f32, f32), scale: f32) -> Bounds {
    let x = (left * scale).round();
    let y = (top * scale).round();
    let right = ((left + width) * scale).round();
    let bottom = ((top + height) * scale).round();
    Bounds {
        x: x as i32,
        y: y as i32,
        width: (right - x).max(1.0) as u32,
        height: (bottom - y).max(1.0) as u32,
    }
}

/// Whether `rect` lies fully inside `clip` (half a DIP of slack for
/// rounding at the edges).
pub fn inside(rect: (f32, f32, f32, f32), clip: (f32, f32, f32, f32)) -> bool {
    let (x, y, w, h) = rect;
    let (cx, cy, cw, ch) = clip;
    x >= cx - 0.5 && y >= cy - 0.5 && x + w <= cx + cw + 0.5 && y + h <= cy + ch + 0.5
}

/// Caper's window as wry's parent.
struct Parent(HWND);

impl HasWindowHandle for Parent {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let hwnd = NonZeroIsize::new(self.0.0 as isize).ok_or(HandleError::Unavailable)?;
        let raw = RawWindowHandle::Win32(Win32WindowHandle::new(hwnd));
        // The window outlives every player (they close before it does).
        Ok(unsafe { WindowHandle::borrow_raw(raw) })
    }
}

/// The app's one player.
pub struct Player {
    pub key: Key,
    /// The `play` page (already checked with `play_page`).
    pub page: url::Url,
    /// What opens in the browser when it cannot play here.
    pub fallback: String,
    /// The conversation it started in; leaving it closes the player.
    pub conversation: Option<String>,
    /// None until the engine is created (outside the app's borrow, because
    /// WebView2 pumps window messages while it starts).
    view: Option<wry::WebView>,
    sent_bounds: Option<Bounds>,
    sent_shown: Option<bool>,
    /// When its card left the laid-out history.
    unseen: Option<Instant>,
    /// Its engine was asked for.
    requested: bool,
}

impl Player {
    pub fn new(key: Key, page: url::Url, fallback: String, conversation: Option<String>) -> Self {
        Self {
            key,
            page,
            fallback,
            conversation,
            view: None,
            sent_bounds: None,
            sent_shown: None,
            unseen: None,
            requested: false,
        }
    }

    /// Still waiting for its engine.
    pub fn starting(&self) -> bool {
        self.view.is_none()
    }

    pub fn attach(&mut self, view: wry::WebView) {
        self.view = Some(view);
        self.sent_bounds = None;
        self.sent_shown = Some(false);
    }

    /// Moves the view over its box and shows it, or hides it where it is.
    pub fn place(&mut self, bounds: Option<Bounds>) {
        let Some(view) = &self.view else {
            return;
        };
        if let Some(bounds) = bounds
            && self.sent_bounds != Some(bounds)
        {
            let _ = view.set_bounds(rect(bounds));
            self.sent_bounds = Some(bounds);
        }
        let shown = bounds.is_some();
        if self.sent_shown != Some(shown) {
            let _ = view.set_visible(shown);
            self.sent_shown = Some(shown);
        }
    }

    /// A click outside the view: keys come back to Caper.
    pub fn blur(&self) {
        if let Some(view) = &self.view {
            let _ = view.focus_parent();
        }
    }
}

fn rect(bounds: Bounds) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::PhysicalPosition::new(bounds.x, bounds.y).into(),
        size: wry::dpi::PhysicalSize::new(bounds.width, bounds.height).into(),
    }
}

/// Whether the WebView2 runtime is installed (a separate system component).
pub fn available() -> bool {
    wry::webview_version().is_ok()
}

/// Starts WebView2 on `page` as a hidden child of `hwnd`; the next frame
/// places it. Links followed out of the player open in the system browser.
/// Must run outside the app's borrow: it pumps window messages until
/// WebView2 is ready.
pub fn create(hwnd: HWND, page: &url::Url) -> Option<wry::WebView> {
    let open: shared::Open = Rc::new(|url: String| crate::open_link(&url));
    shared::builder(page, open)
        .with_bounds(rect(Bounds {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }))
        .with_visible(false)
        .build_as_child(&Parent(hwnd))
        .ok()
}

impl crate::App {
    /// Play on a link preview's player: nothing when it already plays;
    /// otherwise it replaces whatever plays. Without a Caper `play` page or
    /// the WebView2 runtime, the card's link opens in the browser.
    pub fn play_embed(&mut self, key: Key, play: Option<&str>, fallback: String) {
        if self.web_player.as_ref().is_some_and(|p| p.key == key) {
            return;
        }
        self.stop_web_player();
        match play
            .and_then(caper_web_player::play_page)
            .filter(|_| available())
        {
            Some(page) => {
                let conversation = self.ids.open.clone();
                self.web_player = Some(Player::new(key, page, fallback, conversation));
                self.web_player_redraw();
            }
            None => open(&fallback),
        }
    }

    pub fn stop_web_player(&mut self) {
        if self.web_player.take().is_some() {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_WEB_PLAYER);
            }
            self.web_player_redraw();
        }
    }

    /// The card marks (or stops marking) its poster "Playing".
    fn web_player_redraw(&mut self) {
        self.drawn = None;
        self.need = true;
    }

    /// The playing card's row, card and picture, for drawing.
    pub fn web_player_spot(&self) -> Option<(usize, usize, usize)> {
        let p = self.web_player.as_ref()?;
        let i = *self.ids.rows.get(&p.key.message)?;
        Some((i, p.key.card, p.key.picture))
    }

    /// The page whose engine the main loop should create now (once).
    pub fn web_player_to_start(&mut self) -> Option<url::Url> {
        let p = self
            .web_player
            .as_mut()
            .filter(|p| p.starting() && !p.requested)?;
        p.requested = true;
        Some(p.page.clone())
    }

    /// The engine for `page` started (or not: the browser plays it).
    pub fn web_player_started(&mut self, page: &url::Url, view: Option<wry::WebView>) {
        let Some(p) = self
            .web_player
            .as_mut()
            .filter(|p| p.starting() && p.page == *page)
        else {
            // Replaced or closed meanwhile; the view goes with `view`.
            return;
        };
        match view {
            Some(view) => {
                p.attach(view);
                self.need = true;
            }
            None => {
                let fallback = p.fallback.clone();
                self.stop_web_player();
                open(&fallback);
            }
        }
    }

    /// The player's box in client DIPs, while its card is laid out.
    fn web_player_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let key = &self.web_player.as_ref()?.key;
        let i = *self.ids.rows.get(&key.message)?;
        let m = self.c.messages.get(i).filter(|m| m.id == key.message)?;
        let r = self
            .rows
            .embed_picture(i, m.grouped, key.card, key.picture)?;
        let y = crate::theme::HEADER_H + (self.rows.top[i] - self.scroll_top) as f32;
        Some((
            self.list_x() + r.left,
            y + r.top,
            r.right - r.left,
            r.bottom - r.top,
        ))
    }

    /// Anything drawn over the history that the view would cover.
    fn web_player_covered(&self) -> bool {
        self.viewer.is_some()
            || self.dialog.is_some()
            || self.menu.is_some()
            || self.attachment_editor.is_some()
            || self.picker.is_some()
            || self.gifs.is_some()
            || self.settings.is_some()
            || self.account_menu.is_some()
            || self.dm_menu.is_some()
            || self.card.is_some()
            || self.signin.is_some()
    }

    /// After each frame: closes the player when its conversation was left or
    /// its card stayed away; otherwise shows it over its box or hides it.
    pub fn place_web_player(&mut self) {
        let Some(p) = &self.web_player else {
            return;
        };
        if p.conversation != self.ids.open {
            return self.stop_web_player();
        }
        let rect = self.web_player_rect();
        let history = (
            self.list_x(),
            crate::theme::HEADER_H,
            self.list_w(),
            self.vh(),
        );
        let shown = rect.filter(|r| !self.web_player_covered() && inside(*r, history));
        let bounds = shown.map(|r| physical(r, self.scale));
        let hwnd = self.hwnd;
        let Some(p) = &mut self.web_player else {
            return;
        };
        if rect.is_some() {
            p.unseen = None;
        } else {
            let since = *p.unseen.get_or_insert_with(Instant::now);
            let left = FORGET_AFTER.saturating_sub(since.elapsed());
            if left.is_zero() {
                return self.stop_web_player();
            }
            unsafe {
                SetTimer(
                    Some(hwnd),
                    TIMER_WEB_PLAYER,
                    left.as_millis() as u32 + 1,
                    None,
                );
            }
        }
        p.place(bounds);
    }
}

/// The card's link in the browser (http(s) only, as message links).
fn open(url: &str) {
    if caper_web_player::openable(url) {
        crate::open_link(url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_bounds_follow_the_scale_and_never_gap() {
        assert_eq!(
            physical((371.0, 500.0, 382.0, 215.0), 1.0),
            Bounds {
                x: 371,
                y: 500,
                width: 382,
                height: 215
            }
        );
        assert_eq!(
            physical((10.25, 20.5, 400.0, 225.0), 1.5),
            Bounds {
                x: 15,
                y: 31,
                width: 600,
                height: 337
            }
        );
        let tiny = physical((1.0, 1.0, 0.1, 0.1), 2.0);
        assert_eq!((tiny.width, tiny.height), (1, 1));
    }

    #[test]
    fn the_view_shows_only_when_its_box_is_fully_inside_the_history() {
        let history = (340.0, 55.0, 720.0, 350.0);
        assert!(inside((415.0, 113.0, 400.0, 225.0), history));
        // Rounding at the edge still counts as inside.
        assert!(inside((415.0, 55.2, 400.0, 225.0), history));
        for clipped in [
            (415.0, 40.0, 400.0, 225.0),
            (415.0, 300.0, 400.0, 225.0),
            (900.0, 113.0, 400.0, 225.0),
            (300.0, 113.0, 400.0, 225.0),
        ] {
            assert!(!inside(clipped, history), "{clipped:?}");
        }
    }
}
