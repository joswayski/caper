//! "Jump to latest", as every client: a floating button over a message
//! list's bottom right corner while the reader is more than half a screen
//! away from the newest loaded message and that history reaches the live
//! end. While it shows it counts others' messages arriving at the live end.
//! The clients own the widget, its scrolling and its entrance; this is its
//! state, its text and its accessible name.

/// The count reads "99+" past this.
const CAP: u32 = 99;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Jump {
    pub shown: bool,
    /// Others' messages that arrived at the live end while it showed.
    pub count: u32,
}

impl Jump {
    /// After a scroll or a change in the list's height: `distance` from the
    /// bottom of the newest loaded message to the bottom of the viewport,
    /// `page` the viewport's height, `at_end` whether the list counts as
    /// read at its newest message, and `live` whether the loaded history
    /// reaches the live end (a detached history window has its own
    /// controls). Returns whether anything shown changed.
    pub fn scrolled(&mut self, distance: f64, page: f64, at_end: bool, live: bool) -> bool {
        let before = *self;
        if at_end || !live {
            *self = Self::default();
        } else if distance > page / 2.0 {
            self.shown = true;
        }
        *self != before
    }

    /// Someone else's message arrived at the live end. Returns whether it
    /// was counted (only while the button shows).
    pub fn arrived(&mut self) -> bool {
        if self.shown {
            self.count = self.count.saturating_add(1);
        }
        self.shown
    }

    /// "3 new messages" beside the arrow, or nothing before any arrive.
    pub fn text(&self) -> Option<String> {
        match self.count {
            0 => None,
            1 => Some("1 new message".into()),
            count if count > CAP => Some(format!("{CAP}+ new messages")),
            count => Some(format!("{count} new messages")),
        }
    }

    /// The accessible name: "Jump to latest", or "Jump to latest, 3 new
    /// messages".
    pub fn name(&self) -> String {
        match self.text() {
            Some(text) => format!("Jump to latest, {text}"),
            None => "Jump to latest".into(),
        }
    }

    /// Whether its border turns terracotta: new messages are waiting.
    pub fn unread(&self) -> bool {
        self.count > 0
    }
}

/// Whether a jump glides there (animated) or lands at once: reduced motion,
/// or more than a few screens away, jumps instantly.
pub fn glides(distance: f64, page: f64, reduced_motion: bool) -> bool {
    !reduced_motion && distance <= page * 3.0
}

/// The glide's position at `t` (0 to 1) of the way from `from` to `to`, on
/// the shared ease-out curve (cubic-bezier(0.16, 1, 0.3, 1), approximated
/// by an ease-out quint).
pub fn glide_at(from: f64, to: f64, t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    from + (to - from) * (1.0 - (1.0 - t).powi(5))
}

/// How long a glide takes, `--motion-slow`.
pub const GLIDE: std::time::Duration = std::time::Duration::from_millis(260);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_past_half_a_screen_and_hides_only_at_the_end() {
        let mut jump = Jump::default();
        // A little away from the end: not yet.
        assert!(!jump.scrolled(300.0, 800.0, false, true));
        assert!(!jump.shown);
        assert!(jump.scrolled(401.0, 800.0, false, true));
        assert!(jump.shown);
        // Coming back part of the way keeps it until the end is reached.
        assert!(!jump.scrolled(100.0, 800.0, false, true));
        assert!(jump.shown);
        assert!(jump.scrolled(0.0, 800.0, true, true));
        assert_eq!(jump, Jump::default());
    }

    #[test]
    fn a_detached_history_window_never_shows_it() {
        let mut jump = Jump::default();
        assert!(!jump.scrolled(5000.0, 800.0, false, false));
        assert!(!jump.shown);
        jump.scrolled(5000.0, 800.0, false, true);
        jump.arrived();
        // Jumping to an old message detaches the window: gone, count and all.
        assert!(jump.scrolled(5000.0, 800.0, false, false));
        assert_eq!(jump, Jump::default());
    }

    #[test]
    fn counts_only_while_shown_and_clears_at_the_end() {
        let mut jump = Jump::default();
        assert!(!jump.arrived());
        assert_eq!(jump.count, 0);
        jump.scrolled(900.0, 800.0, false, true);
        assert!(jump.arrived());
        assert_eq!(jump.text().as_deref(), Some("1 new message"));
        assert!(jump.unread());
        jump.arrived();
        jump.arrived();
        assert_eq!(jump.name(), "Jump to latest, 3 new messages");
        // Scrolling about (still away) keeps the count.
        jump.scrolled(1200.0, 800.0, false, true);
        assert_eq!(jump.count, 3);
        jump.scrolled(0.0, 800.0, true, true);
        assert_eq!(jump.count, 0);
        assert_eq!(jump.name(), "Jump to latest");
        assert_eq!(jump.text(), None);
    }

    #[test]
    fn the_count_caps_at_99_plus() {
        let mut jump = Jump {
            shown: true,
            count: 99,
        };
        assert_eq!(jump.text().as_deref(), Some("99 new messages"));
        jump.arrived();
        assert_eq!(jump.text().as_deref(), Some("99+ new messages"));
        assert_eq!(jump.name(), "Jump to latest, 99+ new messages");
    }

    #[test]
    fn glides_nearby_and_lands_at_once_far_away_or_with_reduced_motion() {
        assert!(glides(2400.0, 800.0, false));
        assert!(!glides(2401.0, 800.0, false));
        assert!(!glides(500.0, 800.0, true));
        assert_eq!(glide_at(100.0, 500.0, 0.0), 100.0);
        assert_eq!(glide_at(100.0, 500.0, 1.0), 500.0);
        assert_eq!(glide_at(100.0, 500.0, 2.0), 500.0);
        // Decelerating: well past halfway at half the time.
        assert!(glide_at(0.0, 100.0, 0.5) > 90.0);
    }
}
