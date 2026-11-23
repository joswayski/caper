//! Plays animated pictures (avatars, GIF tiles) on one main-loop timer that
//! fires only when some animation's frame changes.
//!
//! A tick callback per picture kept GTK's frame clock running every refresh
//! (60 frames a second for a 12 fps GIF), each one a full update cycle.
//! The timer moves every picture whose frame changed in one go, so they all
//! redraw in one frame, and the frame clock idles between frames.
//!
//! Only drawable pictures (shown, in a mapped window that is not minimized)
//! are moved. While none are, the timer checks again every
//! [`HIDDEN_CHECK`], so a picture scrolled back into view moves again.

use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// An animation's frames and how long each shows.
pub type Frames = Vec<(gdk::Texture, Duration)>;

/// How often to look again while no playing picture can be seen.
const HIDDEN_CHECK: Duration = Duration::from_millis(500);
/// The shortest wait, so a malformed animation cannot spin the loop.
const MIN_WAIT: Duration = Duration::from_millis(10);

type Setter = Rc<dyn Fn(&gtk::Widget, &gdk::Texture)>;

struct Target {
    widget: glib::WeakRef<gtk::Widget>,
    set: Setter,
    shown: Cell<usize>,
}

struct Animation {
    frames: Rc<Frames>,
    /// Where its timeline starts, in monotonic microseconds.
    start: i64,
    targets: Vec<Target>,
}

#[derive(Default)]
struct Player {
    animations: Vec<Animation>,
    timer: Option<glib::SourceId>,
}

thread_local! {
    static PLAYER: RefCell<Player> = RefCell::default();
}

/// Plays `frames` in `widget` from `start` (monotonic microseconds; 0 puts
/// every caller on one shared timeline), through `set`. Replaces whatever
/// `widget` played before. Returns the frame showing now.
pub fn play(
    widget: &impl IsA<gtk::Widget>,
    frames: &Rc<Frames>,
    start: i64,
    set: impl Fn(&gtk::Widget, &gdk::Texture) + 'static,
) -> gdk::Texture {
    let widget = widget.as_ref();
    let now = glib::monotonic_time();
    let index = frame_at(frames, now - start);
    PLAYER.with_borrow_mut(|player| {
        player.remove(widget);
        let target = Target {
            widget: widget.downgrade(),
            set: Rc::new(set),
            shown: Cell::new(index),
        };
        match player
            .animations
            .iter_mut()
            .find(|animation| Rc::ptr_eq(&animation.frames, frames) && animation.start == start)
        {
            Some(animation) => animation.targets.push(target),
            None => player.animations.push(Animation {
                frames: frames.clone(),
                start,
                targets: vec![target],
            }),
        }
    });
    schedule();
    frames[index].0.clone()
}

/// Stops whatever `widget` played.
pub fn stop(widget: &impl IsA<gtk::Widget>) {
    PLAYER.with_borrow_mut(|player| player.remove(widget.as_ref()));
}

impl Player {
    fn remove(&mut self, widget: &gtk::Widget) {
        for animation in &mut self.animations {
            animation.targets.retain(|target| {
                target
                    .widget
                    .upgrade()
                    .is_some_and(|shown| shown != *widget)
            });
        }
        self.animations
            .retain(|animation| !animation.targets.is_empty());
    }
}

/// Whether `widget` is on screen: drawable, in a window that is not
/// minimized.
pub fn visible(widget: &gtk::Widget) -> bool {
    widget.is_drawable()
        && !widget
            .native()
            .and_then(|native| native.surface())
            .and_then(|surface| surface.downcast::<gdk::Toplevel>().ok())
            .is_some_and(|toplevel| toplevel.state().contains(gdk::ToplevelState::MINIMIZED))
}

/// Moves every visible picture whose frame changed, drops pictures that are
/// gone, and arms the timer for the next change.
fn advance() {
    let now = glib::monotonic_time();
    let mut changes: Vec<(gtk::Widget, gdk::Texture, Setter)> = Vec::new();
    PLAYER.with_borrow_mut(|player| {
        for animation in &mut player.animations {
            animation
                .targets
                .retain(|target| target.widget.upgrade().is_some());
            let index = frame_at(&animation.frames, now - animation.start);
            for target in &animation.targets {
                let Some(widget) = target.widget.upgrade() else {
                    continue;
                };
                if target.shown.get() != index && visible(&widget) {
                    target.shown.set(index);
                    changes.push((
                        widget,
                        animation.frames[index].0.clone(),
                        target.set.clone(),
                    ));
                }
            }
        }
        player
            .animations
            .retain(|animation| !animation.targets.is_empty());
    });
    // Outside the borrow: a setter may cause another picture to play.
    for (widget, texture, set) in changes {
        set(&widget, &texture);
    }
    schedule();
}

/// Arms the timer for the soonest frame change among visible pictures.
fn schedule() {
    let now = glib::monotonic_time();
    PLAYER.with_borrow_mut(|player| {
        if let Some(timer) = player.timer.take() {
            timer.remove();
        }
        if player.animations.is_empty() {
            return;
        }
        let wait = player
            .animations
            .iter()
            .filter(|animation| {
                animation
                    .targets
                    .iter()
                    .filter_map(|target| target.widget.upgrade())
                    .any(|widget| visible(&widget))
            })
            .map(|animation| next_change(&animation.frames, now - animation.start))
            .min()
            .map_or(HIDDEN_CHECK, |micros| {
                Duration::from_micros(micros.max(0) as u64)
            })
            .max(MIN_WAIT);
        player.timer = Some(glib::timeout_add_local_once(wait, || {
            PLAYER.with_borrow_mut(|player| player.timer = None);
            advance();
        }));
    });
}

fn length(delay: &Duration) -> i64 {
    delay.as_micros() as i64
}

/// The frame of `frames` (never empty) showing at `time` microseconds into
/// its timeline, which loops. Avatars all share one timeline, as a browser
/// plays one image: a picture moves in step wherever it shows.
pub fn frame_at<T>(frames: &[(T, Duration)], time: i64) -> usize {
    let total: i64 = frames.iter().map(|(_, delay)| length(delay)).sum();
    let mut time = time.rem_euclid(total.max(1));
    for (index, (_, delay)) in frames.iter().enumerate() {
        if time < length(delay) {
            return index;
        }
        time -= length(delay);
    }
    0
}

/// Microseconds from `time` until the frame showing then ends.
fn next_change<T>(frames: &[(T, Duration)], time: i64) -> i64 {
    let total: i64 = frames.iter().map(|(_, delay)| length(delay)).sum();
    let mut time = time.rem_euclid(total.max(1));
    for (_, delay) in frames {
        if time < length(delay) {
            return length(delay) - time;
        }
        time -= length(delay);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avatars_share_one_animation_timeline() {
        let millis = Duration::from_millis;
        let frames = [((), millis(100)), ((), millis(50)), ((), millis(250))];
        assert_eq!(frame_at(&frames, 0), 0);
        assert_eq!(frame_at(&frames, 99_999), 0);
        assert_eq!(frame_at(&frames, 100_000), 1);
        assert_eq!(frame_at(&frames, 150_000), 2);
        // The loop is 400 ms: the clock decides the frame, not when a view
        // started showing it.
        assert_eq!(frame_at(&frames, 7 * 400_000 + 120_000), 1);
    }

    #[test]
    fn the_timer_waits_for_the_showing_frame_to_end() {
        let millis = Duration::from_millis;
        let frames = [((), millis(100)), ((), millis(50)), ((), millis(250))];
        assert_eq!(next_change(&frames, 0), 100_000);
        assert_eq!(next_change(&frames, 99_999), 1);
        assert_eq!(next_change(&frames, 120_000), 30_000);
        // Into the next loop, the last frame's end is the loop's.
        assert_eq!(next_change(&frames, 400_000 + 200_000), 200_000);
        // Never a negative wait before a timeline starts.
        assert_eq!(next_change(&frames, -50_000), 50_000);
    }
}
