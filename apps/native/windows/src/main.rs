//! Caper for Windows: Win32 window, a DirectComposition visual tree with one
//! retained surface per panel, Direct2D + DirectWrite drawing, UI Automation.
//!
//! Damage model: each panel (rail, sidebar, header, members, composer, hover
//! toolbar) has its own composition surface, redrawn only when its own state
//! changes. The message history is one tall virtual surface: scrolling moves
//! the visual without redrawing, only newly exposed strips are drawn, and
//! hover or selection redraw just the rows they touch.
//!
//! Not shipped yet: kept out of the release workflow and installers until it
//! reaches parity with the egui client. It shows a labelled development mock
//! conversation until the shared client core (`apps/native/client`) is wired
//! in.
#![windows_subsystem = "windows"]

mod bench;
mod composer;
mod emoji;
mod gfx;
mod list;
mod model;
mod text;
mod theme;
mod uia;

use bench::{Action, Bench};
use composer::Composer;
use gfx::{Gfx, fill_round, rect, stroke_round};
use list::Rows;
use model::Conversation;
use std::cell::RefCell;
use std::ops::Range;
use std::time::Instant;
use theme::*;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::Ime::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;
use windows_numerics::Matrix3x2;

/// Pixel row of the anchor (bottom of the startup history) in the virtual surface.
const ORIGIN: f64 = 4_000_000.0;
const VS_H: u32 = 1 << 23;
const EV_TEXT: u8 = 1;
const EV_FOCUS: u8 = 2;
const EV_ROWS: u8 = 4;
const TIMER_BENCH: usize = 1;
const TIMER_CARET: usize = 2;
const WHEEL_STEP: f64 = 48.0;
const WM_MOUSELEAVE: u32 = 0x02A3;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app unless it is already borrowed (re-entrant window
/// messages, UIA calls arriving mid-frame) or not created yet.
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

#[derive(Default, Clone, Copy)]
struct Dirty {
    layout: bool,
    rail: bool,
    sidebar: bool,
    header: bool,
    members: bool,
    composer: bool,
    toolbar: bool,
}

impl Dirty {
    fn all() -> Self {
        Self {
            layout: true,
            rail: true,
            sidebar: true,
            header: true,
            members: true,
            composer: true,
            toolbar: true,
        }
    }
}

struct Panel {
    visual: IDCompositionVisual2,
    surface: Option<IDCompositionSurface>,
    size: (u32, u32),
}

impl Panel {
    fn new(g: &Gfx) -> Result<Self> {
        Ok(Self {
            visual: g.visual()?,
            surface: None,
            size: (0, 0),
        })
    }

    /// Ensures a surface of this pixel size; true when it was (re)created.
    fn ensure(&mut self, g: &Gfx, w: u32, h: u32) -> Result<bool> {
        if self.surface.is_some() && self.size == (w, h) {
            return Ok(false);
        }
        let s = g.surface(w, h)?;
        unsafe { self.visual.SetContent(&s)? };
        self.surface = Some(s);
        self.size = (w, h);
        Ok(true)
    }

    fn offset(&self, x: f32, y: f32) -> Result<()> {
        unsafe {
            self.visual.SetOffsetX2(x)?;
            self.visual.SetOffsetY2(y)
        }
    }

    fn draw(&self, scale: f32, f: impl FnOnce(&ID2D1DeviceContext) -> Result<()>) -> Result<()> {
        match &self.surface {
            Some(s) => gfx::draw(s, scale, f),
            None => Ok(()),
        }
    }
}

/// A text selection across message rows: (row, UTF-16 position) anchor and focus.
pub type Sel = ((usize, u32), (usize, u32));

pub struct App {
    pub hwnd: HWND,
    pub g: Gfx,
    pub c: Conversation,
    bench: Bench,
    pub rows: Rows,
    pub composer: Composer,
    pub scale: f32,
    pub w: f32,
    pub h: f32,
    _target: IDCompositionTarget,
    background: IDCompositionVisual2,
    rail: Panel,
    sidebar: Panel,
    header: Panel,
    members: Panel,
    composer_p: Panel,
    toolbar: Panel,
    history_clip: IDCompositionVisual2,
    content: IDCompositionVisual2,
    vs: IDCompositionVirtualSurface,
    vs_w: u32,
    draw_w: u32,
    drawn_scale: f32,
    /// Pixel rows [a, b) of the virtual surface that hold current content.
    drawn: Option<(i64, i64)>,
    scroll_top: f64,
    at_end: bool,
    hover_row: Option<usize>,
    hover_space: Option<usize>,
    hover_channel: Option<usize>,
    hover_member: Option<usize>,
    pointer: Option<(f32, f32)>,
    tracking: bool,
    pub sel: Option<Sel>,
    selecting: bool,
    composer_drag: bool,
    redraw: Vec<usize>,
    dirty: Dirty,
    need: bool,
    box_h: f32,
    pending_resize: Option<(f32, f32)>,
    events: u8,
    visible: Range<usize>,
    channel_layouts: Vec<IDWriteTextLayout>,
    member_layouts: Vec<IDWriteTextLayout>,
    quit: bool,
}

impl App {
    fn new(hwnd: HWND, g: Gfx, c: Conversation, bench: Bench) -> Result<Self> {
        unsafe {
            let target = g.dcomp.CreateTargetForHwnd(hwnd, true)?;
            let root = g.visual()?;
            root.SetBorderMode(DCOMPOSITION_BORDER_MODE_HARD)?;
            target.SetRoot(&root)?;
            // A 1×1 surface scaled to the window, behind everything.
            let background = g.visual()?;
            let solid = g.surface(1, 1)?;
            gfx::draw(&solid, 1.0, |dc| {
                dc.Clear(Some(&gfx::color(SURFACE_CONVERSATION, 1.0)));
                Ok(())
            })?;
            background.SetContent(&solid)?;
            background.SetBitmapInterpolationMode(
                DCOMPOSITION_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            )?;
            let rail = Panel::new(&g)?;
            let sidebar = Panel::new(&g)?;
            let header = Panel::new(&g)?;
            let members = Panel::new(&g)?;
            let composer_p = Panel::new(&g)?;
            let toolbar = Panel::new(&g)?;
            let history_clip = g.visual()?;
            let content = g.visual()?;
            let vs = g.dcomp.CreateVirtualSurface(
                1,
                VS_H,
                windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                windows::Win32::Graphics::Dxgi::Common::DXGI_ALPHA_MODE_PREMULTIPLIED,
            )?;
            content.SetContent(&vs)?;
            history_clip.AddVisual(&content, false, None)?;
            history_clip.AddVisual(&toolbar.visual, true, &content)?;
            root.AddVisual(&background, false, None)?;
            let mut prev: IDCompositionVisual = background.cast()?;
            for v in [
                &rail.visual,
                &sidebar.visual,
                &header.visual,
                &history_clip,
                &composer_p.visual,
                &members.visual,
            ] {
                root.AddVisual(v, true, &prev)?;
                prev = v.cast()?;
            }
            let channel_layouts = c
                .channels
                .iter()
                .map(|ch| g.layout(&gfx::wide(&ch.name), &g.formats.channel, 10_000.0))
                .collect::<Result<_>>()?;
            let member_layouts = c
                .members
                .iter()
                .map(|m| {
                    let l = g.layout(&gfx::wide(&m.name), &g.formats.member, 10_000.0)?;
                    emoji::apply(&l, &m.name, 14.0, 1.0)?;
                    Ok(l)
                })
                .collect::<Result<_>>()?;
            let n = c.messages.len();
            let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc)?;
            Ok(Self {
                hwnd,
                g,
                c,
                bench,
                rows: Rows::new(n),
                composer: Composer::default(),
                scale,
                w: rc.right as f32 / scale,
                h: rc.bottom as f32 / scale,
                _target: target,
                background,
                rail,
                sidebar,
                header,
                members,
                composer_p,
                toolbar,
                history_clip,
                content,
                vs,
                vs_w: 1,
                draw_w: 1,
                drawn_scale: scale,
                drawn: None,
                scroll_top: 0.0,
                at_end: true,
                hover_row: None,
                hover_space: None,
                hover_channel: None,
                hover_member: None,
                pointer: None,
                tracking: false,
                sel: None,
                selecting: false,
                composer_drag: false,
                redraw: Vec::new(),
                dirty: Dirty::all(),
                need: true,
                box_h: 0.0,
                pending_resize: None,
                events: 0,
                visible: 0..0,
                channel_layouts,
                member_layouts,
                quit: false,
            })
        }
    }

    // ---- geometry (client DIPs) ----

    pub fn list_x(&self) -> f32 {
        RAIL_W + SIDEBAR_W
    }

    pub fn list_w(&self) -> f32 {
        (self.w - RAIL_W - SIDEBAR_W - MEMBERS_W).max(160.0)
    }

    pub fn box_rect(&self) -> (f32, f32, f32, f32) {
        (
            self.list_x() + 18.0,
            self.h - 16.0 - self.box_h,
            self.list_w() - 36.0,
            self.box_h,
        )
    }

    /// Height of the visible history.
    pub fn vh(&self) -> f32 {
        (self.box_rect().1 - 10.0 - HEADER_H).max(0.0)
    }

    pub fn visible_rows(&self) -> Range<usize> {
        self.rows
            .range(self.scroll_top, self.scroll_top + self.vh() as f64)
    }

    pub fn composer_text_origin(&self) -> (f32, f32) {
        let (x, y, _, _) = self.box_rect();
        (
            x + composer::PAD,
            y + composer::PAD - self.composer.scroll(),
        )
    }

    /// Client position of row `i`'s message text origin.
    pub fn row_text_origin(&self, i: usize) -> (f32, f32) {
        let ty = self.rows.row(i).map_or(0.0, |r| r.text_y);
        (
            self.list_x() + list::BODY_X,
            HEADER_H
                + (self.rows.top[i] - self.scroll_top) as f32
                + list::content_top(self.c.messages[i].grouped)
                + ty,
        )
    }

    pub fn space_rect(i: usize) -> (f32, f32, f32, f32) {
        (
            (RAIL_W - RAIL_ITEM) / 2.0,
            RAIL_TOP + RAIL_PITCH * i as f32,
            RAIL_ITEM,
            RAIL_ITEM,
        )
    }

    pub fn channel_rect(i: usize) -> (f32, f32, f32, f32) {
        (
            RAIL_W + 12.0,
            CHANNEL_TOP + CHANNEL_H * i as f32,
            SIDEBAR_W - 24.0,
            CHANNEL_H - 2.0,
        )
    }

    pub fn member_rect(&self, i: usize) -> (f32, f32, f32, f32) {
        (
            self.w - MEMBERS_W + 8.0,
            MEMBER_TOP + MEMBER_H * i as f32,
            MEMBERS_W - 16.0,
            MEMBER_H,
        )
    }

    pub fn row_rect(&self, i: usize) -> (f32, f32, f32, f32) {
        (
            self.list_x(),
            HEADER_H + (self.rows.top[i] - self.scroll_top) as f32,
            self.list_w(),
            self.rows.height(i),
        )
    }

    // ---- state changes ----

    pub fn composer_changed(&mut self) {
        self.composer.caret_on = true;
        let (_, _, bw, _) = self.box_rect();
        if let Ok(h) = self.composer.relayout(&self.g, bw - 2.0 * composer::PAD)
            && (h - self.box_h).abs() > 0.01
        {
            self.dirty.layout = true;
        }
        self.dirty.composer = true;
        self.events |= EV_TEXT;
        self.need = true;
        self.ime_position();
    }

    pub fn set_composer_focus(&mut self, focused: bool) {
        if self.composer.focused == focused {
            return;
        }
        self.composer.focused = focused;
        self.composer.caret_on = true;
        unsafe {
            if focused {
                SetTimer(Some(self.hwnd), TIMER_CARET, GetCaretBlinkTime(), None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_CARET);
            }
        }
        self.dirty.composer = true;
        self.events |= EV_FOCUS;
        self.need = true;
    }

    pub fn select_space(&mut self, i: usize) {
        if i != self.c.space && i < self.c.spaces.len() {
            self.c.space = i;
            self.dirty.rail = true;
            self.dirty.sidebar = true;
            self.need = true;
        }
    }

    pub fn select_channel(&mut self, i: usize) {
        if i != self.c.channel && i < self.c.channels.len() {
            self.c.channel = i;
            self.dirty.sidebar = true;
            self.dirty.header = true;
            self.dirty.composer = true;
            self.need = true;
        }
    }

    /// The selected UTF-16 range of row `i`'s text, if any.
    pub fn sel_for(sel: Option<Sel>, i: usize, len: u32) -> Option<(u32, u32)> {
        let (a, b) = sel?;
        let (a, b) = if a <= b { (a, b) } else { (b, a) };
        if i < a.0 || i > b.0 {
            return None;
        }
        let s = if i == a.0 { a.1 } else { 0 };
        let e = if i == b.0 { b.1 } else { len };
        (e > s).then_some((s, e))
    }

    pub fn row_len(&self, i: usize) -> u32 {
        self.rows.row(i).map_or(0, |r| r.plain16.len() as u32)
    }

    pub fn set_sel(&mut self, new: Option<Sel>) {
        let old = self.sel;
        let span = |s: Option<Sel>| s.map(|((a, _), (b, _))| (a.min(b), a.max(b)));
        let (lo, hi) = match (span(old), span(new)) {
            (Some(a), Some(b)) => (a.0.min(b.0), a.1.max(b.1)),
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => return,
        };
        for i in lo..=hi.min(self.rows.len().saturating_sub(1)) {
            let len = self.row_len(i);
            if Self::sel_for(old, i, len) != Self::sel_for(new, i, len) {
                self.redraw.push(i);
            }
        }
        self.sel = new;
        self.need = true;
    }

    fn msg_hit(&self, x: f32, y: f32) -> Option<(usize, u32)> {
        let ay = self.scroll_top + (y - HEADER_H) as f64;
        let vis = self.visible_rows();
        if vis.is_empty() {
            return None;
        }
        let i = self
            .rows
            .row_at(ay)
            .unwrap_or(if ay < self.rows.top[vis.start] {
                vis.start
            } else {
                vis.end - 1
            });
        let row = self.rows.row(i)?;
        let (ox, oy) = self.row_text_origin(i);
        Some((i, gfx::hit_point(&row.text, x - ox, y - oy)))
    }

    fn selected_text(&self) -> Vec<u16> {
        let mut out = Vec::new();
        let Some(((ra, _), (rb, _))) = self.sel else {
            return out;
        };
        for i in ra.min(rb)..=ra.max(rb) {
            if let (Some((s, e)), Some(r)) = (
                Self::sel_for(self.sel, i, self.row_len(i)),
                self.rows.row(i),
            ) {
                if !out.is_empty() {
                    out.extend_from_slice(&gfx::wide("\r\n"));
                }
                out.extend_from_slice(&r.plain16[s as usize..e as usize]);
            }
        }
        out
    }

    fn in_box(&self, x: f32, y: f32) -> bool {
        let (bx, by, bw, bh) = self.box_rect();
        x >= bx && x < bx + bw && y >= by && y < by + bh
    }

    fn in_history(&self, x: f32, y: f32) -> bool {
        x >= self.list_x()
            && x < self.list_x() + self.list_w()
            && y >= HEADER_H
            && y < HEADER_H + self.vh()
    }

    fn update_hover(&mut self) {
        let inside = |(l, t, w, h): (f32, f32, f32, f32), x: f32, y: f32| {
            x >= l && x < l + w && y >= t && y < t + h
        };
        let (mut space, mut channel, mut member) = (None, None, None);
        if let Some((x, y)) = self.pointer {
            space = (0..self.c.spaces.len()).find(|&i| inside(Self::space_rect(i), x, y));
            channel = (0..self.c.channels.len()).find(|&i| inside(Self::channel_rect(i), x, y));
            member = (0..self.c.members.len()).find(|&i| inside(self.member_rect(i), x, y));
        }
        if space != self.hover_space {
            self.hover_space = space;
            self.dirty.rail = true;
            self.need = true;
        }
        if channel != self.hover_channel {
            self.hover_channel = channel;
            self.dirty.sidebar = true;
            self.need = true;
        }
        if member != self.hover_member {
            self.hover_member = member;
            self.dirty.members = true;
            self.need = true;
        }
    }

    fn history_hover(&self) -> Option<usize> {
        let (x, y) = self.pointer?;
        if !self.in_history(x, y) {
            return None;
        }
        self.rows.row_at(self.scroll_top + (y - HEADER_H) as f64)
    }

    /// Sends the composer's text. Until the client core is wired in, the
    /// message only joins the local development mock.
    fn send(&mut self) -> Result<()> {
        let text = String::from_utf16_lossy(&self.composer.text)
            .trim()
            .to_string();
        if text.is_empty() {
            return Ok(());
        }
        self.c.send_local(text);
        self.rows.append(&self.g, &self.c)?;
        self.at_end = true;
        self.composer.text.clear();
        self.composer.caret = 0;
        self.composer.anchor = 0;
        self.composer_changed();
        Ok(())
    }

    // ---- input ----

    fn handle(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
        let x = (lp.0 & 0xffff) as i16 as f32 / self.scale;
        let y = ((lp.0 >> 16) & 0xffff) as i16 as f32 / self.scale;
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        match msg {
            WM_SIZE => {
                let (pw, ph) = ((lp.0 & 0xffff) as f32, ((lp.0 >> 16) & 0xffff) as f32);
                if pw > 0.0 && ph > 0.0 {
                    self.w = pw / self.scale;
                    self.h = ph / self.scale;
                    self.dirty.layout = true;
                    self.need = true;
                }
            }
            WM_TIMER => match wp.0 {
                TIMER_BENCH if self.bench.tick() => self.quit = true,
                TIMER_CARET => {
                    self.composer.caret_on = !self.composer.caret_on;
                    self.dirty.composer = true;
                    self.need = true;
                }
                _ => {}
            },
            WM_MOUSEMOVE => {
                if !self.tracking {
                    let mut t = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: self.hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut t);
                    }
                    self.tracking = true;
                }
                self.pointer = Some((x, y));
                self.update_hover();
                if self.history_hover() != self.hover_row {
                    self.need = true;
                }
                if self.composer_drag {
                    let (tx, ty) = self.composer_text_origin();
                    let p = self.composer.hit(x - tx, y - ty - self.composer.scroll());
                    if p != self.composer.caret {
                        self.composer.move_to(p, true);
                        self.composer_changed();
                    }
                } else if self.selecting
                    && let (Some(h), Some((a, _))) = (self.msg_hit(x, y), self.sel)
                {
                    self.set_sel(Some((a, h)));
                }
            }
            WM_MOUSELEAVE => {
                self.tracking = false;
                self.pointer = None;
                self.update_hover();
                self.need = true;
            }
            WM_LBUTTONDOWN => {
                unsafe {
                    SetCapture(self.hwnd);
                }
                if self.in_box(x, y) {
                    self.set_composer_focus(true);
                    let (tx, ty) = self.composer_text_origin();
                    let p = self.composer.hit(x - tx, y - ty - self.composer.scroll());
                    self.composer.move_to(p, shift);
                    self.composer_drag = true;
                    self.composer_changed();
                } else {
                    self.set_composer_focus(false);
                    if self.in_history(x, y) {
                        if let Some(h) = self.msg_hit(x, y) {
                            let anchor = if shift {
                                self.sel.map_or(h, |s| s.0)
                            } else {
                                h
                            };
                            self.set_sel(Some((anchor, h)));
                            self.selecting = true;
                        }
                    } else if let Some(i) = self.hover_channel {
                        self.select_channel(i);
                    } else if let Some(i) = self.hover_space {
                        self.select_space(i);
                    }
                }
            }
            WM_LBUTTONDBLCLK if self.in_history(x, y) => {
                // Double-click selects a word.
                if let Some((i, p)) = self.msg_hit(x, y) {
                    let text = self
                        .rows
                        .row(i)
                        .map(|r| r.plain16.clone())
                        .unwrap_or_default();
                    let word = |c: u16| char::from_u32(c as u32).is_some_and(char::is_alphanumeric);
                    let (mut s, mut e) = (p as usize, p as usize);
                    while s > 0 && word(text[s - 1]) {
                        s -= 1;
                    }
                    while e < text.len() && word(text[e]) {
                        e += 1;
                    }
                    self.set_sel(Some(((i, s as u32), (i, e as u32))));
                }
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                self.selecting = false;
                self.composer_drag = false;
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f64;
                if self.pointer.is_some_and(|(x, y)| self.in_history(x, y)) {
                    self.scroll_by(-delta / 120.0 * WHEEL_STEP);
                }
            }
            WM_SETCURSOR => {
                if (lp.0 & 0xffff) as u32 != HTCLIENT {
                    return None;
                }
                let beam = self
                    .pointer
                    .is_some_and(|(x, y)| self.in_box(x, y) || self.in_history(x, y));
                let hand = self.hover_channel.is_some() || self.hover_space.is_some();
                unsafe {
                    SetCursor(
                        LoadCursorW(
                            None,
                            if beam {
                                IDC_IBEAM
                            } else if hand {
                                IDC_HAND
                            } else {
                                IDC_ARROW
                            },
                        )
                        .ok(),
                    );
                }
                return Some(LRESULT(1));
            }
            WM_KEYDOWN => {
                return self
                    .key(VIRTUAL_KEY(wp.0 as u16), ctrl, shift)
                    .then_some(LRESULT(0));
            }
            WM_CHAR => {
                let c = wp.0 as u16;
                if self.composer.focused && !ctrl && c >= 0x20 && c != 0x7f {
                    self.composer.insert(&[c]);
                    self.composer_changed();
                }
                return Some(LRESULT(0));
            }
            WM_IME_STARTCOMPOSITION if self.composer.focused => {
                self.ime_position();
                return Some(LRESULT(0));
            }
            WM_IME_COMPOSITION if self.composer.focused => {
                self.ime_composition(lp);
                return Some(LRESULT(0));
            }
            WM_IME_ENDCOMPOSITION if self.composer.focused => {
                self.composer.comp.clear();
                self.composer_changed();
                return Some(LRESULT(0));
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }

    pub fn scroll_by(&mut self, dy: f64) {
        self.scroll_top += dy;
        self.at_end = false;
        self.need = true;
    }

    fn key(&mut self, vk: VIRTUAL_KEY, ctrl: bool, shift: bool) -> bool {
        if ctrl && vk == VIRTUAL_KEY('C' as u16) {
            let text = if self.composer.focused {
                let (a, b) = self.composer.selection();
                self.composer.text[a..b].to_vec()
            } else {
                self.selected_text()
            };
            if !text.is_empty() {
                set_clipboard(self.hwnd, &text);
            }
            return true;
        }
        if !self.composer.focused {
            let page = self.vh() as f64 * 0.9;
            match vk {
                VK_PRIOR => self.scroll_by(-page),
                VK_NEXT => self.scroll_by(page),
                VK_END if ctrl => {
                    self.at_end = true;
                    self.need = true;
                }
                _ => return false,
            }
            return true;
        }
        let c = &mut self.composer;
        let (a, b) = c.selection();
        match vk {
            VK_RETURN if shift => c.insert(&['\n' as u16]),
            VK_RETURN => {
                if let Err(e) = self.send() {
                    eprintln!("send failed: {e:?}");
                }
                return true;
            }
            VK_LEFT => {
                let to = if !shift && a != b {
                    a
                } else if ctrl {
                    c.word_left(c.caret)
                } else {
                    c.prev(c.caret)
                };
                c.move_to(to, shift);
            }
            VK_RIGHT => {
                let to = if !shift && a != b {
                    b
                } else if ctrl {
                    c.word_right(c.caret)
                } else {
                    c.next(c.caret)
                };
                c.move_to(to, shift);
            }
            VK_HOME => c.move_to(0, shift),
            VK_END => {
                let n = c.text.len();
                c.move_to(n, shift);
            }
            VK_BACK => c.backspace(ctrl),
            VK_DELETE => c.delete(ctrl),
            VK_ESCAPE => {
                self.set_composer_focus(false);
                return true;
            }
            _ if ctrl && vk == VIRTUAL_KEY('A' as u16) => {
                c.anchor = 0;
                c.caret = c.text.len();
            }
            _ if ctrl && vk == VIRTUAL_KEY('X' as u16) => {
                let t = c.text[a..b].to_vec();
                set_clipboard(self.hwnd, &t);
                self.composer.insert(&[]);
            }
            _ if ctrl && vk == VIRTUAL_KEY('V' as u16) => {
                let t: Vec<u16> = get_clipboard(self.hwnd)
                    .into_iter()
                    .filter(|c| *c != '\r' as u16)
                    .collect();
                self.composer.insert(&t);
            }
            _ => return false,
        }
        self.composer_changed();
        true
    }

    fn ime_position(&self) {
        if !self.composer.focused {
            return;
        }
        let (tx, ty) = self.composer_text_origin();
        let (cx, cy, ch) = self.composer.caret_pos(self.composer.display_caret());
        let s = self.scale;
        let p = POINT {
            x: ((tx + cx) * s) as i32,
            y: ((ty + cy) * s) as i32,
        };
        unsafe {
            let himc = ImmGetContext(self.hwnd);
            if himc.is_invalid() {
                return;
            }
            let form = COMPOSITIONFORM {
                dwStyle: CFS_POINT,
                ptCurrentPos: p,
                rcArea: RECT::default(),
            };
            let _ = ImmSetCompositionWindow(himc, &form);
            let cand = CANDIDATEFORM {
                dwIndex: 0,
                dwStyle: CFS_EXCLUDE,
                ptCurrentPos: p,
                rcArea: RECT {
                    left: p.x,
                    top: p.y,
                    right: p.x + 2,
                    bottom: p.y + (ch * s) as i32,
                },
            };
            let _ = ImmSetCandidateWindow(himc, &cand);
            let _ = ImmReleaseContext(self.hwnd, himc);
        }
    }

    fn ime_composition(&mut self, lp: LPARAM) {
        let flags = lp.0 as u32;
        unsafe {
            let himc = ImmGetContext(self.hwnd);
            if himc.is_invalid() {
                return;
            }
            let read = |kind: IME_COMPOSITION_STRING| -> Vec<u16> {
                let bytes = ImmGetCompositionStringW(himc, kind, None, 0);
                if bytes <= 0 {
                    return Vec::new();
                }
                let mut buf = vec![0u16; bytes as usize / 2];
                ImmGetCompositionStringW(himc, kind, Some(buf.as_mut_ptr().cast()), bytes as u32);
                buf
            };
            if flags & GCS_RESULTSTR.0 != 0 {
                let s = read(GCS_RESULTSTR);
                self.composer.comp.clear();
                self.composer.insert(&s);
            }
            if flags & GCS_COMPSTR.0 != 0 {
                self.composer.comp = read(GCS_COMPSTR);
                let cur = ImmGetCompositionStringW(himc, GCS_CURSORPOS, None, 0);
                self.composer.comp_cursor = (cur.max(0) as usize).min(self.composer.comp.len());
            } else if flags & GCS_RESULTSTR.0 != 0 {
                self.composer.comp.clear();
            }
            let _ = ImmReleaseContext(self.hwnd, himc);
        }
        self.composer_changed();
    }

    // ---- frame ----

    fn set_scale(&mut self, scale: f32) {
        self.scale = scale;
        self.dirty = Dirty::all();
        self.need = true;
    }

    fn apply_layout(&mut self) -> Result<()> {
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let (w, h, lx, lw) = (self.w, self.h, self.list_x(), self.list_w());
        unsafe {
            self.background.SetTransform2(&Matrix3x2 {
                M11: w * s,
                M12: 0.0,
                M21: 0.0,
                M22: h * s,
                M31: 0.0,
                M32: 0.0,
            })?;
        }
        self.dirty.rail |= self.rail.ensure(&self.g, px(RAIL_W), px(h))?;
        self.dirty.sidebar |= self.sidebar.ensure(&self.g, px(SIDEBAR_W), px(h))?;
        self.sidebar.offset((RAIL_W * s).round(), 0.0)?;
        self.dirty.header |= self.header.ensure(&self.g, px(lw), px(HEADER_H))?;
        self.header.offset((lx * s).round(), 0.0)?;
        self.dirty.members |= self.members.ensure(&self.g, px(MEMBERS_W), px(h))?;
        self.members.offset(((w - MEMBERS_W) * s).round(), 0.0)?;
        self.dirty.toolbar |= self.toolbar.ensure(&self.g, px(TOOLBAR.0), px(TOOLBAR.1))?;
        let bw = lw - 36.0;
        self.box_h = self.composer.relayout(&self.g, bw - 2.0 * composer::PAD)?;
        let (bx, by, _, _) = self.box_rect();
        self.composer_p.ensure(&self.g, px(bw), px(self.box_h))?;
        self.composer_p.offset((bx * s).round(), (by * s).round())?;
        self.dirty.composer = true;
        let vh = self.vh();
        unsafe {
            self.history_clip.SetOffsetX2((lx * s).round())?;
            self.history_clip.SetOffsetY2((HEADER_H * s).round())?;
            self.history_clip
                .SetClip2(&rect(0.0, 0.0, (lw * s).round(), (vh * s).round()))?;
        }
        let body = Rows::body_width(lw);
        if (body - self.rows.width).abs() > 0.01 {
            // Text rewraps: keep the top visible row in place unless at the live end.
            let keep = (!self.at_end)
                .then(|| {
                    self.rows
                        .row_at(self.scroll_top)
                        .map(|i| (i, self.scroll_top - self.rows.top[i]))
                })
                .flatten();
            self.rows.set_width(body);
            if let Some((i, frac)) = keep {
                while self.rows.lo > i {
                    let y = if self.rows.lo < self.rows.len() {
                        self.rows.top[self.rows.lo] - 1.0
                    } else {
                        0.0
                    };
                    self.rows.measure_up_to(&self.g, &self.c, y)?;
                }
                self.scroll_top = self.rows.top[i] + frac;
            }
            self.drawn = None;
        }
        // The surface only grows, in 512 px steps: resizing it every frame of
        // a window drag would reallocate its tiles. The clip hides the rest.
        if px(lw) > self.vs_w {
            self.vs_w = px(lw).next_multiple_of(512);
            unsafe { self.vs.Resize(self.vs_w, VS_H)? };
            self.drawn = None;
        }
        if px(lw) != self.draw_w || self.drawn_scale != s {
            self.draw_w = px(lw);
            self.drawn_scale = s;
            self.drawn = None;
        }
        Ok(())
    }

    fn row_px(&self, y: f64) -> f64 {
        ORIGIN + y * self.scale as f64
    }

    /// Draws pixel rows [y0, y1) of the history surface.
    fn draw_strip(&mut self, y0: i64, y1: i64) -> Result<()> {
        if y1 <= y0 {
            return Ok(());
        }
        let s = self.scale;
        let r = RECT {
            left: 0,
            top: y0 as i32,
            right: self.draw_w as i32,
            bottom: y1 as i32,
        };
        let range = self.rows.range(
            (y0 as f64 - ORIGIN) / s as f64,
            (y1 as f64 - ORIGIN) / s as f64,
        );
        let lw = self.list_w();
        let (hover, sel) = (self.hover_row, self.sel);
        let tops: Vec<(usize, f32, f32, Option<(u32, u32)>)> = range
            .map(|i| {
                (
                    i,
                    ((self.row_px(self.rows.top[i]) - y0 as f64) / s as f64) as f32,
                    self.rows.height(i),
                    Self::sel_for(sel, i, self.row_len(i)),
                )
            })
            .collect();
        let (g, c, rows) = (&self.g, &self.c, &mut self.rows);
        gfx::draw_local(&self.vs, r, s, |dc| unsafe {
            dc.Clear(Some(&gfx::color(SURFACE_CONVERSATION, 1.0)));
            for (i, y, h, rs) in tops {
                if hover == Some(i) {
                    dc.FillRectangle(&rect(0.0, y, lw, y + h), &g.brushes.hover);
                }
                list::draw_row(g, dc, c, rows, i, y, rs)?;
            }
            Ok(())
        })
    }

    fn update_history(&mut self) -> Result<()> {
        let s = self.scale as f64;
        let vh = self.vh() as f64;
        // At startup and after a rewrap only the viewport is measured and
        // drawn this frame; the margins fill in on the following frames.
        let fresh = self.drawn.is_none();
        let margin = if fresh { 0.0 } else { vh };
        if self.at_end {
            self.scroll_top = self.rows.bottom() - vh;
        }
        self.rows
            .measure_up_to(&self.g, &self.c, self.scroll_top - margin)?;
        let min = if self.rows.lo == 0 {
            self.rows.top[0]
        } else {
            f64::NEG_INFINITY
        };
        let max = (self.rows.bottom() - vh).max(min);
        self.scroll_top = self.scroll_top.clamp(min, max);
        self.at_end = self.scroll_top >= max - 0.5;
        // Snap to device pixels so the composition offset and the drawn rows agree.
        let off = self.row_px(self.scroll_top).round();
        self.scroll_top = (off - ORIGIN) / s;
        unsafe { self.content.SetOffsetY2(-(off as f32))? };
        let visible = self.visible_rows();
        if visible != self.visible {
            self.visible = visible;
            self.events |= EV_ROWS;
        }
        let hover = self.history_hover();
        if hover != self.hover_row {
            self.redraw.extend(self.hover_row);
            self.redraw.extend(hover);
            self.hover_row = hover;
        }

        // Keep a quarter viewport drawn beyond each edge; when that runs out,
        // draw ahead in half-viewport chunks so most scroll frames only move
        // the composition offset.
        let known_top = if self.rows.lo < self.rows.len() {
            self.rows.top[self.rows.lo]
        } else {
            self.scroll_top
        };
        let (lim0, lim1) = (
            self.row_px(known_top).floor() as i64,
            self.row_px(self.rows.bottom()).ceil() as i64,
        );
        let (above, below) = if fresh { (0.0, 1.0) } else { (0.25, 1.25) };
        let n0 = (self.row_px(self.scroll_top - vh * above).floor() as i64).max(lim0);
        let n1 = (self.row_px(self.scroll_top + vh * below).ceil() as i64)
            .min(lim1)
            .max(n0);
        let chunk = (vh * 0.5 * s) as i64;
        match self.drawn {
            Some((d0, d1)) if n0 < d1 && n1 > d0 => {
                let n0 = if n0 < d0 { (n0 - chunk).max(lim0) } else { n0 };
                let n1 = if n1 > d1 { (n1 + chunk).min(lim1) } else { n1 };
                if n0 < d0 {
                    self.draw_strip(n0, d0)?;
                }
                if n1 > d1 {
                    self.draw_strip(d1, n1)?;
                }
                let (m0, m1) = (n0.min(d0), n1.max(d1));
                let span = (vh * s) as i64;
                if m1 - m0 > span * 6 {
                    let (k0, k1) = ((n0 - span * 2).max(m0), (n1 + span * 2).min(m1));
                    unsafe {
                        self.vs.Trim(Some(&[RECT {
                            left: 0,
                            top: k0 as i32,
                            right: self.draw_w as i32,
                            bottom: k1 as i32,
                        }]))?
                    };
                    self.drawn = Some((k0, k1));
                } else {
                    self.drawn = Some((m0, m1));
                }
            }
            _ => {
                self.draw_strip(n0, n1)?;
                unsafe {
                    self.vs.Trim(Some(&[RECT {
                        left: 0,
                        top: n0 as i32,
                        right: self.draw_w as i32,
                        bottom: n1 as i32,
                    }]))?
                };
                self.drawn = Some((n0, n1));
            }
        }

        // Rows whose hover or selection changed.
        let mut redraw = std::mem::take(&mut self.redraw);
        redraw.sort_unstable();
        redraw.dedup();
        if let Some((d0, d1)) = self.drawn {
            for i in redraw {
                if i < self.rows.lo || self.rows.row(i).is_none() {
                    continue;
                }
                let y0 = (self.row_px(self.rows.top[i]).floor() as i64).max(d0);
                let y1 = (self
                    .row_px(self.rows.top[i] + self.rows.height(i) as f64)
                    .ceil() as i64)
                    .min(d1);
                self.draw_strip(y0, y1)?;
            }
        }
        match self.hover_row {
            Some(i) => self.toolbar.offset(
                ((self.list_w() - TOOLBAR.0 - 16.0) * self.scale).round(),
                (((self.rows.top[i] - self.scroll_top) as f32 - 12.0).max(0.0) * self.scale)
                    .round(),
            )?,
            None => self.toolbar.offset(-10_000.0, 0.0)?,
        }
        Ok(())
    }

    fn draw_panels(&mut self) -> Result<()> {
        let s = self.scale;
        let g = &self.g;
        let c = &self.c;
        let d = std::mem::take(&mut self.dirty);
        if d.rail {
            let hover = self.hover_space;
            self.rail.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(BLACKOUT, 1.0)));
                for (i, space) in c.spaces.iter().enumerate() {
                    let (x, y, w, h) = Self::space_rect(i);
                    let r = rect(x, y, x + w, y + h);
                    fill_round(
                        dc,
                        r,
                        12.0,
                        if hover == Some(i) {
                            &g.brushes.raised
                        } else {
                            &g.brushes.composer
                        },
                    );
                    if i == c.space {
                        stroke_round(
                            dc,
                            rect(r.left - 2.0, r.top - 2.0, r.right + 2.0, r.bottom + 2.0),
                            14.0,
                            &g.brushes.terracotta,
                        );
                    }
                    let initial: String = space
                        .name
                        .chars()
                        .next()
                        .map(String::from)
                        .unwrap_or_default();
                    let l = g.layout(&gfx::wide(&initial), &g.formats.rail, w)?;
                    l.SetMaxHeight(h)?;
                    text::draw(dc, &l, r.left, r.top, &g.brushes.text);
                }
                Ok(())
            })?;
        }
        if d.sidebar {
            let (hover, layouts) = (self.hover_channel, &self.channel_layouts);
            self.sidebar.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(SURFACE_SIDEBAR, 1.0)));
                let t = g.layout(
                    &gfx::wide(&c.spaces[c.space].name),
                    &g.formats.space,
                    10_000.0,
                )?;
                text::draw(dc, &t, 20.0, 16.0, &g.brushes.text);
                dc.FillRectangle(
                    &rect(0.0, HEADER_H - 1.0, SIDEBAR_W, HEADER_H),
                    &g.brushes.border,
                );
                let label = g.layout(
                    &gfx::wide(&format!("Channels  {}", c.channels.len())),
                    &g.formats.section,
                    10_000.0,
                )?;
                text::draw(dc, &label, 20.0, SECTION_TOP + 4.0, &g.brushes.muted);
                for (i, l) in layouts.iter().enumerate() {
                    let (_, y, w, h) = Self::channel_rect(i);
                    let r = rect(12.0, y, 12.0 + w, y + h);
                    if i == c.channel {
                        fill_round(dc, r, RADIUS, &g.brushes.selected);
                    } else if hover == Some(i) {
                        fill_round(dc, r, RADIUS, &g.brushes.hover);
                    }
                    let mark = g.layout(
                        &gfx::wide(if c.channels[i].private { "🔒" } else { "#" }),
                        &g.formats.channel,
                        40.0,
                    )?;
                    emoji::apply(
                        &mark,
                        if c.channels[i].private { "🔒" } else { "#" },
                        15.0,
                        0.9,
                    )?;
                    text::draw(
                        dc,
                        &mark,
                        24.0,
                        y + 5.0,
                        if i == c.channel {
                            &g.brushes.terracotta
                        } else {
                            &g.brushes.muted
                        },
                    );
                    text::draw(
                        dc,
                        l,
                        46.0,
                        y + 5.0,
                        if i == c.channel {
                            &g.brushes.text
                        } else {
                            &g.brushes.muted
                        },
                    );
                }
                Ok(())
            })?;
        }
        if d.header {
            let name = c.channels[c.channel].name.clone();
            let lw = self.list_w();
            self.header.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(SURFACE_CONVERSATION, 1.0)));
                let hash = g.layout(&gfx::wide("#"), &g.formats.header, 40.0)?;
                text::draw(dc, &hash, 24.0, 16.0, &g.brushes.muted);
                let l = g.layout(&gfx::wide(&name), &g.formats.header, 10_000.0)?;
                text::draw(dc, &l, 40.0, 16.0, &g.brushes.text);
                dc.FillRectangle(&rect(0.0, HEADER_H - 1.0, lw, HEADER_H), &g.brushes.border);
                Ok(())
            })?;
        }
        if d.members {
            let (hover, layouts) = (self.hover_member, &self.member_layouts);
            self.members.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(SURFACE_SIDEBAR, 1.0)));
                dc.FillRectangle(&rect(0.0, 0.0, 1.0, 100_000.0), &g.brushes.border);
                let t = g.layout(
                    &gfx::wide(&format!("Members  {}", c.members.len())),
                    &g.formats.section,
                    10_000.0,
                )?;
                text::draw(dc, &t, 20.0, 18.0, &g.brushes.text);
                dc.FillRectangle(
                    &rect(0.0, HEADER_H - 1.0, MEMBERS_W, HEADER_H),
                    &g.brushes.border,
                );
                let online = c.members.iter().filter(|m| m.online).count();
                let o = g.layout(
                    &gfx::wide(&format!("Online  {online}")),
                    &g.formats.section,
                    10_000.0,
                )?;
                text::draw(dc, &o, 12.0, SECTION_TOP + 4.0, &g.brushes.muted);
                for (i, l) in layouts.iter().enumerate() {
                    let top = MEMBER_TOP + MEMBER_H * i as f32;
                    if hover == Some(i) {
                        fill_round(
                            dc,
                            rect(8.0, top, MEMBERS_W - 8.0, top + MEMBER_H - 2.0),
                            RADIUS,
                            &g.brushes.hover,
                        );
                    }
                    let m = &c.members[i];
                    let av = rect(16.0, top + 5.0, 46.0, top + 35.0);
                    fill_round(dc, av, 15.0, &g.solid(m.color));
                    if m.online {
                        let e = D2D1_ELLIPSE {
                            point: windows_numerics::Vector2 {
                                X: av.right - 3.0,
                                Y: av.bottom - 3.0,
                            },
                            radiusX: 4.5,
                            radiusY: 4.5,
                        };
                        dc.FillEllipse(&e, &g.brushes.blackout);
                        let e = D2D1_ELLIPSE {
                            radiusX: 3.5,
                            radiusY: 3.5,
                            ..e
                        };
                        dc.FillEllipse(&e, &g.brushes.caper);
                    }
                    text::draw(
                        dc,
                        l,
                        56.0,
                        top + 11.0,
                        if m.online {
                            &g.brushes.text
                        } else {
                            &g.brushes.muted
                        },
                    );
                }
                Ok(())
            })?;
        }
        if d.toolbar {
            self.toolbar.draw(s, |dc| unsafe {
                dc.Clear(Some(&D2D1_COLOR_F::default()));
                let r = rect(0.0, 0.0, TOOLBAR.0, TOOLBAR.1);
                fill_round(dc, r, RADIUS, &g.brushes.raised);
                stroke_round(dc, r, RADIUS, &g.brushes.border);
                for (i, label) in ["☺", "↩", "⋯"].iter().enumerate() {
                    let l = g.layout(&gfx::wide(label), &g.formats.toolbar, 30.0)?;
                    l.SetMaxHeight(22.0)?;
                    text::draw(dc, &l, 3.0 + 30.0 * i as f32, 3.0, &g.brushes.text);
                }
                Ok(())
            })?;
        }
        if d.composer {
            let (_, _, bw, bh) = self.box_rect();
            let placeholder = format!("Message #{}", c.channels[c.channel].name);
            let comp = &self.composer;
            self.composer_p
                .draw(s, |dc| comp.draw(g, dc, bw, bh, &placeholder))?;
        }
        Ok(())
    }

    fn frame(&mut self) -> Result<()> {
        let started = Instant::now();
        match self.bench.frame() {
            Action::ScrollUp(dy) => self.scroll_by(-(dy as f64)),
            Action::Append => {
                self.c.push_demo();
                self.rows.append(&self.g, &self.c)?;
            }
            Action::Resize(w, h) => self.pending_resize = Some((w, h)),
            Action::Type(ch) => {
                let mut b = [0u16; 2];
                let s = ch.encode_utf16(&mut b).to_vec();
                self.composer.insert(&s);
                self.composer_changed();
            }
            Action::Quit => self.quit = true,
            Action::None => {}
        }
        if self.dirty.layout {
            self.dirty.layout = false;
            self.apply_layout()?;
        }
        self.update_history()?;
        self.draw_panels()?;
        unsafe { self.g.dcomp.Commit()? };
        self.bench.log_frame(started.elapsed());
        Ok(())
    }
}

fn set_clipboard(hwnd: HWND, text: &[u16]) {
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, (text.len() + 1) * 2) {
            let p = GlobalLock(h).cast::<u16>();
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(text.as_ptr(), p, text.len());
                *p.add(text.len()) = 0;
                let _ = GlobalUnlock(h);
                let _ = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(h.0)));
            }
        }
        let _ = CloseClipboard();
    }
}

fn get_clipboard(hwnd: HWND) -> Vec<u16> {
    let mut out = Vec::new();
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return out;
        }
        if let Ok(h) = GetClipboardData(CF_UNICODETEXT.0 as u32) {
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g).cast::<u16>();
            if !p.is_null() {
                let mut i = 0;
                while *p.add(i) != 0 {
                    out.push(*p.add(i));
                    i += 1;
                }
                let _ = GlobalUnlock(g);
            }
        }
        let _ = CloseClipboard();
    }
    out
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_GETOBJECT => {
                // Outside the app borrow: UIA may call straight back into the providers.
                if lp.0 as i32 == UiaRootObjectId {
                    return UiaReturnRawElementProvider(hwnd, wp, lp, &uia::node(uia::Id::Root));
                }
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            WM_DESTROY => {
                let _ = UiaReturnRawElementProvider(hwnd, WPARAM(0), LPARAM(0), None);
                PostQuitMessage(0);
                return LRESULT(0);
            }
            // An Alt tap must not enter menu mode (there is no menu bar).
            WM_SYSCOMMAND if (wp.0 & 0xFFF0) as u32 == SC_KEYMENU => return LRESULT(0),
            WM_ERASEBKGND => return LRESULT(1),
            WM_PAINT => {
                let _ = ValidateRect(Some(hwnd), None);
                return LRESULT(0);
            }
            WM_DPICHANGED => {
                let dpi = (wp.0 & 0xffff) as f32;
                with_app(|a| a.set_scale(dpi / 96.0));
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                return LRESULT(0);
            }
            // The composition is drawn inline in the composer.
            WM_IME_SETCONTEXT => {
                return DefWindowProcW(
                    hwnd,
                    msg,
                    wp,
                    LPARAM(lp.0 & !(ISC_SHOWUICOMPOSITIONWINDOW as isize)),
                );
            }
            _ => {}
        }
        if let Some(Some(r)) = with_app(|a| a.handle(msg, wp, lp)) {
            return r;
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}

fn resize_window(hwnd: HWND, w: f32, h: f32) {
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        let s = dpi as f32 / 96.0;
        let mut r = RECT {
            left: 0,
            top: 0,
            right: (w * s).round() as i32,
            bottom: (h * s).round() as i32,
        };
        let _ = AdjustWindowRectExForDpi(
            &mut r,
            WS_OVERLAPPEDWINDOW,
            false,
            WS_EX_NOREDIRECTIONBITMAP,
            dpi,
        );
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            r.right - r.left,
            r.bottom - r.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn main() -> Result<()> {
    let process_start = Instant::now();
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: w!("CaperWindows"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("CaperWindows"),
            w!("Caper"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1440,
            900,
            None,
            None,
            Some(instance.into()),
            None,
        )?;
        resize_window(hwnd, 1440.0, 900.0);
        let messages = std::env::var("MESSAGES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3000);
        let bench = Bench::from_env(process_start);
        let active = bench.active();
        let app = App::new(hwnd, Gfx::new()?, Conversation::demo(messages), bench)?;
        APP.with(|a| *a.borrow_mut() = Some(app));
        if active {
            SetTimer(Some(hwnd), TIMER_BENCH, 50, None);
        }
        let _ = ShowWindow(hwnd, SW_SHOW);
        let mut clock = true;
        loop {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let state = with_app(|a| {
                let render = !a.quit && (a.need || a.bench.wants_frames());
                if render {
                    a.need = false;
                    if let Err(e) = a.frame() {
                        eprintln!("frame failed: {e:?}");
                    }
                }
                (
                    render,
                    a.quit,
                    a.pending_resize.take(),
                    std::mem::take(&mut a.events),
                )
            });
            let Some((rendered, quit, resize, events)) = state else {
                continue;
            };
            if let Some((w, h)) = resize {
                resize_window(hwnd, w, h);
            }
            if events & EV_TEXT != 0 {
                uia::raise_composer_events();
            }
            if events & EV_FOCUS != 0 {
                uia::raise_focus();
            }
            if events & EV_ROWS != 0 {
                uia::raise_rows_changed();
            }
            if quit {
                let _ = DestroyWindow(hwnd);
                continue;
            }
            if rendered {
                // One frame per compositor tick (display refresh).
                if clock && DCompositionWaitForCompositorClock(None, 100) == u32::MAX {
                    clock = false;
                }
                if !clock {
                    let _ = DwmFlush();
                }
            } else {
                let _ = WaitMessage();
            }
        }
    }
}
