//! Pins, as egui's `pins_dialog`: the open conversation's pinned messages,
//! newest pin first with your pending pins and unpins applied, each on a
//! card tinted by who pinned it (terracotta for your own) with Go to
//! message. Going to a message, or clicking its card, closes Pins over it,
//! as on web. A modal card over the conversation on its own panel, with
//! its own UI Automation identities; it follows live pin changes.
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::list::{self, Rows};
use crate::model::Conversation;
use crate::overlay::Rect;
use crate::theme::*;
use crate::{App, EV_FOCUS, EV_SCREEN, WHEEL_STEP, text};
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;
use windows_numerics::Matrix3x2;

/// The next GIF frame of a pinned message.
pub const TIMER_PINS: usize = 21;
const CARD_W: f32 = 560.0;
/// The title row above the separator and the scrolling list.
const HEAD_H: f32 = 61.0;
const PAD: f32 = 16.0;
const GAP: f32 = 12.0;
const GO_H: f32 = 32.0;
/// A card's frame above its message and, below it, the Go to message line.
const CARD_TOP: f32 = 8.0;
const CARD_FOOT: f32 = 6.0 + GO_H + 12.0 + 8.0;
const EMPTY_H: f32 = 80.0;
const GO_LABEL: &str = "Go to message";
/// egui's tints: your pins in terracotta, others' in a faint yellow.
const MINE: (u32, f32) = (TERRACOTTA, 31.0 / 255.0);
const OTHERS: (u32, f32) = (0xE4C76A, 15.0 / 255.0);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Close,
    /// Pinned message `k`'s card, and its Go to message.
    Card(usize),
    Go(usize),
}

pub struct Pins {
    pub c: Conversation,
    pub rows: Rows,
    /// Per card: whether you pinned it, and its thread root.
    about: Vec<(bool, Option<String>)>,
    /// Each card's top in the list's content, and the content's height.
    tops: Vec<f32>,
    content_h: f32,
    scroll: f32,
    pub hover: Option<Part>,
    pub focus: Option<Part>,
    pressed: Option<Part>,
    /// The card and its scrolling list (client DIPs).
    card: Rect,
    body: Rect,
    go_w: f32,
}

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

impl Pins {
    fn new(c: Conversation, about: Vec<(bool, Option<String>)>) -> Self {
        Self {
            rows: Rows::new(c.messages.len()),
            c,
            about,
            tops: Vec::new(),
            content_h: 0.0,
            scroll: 0.0,
            hover: None,
            focus: None,
            pressed: None,
            card: (0.0, 0.0, 0.0, 0.0),
            body: (0.0, 0.0, 0.0, 0.0),
            go_w: 0.0,
        }
    }

    /// Measures every card for a `w` × `h` window and places the card.
    fn relayout(&mut self, g: &Gfx, w: f32, h: f32) -> Result<()> {
        let cw = CARD_W.min(w - 48.0).max(240.0);
        self.rows.set_width(Rows::body_width(cw - 2.0 * PAD));
        self.rows.measure_up_to(g, &self.c, f64::NEG_INFINITY)?;
        let mut y = PAD;
        self.tops.clear();
        for i in 0..self.c.messages.len() {
            self.tops.push(y);
            y += CARD_TOP + self.rows.height(i) + CARD_FOOT + GAP;
        }
        self.content_h = if self.tops.is_empty() {
            EMPTY_H + 2.0 * PAD
        } else {
            y - GAP + PAD
        };
        let label = g.layout(&gfx::wide(GO_LABEL), &g.formats.quote_meta, 1e6)?;
        self.go_w = gfx::metrics(&label).widthIncludingTrailingWhitespace + 14.0 + 6.0 + 20.0;
        let list_h = self.content_h.min((h - 128.0).max(60.0));
        let ch = HEAD_H + 1.0 + list_h;
        let (cx, cy) = (((w - cw) / 2.0).round(), ((h - ch) / 2.0).round());
        self.card = (cx, cy, cw, ch);
        self.body = (cx, cy + HEAD_H + 1.0, cw, list_h);
        self.scroll_by(0.0);
        Ok(())
    }

    fn scroll_by(&mut self, dy: f32) {
        let max = (self.content_h - self.body.3).max(0.0);
        self.scroll = (self.scroll + dy).clamp(0.0, max);
    }

    fn card_h(&self, k: usize) -> f32 {
        CARD_TOP + self.rows.height(k) + CARD_FOOT
    }

    pub fn on_card(&self, x: f32, y: f32) -> bool {
        contains(self.card, x, y)
    }

    /// A part's rectangle (client DIPs); a card or button may lie partly
    /// outside the list's viewport.
    pub fn part_rect(&self, p: Part) -> Option<Rect> {
        let (cx, cy, cw, _) = self.card;
        let (_, by, _, _) = self.body;
        match p {
            Part::Close => Some((
                cx + cw - 18.0 - 28.0,
                cy + (HEAD_H - 28.0) / 2.0,
                28.0,
                28.0,
            )),
            Part::Card(k) => {
                let top = *self.tops.get(k)?;
                Some((
                    cx + PAD,
                    by + top - self.scroll,
                    cw - 2.0 * PAD,
                    self.card_h(k),
                ))
            }
            Part::Go(k) => {
                let (x, y, w, h) = self.part_rect(Part::Card(k))?;
                Some((
                    x + w - 18.0 - self.go_w,
                    y + h - 8.0 - 12.0 - GO_H,
                    self.go_w,
                    GO_H,
                ))
            }
        }
    }

    /// Whether a part shows in the list's viewport.
    fn shown(&self, p: Part) -> bool {
        let (_, by, _, bh) = self.body;
        self.part_rect(p)
            .is_some_and(|(_, y, _, h)| p == Part::Close || (y + h > by && y < by + bh))
    }

    /// The shown parts, in reading order.
    pub fn parts(&self) -> Vec<Part> {
        let mut out = vec![Part::Close];
        for k in 0..self.c.messages.len() {
            if self.shown(Part::Card(k)) {
                out.push(Part::Card(k));
                if self.shown(Part::Go(k)) {
                    out.push(Part::Go(k));
                }
            }
        }
        out
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<Part> {
        if self
            .part_rect(Part::Close)
            .is_some_and(|r| contains(r, x, y))
        {
            return Some(Part::Close);
        }
        if !contains(self.body, x, y) {
            return None;
        }
        (0..self.c.messages.len()).find_map(|k| {
            if self
                .part_rect(Part::Go(k))
                .is_some_and(|r| contains(r, x, y))
            {
                Some(Part::Go(k))
            } else {
                self.part_rect(Part::Card(k))
                    .is_some_and(|r| contains(r, x, y))
                    .then_some(Part::Card(k))
            }
        })
    }

    /// The link under (x, y) in card `k`'s message text.
    fn link_at(&self, k: usize, x: f32, y: f32) -> Option<String> {
        let (cx, cy, _, _) = self.part_rect(Part::Card(k))?;
        let row = self.rows.row(k)?;
        let (ox, oy) = (
            cx + list::BODY_X,
            cy + CARD_TOP + list::content_top(false) + row.text_y,
        );
        if !gfx::on_text(&row.text, x - ox, y - oy) {
            return None;
        }
        let p = gfx::hit_point(&row.text, x - ox, y - oy);
        row.links
            .iter()
            .find_map(|(s, l, href)| (p >= *s && p < s + l).then(|| href.clone()))
    }

    /// Tab order: Close, then each card's Go to message.
    fn tab(&mut self, backwards: bool) {
        let mut order = vec![Part::Close];
        order.extend((0..self.c.messages.len()).map(Part::Go));
        let at = self.focus.and_then(|f| order.iter().position(|p| *p == f));
        let n = order.len();
        let next = match (at, backwards) {
            (None, false) => 0,
            (None, true) => n - 1,
            (Some(i), false) => (i + 1) % n,
            (Some(i), true) => (i + n - 1) % n,
        };
        self.focus_part(order[next]);
    }

    /// Focuses a part, scrolling its card into view.
    pub fn focus_part(&mut self, p: Part) {
        if let Part::Card(k) | Part::Go(k) = p
            && let Some((_, y, _, h)) = self.part_rect(Part::Card(k))
        {
            let (_, by, _, bh) = self.body;
            if y < by {
                self.scroll_by(y - by - PAD);
            } else if y + h > by + bh {
                self.scroll_by((y + h - by - bh + PAD).min(y - by - PAD));
            }
        }
        self.focus = Some(p);
    }

    pub fn name(&self, p: Part) -> String {
        match p {
            Part::Close => "Close pins".into(),
            Part::Go(_) => GO_LABEL.into(),
            Part::Card(_) => String::new(),
        }
    }

    fn draw(
        &mut self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        media: Option<&mut crate::files::Ctx>,
    ) -> Result<()> {
        let (cx, cy, cw, ch) = self.card;
        let (bx, by, bw, bh) = self.body;
        unsafe {
            dc.Clear(Some(&gfx::color(BLACKOUT, 0.6)));
            let card = rect(cx, cy, cx + cw, cy + ch);
            fill_round(dc, card, RADIUS, &g.brushes.surface);
            stroke_round(dc, card, RADIUS, &g.brushes.border);
            let title = g.layout(&gfx::wide("Pins"), &g.formats.dialog_title, 1e6)?;
            let tm = gfx::metrics(&title);
            text::draw(
                dc,
                &title,
                cx + 22.0,
                cy + (HEAD_H - tm.height) / 2.0,
                &g.brushes.text,
            );
            if let Some((x, y, w, h)) = self.part_rect(Part::Close) {
                let on = self.hover == Some(Part::Close) || self.focus == Some(Part::Close);
                if on {
                    fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
                }
                if self.focus == Some(Part::Close) {
                    stroke_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.terracotta);
                }
                icons::draw(
                    dc,
                    Icon::Close,
                    rect(x + 5.0, y + 5.0, x + w - 5.0, y + h - 5.0),
                    if on { TEXT } else { TEXT_MUTED },
                    1.0,
                );
            }
            dc.FillRectangle(&rect(cx, by - 1.0, cx + cw, by), &g.brushes.border);
            dc.PushAxisAlignedClip(&rect(bx, by, bx + bw, by + bh), D2D1_ANTIALIAS_MODE_ALIASED);
            let result = self.draw_list(g, dc, media);
            dc.PopAxisAlignedClip();
            result
        }
    }

    /// The cards, or the empty note (the caller clips to the list).
    fn draw_list(
        &mut self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        mut media: Option<&mut crate::files::Ctx>,
    ) -> Result<()> {
        let (bx, by, bw, _) = self.body;
        if self.c.messages.is_empty() {
            let mid = by + PAD + EMPTY_H / 2.0;
            let head = g.layout(&gfx::wide("No pinned messages."), &g.formats.label, 1e6)?;
            let note = g.layout(
                &gfx::wide("Messages you pin will appear here."),
                &g.formats.quote,
                1e6,
            )?;
            let (hm, nm) = (gfx::metrics(&head), gfx::metrics(&note));
            let top = mid - (hm.height + 2.0 + nm.height) / 2.0;
            text::draw(dc, &head, bx + (bw - hm.width) / 2.0, top, &g.brushes.text);
            let ny = top + hm.height + 2.0;
            text::draw(dc, &note, bx + (bw - nm.width) / 2.0, ny, &g.brushes.muted);
            return Ok(());
        }
        let mine = unsafe { dc.CreateSolidColorBrush(&gfx::color(MINE.0, MINE.1), None)? };
        let others = unsafe { dc.CreateSolidColorBrush(&gfx::color(OTHERS.0, OTHERS.1), None)? };
        for k in 0..self.c.messages.len() {
            if !self.shown(Part::Card(k)) {
                continue;
            }
            let Some((x, y, w, h)) = self.part_rect(Part::Card(k)) else {
                continue;
            };
            let r = rect(x, y, x + w, y + h);
            let tint = if self.about.get(k).is_some_and(|a| a.0) {
                &mine
            } else {
                &others
            };
            fill_round(dc, r, RADIUS, tint);
            if self.hover == Some(Part::Card(k)) {
                fill_round(dc, r, RADIUS, &g.brushes.hover);
            }
            unsafe {
                let mut before = Matrix3x2::default();
                dc.GetTransform(&mut before);
                dc.SetTransform(&(Matrix3x2::translation(x, 0.0) * before));
                let drawn = list::draw_row(
                    g,
                    dc,
                    &self.c,
                    &mut self.rows,
                    k,
                    y + CARD_TOP,
                    None,
                    media.as_deref_mut(),
                );
                dc.SetTransform(&before);
                drawn?;
            }
            self.draw_go(g, dc, k)?;
        }
        Ok(())
    }

    fn draw_go(&self, g: &Gfx, dc: &ID2D1DeviceContext, k: usize) -> Result<()> {
        let Some((x, y, w, h)) = self.part_rect(Part::Go(k)) else {
            return Ok(());
        };
        let r = rect(x, y, x + w, y + h);
        let (hover, focus) = (
            self.hover == Some(Part::Go(k)),
            self.focus == Some(Part::Go(k)),
        );
        fill_round(dc, r, RADIUS, &g.brushes.raised);
        if hover {
            fill_round(dc, r, RADIUS, &g.brushes.hover);
        }
        stroke_round(
            dc,
            r,
            RADIUS,
            if focus {
                &g.brushes.terracotta
            } else {
                &g.brushes.border
            },
        );
        let icon_y = y + (h - 14.0) / 2.0;
        icons::draw(
            dc,
            Icon::ArrowRight,
            rect(x + 10.0, icon_y, x + 24.0, icon_y + 14.0),
            TEXT,
            1.0,
        );
        let label = g.layout(&gfx::wide(GO_LABEL), &g.formats.quote_meta, 1e6)?;
        let lm = gfx::metrics(&label);
        text::draw(
            dc,
            &label,
            x + 30.0,
            y + (h - lm.height) / 2.0,
            &g.brushes.text,
        );
        Ok(())
    }
}

impl App {
    /// The header's Pins button (client DIPs), left of the member-list
    /// toggle, as in egui.
    pub fn pins_button_rect(&self) -> Option<Rect> {
        (!self.is_narrow() && self.c.is_live() && self.c.has_conversation()).then(|| {
            let right = self
                .members_toggle_rect()
                .map_or(self.list_x() + self.list_w() - 18.0, |(x, ..)| x - 6.0);
            (right - 28.0, (HEADER_H - 28.0) / 2.0, 28.0, 28.0)
        })
    }

    pub fn on_pins_button(&self, x: f32, y: f32) -> bool {
        self.pins_button_rect().is_some_and(|r| contains(r, x, y))
    }

    pub fn open_pins(&mut self) {
        let Some((c, about)) = self.live.as_ref().and_then(|l| l.pinned_view()) else {
            return;
        };
        self.close_menu();
        self.close_dialog();
        self.close_picker();
        self.close_gifs(false);
        self.set_composer_focus(false);
        // Focus starts on Close, so Narrator announces Pins as it opens.
        let mut pins = Pins::new(c, about);
        pins.focus = Some(Part::Close);
        self.pins = Some(pins);
        self.pins_hover = false;
        self.dirty.header = true;
        self.pins_changed();
        self.events |= EV_FOCUS;
    }

    pub fn close_pins(&mut self) {
        if self.pins.take().is_some() {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_PINS);
            }
            self.dirty.pins = true;
            self.dirty.header = true;
            self.events |= EV_SCREEN | EV_FOCUS;
            self.need = true;
        }
    }

    /// Close and Escape give the keyboard back to the composer Pins took
    /// it from.
    fn close_pins_refocus(&mut self) {
        self.close_pins();
        self.set_composer_focus(true);
    }

    /// Pins laid out again for the window or its contents.
    pub fn pins_changed(&mut self) {
        let (g, w, h) = (&self.g, self.w, self.h);
        if let Some(p) = &mut self.pins
            && let Err(e) = p.relayout(g, w, h)
        {
            eprintln!("pins layout failed: {e:?}");
        }
        self.dirty.pins = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// Follows live pins, unpins, edits and reactions while Pins is open.
    pub fn refresh_pins(&mut self) {
        if self.pins.is_none() {
            return;
        }
        let Some((mut c, about)) = self.live.as_ref().and_then(|l| l.pinned_view()) else {
            return self.close_pins();
        };
        let Some(p) = &mut self.pins else { return };
        // What the reader revealed stays revealed.
        for m in &mut c.messages {
            if let Some(old) = p.c.messages.iter().find(|o| o.id == m.id) {
                m.revealed.clone_from(&old.revealed);
                m.forward_revealed.clone_from(&old.forward_revealed);
            }
        }
        if c.messages == p.c.messages && about == p.about {
            return;
        }
        let (scroll, focus) = (p.scroll, p.focus);
        let mut next = Pins::new(c, about);
        next.scroll = scroll;
        next.focus = focus.filter(|f| match f {
            Part::Close => true,
            Part::Card(k) | Part::Go(k) => *k < next.c.messages.len(),
        });
        *p = next;
        self.pins_changed();
    }

    pub fn pins_activate(&mut self, part: Part) {
        let Some(p) = &self.pins else { return };
        match part {
            Part::Close => self.close_pins_refocus(),
            Part::Card(k) | Part::Go(k) => {
                let (Some(m), Some((_, root))) = (p.c.messages.get(k), p.about.get(k)) else {
                    return;
                };
                let (id, root) = (m.id.clone(), root.clone());
                self.close_pins();
                // A thread reply opens its thread at the reply, as egui's
                // load_context(root, id).
                // The thread's reply box takes focus once the thread lands;
                // a channel message leaves the keyboard on the composer.
                let thread = root.is_some();
                if let Some(live) = &mut self.live {
                    match root {
                        Some(root) => live.open_thread_at(&root, &id),
                        None => live.view_original(&id, None),
                    }
                }
                if !thread {
                    self.set_composer_focus(true);
                }
            }
        }
    }

    pub fn pins_tick(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_PINS);
        }
        if self.pins.is_some() {
            self.dirty.pins = true;
            self.need = true;
        }
    }

    fn set_pins_hover(&mut self, hover: Option<Part>) {
        if let Some(p) = &mut self.pins
            && p.hover != hover
        {
            p.hover = hover;
            self.dirty.pins = true;
            self.need = true;
        }
    }

    /// Input while Pins is open: it is modal, as egui's.
    pub fn pins_input(&mut self, msg: u32, wp: WPARAM, x: f32, y: f32) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let p = self.pins.as_mut()?;
        match msg {
            WM_MOUSEMOVE => {
                self.pointer = Some((x, y));
                let hover = p.hit(x, y);
                self.set_pins_hover(hover);
                done
            }
            WM_LBUTTONDOWN => {
                let part = p.hit(x, y);
                p.pressed = part;
                if part.is_none() && !p.on_card(x, y) {
                    self.close_pins();
                }
                done
            }
            WM_LBUTTONUP => {
                let Some(part) = p.pressed.take().filter(|q| p.hit(x, y) == Some(*q)) else {
                    return done;
                };
                // A link in a message opens; the rest of its card goes to it.
                if let Part::Card(k) = part
                    && let Some(href) = p.link_at(k, x, y)
                {
                    crate::open_link(&href);
                    return done;
                }
                self.pins_activate(part);
                done
            }
            WM_MOUSEWHEEL => {
                let notches = (wp.0 >> 16) as u16 as i16 as f32 / 120.0;
                p.scroll_by(-notches * WHEEL_STEP as f32);
                let hover = p.hit(x, y);
                self.set_pins_hover(hover);
                self.pins_changed();
                done
            }
            WM_SETCURSOR => {
                let hand = self.pointer.and_then(|(x, y)| p.hit(x, y)).is_some();
                unsafe {
                    SetCursor(LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }).ok());
                }
                Some(Some(LRESULT(1)))
            }
            WM_KEYDOWN => {
                let page = p.body.3 - 48.0;
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_TAB => {
                        let backwards = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
                        p.tab(backwards);
                        self.events |= EV_FOCUS;
                    }
                    VK_RETURN | VK_SPACE => {
                        if let Some(part) = p.focus {
                            self.pins_activate(part);
                        }
                        return done;
                    }
                    VK_ESCAPE => {
                        self.close_pins_refocus();
                        return done;
                    }
                    VK_UP => p.scroll_by(-(WHEEL_STEP as f32)),
                    VK_DOWN => p.scroll_by(WHEEL_STEP as f32),
                    VK_PRIOR => p.scroll_by(-page),
                    VK_NEXT => p.scroll_by(page),
                    VK_HOME => p.scroll_by(-f32::MAX / 2.0),
                    VK_END => p.scroll_by(f32::MAX / 2.0),
                    _ => return done,
                }
                self.pins_changed();
                done
            }
            WM_CHAR | WM_LBUTTONDBLCLK | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MBUTTONDOWN
            | WM_MOUSEHWHEEL | WM_CONTEXTMENU => done,
            _ => None,
        }
    }

    /// Redraws Pins when it changed, with its messages' media.
    pub fn draw_pins(&mut self) -> Result<()> {
        if !std::mem::take(&mut self.dirty.pins) {
            return Ok(());
        }
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let g = &self.g;
        let Some(p) = &mut self.pins else {
            return self.pins_p.offset(-100_000.0, 0.0);
        };
        self.pins_p.ensure(g, px(self.w), px(self.h))?;
        self.pins_p.offset(0.0, 0.0)?;
        let mut tex = crate::files::D2d(g.dc.clone());
        let mut media = self.live.as_mut().map(|l| crate::files::Ctx {
            voice: self.playback.view(),
            ..crate::files::Ctx::new(&mut l.media, &mut tex)
        });
        self.pins_p.draw(s, |dc| p.draw(g, dc, media.as_mut()))?;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_PINS);
            if let Some(ctx) = media {
                if let Some(next) = ctx.next {
                    SetTimer(
                        Some(self.hwnd),
                        TIMER_PINS,
                        next.as_millis().clamp(10, u32::MAX as u128) as u32,
                        None,
                    );
                }
                if ctx.media.busy() {
                    SetTimer(Some(self.hwnd), crate::TIMER_LIVE, 100, None);
                }
            }
        }
        Ok(())
    }
}
