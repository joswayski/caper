//! Middle-click history scrolling, with GTK's 12-DIP dead zone and speed.
//! A click latches; a held drag stops on release. The history's usual scroll
//! path keeps paging, clamping and live-end tracking intact.
use crate::{App, EV_FOCUS, EV_SCREEN, theme::HEADER_H};
use std::time::Instant;
use windows::Win32::{
    Foundation::{LRESULT, WPARAM},
    Graphics::Direct2D::{Common::D2D1_COLOR_F, D2D1_ELLIPSE},
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};
use windows::core::Result;
use windows_numerics::Vector2;

pub const TIMER_AUTOSCROLL: usize = 44;
const DEAD_ZONE: f32 = 12.0;

pub struct Run {
    channel: Option<String>,
    root: Option<String>,
    anchor: (f32, f32),
    pointer: (f32, f32),
    held: bool,
    dragged: bool,
    last: Instant,
    /// The conversation snaps each scroll to pixels; retain subpixel travel.
    carry: f32,
    /// The anchor marker is drawn for this run, in its theme.
    drawn: bool,
}

impl Run {
    fn new(channel: Option<String>, root: Option<String>, point: (f32, f32)) -> Self {
        Self {
            channel,
            root,
            anchor: point,
            pointer: point,
            held: true,
            dragged: false,
            last: Instant::now(),
            carry: 0.0,
            drawn: false,
        }
    }

    fn moved(&mut self, point: (f32, f32)) {
        self.pointer = point;
        let (dx, dy) = (point.0 - self.anchor.0, point.1 - self.anchor.1);
        self.dragged |= self.held && dx.hypot(dy) > DEAD_ZONE;
    }

    fn delta(&self, seconds: f32) -> f32 {
        let y = self.pointer.1 - self.anchor.1;
        (y.abs() - DEAD_ZONE).max(0.0) * 8.0 * y.signum() * seconds.clamp(0.0, 0.1)
    }

    fn step(&mut self, seconds: f32, scale: f32) -> f32 {
        self.carry += self.delta(seconds) * scale;
        let pixels = self.carry.trunc();
        self.carry -= pixels;
        pixels / scale
    }
}

impl App {
    fn autoscroll_covered(&self) -> bool {
        self.browsing()
            || self.narrow.menu
            || (self.is_narrow() && self.members_w() > 0.0)
            || self.signin.is_some()
            || self.settings.is_some()
            || self.dialog.is_some()
            || self.viewer.is_some()
            || self.attachment_editor.is_some()
            || self.pins.is_some()
            || self.profile.is_some()
            || self.menu.is_some()
            || self.picker.is_some()
            || self.gifs.is_some()
            || self.dm_menu.is_some()
            || self.account_menu.is_some()
            || self.drop_overlay.is_some()
    }

    pub fn stop_autoscroll(&mut self) {
        if let Some(run) = self.autoscroll.take() {
            self.pointer = Some(run.pointer);
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_AUTOSCROLL);
                if run.held {
                    let _ = ReleaseCapture();
                }
                SetCursor(LoadCursorW(None, IDC_ARROW).ok());
            }
            self.need = true;
        }
    }

    /// Before surface input, so a stopping click cannot also activate a link
    /// or control. Wheel and focus messages still reach their normal handlers.
    pub fn autoscroll_input(&mut self, msg: u32, wp: WPARAM, x: f32, y: f32) -> Option<LRESULT> {
        if self.autoscroll_release == Some(msg) {
            self.autoscroll_release = None;
            return Some(LRESULT(0));
        }
        if self.autoscroll.is_some() {
            match msg {
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN
                | WM_LBUTTONDBLCLK | WM_RBUTTONDBLCLK | WM_MBUTTONDBLCLK | WM_XBUTTONDBLCLK => {
                    self.stop_autoscroll();
                    self.autoscroll_release = Some(match msg {
                        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => WM_LBUTTONUP,
                        WM_RBUTTONDOWN | WM_RBUTTONDBLCLK => WM_RBUTTONUP,
                        WM_MBUTTONDOWN | WM_MBUTTONDBLCLK => WM_MBUTTONUP,
                        _ => WM_XBUTTONUP,
                    });
                    return Some(LRESULT(0));
                }
                WM_KEYDOWN if wp.0 as u16 == VK_ESCAPE.0 => {
                    self.stop_autoscroll();
                    return Some(LRESULT(0));
                }
                WM_MOUSEWHEEL | WM_MOUSEHWHEEL | WM_KILLFOCUS | WM_SIZE => self.stop_autoscroll(),
                WM_ACTIVATEAPP if wp.0 == 0 => self.stop_autoscroll(),
                WM_CAPTURECHANGED if self.autoscroll.as_ref().is_some_and(|a| a.held) => {
                    self.stop_autoscroll()
                }
                crate::WM_MOUSELEAVE if self.autoscroll.as_ref().is_some_and(|a| !a.held) => {
                    self.stop_autoscroll()
                }
                WM_MOUSEMOVE => {
                    self.autoscroll.as_mut()?.moved((x, y));
                    unsafe {
                        SetCursor(LoadCursorW(None, IDC_SIZENS).ok());
                    }
                    return Some(LRESULT(0));
                }
                WM_MBUTTONUP => {
                    let run = self.autoscroll.as_mut()?;
                    run.moved((x, y));
                    run.held = false;
                    if run.dragged {
                        self.stop_autoscroll();
                    }
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    return Some(LRESULT(0));
                }
                WM_SETCURSOR => {
                    unsafe {
                        SetCursor(LoadCursorW(None, IDC_SIZENS).ok());
                    }
                    return Some(LRESULT(1));
                }
                _ => {}
            }
            return None;
        }
        if !matches!(msg, WM_MBUTTONDOWN | WM_MBUTTONDBLCLK) || self.autoscroll_covered() {
            return None;
        }
        let root = if self.thread_w() > 0.0
            && self.in_thread(x, y)
            && y >= HEADER_H
            && y < HEADER_H + self.thread_hit_h()
        {
            Some(self.thread.as_ref()?.root.clone())
        } else if self.c.has_conversation() && self.in_history(x, y) {
            None
        } else {
            return None;
        };
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        self.suggest_dismiss();
        self.hide_card();
        if let Some(t) = &mut self.thread {
            t.history_focused = root.is_some();
        }
        self.autoscroll = Some(Run::new(self.ids.open.clone(), root, (x, y)));
        self.pointer = Some((x, y));
        self.events |= EV_FOCUS;
        self.need = true;
        unsafe {
            SetCapture(self.hwnd);
            SetCursor(LoadCursorW(None, IDC_SIZENS).ok());
            SetTimer(Some(self.hwnd), TIMER_AUTOSCROLL, 16, None);
        }
        Some(LRESULT(0))
    }

    pub fn autoscroll_tick(&mut self) {
        let Some(run) = &self.autoscroll else { return };
        if run.channel != self.ids.open
            || self.autoscroll_covered()
            || run.root.as_ref().is_some_and(|root| {
                self.thread_w() <= 0.0 || self.thread.as_ref().is_none_or(|t| &t.root != root)
            })
        {
            return self.stop_autoscroll();
        }
        let run = self.autoscroll.as_mut().unwrap();
        let now = Instant::now();
        let delta = run.step(now.duration_since(run.last).as_secs_f32(), self.scale);
        run.last = now;
        if delta != 0.0 {
            if run.root.is_some() {
                self.scroll_thread_by(delta);
            } else {
                self.scroll_by(delta as f64);
            }
            self.events |= EV_SCREEN;
        }
    }

    pub fn draw_autoscroll(&mut self) -> Result<()> {
        let Some(run) = &mut self.autoscroll else {
            return self.autoscroll_p.offset(-100_000.0, 0.0);
        };
        let (x, y) = run.anchor;
        let fresh = !std::mem::replace(&mut run.drawn, true);
        let s = self.scale;
        // Each run redraws it, so a theme change since the last one shows.
        if self
            .autoscroll_p
            .ensure(&self.g, (28.0 * s).ceil() as u32, (28.0 * s).ceil() as u32)?
            || fresh
        {
            let g = &self.g;
            self.autoscroll_p.draw(s, |dc| {
                let point = |x, y| Vector2 { X: x, Y: y };
                // A surface starts undefined: without this, the circle sat
                // on an opaque square.
                unsafe { dc.Clear(Some(&D2D1_COLOR_F::default())) };
                let ring = D2D1_ELLIPSE {
                    point: point(14.0, 14.0),
                    radiusX: 13.0,
                    radiusY: 13.0,
                };
                unsafe {
                    dc.FillEllipse(&ring, &g.brushes.surface);
                    dc.DrawEllipse(&ring, &g.brushes.border, 1.0, None);
                    dc.FillEllipse(
                        &D2D1_ELLIPSE {
                            radiusX: 2.0,
                            radiusY: 2.0,
                            ..ring
                        },
                        &g.brushes.text,
                    );
                    for sign in [-1.0, 1.0] {
                        dc.DrawLine(
                            point(10.0, 14.0 + 5.0 * sign),
                            point(14.0, 14.0 + 9.0 * sign),
                            &g.brushes.text,
                            1.5,
                            None,
                        );
                        dc.DrawLine(
                            point(14.0, 14.0 + 9.0 * sign),
                            point(18.0, 14.0 + 5.0 * sign),
                            &g.brushes.text,
                            1.5,
                            None,
                        );
                    }
                }
                Ok(())
            })?;
        }
        self.autoscroll_p
            .offset(((x - 14.0) * s).round(), ((y - 14.0) * s).round())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_zone_speed_frame_cap_and_latched_drag_match_gtk() {
        let mut run = Run::new(None, None, (150.0, 230.0));
        run.moved((150.0, 242.0));
        assert_eq!(run.delta(0.1), 0.0);
        assert!(!run.dragged);
        run.moved((150.0, 252.0));
        assert_eq!(run.delta(0.05), 4.0);
        assert!(run.dragged);
        run.moved((150.0, 198.0));
        assert_eq!(run.delta(5.0), -16.0);
        run.moved((150.0, 230.0));
        assert_eq!(run.delta(0.1), 0.0);
        assert!(
            run.dragged,
            "returning to the anchor does not unlatch a held drag"
        );
        let mut click = Run::new(None, None, (0.0, 0.0));
        click.held = false;
        click.moved((50.0, 22.0));
        assert!(!click.dragged, "movement after a plain click stays latched");
        assert_eq!(click.delta(0.1), 8.0);
        let mut horizontal = Run::new(None, None, (0.0, 0.0));
        horizontal.moved((13.0, 0.0));
        assert!(horizontal.dragged);
        assert_eq!(horizontal.delta(0.1), 0.0);
        horizontal.moved((0.0, 13.0));
        let distance: f32 = (0..60).map(|_| horizontal.step(1.0 / 60.0, 1.5)).sum();
        assert!(
            (distance - 8.0).abs() < 1.0 / 1.5,
            "slow motion must accumulate through pixel snapping"
        );
    }
}
