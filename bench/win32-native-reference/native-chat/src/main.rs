//! Caper's conversation screen as a fully native Windows app: Win32 window, DirectComposition
//! visual tree with one retained surface per panel, Direct2D + DirectWrite drawing, UI Automation.
//!
//! Damage model: every panel has its own composition surface and is redrawn only when its own
//! state changes. The message list is one tall virtual surface: scrolling moves the visual
//! (no redraw) and only newly exposed strips are drawn; hover and selection redraw the rows they
//! touch; the hover toolbar is its own layer that only moves.
#![windows_subsystem = "windows"]

mod bench_probe;
mod composer;
mod gfx;
mod rows;
mod uia;

use composer::Composer;
use fixture::{Action, Bench, Fixture};
use gfx::{Gfx, fill_round, rect, stroke_round};
use rows::Rows;
use std::cell::RefCell;
use std::time::Instant;
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

const RAIL_W: f32 = 64.0;
const CHAN_W: f32 = 240.0;
const MEM_W: f32 = 220.0;
const HEADER_H: f32 = 45.0;
const CHAN_TOP: f32 = 40.0;
const CHAN_H: f32 = 28.0;
const MEM_TOP: f32 = 34.0;
const MEM_H: f32 = 31.0;
const TOOLBAR: (f32, f32) = (96.0, 28.0);
/// Pixel row of the anchor (bottom of the startup history) inside the virtual surface.
const ORIGIN: f64 = 4_000_000.0;
const VS_H: u32 = 1 << 23;
const RAIL_ABBR: [&str; 8] = ["Ca", "Fr", "Gm", "Mu", "Wk", "Ds", "Rd", "Ph"];
const EV_TEXT: u8 = 1;
const EV_FOCUS: u8 = 2;
const EV_ROWS: u8 = 4;
const TIMER_BENCH: usize = 1;
const TIMER_CARET: usize = 2;

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

/// Runs `f` on the app unless it is already borrowed (re-entrant window messages, UIA calls
/// arriving while the app is busy) or not created yet.
pub fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

#[derive(Default, Clone, Copy)]
struct Dirty {
    layout: bool,
    rail: bool,
    channels: bool,
    header: bool,
    members: bool,
    composer: bool,
    toolbar: bool,
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

    /// Ensures a surface of this pixel size; true when it was (re)created and needs drawing.
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
            Some(s) => gfx::draw(s, None, scale, f),
            None => Ok(()),
        }
    }
}

type Sel = ((usize, u32), (usize, u32));

pub struct App {
    pub hwnd: HWND,
    pub g: Gfx,
    pub fx: Fixture,
    bench: Bench,
    pub rows: Rows,
    pub composer: Composer,
    pub selected_channel: usize,
    pub scale: f32,
    pub w: f32,
    pub h: f32,
    _target: IDCompositionTarget,
    background: Panel,
    rail: Panel,
    channels: Panel,
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
    hover_rail: Option<usize>,
    hover_channel: Option<usize>,
    pointer: Option<(f32, f32)>,
    tracking: bool,
    sel: Option<Sel>,
    selecting: bool,
    composer_drag: bool,
    redraw: Vec<usize>,
    dirty: Dirty,
    need: bool,
    box_h: f32,
    pending_resize: Option<(f32, f32)>,
    events: u8,
    visible: std::ops::Range<usize>,
    channel_layouts: Vec<IDWriteTextLayout>,
    member_layouts: Vec<IDWriteTextLayout>,
    quit: bool,
}

impl App {
    fn new(hwnd: HWND, g: Gfx, fx: Fixture, bench: Bench) -> Result<Self> {
        unsafe {
            let target = g.dcomp.CreateTargetForHwnd(hwnd, true)?;
            let root = g.visual()?;
            root.SetBorderMode(DCOMPOSITION_BORDER_MODE_HARD)?;
            target.SetRoot(&root)?;
            let background = Panel::new(&g)?;
            background.ensure_solid(&g)?;
            background.visual.SetBitmapInterpolationMode(
                DCOMPOSITION_BITMAP_INTERPOLATION_MODE_NEAREST_NEIGHBOR,
            )?;
            let rail = Panel::new(&g)?;
            let channels = Panel::new(&g)?;
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
            let mut prev: IDCompositionVisual = background.visual.cast()?;
            root.AddVisual(&background.visual, false, None)?;
            for v in [
                &rail.visual,
                &channels.visual,
                &header.visual,
                &history_clip,
                &composer_p.visual,
                &members.visual,
            ] {
                root.AddVisual(v, true, &prev)?;
                prev = v.cast()?;
            }
            let layout = |s: String, f: &IDWriteTextFormat| g.layout(&gfx::wide(&s), f, 10_000.0);
            let channel_layouts = fx
                .channels
                .iter()
                .map(|c| layout(format!("# {c}"), &g.formats.channel))
                .collect::<Result<_>>()?;
            let member_layouts = fx
                .authors
                .iter()
                .map(|a| layout(a.name.clone(), &g.formats.member))
                .collect::<Result<_>>()?;
            let n = fx.messages.len();
            let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
            let mut rc = RECT::default();
            GetClientRect(hwnd, &mut rc)?;
            Ok(Self {
                hwnd,
                g,
                fx,
                bench,
                rows: Rows::new(n),
                composer: Composer::default(),
                selected_channel: 0,
                scale,
                w: rc.right as f32 / scale,
                h: rc.bottom as f32 / scale,
                _target: target,
                background,
                rail,
                channels,
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
                hover_rail: None,
                hover_channel: None,
                pointer: None,
                tracking: false,
                sel: None,
                selecting: false,
                composer_drag: false,
                redraw: Vec::new(),
                dirty: Dirty {
                    layout: true,
                    rail: true,
                    channels: true,
                    header: true,
                    members: true,
                    composer: true,
                    toolbar: true,
                },
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

    fn cx(&self) -> f32 {
        RAIL_W + CHAN_W
    }

    fn cw(&self) -> f32 {
        (self.w - RAIL_W - CHAN_W - MEM_W).max(120.0)
    }

    fn box_rect(&self) -> (f32, f32, f32, f32) {
        let w = self.cw() - 36.0;
        (self.cx() + 18.0, self.h - 16.0 - self.box_h, w, self.box_h)
    }

    fn vh(&self) -> f32 {
        (self.box_rect().1 - 8.0 - HEADER_H).max(0.0)
    }

    pub fn visible_rows(&self) -> std::ops::Range<usize> {
        self.rows
            .range(self.scroll_top, self.scroll_top + self.vh() as f64)
    }

    pub fn composer_text_origin(&self) -> (f32, f32, f32, f32) {
        let (x, y, w, h) = self.box_rect();
        (
            x + composer::PAD,
            y + composer::PAD - self.composer.scroll(),
            w,
            h,
        )
    }

    pub fn node_rect(&self, id: uia::Id) -> (f32, f32, f32, f32) {
        use uia::Id;
        let (w, h) = (self.w, self.h);
        match id {
            Id::Root => (0.0, 0.0, w, h),
            Id::Rail => (0.0, 0.0, RAIL_W, h),
            Id::RailItem(i) => (12.0, 12.0 + 48.0 * i as f32, 40.0, 40.0),
            Id::Channels => (RAIL_W, 0.0, CHAN_W, h),
            Id::Channel(i) => (RAIL_W + 12.0, CHAN_TOP + CHAN_H * i as f32, 216.0, CHAN_H),
            Id::History => (self.cx(), HEADER_H, self.cw(), self.vh()),
            Id::Row(i) => {
                let r = self.rows.rows[i].as_ref().map_or(0.0, |r| r.height);
                (
                    self.cx(),
                    HEADER_H + (self.rows.top[i] - self.scroll_top) as f32,
                    self.cw(),
                    r,
                )
            }
            Id::Members => (w - MEM_W, 0.0, MEM_W, h),
            Id::Member(i) => (
                w - MEM_W + 12.0,
                MEM_TOP + MEM_H * i as f32,
                MEM_W - 24.0,
                28.0,
            ),
            Id::Composer => self.box_rect(),
        }
    }

    // ---- state changes ----

    pub fn composer_changed(&mut self) {
        self.composer.caret_on = true;
        let (_, _, bw, _) = self.box_rect();
        if let Ok(h) = self.composer.relayout(&self.g, bw - 2.0 * composer::PAD) {
            if (h - self.box_h).abs() > 0.01 {
                self.dirty.layout = true;
            }
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

    pub fn invoke(&mut self, id: uia::Id) {
        if let uia::Id::Channel(i) = id {
            self.select_channel(i);
        }
    }

    fn select_channel(&mut self, i: usize) {
        if i != self.selected_channel {
            self.selected_channel = i;
            self.dirty.channels = true;
            self.dirty.header = true;
            self.dirty.composer = true;
            self.need = true;
        }
    }

    fn sel_for(sel: Option<Sel>, i: usize, len: u32) -> Option<(u32, u32)> {
        let ((ra, pa), (rb, pb)) = sel?;
        let ((ra, pa), (rb, pb)) = if (ra, pa) <= (rb, pb) {
            ((ra, pa), (rb, pb))
        } else {
            ((rb, pb), (ra, pa))
        };
        if i < ra || i > rb {
            return None;
        }
        let s = if i == ra { pa } else { 0 };
        let e = if i == rb { pb } else { len };
        (e > s).then_some((s, e))
    }

    fn row_len(&self, i: usize) -> u32 {
        self.rows.rows[i]
            .as_ref()
            .map_or(0, |r| r.plain.len() as u32)
    }

    fn set_sel(&mut self, new: Option<Sel>) {
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
        let row = self.rows.rows[i].as_ref()?;
        let pad = if self.fx.messages[i].grouped {
            2.0
        } else {
            10.0
        };
        let tx = x - (self.cx() + rows::PAD_X + rows::AVATAR + rows::GAP);
        let ty = (ay - self.rows.top[i]) as f32 - pad - row.text_y;
        let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
        let mut m = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            let _ = row
                .text
                .HitTestPoint(tx, ty, &mut trailing, &mut inside, &mut m);
        }
        Some((
            i,
            m.textPosition + if trailing.as_bool() { m.length } else { 0 },
        ))
    }

    fn selected_text(&self) -> Vec<u16> {
        let mut out = Vec::new();
        let Some(((ra, _), (rb, _))) = self.sel else {
            return out;
        };
        for i in ra.min(rb)..=ra.max(rb) {
            if let (Some((s, e)), Some(r)) = (
                Self::sel_for(self.sel, i, self.row_len(i)),
                self.rows.rows[i].as_ref(),
            ) {
                if !out.is_empty() {
                    out.extend_from_slice(&gfx::wide("\r\n"));
                }
                out.extend_from_slice(&r.plain[s as usize..e as usize]);
            }
        }
        out
    }

    fn in_box(&self, x: f32, y: f32) -> bool {
        let (bx, by, bw, bh) = self.box_rect();
        x >= bx && x < bx + bw && y >= by && y < by + bh
    }

    fn in_history(&self, x: f32, y: f32) -> bool {
        x >= self.cx() && x < self.cx() + self.cw() && y >= HEADER_H && y < HEADER_H + self.vh()
    }

    fn update_hover(&mut self) {
        let (mut rail, mut chan) = (None, None);
        if let Some((x, y)) = self.pointer {
            if x < RAIL_W {
                let i = ((y - 12.0) / 48.0).floor();
                if i >= 0.0
                    && (i as usize) < 8
                    && (12.0..52.0).contains(&x)
                    && (y - 12.0 - 48.0 * i) < 40.0
                {
                    rail = Some(i as usize);
                }
            } else if x < RAIL_W + CHAN_W
                && (RAIL_W + 12.0..RAIL_W + 228.0).contains(&x)
                && y >= CHAN_TOP
            {
                let i = ((y - CHAN_TOP) / CHAN_H) as usize;
                if i < self.fx.channels.len() {
                    chan = Some(i);
                }
            }
        }
        if rail != self.hover_rail {
            self.hover_rail = rail;
            self.dirty.rail = true;
            self.need = true;
        }
        if chan != self.hover_channel {
            self.hover_channel = chan;
            self.dirty.channels = true;
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
                TIMER_BENCH => {
                    if self.bench.tick() {
                        self.quit = true;
                    }
                }
                TIMER_CARET => {
                    self.composer.caret_on = !self.composer.caret_on;
                    self.dirty.composer = true;
                    self.need = true;
                }
                _ => {}
            },
            WM_MOUSEMOVE => {
                if !self.tracking {
                    let mut t = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: self.hwnd, dwHoverTime: 0 };
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
                    let (tx, ty, _, _) = self.composer_text_origin();
                    let p = self.composer.hit(x - tx, y - ty - self.composer.scroll());
                    if p != self.composer.caret {
                        self.composer.move_to(p, true);
                        self.composer_changed();
                    }
                } else if self.selecting {
                    if let (Some(h), Some((a, _))) = (self.msg_hit(x, y), self.sel) {
                        self.set_sel(Some((a, h)));
                    }
                }
            }
            0x02A3 /* WM_MOUSELEAVE */ => {
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
                    let (tx, ty, _, _) = self.composer_text_origin();
                    let p = self.composer.hit(x - tx, y - ty - self.composer.scroll());
                    self.composer.move_to(p, shift);
                    self.composer_drag = true;
                    self.composer_changed();
                } else {
                    self.set_composer_focus(false);
                    if self.in_history(x, y) {
                        if let Some(h) = self.msg_hit(x, y) {
                            let anchor = if shift { self.sel.map_or(h, |s| s.0) } else { h };
                            self.set_sel(Some((anchor, h)));
                            self.selecting = true;
                        }
                    } else if let Some(c) = self.hover_channel {
                        self.select_channel(c);
                    }
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
                    self.scroll_top -= delta / 120.0 * 48.0;
                    self.at_end = false;
                    self.need = true;
                }
            }
            WM_SETCURSOR => {
                if (lp.0 & 0xffff) as u32 != HTCLIENT {
                    return None;
                }
                let beam = self.pointer.is_some_and(|(x, y)| self.in_box(x, y) || self.in_history(x, y));
                unsafe {
                    SetCursor(LoadCursorW(None, if beam { IDC_IBEAM } else { IDC_ARROW }).ok());
                }
                return Some(LRESULT(1));
            }
            WM_KEYDOWN => return self.key(VIRTUAL_KEY(wp.0 as u16), ctrl, shift).then_some(LRESULT(0)),
            WM_CHAR => {
                let c = wp.0 as u16;
                if self.composer.focused && !ctrl && (c >= 0x20 && c != 0x7f || c == '\r' as u16) {
                    self.composer.insert(&[if c == '\r' as u16 { '\n' as u16 } else { c }]);
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
            return false;
        }
        let c = &mut self.composer;
        let (a, b) = c.selection();
        match vk {
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
                c.move_to(n, shift)
            }
            VK_BACK => c.backspace(ctrl),
            VK_DELETE => c.delete(ctrl),
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
        let (tx, ty, _, _) = self.composer_text_origin();
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
            let cf = COMPOSITIONFORM {
                dwStyle: CFS_POINT,
                ptCurrentPos: p,
                rcArea: RECT::default(),
            };
            let _ = ImmSetCompositionWindow(himc, &cf);
            let area = RECT {
                left: p.x,
                top: p.y,
                right: p.x + 2,
                bottom: p.y + (ch * s) as i32,
            };
            let cand = CANDIDATEFORM {
                dwIndex: 0,
                dwStyle: CFS_EXCLUDE,
                ptCurrentPos: p,
                rcArea: area,
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
                ImmGetCompositionStringW(himc, kind, Some(buf.as_mut_ptr() as _), bytes as u32);
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
        self.dirty = Dirty {
            layout: true,
            rail: true,
            channels: true,
            header: true,
            members: true,
            composer: true,
            toolbar: true,
        };
        self.need = true;
    }

    fn apply_layout(&mut self) -> Result<()> {
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let (w, h, cx, cw) = (self.w, self.h, self.cx(), self.cw());
        unsafe {
            self.background.visual.SetTransform2(&Matrix3x2 {
                M11: w * s,
                M12: 0.0,
                M21: 0.0,
                M22: h * s,
                M31: 0.0,
                M32: 0.0,
            })?;
        }
        self.dirty.rail |= self.rail.ensure(&self.g, px(RAIL_W), px(h))?;
        self.dirty.channels |= self.channels.ensure(&self.g, px(CHAN_W), px(h))?;
        self.channels.offset((RAIL_W * s).round(), 0.0)?;
        self.dirty.header |= self.header.ensure(&self.g, px(cw), px(HEADER_H))?;
        self.header.offset((cx * s).round(), 0.0)?;
        self.dirty.members |= self.members.ensure(&self.g, px(MEM_W), px(h))?;
        self.members.offset(((w - MEM_W) * s).round(), 0.0)?;
        self.dirty.toolbar |= self.toolbar.ensure(&self.g, px(TOOLBAR.0), px(TOOLBAR.1))?;
        let bw = cw - 36.0;
        self.box_h = self.composer.relayout(&self.g, bw - 2.0 * composer::PAD)?;
        let (bx, by, _, _) = self.box_rect();
        self.composer_p.ensure(&self.g, px(bw), px(self.box_h))?;
        self.composer_p.offset((bx * s).round(), (by * s).round())?;
        self.dirty.composer = true;
        let vh = self.vh();
        unsafe {
            self.history_clip.SetOffsetX2((cx * s).round())?;
            self.history_clip.SetOffsetY2((HEADER_H * s).round())?;
            self.history_clip
                .SetClip2(&rect(0.0, 0.0, (cw * s).round(), (vh * s).round()))?;
        }
        let body = Rows::body_width(cw);
        if (body - self.rows.width).abs() > 0.01 {
            // Text rewraps: relayout rows, keeping the top visible row in place unless at the end.
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
                    self.rows.measure_up_to(&self.g, &self.fx, y)?;
                }
                self.scroll_top = self.rows.top[i] + frac;
            }
            self.drawn = None;
        }
        // The surface only grows (in 512 px steps): resizing it every frame of a window drag
        // would reallocate its tiles. Columns beyond the list width are clipped by the visual.
        if px(cw) > self.vs_w {
            self.vs_w = px(cw).next_multiple_of(512);
            unsafe { self.vs.Resize(self.vs_w, VS_H)? };
            self.drawn = None;
        }
        if px(cw) != self.draw_w || self.drawn_scale != s {
            self.draw_w = px(cw);
            self.drawn_scale = s;
            self.drawn = None;
        }
        Ok(())
    }

    fn row_px(&self, y: f64) -> f64 {
        ORIGIN + y * self.scale as f64
    }

    /// Draws pixel rows [y0, y1) of the message surface.
    fn draw_strip(&mut self, y0: i64, y1: i64) -> Result<()> {
        let _t = SubTimer(Instant::now());
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
        let (a, b) = (
            (y0 as f64 - ORIGIN) / s as f64,
            (y1 as f64 - ORIGIN) / s as f64,
        );
        let range = self.rows.range(a, b);
        let cw = self.cw();
        let (hover, sel) = (self.hover_row, self.sel);
        let tops: Vec<(usize, f32, f32, Option<(u32, u32)>)> = range
            .map(|i| {
                let local = ((self.row_px(self.rows.top[i]) - y0 as f64) / s as f64) as f32;
                (
                    i,
                    local,
                    self.rows.rows[i].as_ref().unwrap().height,
                    Self::sel_for(sel, i, self.row_len(i)),
                )
            })
            .collect();
        let (g, fx, rows) = (&self.g, &self.fx, &mut self.rows);
        gfx::draw_local(&self.vs, r, s, |dc| unsafe {
            dc.Clear(Some(&gfx::color(fixture::BLACKOUT, 1.0)));
            for (i, y, h, rs) in tops {
                if hover == Some(i) {
                    dc.FillRectangle(&rect(0.0, y, cw, y + h), &g.brushes.surface);
                }
                rows::draw_row(g, dc, fx, rows, i, y, cw, rs)?;
            }
            Ok(())
        })
    }

    fn update_history(&mut self) -> Result<()> {
        let tm = Instant::now();
        let s = self.scale as f64;
        let vh = self.vh() as f64;
        // After a rewrap (or at startup) only the viewport is measured and drawn this frame;
        // the margins fill in on the following frames.
        let fresh = self.drawn.is_none();
        let margin = if fresh { 0.0 } else { vh };
        self.rows
            .measure_up_to(&self.g, &self.fx, self.scroll_top - margin)?;
        if self.at_end {
            self.scroll_top = self.rows.bottom() - vh;
            self.rows
                .measure_up_to(&self.g, &self.fx, self.scroll_top - margin)?;
        }
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
        unsafe {
            self.content.SetOffsetY2(-(off as f32))?;
        }
        SUB[0].store(
            tm.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
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

        // Keep a quarter viewport drawn beyond each edge; when that margin runs out, draw ahead
        // in half-viewport chunks so most scroll frames only move the composition offset.
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
                let (n0, n1) = (
                    if n0 < d0 { (n0 - chunk).max(lim0) } else { n0 },
                    if n1 > d1 { (n1 + chunk).min(lim1) } else { n1 },
                );
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
                let Some(r) = self.rows.rows.get(i).and_then(|r| r.as_ref()) else {
                    continue;
                };
                if i < self.rows.lo {
                    continue;
                }
                let y0 = (self.row_px(self.rows.top[i]).floor() as i64).max(d0);
                let y1 = (self.row_px(self.rows.top[i] + r.height as f64).ceil() as i64).min(d1);
                self.draw_strip(y0, y1)?;
            }
        }

        match self
            .hover_row
            .and_then(|i| self.rows.rows[i].as_ref().map(|_| i))
        {
            Some(i) => self.toolbar.offset(
                ((self.cw() - 112.0) * self.scale).round(),
                (((self.rows.top[i] - self.scroll_top) as f32 - 12.0) * self.scale).round(),
            )?,
            None => self.toolbar.offset(-10_000.0, 0.0)?,
        }
        Ok(())
    }

    fn draw_panels(&mut self) -> Result<()> {
        let s = self.scale;
        let g = &self.g;
        let d = std::mem::take(&mut self.dirty);
        if d.rail {
            let hover = self.hover_rail;
            self.rail.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(fixture::BLACKOUT, 1.0)));
                for (i, abbr) in RAIL_ABBR.iter().enumerate() {
                    let r = rect(12.0, 12.0 + 48.0 * i as f32, 52.0, 52.0 + 48.0 * i as f32);
                    fill_round(
                        dc,
                        r,
                        10.0,
                        if i == 0 || hover == Some(i) {
                            &g.brushes.terracotta
                        } else {
                            &g.brushes.surface
                        },
                    );
                    let l = g.layout(&gfx::wide(abbr), &g.formats.rail, 40.0)?;
                    l.SetMaxHeight(40.0)?;
                    gfx::text(dc, &l, r.left, r.top, &g.brushes.text);
                }
                Ok(())
            })?;
        }
        if d.channels {
            let (hover, selected, layouts) = (
                self.hover_channel,
                self.selected_channel,
                &self.channel_layouts,
            );
            self.channels.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(fixture::SURFACE, 1.0)));
                let t = g.layout(&gfx::wide("Caper Crew"), &g.formats.title, 10_000.0)?;
                gfx::text(dc, &t, 12.0, 12.0, &g.brushes.text);
                for (i, l) in layouts.iter().enumerate() {
                    let top = CHAN_TOP + CHAN_H * i as f32;
                    let r = rect(12.0, top, 228.0, top + CHAN_H);
                    if i == selected {
                        fill_round(dc, r, 4.0, &g.brushes.terracotta_30);
                    } else if hover == Some(i) {
                        fill_round(dc, r, 4.0, &g.brushes.border);
                    }
                    gfx::text(dc, l, 20.0, top + 5.0, &g.brushes.text);
                }
                Ok(())
            })?;
        }
        if d.header {
            let name = format!("# {}", self.fx.channels[self.selected_channel]);
            let cw = self.cw();
            self.header.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(fixture::BLACKOUT, 1.0)));
                let l = g.layout(&gfx::wide(&name), &g.formats.header, 10_000.0)?;
                gfx::text(dc, &l, 18.0, 12.0, &g.brushes.text);
                dc.FillRectangle(&rect(0.0, HEADER_H - 1.0, cw, HEADER_H), &g.brushes.border);
                Ok(())
            })?;
        }
        if d.members {
            let (fx, layouts) = (&self.fx, &self.member_layouts);
            self.members.draw(s, |dc| unsafe {
                dc.Clear(Some(&gfx::color(fixture::BLACKOUT, 1.0)));
                let t = g.layout(
                    &gfx::wide(&format!("Members — {}", fx.authors.len())),
                    &g.formats.members_title,
                    10_000.0,
                )?;
                gfx::text(dc, &t, 12.0, 12.0, &g.brushes.muted);
                for (i, l) in layouts.iter().enumerate() {
                    let top = MEM_TOP + MEM_H * i as f32;
                    let av = rect(12.0, top, 40.0, top + 28.0);
                    fill_round(dc, av, 7.0, &g.brushes.authors[i % 6]);
                    if i % 3 != 0 {
                        let e = D2D1_ELLIPSE {
                            point: windows_numerics::Vector2 {
                                X: av.right - 3.0,
                                Y: av.bottom - 3.0,
                            },
                            radiusX: 4.0,
                            radiusY: 4.0,
                        };
                        dc.FillEllipse(&e, &g.brushes.green);
                    }
                    gfx::text(dc, l, 48.0, top + 6.0, &g.brushes.text);
                }
                Ok(())
            })?;
        }
        if d.toolbar {
            self.toolbar.draw(s, |dc| unsafe {
                dc.Clear(Some(&D2D1_COLOR_F::default()));
                let r = rect(0.0, 0.0, TOOLBAR.0, TOOLBAR.1);
                fill_round(dc, r, 8.0, &g.brushes.surface);
                stroke_round(dc, r, 8.0, &g.brushes.border);
                for (i, label) in ["☺", "↩", "⋯"].iter().enumerate() {
                    let l = g.layout(&gfx::wide(label), &g.formats.toolbar, 30.0)?;
                    l.SetMaxHeight(22.0)?;
                    gfx::text(dc, &l, 3.0 + 30.0 * i as f32, 3.0, &g.brushes.text);
                }
                Ok(())
            })?;
        }
        if d.composer {
            let (_, _, bw, bh) = self.box_rect();
            let c = &self.composer;
            self.composer_p.draw(s, |dc| c.draw(g, dc, bw, bh))?;
        }
        Ok(())
    }

    fn frame(&mut self) -> Result<()> {
        let _probe = bench_probe::Probe::start();
        match self.bench.frame() {
            Action::ScrollUp(dy) => {
                self.scroll_top -= dy as f64;
                self.at_end = false;
            }
            Action::Append => {
                self.fx.push();
                self.rows.append(&self.g, &self.fx)?;
            }
            Action::Resize(w, h) => self.pending_resize = Some((w, h)),
            Action::Type(c) => {
                let mut b = [0u16; 2];
                let s = c.encode_utf16(&mut b).to_vec();
                self.composer.insert(&s);
                self.composer_changed();
            }
            Action::Quit => self.quit = true,
            Action::None => {}
        }
        let t0 = Instant::now();
        if self.dirty.layout {
            self.dirty.layout = false;
            self.apply_layout()?;
        }
        let t1 = Instant::now();
        self.update_history()?;
        let t2 = Instant::now();
        self.draw_panels()?;
        let t3 = Instant::now();
        unsafe { self.g.dcomp.Commit()? };
        phases(&[t0, t1, t2, t3, Instant::now()]);
        Ok(())
    }
}

impl Panel {
    /// A 1×1 Blackout surface, scaled by the visual's transform to cover the window.
    fn ensure_solid(&self, g: &Gfx) -> Result<()> {
        let s = g.surface(1, 1)?;
        gfx::draw(&s, None, 1.0, |dc| unsafe {
            dc.Clear(Some(&gfx::color(fixture::BLACKOUT, 1.0)));
            Ok(())
        })?;
        unsafe { self.visual.SetContent(&s) }
    }
}

pub static SUB: [std::sync::atomic::AtomicU64; 4] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 4];
struct SubTimer(Instant);
impl Drop for SubTimer {
    fn drop(&mut self) {
        SUB[1].fetch_add(
            self.0.elapsed().as_micros() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

/// SCRATCH: with NATIVE_PHASES=<file>, one line per frame: layout, history, panels, commit (µs).
fn phases(t: &[Instant; 5]) {
    use std::io::Write;
    thread_local! {
        static OUT: RefCell<Option<std::io::BufWriter<std::fs::File>>> = RefCell::new(std::env::var("NATIVE_PHASES").ok().and_then(|p| std::fs::File::create(p).ok()).map(std::io::BufWriter::new));
    }
    OUT.with(|o| {
        if let Some(f) = o.borrow_mut().as_mut() {
            let us = |i: usize| (t[i + 1] - t[i]).as_micros();
            let _ = writeln!(
                f,
                "{} {} {} {} {} {} {} {}",
                us(0),
                us(1),
                us(2),
                us(3),
                SUB[0].swap(0, std::sync::atomic::Ordering::Relaxed),
                SUB[1].swap(0, std::sync::atomic::Ordering::Relaxed),
                SUB[2].swap(0, std::sync::atomic::Ordering::Relaxed),
                SUB[3].swap(0, std::sync::atomic::Ordering::Relaxed)
            );
            let _ = f.flush();
        }
    });
}

fn set_clipboard(hwnd: HWND, text: &[u16]) {
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return;
        }
        let _ = EmptyClipboard();
        if let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, (text.len() + 1) * 2) {
            let p = GlobalLock(h) as *mut u16;
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
            let p = GlobalLock(g) as *const u16;
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
                // Handled outside the app borrow: UIA may call straight back into the providers.
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
            // Alt taps (used by the harness to take the foreground) must not enter menu mode.
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
            lpszClassName: w!("CaperNativeBench"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP,
            w!("CaperNativeBench"),
            w!("Caper (native Win32)"),
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
            .unwrap_or(fixture::MESSAGES);
        let fx = Fixture::new(messages);
        let bench = Bench::from_env("native", process_start);
        let active = bench.active();
        let app = App::new(hwnd, Gfx::new()?, fx, bench)?;
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
                // Pace to the compositor clock (one frame per display refresh).
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
