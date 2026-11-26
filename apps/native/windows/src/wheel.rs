//! Eased wheel scrolling for the message histories, as browsers smooth it:
//! each notch adds to the distance still to go, and the view glides there on
//! the shared ease-out instead of jumping a notch at a time. A touchpad's fine
//! deltas already arrive smoothly and apply at once, as does everything while
//! Windows animations are off.
use crate::App;
use std::time::Instant;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

pub const TIMER_WHEEL: usize = 53;
/// Long enough to read as motion, short enough to keep up with a fast wheel.
const GLIDE: f64 = 0.16;

#[derive(Default)]
pub struct Wheel {
    /// The glide's whole distance in DIPs, how much of it has moved (in
    /// whole device pixels), and when the latest notch restarted it.
    glide: Option<(f64, f64, Instant)>,
}

impl Wheel {
    /// A wheel message's `delta` (120 a notch, down negative) scrolling
    /// `step` DIPs a notch: returns what moves now, and a notched wheel's
    /// distance joins the glide instead.
    pub fn wheel(&mut self, delta: i16, step: f64, motion: bool, now: Instant) -> f64 {
        let dips = -f64::from(delta) / 120.0 * step;
        if !motion || delta % 120 != 0 {
            self.glide = None;
            return dips;
        }
        let left = self.glide.map_or(0.0, |(total, done, _)| total - done);
        // Turning the wheel back drops the rest, as browsers do.
        let left = if left * dips < 0.0 { 0.0 } else { left };
        self.glide = Some((left + dips, 0.0, now));
        0.0
    }

    /// This frame's share of the glide in DIPs. Steps are whole device
    /// pixels, as the history snaps to them, so no travel is lost.
    pub fn tick(&mut self, now: Instant, scale: f64) -> f64 {
        let Some((total, done, since)) = &mut self.glide else {
            return 0.0;
        };
        let k = now.saturating_duration_since(*since).as_secs_f64() / GLIDE;
        let to = if k >= 1.0 {
            *total
        } else {
            *total * crate::motion::ease(k)
        };
        let to = (to * scale).round() / scale;
        let step = to - *done;
        *done = to;
        if k >= 1.0 {
            self.glide = None;
        }
        step
    }

    pub fn active(&self) -> bool {
        self.glide.is_some()
    }

    pub fn stop(&mut self) {
        self.glide = None;
    }
}

impl App {
    /// Runs the frame timer while either history glides.
    pub fn arm_wheel(&self) {
        if self.wheel.active() || self.thread.as_ref().is_some_and(|t| t.wheel.active()) {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_WHEEL, 16, None);
            }
        }
    }

    pub fn wheel_tick(&mut self) {
        let (now, scale) = (Instant::now(), f64::from(self.scale));
        let step = self.wheel.tick(now, scale);
        if step != 0.0 {
            self.scroll_by(step);
        }
        let step = self
            .thread
            .as_mut()
            .map_or(0.0, |t| t.wheel.tick(now, scale));
        if step != 0.0 {
            self.scroll_thread_by(step as f32);
        }
        if !self.wheel.active() && self.thread.as_ref().is_none_or(|t| !t.wheel.active()) {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_WHEEL);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn notches_glide_in_whole_pixels_and_add_up_while_fine_deltas_apply_at_once() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut w = Wheel::default();
        // One notch down (negative delta) at 150%: 48 DIPs, eased.
        assert_eq!(w.wheel(-120, 48.0, true, t0), 0.0);
        let first = w.tick(at(16), 1.5);
        assert!(first > 0.0 && first < 48.0, "{first}");
        assert_eq!((first * 1.5).fract(), 0.0, "whole device pixels");
        // A second notch mid-glide carries what was left.
        assert_eq!(w.wheel(-120, 48.0, true, at(16)), 0.0);
        let rest: f64 = (2..=12).map(|f| w.tick(at(16 * f), 1.5)).sum();
        assert!((first + rest - 96.0).abs() < 1e-9, "{}", first + rest);
        assert!(!w.active());
        // Reversing drops the rest of the glide.
        w.wheel(-120, 48.0, true, t0);
        let down = w.tick(at(16), 1.0);
        w.wheel(120, 48.0, true, at(16));
        let up: f64 = (2..=12).map(|f| w.tick(at(16 * f), 1.0)).sum();
        assert!(down > 0.0 && up == -48.0, "{down} {up}");
        // A touchpad's fine delta, or animations off, moves at once.
        assert_eq!(w.wheel(-30, 48.0, true, t0), 12.0);
        assert_eq!(w.wheel(240, 48.0, false, t0), -96.0);
        assert!(!w.active());
    }
}
