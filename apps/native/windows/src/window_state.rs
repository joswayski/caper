//! The window's remembered normal (restore) rectangle and whether it was
//! maximized, as egui keeps them, and egui's Ctrl +/−/0 zoom steps. The
//! rectangle is `GetWindowPlacement`'s, in workspace pixels, so it goes
//! back through `SetWindowPlacement` unchanged; a minimized window is
//! never restored minimized.

/// The preference holding the placement.
pub const KEY: &str = "window-v1";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Placement {
    /// Left, top, right, bottom.
    pub rect: (i32, i32, i32, i32),
    pub maximized: bool,
}

impl Placement {
    pub fn parse(s: &str) -> Option<Self> {
        let mut it = s.split(',').map(|v| v.trim().parse::<i32>());
        let mut next = || it.next()?.ok();
        let (l, t, r, b, maximized) = (next()?, next()?, next()?, next()?, next()?);
        if it.next().is_some() || r - l < 100 || b - t < 100 || !(0..=1).contains(&maximized) {
            return None;
        }
        Some(Self {
            rect: (l, t, r, b),
            maximized: maximized == 1,
        })
    }

    pub fn format(&self) -> String {
        let (l, t, r, b) = self.rect;
        format!("{l},{t},{r},{b},{}", u8::from(self.maximized))
    }
}

/// egui's zoom limits and step: 10% at a time, rounded to the tenth.
pub const MIN_ZOOM: f32 = 0.2;
pub const MAX_ZOOM: f32 = 5.0;

/// The zoom after one Ctrl+Plus (`steps` = 1), Ctrl+Minus (−1) or Ctrl+0 (0).
pub fn zoomed(zoom: f32, steps: i32) -> f32 {
    if steps == 0 {
        return 1.0;
    }
    ((zoom + 0.1 * steps as f32).clamp(MIN_ZOOM, MAX_ZOOM) * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_round_trips_and_rejects_damaged_values() {
        let p = Placement {
            rect: (-1700, 40, -260, 950),
            maximized: true,
        };
        assert_eq!(Placement::parse(&p.format()), Some(p));
        for bad in [
            "",
            "1,2,3",
            "0,0,800,600,2",
            "0,0,800,600,1,9",
            "0,0,50,600,0",
            "a,0,800,600,0",
        ] {
            assert_eq!(Placement::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn zoom_steps_by_tenths_within_limits() {
        // egui's sequence: 100% → 110% → 120% → 110% → 100%, then 90%.
        let mut z = 1.0;
        let mut seen = Vec::new();
        for step in [1, 1, -1, 0, -1] {
            z = zoomed(z, step);
            seen.push((z * 100.0).round() as i32);
        }
        assert_eq!(seen, [110, 120, 110, 100, 90]);
        assert_eq!(zoomed(0.2, -1), 0.2);
        assert_eq!(zoomed(5.0, 1), 5.0);
        // Ten steps from 100% land exactly on 200%, without float drift.
        assert_eq!((0..10).fold(1.0, |z, _| zoomed(z, 1)), 2.0);
    }
}
