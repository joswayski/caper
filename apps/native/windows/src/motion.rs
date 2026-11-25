//! Opt-in compositor entrances and live-end row arrivals. Geometry stays put:
//! only opacity and translation animate, and closing always cancels instantly.
use crate::App;
use std::{cell::Cell, collections::HashMap, time::Instant};
use windows::Win32::{
    Graphics::{Direct2D::*, DirectComposition::*},
    UI::WindowsAndMessaging::*,
};
use windows::core::*;
use windows_numerics::{Matrix3x2, Vector2};

pub const FAST: f64 = 0.14;
pub const BASE: f64 = 0.2;
pub const TIMER_ARRIVAL: usize = 45;

#[derive(Clone, Copy)]
pub enum Kind {
    Fade,
    Menu,
    Dialog,
    Thread,
}

impl Kind {
    fn pose(self) -> (f32, f32, f64) {
        match self {
            Self::Fade => (0.0, 0.0, FAST),
            Self::Menu => (0.0, 4.0, FAST),
            Self::Dialog => (0.0, 8.0, BASE),
            Self::Thread => (12.0, 0.0, BASE),
        }
    }
}

/// `--ease-out`, cubic-bezier(0.16, 1, 0.3, 1), including its time axis.
pub fn ease(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..20 {
        let u = (lo + hi) / 2.0;
        let x = 3.0 * (1.0 - u).powi(2) * u * 0.16 + 3.0 * (1.0 - u) * u * u * 0.3 + u * u * u;
        if x < t {
            lo = u;
        } else {
            hi = u;
        }
    }
    1.0 - (1.0 - (lo + hi) / 2.0).powi(3)
}

fn animation(
    device: &IDCompositionDesktopDevice,
    from: f32,
    to: f32,
    seconds: f64,
) -> Result<IDCompositionAnimation> {
    // Sixteen linear segments approximate the shared Bézier more finely than
    // a display frame; the compositor owns the clock and final value.
    unsafe {
        let a = device.CreateAnimation()?;
        for i in 0..16 {
            let (t0, t1) = (i as f64 / 16.0, (i + 1) as f64 / 16.0);
            let v0 = from as f64 + (to - from) as f64 * ease(t0);
            let v1 = from as f64 + (to - from) as f64 * ease(t1);
            a.AddCubic(
                t0 * seconds,
                v0 as f32,
                ((v1 - v0) / ((t1 - t0) * seconds)) as f32,
                0.0,
                0.0,
            )?;
        }
        a.End(seconds, to)?;
        Ok(a)
    }
}

#[derive(Clone, Copy, Default)]
struct Placement {
    at: Option<(f32, f32)>,
    pending: bool,
}

impl Placement {
    fn shown(at: (f32, f32)) -> bool {
        at.0 > -5000.0 && at.1 > -5000.0
    }

    /// Whether offset must replace the compositor pose now. Repeated placement
    /// at the same shown point leaves an entrance running; movement cancels it.
    fn place(&mut self, at: (f32, f32), enabled: bool) -> bool {
        let shown = Self::shown(at);
        let was_shown = self.at.is_some_and(Self::shown);
        let same = self.at == Some(at);
        self.pending = enabled && shown && (!was_shown || (same && self.pending));
        self.at = Some(at);
        !enabled || !shown || !same
    }
}

pub struct Entrance {
    device: IDCompositionDesktopDevice,
    kind: Kind,
    placement: Cell<Placement>,
}

impl Entrance {
    pub fn new(device: &IDCompositionDesktopDevice, kind: Kind) -> Self {
        Self {
            device: device.clone(),
            kind,
            placement: Cell::default(),
        }
    }

    pub fn place(&self, visual: &IDCompositionVisual2, x: f32, y: f32) -> Result<()> {
        let mut p = self.placement.get();
        let reset = p.place((x, y), crate::files::motion());
        self.placement.set(p);
        if reset {
            self.pose(visual, x, y)?;
        }
        Ok(())
    }

    fn pose(&self, visual: &IDCompositionVisual2, x: f32, y: f32) -> Result<()> {
        unsafe {
            visual.SetOffsetX2(x)?;
            visual.SetOffsetY2(y)?;
            visual.cast::<IDCompositionVisual3>()?.SetOpacity2(1.0)
        }
    }

    /// Called only after content is drawn, with its real DPI × zoom. No
    /// per-frame redraw is needed to advance a panel's animation.
    pub fn start(&self, visual: &IDCompositionVisual2, scale: f32) -> Result<()> {
        let mut p = self.placement.get();
        if !p.pending {
            return Ok(());
        }
        p.pending = false;
        self.placement.set(p);
        let Some((x, y)) = p.at else {
            return Ok(());
        };
        if !crate::files::motion() {
            return self.pose(visual, x, y);
        }
        let (dx, dy, seconds) = self.kind.pose();
        unsafe {
            visual
                .cast::<IDCompositionVisual3>()?
                .SetOpacity(&animation(&self.device, 0.0, 1.0, seconds)?)?;
            if dx != 0.0 {
                visual.SetOffsetX(&animation(&self.device, x + dx * scale, x, seconds)?)?;
            }
            if dy != 0.0 {
                visual.SetOffsetY(&animation(&self.device, y + dy * scale, y, seconds)?)?;
            }
        }
        Ok(())
    }

    pub fn settle(&self, visual: &IDCompositionVisual2) -> Result<()> {
        let mut p = self.placement.get();
        p.pending = false;
        self.placement.set(p);
        if let Some((x, y)) = p.at {
            self.pose(visual, x, y)?;
        }
        Ok(())
    }
}

/// Per-list timestamps, not per-layout: remeasuring a row never restarts it.
#[derive(Default)]
pub struct Arrivals(HashMap<String, Instant>);

impl Arrivals {
    /// A confirmed live message, including the first one in an empty channel.
    pub fn received(&mut self, id: &str, watching: bool, now: Instant) {
        if watching && !id.is_empty() {
            self.0.entry(id.to_owned()).or_insert(now);
        }
    }

    pub fn observe(&mut self, before: &[String], after: &[String], watching: bool, now: Instant) {
        if !watching {
            return;
        }
        let Some(tail) = before
            .iter()
            .rev()
            .find_map(|id| after.iter().position(|next| next == id))
        else {
            return;
        };
        for id in &after[tail + 1..] {
            if !before.contains(id) {
                self.received(id, true, now);
            }
        }
    }

    pub fn progress(&self, id: &str) -> f32 {
        self.0
            .get(id)
            .map_or(1.0, |at| ease(at.elapsed().as_secs_f64() / BASE) as f32)
    }

    pub fn active(&self) -> bool {
        !self.0.is_empty()
    }

    /// Include the final frame in the damage, then discard its timestamp.
    pub fn tick(&mut self, enabled: bool) -> Vec<String> {
        let ids = self.0.keys().cloned().collect();
        self.0
            .retain(|_, at| enabled && at.elapsed().as_secs_f64() < BASE);
        ids
    }
}

/// The row is clipped to its final bounds, so a short rise never paints into
/// its neighbor. Only the arriving rows need redrawing in the virtual surface.
pub fn row(
    dc: &ID2D1DeviceContext,
    bounds: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F,
    progress: f32,
    draw: impl FnOnce(f32) -> Result<()>,
) -> Result<()> {
    if progress >= 1.0 {
        return draw(0.0);
    }
    unsafe {
        dc.PushLayer(
            &D2D1_LAYER_PARAMETERS1 {
                contentBounds: bounds,
                opacity: progress,
                maskTransform: Matrix3x2::identity(),
                ..Default::default()
            },
            None,
        );
        let result = draw(10.0 * (1.0 - progress));
        dc.PopLayer();
        result
    }
}

/// How far above a history's recede line (`recede_t`) a row's top edge
/// starts receding.
pub const RECEDE: f64 = 40.0;

/// How present a row with its top edge at `top` is as it slides under the
/// typing row and composer, from 1 (as drawn) down to 0 at `line`: web's
/// `caper-recede` over the last 40 px. `line` is 24 DIPs below where the
/// newest message rests at the live end, so a row there (at least 16 tall)
/// never recedes; an infinite `line` (animations off) recedes nothing.
pub fn recede_t(line: f64, top: f64) -> f32 {
    ((line - top) / RECEDE).clamp(0.0, 1.0) as f32
}

/// Draws a row within `bounds` receded by `t` (`recede_t`): dimmed to
/// 0.7 + 0.3t, shrunk to 0.98 + 0.02t about its bottom centre and lowered
/// 4(1 − t), tracking the scroll rather than a clock. At 1 it draws exactly
/// as before; it nests around `row`'s arrival, never replacing it.
pub fn recede(
    dc: &ID2D1DeviceContext,
    bounds: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F,
    t: f32,
    draw: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if t >= 1.0 {
        return draw();
    }
    let scale = 0.98 + 0.02 * t;
    let centre = Vector2 {
        X: (bounds.left + bounds.right) / 2.0,
        Y: bounds.bottom,
    };
    unsafe {
        let mut before = Matrix3x2::default();
        dc.GetTransform(&mut before);
        dc.SetTransform(
            &(Matrix3x2::scale_around(scale, scale, centre)
                * Matrix3x2::translation(0.0, 4.0 * (1.0 - t))
                * before),
        );
        dc.PushLayer(
            &D2D1_LAYER_PARAMETERS1 {
                contentBounds: bounds,
                opacity: 0.7 + 0.3 * t,
                maskTransform: Matrix3x2::identity(),
                ..Default::default()
            },
            None,
        );
        let result = draw();
        dc.PopLayer();
        dc.SetTransform(&before);
        result
    }
}

impl App {
    pub fn watching_arrivals(&self) -> bool {
        crate::files::motion()
            && !self.browsing()
            && !self.narrow.menu
            && !(self.is_narrow() && self.members_w() > 0.0)
            && self.signin.is_none()
            && self.dialog.is_none()
            && self.viewer.is_none()
            && self.settings.is_none()
            && self.profile.is_none()
            && self.pins.is_none()
            && self.attachment_editor.is_none()
            && unsafe { GetForegroundWindow() == self.hwnd && !IsIconic(self.hwnd).as_bool() }
    }

    pub fn arm_arrivals(&self) {
        if self.rows.arrivals.active()
            || self
                .thread
                .as_ref()
                .is_some_and(|t| t.rows.arrivals.active())
        {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_ARRIVAL, 16, None);
            }
        }
    }

    pub fn arrivals_tick(&mut self) {
        let enabled = self.watching_arrivals();
        for id in self
            .rows
            .arrivals
            .tick(enabled && self.chat_shown() && self.at_end)
        {
            if let Some(&i) = self.ids.rows.get(&id) {
                self.redraw.push(i);
            }
        }
        let thread_shown = self.thread_shown();
        if let Some(t) = &mut self.thread {
            self.dirty.thread |= !t
                .rows
                .arrivals
                .tick(enabled && thread_shown && t.at_end)
                .is_empty();
        }
        if !self.rows.arrivals.active()
            && self
                .thread
                .as_ref()
                .is_none_or(|t| !t.rows.arrivals.active())
        {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_ARRIVAL);
            }
        }
        self.need = true;
    }

    pub fn motion_changed(&mut self) {
        if crate::files::motion() {
            return;
        }
        for panel in [
            &self.menu_p,
            &self.dialog_p,
            &self.dialog_card_p,
            &self.picker_p,
            &self.gifs_p,
            &self.thread_p,
            &self.thread_box_p,
            &self.card_p,
            &self.profile_p,
            &self.suggest_p,
            &self.tip_p,
            &self.settings_p,
            &self.dm_menu_p,
            &self.pins_p,
        ] {
            if let Some(entrance) = &panel.entrance {
                let _ = entrance.settle(&panel.visual);
            }
        }
        self.arrivals_tick();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_hidden_to_shown_starts_and_moves_hide_reduce_cancel() {
        let mut p = Placement::default();
        assert!(p.place((-10000.0, 0.0), true));
        assert!(!p.pending);
        assert!(p.place((30.0, 40.0), true));
        assert!(p.pending);
        p.pending = false;
        assert!(!p.place((30.0, 40.0), true));
        assert!(!p.pending);
        assert!(p.place((31.0, 40.0), true));
        assert!(!p.pending);
        p.place((-10000.0, 0.0), true);
        assert!(!p.pending);
        p.place((31.0, 40.0), false);
        assert!(!p.pending);
        p.place((-10000.0, 0.0), true);
        p.place((31.0, 40.0), true);
        assert!(p.pending);
        assert!(p.place((31.0, 40.0), false));
        assert!(!p.pending);
        assert_eq!((ease(0.0), ease(1.0)), (0.0, 1.0));
        assert!((ease(0.2975) - 0.875).abs() < 0.00001);
    }

    #[test]
    fn first_pages_history_edits_and_unwatched_appends_stay_still() {
        let ids = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut a = Arrivals::default();
        let now = Instant::now() - std::time::Duration::from_millis(80);
        a.observe(&[], &ids(&["a", "b"]), true, now);
        assert!(!a.active());
        a.observe(&ids(&["a", "b"]), &ids(&["older", "a", "b"]), true, now);
        assert!(!a.active());
        a.observe(&ids(&["a", "b"]), &ids(&["a", "b", "c"]), false, now);
        assert!(!a.active());
        a.observe(&ids(&["a", "b"]), &ids(&["a", "b", "c"]), true, now);
        assert!(a.active());
        a.observe(
            &ids(&["a", "b", "c"]),
            &ids(&["a", "b", "c"]),
            true,
            Instant::now(),
        );
        assert!(a.progress("c") > 0.8, "an edit must not restart an arrival");
        assert_eq!(a.tick(false), vec!["c".to_string()]);
        assert!(!a.active());
        assert_eq!(a.progress("c"), 1.0);
        a.received("first", true, now);
        assert!(a.active());
    }
}
