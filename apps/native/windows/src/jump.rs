//! "You're viewing older messages" over the history (`caper_client::jump`),
//! as Discord and web: a floating toast centered 12 px above the history's
//! bottom, just over the composer, with its label and a primary "Jump to
//! latest" button. Over live history it shows once ten or more messages lie
//! below the viewport and counts others' messages landing meanwhile; the
//! button glides to the newest message (instantly with reduced motion or
//! from far away) and follows new messages again. In a detached window
//! (after View original or a pin's Go) it always shows, and the button
//! reloads the latest page and lands at its end. It rises in with the menus'
//! entrance and leaves at once. Only the button is a control: the label is
//! text, and the rest of the toast just keeps the pointer off the rows
//! beneath it. The thread pane has its own (`thread::Part::Jump`), over its
//! history.
use crate::narrow::contains;
use crate::overlay::Rect;
use crate::{App, EV_FOCUS, EV_ROWS, gfx, text, theme::*};
use caper_client::jump::{self, Jump};
use std::time::Instant;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext;
use windows::Win32::Graphics::DirectWrite::IDWriteTextLayout;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

pub const TIMER_JUMP: usize = 49;
/// Inside its 1 px border: 14 px before the label, 6 px around the button,
/// and 14 px between them.
const LINE: f32 = 1.0;
const PAD_L: f32 = 14.0;
const PAD: f32 = 6.0;
const GAP: f32 = 14.0;
/// The button: 30 px tall, 12 px either side of its label.
const BUTTON_H: f32 = 30.0;
const BUTTON_PAD: f32 = 12.0;
/// From the history's bottom, and the least room left at either side.
const INSET: f32 = 12.0;
/// Room around the toast on its panel for its drop shadow (0 8px 24px,
/// black at 35%, drawn as stacked translucent layers).
const MARGIN: f32 = 24.0;
const SHADOW_Y: f32 = 8.0;
const SHADOW_LAYERS: u8 = 8;

/// What a panel last drew: the state, the button's hover, keyboard focus,
/// the scale and the toast's size.
pub type Drawn = (Jump, bool, bool, f32, (f32, f32));

/// A toast laid out for the width it may take.
struct Toast {
    label: IDWriteTextLayout,
    button: IDWriteTextLayout,
    w: f32,
    h: f32,
    /// The label's size and the button's width.
    lw: f32,
    lh: f32,
    bw: f32,
}

impl Toast {
    /// At most `room` wide: the label wraps rather than truncating.
    fn new(g: &gfx::Gfx, jump: Jump, room: f32) -> Option<Self> {
        let button = g
            .layout(&gfx::wide(jump::BUTTON), &g.formats.toast_button, 1e6)
            .ok()?;
        let bw = gfx::metrics(&button)
            .widthIncludingTrailingWhitespace
            .ceil()
            + 2.0 * BUTTON_PAD;
        let max = (room - 2.0 * LINE - PAD_L - GAP - bw - PAD).max(48.0);
        let label = g
            .layout(&gfx::wide(&jump.text()), &g.formats.toast, max)
            .ok()?;
        let m = gfx::metrics(&label);
        let (lw, lh) = (m.width.min(max).ceil(), m.height.ceil());
        Some(Self {
            label,
            button,
            w: 2.0 * LINE + PAD_L + lw + GAP + bw + PAD,
            h: 2.0 * LINE + 2.0 * PAD + lh.max(BUTTON_H),
            lw,
            lh,
            bw,
        })
    }

    /// The label, from the toast's top left.
    fn label_at(&self) -> Rect {
        (LINE + PAD_L, (self.h - self.lh) / 2.0, self.lw, self.lh)
    }

    /// The button, from the toast's top left.
    fn button_at(&self) -> Rect {
        (
            self.w - LINE - PAD - self.bw,
            (self.h - BUTTON_H) / 2.0,
            self.bw,
            BUTTON_H,
        )
    }
}

/// `r` (from a toast's top left) in client DIPs, the toast at (x, y).
fn at((x, y): (f32, f32), (l, t, w, h): Rect) -> Rect {
    (x + l, y + t, w, h)
}

/// Draws the toast `t` on its panel, inset by `MARGIN`.
fn paint(g: &gfx::Gfx, dc: &ID2D1DeviceContext, t: &Toast, look: Drawn) -> Result<()> {
    let (jump, hover, kb, ..) = look;
    unsafe {
        dc.Clear(Some(&D2D1_COLOR_F::default()));
    }
    let (x, y) = (MARGIN, MARGIN);
    let shade_alpha = if crate::theme::is_light() { 0.14 } else { 0.35 };
    let shade = unsafe {
        dc.CreateSolidColorBrush(
            &gfx::color(0x000000, shade_alpha / f32::from(SHADOW_LAYERS)),
            None,
        )?
    };
    for k in (1..=SHADOW_LAYERS).rev() {
        let s = f32::from(k) * (MARGIN - SHADOW_Y) / f32::from(SHADOW_LAYERS);
        let r = gfx::rect(x - s, y + SHADOW_Y - s, x + t.w + s, y + SHADOW_Y + t.h + s);
        gfx::fill_round(dc, r, RADIUS_CARD + s, &shade);
    }
    let r = gfx::rect(x, y, x + t.w, y + t.h);
    gfx::fill_round(dc, r, RADIUS_CARD, &g.brushes.raised);
    gfx::stroke_round(
        dc,
        r,
        RADIUS_CARD,
        if jump.unread() {
            &g.brushes.terracotta
        } else {
            &g.brushes.border
        },
    );
    let (lx, ly, ..) = t.label_at();
    text::draw(dc, &t.label, x + lx, y + ly, &g.brushes.text);
    // The primary button: action fill with on_action label, brighter on hover.
    let (bx, by, bw, bh) = t.button_at();
    let b = gfx::rect(x + bx, y + by, x + bx + bw, y + by + bh);
    let fill = g.solid(if hover {
        crate::theme::action_hover()
    } else {
        crate::theme::action()
    })?;
    gfx::fill_round(dc, b, RADIUS, &fill);
    let th = gfx::metrics(&t.button).height;
    text::draw(
        dc,
        &t.button,
        b.left + BUTTON_PAD,
        b.top + (bh - th) / 2.0,
        &g.brushes.on_action,
    );
    if kb {
        let ring = gfx::rect(b.left - 3.0, b.top - 3.0, b.right + 3.0, b.bottom + 3.0);
        gfx::stroke_round(dc, ring, RADIUS + 3.0, &g.brushes.terracotta);
    }
    Ok(())
}

/// Places panel `p`'s toast at client (x, y) and draws it when its look changed.
fn place(
    p: &mut crate::Panel,
    drawn: &mut Option<Drawn>,
    g: &gfx::Gfx,
    (x, y): (f32, f32),
    t: &Toast,
    look: Drawn,
) -> Result<()> {
    let s = look.3;
    let px = |v: f32| (v * s).ceil().max(1.0) as u32;
    let created = p.ensure(g, px(t.w + 2.0 * MARGIN), px(t.h + 2.0 * MARGIN))?;
    p.offset(((x - MARGIN) * s).round(), ((y - MARGIN) * s).round())?;
    if !created && *drawn == Some(look) {
        return Ok(());
    }
    *drawn = Some(look);
    p.draw(s, |dc| paint(g, dc, t, look))
}

impl App {
    /// The toast and its top left in client DIPs, while it shows over a
    /// shown history.
    fn jump_toast(&self) -> Option<((f32, f32), Toast)> {
        let shown = self.jump.shown
            && self.live.is_some()
            && self.c.has_conversation()
            && self.chat_shown()
            && !self.narrow.menu
            && !(self.is_narrow() && self.members_w() > 0.0);
        if !shown {
            return None;
        }
        let t = Toast::new(&self.g, self.jump, self.list_w() - 2.0 * INSET)?;
        if self.vh() <= t.h + 2.0 * INSET {
            return None;
        }
        let x = self.list_x() + (self.list_w() - t.w) / 2.0;
        let y = self.history_y() + self.vh() - INSET - t.h;
        Some(((x, y), t))
    }

    /// The whole toast in client DIPs, while it shows.
    pub fn jump_rect(&self) -> Option<Rect> {
        self.jump_toast().map(|((x, y), t)| (x, y, t.w, t.h))
    }

    /// Its Jump to latest button in client DIPs.
    pub fn jump_button_rect(&self) -> Option<Rect> {
        self.jump_toast().map(|(o, t)| at(o, t.button_at()))
    }

    /// Its label in client DIPs.
    pub fn jump_label_rect(&self) -> Option<Rect> {
        self.jump_toast().map(|(o, t)| at(o, t.label_at()))
    }

    /// Whether (x, y) is on the toast, which floats over the rows.
    pub fn jump_at(&self, x: f32, y: f32) -> bool {
        self.jump_rect().is_some_and(|r| contains(r, x, y))
    }

    /// Whether (x, y) is on its button.
    pub fn jump_button_at(&self, x: f32, y: f32) -> bool {
        self.jump_button_rect().is_some_and(|r| contains(r, x, y))
    }

    /// After the history moved or changed: `below`, the messages wholly
    /// below the viewport.
    pub fn sync_jump(&mut self, below: usize) {
        if self.jump_glide.is_some() {
            return;
        }
        let live = self
            .live
            .as_ref()
            .is_some_and(|l| !crate::live::Live::has_newer(l));
        let was = self.jump.shown;
        let mut next = self.jump;
        if next.scrolled(below, self.at_end, live) {
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

    /// A conversation opened afresh: no toast, no count.
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

    /// The toast went away while its button had the keyboard: the composer
    /// takes it.
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

    /// Click, Enter/Space or UIA Invoke on the button: to the newest
    /// message, or in a detached window the latest page.
    pub fn jump_activate(&mut self) {
        if std::mem::take(&mut self.jump_kb) {
            self.set_composer_focus(true);
            self.events |= EV_FOCUS;
        }
        if self.live.as_ref().is_some_and(crate::live::Live::has_newer) {
            // The toast stays until the latest page replaces the window.
            self.load_latest();
            return;
        }
        self.set_jump(Jump::default());
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
        let hover = self.pointer.is_some_and(|(x, y)| self.jump_button_at(x, y));
        if hover != self.jump_hover {
            self.jump_hover = hover;
            self.need = true;
        }
    }

    /// Places and draws the toast when its look changed.
    pub fn draw_jump(&mut self) -> Result<()> {
        let Some((o, t)) = self.jump_toast() else {
            self.jump_drawn = None;
            self.jump_p.offset(-100_000.0, 0.0)?;
            return self.draw_thread_jump();
        };
        let look = (
            self.jump,
            self.jump_hover,
            self.jump_kb,
            self.scale,
            (t.w, t.h),
        );
        place(&mut self.jump_p, &mut self.jump_drawn, &self.g, o, &t, look)?;
        self.draw_thread_jump()
    }

    // ---- the thread's ----

    /// The thread's toast and its top left in client DIPs, while it shows.
    fn thread_jump_toast(&self) -> Option<((f32, f32), Toast)> {
        let p = self.thread.as_ref()?;
        if !p.jump.shown || !self.thread_shown() {
            return None;
        }
        let t = Toast::new(&self.g, p.jump, self.thread_w() - 2.0 * INSET)?;
        let vh = self.thread_vh();
        if vh <= t.h + 2.0 * INSET {
            return None;
        }
        let x = self.thread_x() + (self.thread_w() - t.w) / 2.0;
        Some(((x, HEADER_H + vh - INSET - t.h), t))
    }

    /// The thread's whole toast in client DIPs, while it shows.
    pub fn thread_jump_rect(&self) -> Option<Rect> {
        self.thread_jump_toast().map(|((x, y), t)| (x, y, t.w, t.h))
    }

    /// The thread toast's button in client DIPs.
    pub fn thread_jump_button_rect(&self) -> Option<Rect> {
        self.thread_jump_toast().map(|(o, t)| at(o, t.button_at()))
    }

    /// The thread toast's label in client DIPs.
    pub fn thread_jump_label_rect(&self) -> Option<Rect> {
        self.thread_jump_toast().map(|(o, t)| at(o, t.label_at()))
    }

    /// After the thread scrolled or changed, before it draws.
    pub fn sync_thread_jump(&mut self) {
        let vh = self.thread_vh();
        let Some(t) = &mut self.thread else { return };
        if t.jump_glide.is_some() {
            return;
        }
        // Replies that start below the viewport.
        let fold = t.scroll + vh;
        let below = (0..t.rows.len())
            .rev()
            .take_while(|&i| t.row_y(i).is_some_and(|y| y >= fold))
            .count();
        if t.jump.scrolled(below, t.at_end, true) {
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
        let Some((o, t)) = self.thread_jump_toast() else {
            self.thread_jump_drawn = None;
            return self.thread_jump_p.offset(-100_000.0, 0.0);
        };
        let Some(p) = &self.thread else {
            return Ok(());
        };
        let hover = p.hover == Some(crate::thread::Part::Jump);
        let look = (p.jump, hover, false, self.scale, (t.w, t.h));
        place(
            &mut self.thread_jump_p,
            &mut self.thread_jump_drawn,
            &self.g,
            o,
            &t,
            look,
        )
    }
}
