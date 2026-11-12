//! Tooltips, as egui shows them: rest the pointer on an icon-only control
//! (a space's rail tile, a toolbar or call button, a viewer or editor tool)
//! for half a second and its name appears under it. Moving off it, a click,
//! a key, the wheel or leaving the window hides it. The names are the
//! controls' UI Automation names (`uia::tip_at`).
use crate::gfx::{fill_round, rect, stroke_round};
use crate::overlay::{Rect, height, layout};
use crate::{App, EV_SCREEN};
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_METRICS;
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

pub const TIMER_TIP: usize = 41;
/// egui's tooltip delay.
const DELAY_MS: u32 = 500;
const MAX_W: f32 = 280.0;
const PAD_X: f32 = 8.0;
const PAD_Y: f32 = 5.0;

#[derive(Default)]
pub struct State {
    /// The shown tip: its text and the control it names.
    shown: Option<(String, Rect)>,
    /// Where the pointer last moved, so a repeated WM_MOUSEMOVE that did
    /// not move it keeps the timer.
    last: Option<(f32, f32)>,
}

impl App {
    /// Follows the pointer and the keys, after the window handled them.
    pub fn tip_input(&mut self, msg: u32, x: f32, y: f32) {
        match msg {
            WM_MOUSEMOVE => {
                if self.tip.last == Some((x, y)) {
                    return;
                }
                self.tip.last = Some((x, y));
                if let Some((_, (l, t, w, h))) = &self.tip.shown
                    && x >= *l
                    && x < l + w
                    && y >= *t
                    && y < t + h
                {
                    return;
                }
                self.hide_tip();
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_TIP, DELAY_MS, None);
                }
            }
            crate::WM_MOUSELEAVE
            | WM_LBUTTONDOWN
            | WM_RBUTTONDOWN
            | WM_MBUTTONDOWN
            | WM_MOUSEWHEEL
            | WM_MOUSEHWHEEL
            | WM_KEYDOWN
            | WM_SYSKEYDOWN
            | WM_KILLFOCUS => {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_TIP);
                }
                self.hide_tip();
            }
            _ => {}
        }
    }

    fn hide_tip(&mut self) {
        if self.tip.shown.take().is_some() {
            self.tip_changed();
        }
    }

    fn tip_changed(&mut self) {
        self.dirty.tip = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// The pointer rested: the control under it, if it has a tip.
    pub fn tip_tick(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_TIP);
        }
        if unsafe { GetForegroundWindow() } == self.hwnd {
            let mut p = POINT::default();
            unsafe {
                let _ = GetCursorPos(&mut p);
                let _ = ScreenToClient(self.hwnd, &mut p);
            }
            let (x, y) = (p.x as f32 / self.scale, p.y as f32 / self.scale);
            let inside = self.signin.is_none() && x >= 0.0 && y >= 0.0 && x < self.w && y < self.h;
            self.tip.shown = inside.then(|| crate::uia::tip_at(self, x, y)).flatten();
            if self.tip.shown.is_some() {
                self.tip_changed();
            }
        }
    }

    /// Redraws the tip's surface when it changed: under the control, or
    /// above it near the window's bottom, kept inside the window.
    pub fn draw_tip(&mut self) -> Result<()> {
        if !std::mem::take(&mut self.dirty.tip) {
            return Ok(());
        }
        let Some((text, (cl, ct, cw, ch))) = &self.tip.shown else {
            return self.tip_p.offset(-100_000.0, 0.0);
        };
        let g = &self.g;
        let l = layout(g, text, &g.formats.direct_note, MAX_W - 2.0 * PAD_X)?;
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { l.GetMetrics(&mut m)? };
        let (w, h) = (m.width.ceil() + 2.0 * PAD_X, height(&l) + 2.0 * PAD_Y);
        let x = (cl + cw / 2.0 - w / 2.0).clamp(4.0, (self.w - w - 4.0).max(4.0));
        let below = ct + ch + 6.0;
        let y = if below + h <= self.h - 4.0 {
            below
        } else {
            (ct - 6.0 - h).max(4.0)
        };
        let scale = self.scale;
        let size = |v: f32| (v * scale).ceil().max(1.0) as u32;
        self.tip_p.ensure(g, size(w), size(h))?;
        self.tip_p
            .offset((x * scale).round(), (y * scale).round())?;
        self.tip_p.draw(scale, |dc| unsafe {
            dc.Clear(Some(&D2D1_COLOR_F::default()));
            let body = rect(0.5, 0.5, w - 0.5, h - 0.5);
            fill_round(dc, body, 6.0, &g.brushes.surface);
            stroke_round(dc, body, 6.0, &g.brushes.border);
            crate::text::draw(dc, &l, PAD_X, PAD_Y, &g.brushes.text);
            Ok(())
        })
    }
}
