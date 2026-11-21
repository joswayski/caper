//! "Jump to latest" over the history (`caper_client::jump`): a floating
//! button at the history's bottom right, 16 px above the composer, shown
//! more than half a screen from the newest message while the history
//! reaches the live end. Others' messages landing meanwhile are counted;
//! activating it glides to the newest message (instantly with reduced
//! motion or from far away) and follows new messages again. It rises in
//! with the menus' entrance and leaves at once. The thread pane has its own
//! (`thread::Part::Jump`), over its history's bottom right.
use crate::{App, EV_FOCUS, EV_ROWS, gfx, icons, text, theme::*};
use caper_client::jump::{self, Jump};
use std::time::Instant;
use windows::Win32::Graphics::DirectWrite::IDWriteTextLayout;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

pub const TIMER_JUMP: usize = 49;
const H: f32 = 36.0;
const ICON: f32 = 16.0;
const PAD_X: f32 = 10.0;
const GAP: f32 = 6.0;
/// Room around the button for its keyboard focus ring.
pub const RING: f32 = 3.0;

/// What a panel last drew: the state, hover, keyboard focus and scale.
pub type Drawn = (Jump, bool, bool, f32);

/// Draws a button `w` × `H` with its count `label`, inset by `RING`.
fn paint(
    g: &crate::gfx::Gfx,
    dc: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext,
    w: f32,
    label: Option<&IDWriteTextLayout>,
    (jump, hover, kb, _): Drawn,
) -> Result<()> {
    let h = H;
    unsafe {
        dc.Clear(Some(
            &windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F::default(),
        ));
    }
    let r = gfx::rect(RING, RING, RING + w, RING + h);
    gfx::fill_round(
        dc,
        r,
        RADIUS,
        if hover {
            &g.brushes.raised
        } else {
            &g.brushes.surface
        },
    );
    gfx::stroke_round(
        dc,
        r,
        RADIUS,
        if jump.unread() {
            &g.brushes.terracotta
        } else {
            &g.brushes.border
        },
    );
    if kb {
        let ring = gfx::rect(RING - 2.5, RING - 2.5, RING + w + 2.5, RING + h + 2.5);
        gfx::stroke_round(dc, ring, RADIUS + 2.0, &g.brushes.terracotta);
    }
    let ix = match label {
        Some(_) => RING + PAD_X,
        None => RING + (w - ICON) / 2.0,
    };
    let iy = RING + (h - ICON) / 2.0;
    icons::draw(
        dc,
        icons::Icon::ArrowDown,
        gfx::rect(ix, iy, ix + ICON, iy + ICON),
        TEXT,
        1.0,
    );
    if let Some(l) = label {
        let th = gfx::metrics(l).height;
        text::draw(
            dc,
            l,
            ix + ICON + GAP,
            RING + (h - th) / 2.0,
            &g.brushes.text,
        );
    }
    Ok(())
}

impl App {
    fn jump_label(&self, jump: Jump) -> Option<IDWriteTextLayout> {
        let text = jump.text()?;
        self.g
            .layout(&gfx::wide(&text), &self.g.formats.card, 1e6)
            .ok()
    }

    fn jump_width(&self, label: Option<&IDWriteTextLayout>) -> f32 {
        match label {
            Some(l) => {
                let w = gfx::metrics(l).widthIncludingTrailingWhitespace.ceil();
                PAD_X + ICON + GAP + w + PAD_X
            }
            None => H,
        }
    }

    /// The button in client DIPs, while it shows over a shown history.
    pub fn jump_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let shown = self.jump.shown
            && self.live.is_some()
            && self.c.has_conversation()
            && self.chat_shown()
            && !self.narrow.menu
            && !(self.is_narrow() && self.members_w() > 0.0)
            && self.vh() > H + 16.0;
        if !shown {
            return None;
        }
        let w = self.jump_width(self.jump_label(self.jump).as_ref());
        let x = self.list_x() + self.list_w() - 16.0 - w;
        // The history ends 16 px above the composer (and its reply).
        let y = self.history_y() + self.vh() - H;
        Some((x, y, w, H))
    }

    pub fn jump_at(&self, x: f32, y: f32) -> bool {
        self.jump_rect()
            .is_some_and(|(l, t, w, h)| x >= l && x < l + w && y >= t && y < t + h)
    }

    /// After the history moved or changed: `distance` from the newest
    /// message, the viewport's height `page`.
    pub fn sync_jump(&mut self, distance: f64, page: f64) {
        if self.jump_glide.is_some() {
            return;
        }
        let live = self
            .live
            .as_ref()
            .is_some_and(|l| !crate::live::Live::has_newer(l));
        let was = self.jump.shown;
        let mut next = self.jump;
        if next.scrolled(distance, page, self.at_end, live) {
            self.set_jump(next);
        }
        if was && !self.jump.shown {
            self.jump_lost_focus();
        }
    }

    /// Someone else's message landed at the live end.
    pub fn jump_arrived(&mut self) {
        let mut next = self.jump;
        if next.arrived() {
            self.set_jump(next);
        }
    }

    /// A conversation opened afresh: no button, no count.
    pub fn reset_jump(&mut self) {
        self.stop_glide();
        self.set_jump(Jump::default());
        self.jump_kb = false;
    }

    fn set_jump(&mut self, next: Jump) {
        if next == self.jump {
            return;
        }
        if next.shown != self.jump.shown {
            // The history's children (UI Automation) gain or lose it.
            self.events |= EV_ROWS;
        }
        self.jump = next;
        self.need = true;
    }

    /// The button went away while it had the keyboard: the composer takes it.
    fn jump_lost_focus(&mut self) {
        if std::mem::take(&mut self.jump_kb) {
            self.set_composer_focus(true);
            self.events |= EV_FOCUS;
        }
    }

    /// Gives the button the keyboard (Tab order, Narrator), or takes it away.
    pub fn set_jump_kb(&mut self, on: bool) {
        if on == self.jump_kb {
            return;
        }
        if on {
            self.set_composer_focus(false);
            self.set_thread_focus(false);
            self.set_kb(None);
            self.clear_side_focus();
        }
        self.jump_kb = on;
        self.events |= EV_FOCUS;
        self.need = true;
    }

    /// Click, Enter/Space or UIA Invoke: to the newest message.
    pub fn jump_activate(&mut self) {
        let had_kb = self.jump_kb;
        self.set_jump(Jump::default());
        self.jump_kb = false;
        if had_kb {
            self.set_composer_focus(true);
            self.events |= EV_FOCUS;
        }
        let vh = self.vh() as f64;
        let distance = self.history_max() - self.scroll_top;
        if !jump::glides(distance, vh, !crate::files::motion()) {
            self.land();
            return;
        }
        self.jump_glide = Some((self.scroll_top, Instant::now()));
        unsafe {
            SetTimer(Some(self.hwnd), TIMER_JUMP, 16, None);
        }
    }

    /// The farthest the history scrolls: its newest message at the bottom.
    fn history_max(&self) -> f64 {
        self.rows.bottom() + self.foot_h() as f64 - self.vh() as f64
    }

    pub fn jump_tick(&mut self) {
        self.thread_glide_tick();
        // Something else (a send, End) may already have reached the end.
        if let Some((from, start)) = self.jump_glide {
            let t = start.elapsed().as_secs_f64() / jump::GLIDE.as_secs_f64();
            if self.at_end {
                self.jump_glide = None;
            } else if t >= 1.0 {
                self.jump_glide = None;
                self.land();
            } else {
                self.scroll_top = jump::glide_at(from, self.history_max(), t);
                self.at_end = false;
                self.need = true;
            }
        }
        self.stop_timer_when_idle();
    }

    /// The reader scrolled (or the conversation changed) mid-glide.
    pub fn stop_glide(&mut self) {
        self.jump_glide = None;
        self.stop_timer_when_idle();
    }

    fn stop_timer_when_idle(&self) {
        if self.jump_glide.is_none() && self.thread.as_ref().is_none_or(|t| t.jump_glide.is_none())
        {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_JUMP);
            }
        }
    }

    fn land(&mut self) {
        self.stop_autoscroll();
        self.at_end = true;
        self.need = true;
    }

    pub fn track_jump_hover(&mut self) {
        let hover = self.pointer.is_some_and(|(x, y)| self.jump_at(x, y));
        if hover != self.jump_hover {
            self.jump_hover = hover;
            self.need = true;
        }
    }

    /// Places and draws the button when its look changed.
    pub fn draw_jump(&mut self) -> Result<()> {
        let Some((x, y, w, h)) = self.jump_rect() else {
            self.jump_drawn = None;
            self.jump_p.offset(-100_000.0, 0.0)?;
            return self.draw_thread_jump();
        };
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let (pw, ph) = (w + 2.0 * RING, h + 2.0 * RING);
        let created = self.jump_p.ensure(&self.g, px(pw), px(ph))?;
        self.jump_p
            .offset(((x - RING) * s).round(), ((y - RING) * s).round())?;
        let look = (self.jump, self.jump_hover, self.jump_kb, s);
        if !created && self.jump_drawn == Some(look) {
            return Ok(());
        }
        self.jump_drawn = Some(look);
        let label = self.jump_label(self.jump);
        let g = &self.g;
        self.jump_p
            .draw(s, |dc| paint(g, dc, w, label.as_ref(), look))?;
        self.draw_thread_jump()
    }

    // ---- the thread's ----

    /// The thread's button in client DIPs, while it shows.
    pub fn thread_jump_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let t = self.thread.as_ref()?;
        let vh = self.thread_vh();
        if !t.jump.shown || !self.thread_shown() || vh <= H + 16.0 {
            return None;
        }
        let w = self.jump_width(self.jump_label(t.jump).as_ref());
        let x = self.thread_x() + self.thread_w() - 16.0 - w;
        Some((x, HEADER_H + vh - H, w, H))
    }

    /// After the thread scrolled or changed, before it draws.
    pub fn sync_thread_jump(&mut self) {
        let vh = self.thread_vh();
        let Some(t) = &mut self.thread else { return };
        if t.jump_glide.is_some() {
            return;
        }
        let max = (t.content_h() - vh).max(0.0);
        if t.jump
            .scrolled((max - t.scroll) as f64, vh as f64, t.at_end, true)
        {
            self.dirty.thread = true;
            self.events |= EV_ROWS;
            self.need = true;
        }
    }

    /// Someone else's reply landed at the thread's end.
    pub fn thread_jump_arrived(&mut self) {
        if let Some(t) = &mut self.thread
            && t.jump.arrived()
        {
            self.need = true;
        }
    }

    /// The thread's button: to its newest reply, then following it.
    pub fn thread_jump_activate(&mut self) {
        let vh = self.thread_vh();
        let Some(t) = &mut self.thread else { return };
        t.jump = Jump::default();
        let max = (t.content_h() - vh).max(0.0);
        if jump::glides((max - t.scroll) as f64, vh as f64, !crate::files::motion()) {
            t.jump_glide = Some((t.scroll, Instant::now()));
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_JUMP, 16, None);
            }
        } else {
            t.scroll = max;
            t.at_end = true;
        }
        self.dirty.thread = true;
        self.events |= EV_ROWS;
        self.need = true;
    }

    /// One step of the thread's glide.
    fn thread_glide_tick(&mut self) {
        let vh = self.thread_vh();
        let Some(t) = &mut self.thread else { return };
        let Some((from, start)) = t.jump_glide.filter(|_| !t.at_end) else {
            t.jump_glide = None;
            return;
        };
        let max = (t.content_h() - vh).max(0.0);
        let k = start.elapsed().as_secs_f64() / jump::GLIDE.as_secs_f64();
        self.dirty.thread = true;
        self.need = true;
        if k >= 1.0 {
            t.jump_glide = None;
            t.scroll = max;
            t.at_end = true;
            return;
        }
        t.scroll = jump::glide_at(from as f64, max as f64, k) as f32;
        t.at_end = false;
    }

    fn draw_thread_jump(&mut self) -> Result<()> {
        let Some((x, y, w, h)) = self.thread_jump_rect() else {
            self.thread_jump_drawn = None;
            return self.thread_jump_p.offset(-100_000.0, 0.0);
        };
        let Some(t) = &self.thread else {
            return Ok(());
        };
        let s = self.scale;
        let look = (t.jump, t.hover == Some(crate::thread::Part::Jump), false, s);
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let created = self
            .thread_jump_p
            .ensure(&self.g, px(w + 2.0 * RING), px(h + 2.0 * RING))?;
        self.thread_jump_p
            .offset(((x - RING) * s).round(), ((y - RING) * s).round())?;
        if !created && self.thread_jump_drawn == Some(look) {
            return Ok(());
        }
        self.thread_jump_drawn = Some(look);
        let label = self.jump_label(look.0);
        let g = &self.g;
        self.thread_jump_p
            .draw(s, |dc| paint(g, dc, w, label.as_ref(), look))
    }
}
