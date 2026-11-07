//! The resizable channel sidebar and member list, as on the egui desktop:
//! each is 220–440 px, saved per device (`sidebar-width-v1`,
//! `members-width-v1`). Drag the line between panes, or focus it (F8) and
//! use the arrow keys (10 px), Home and End; double-click resets it. The
//! conversation keeps room beside them, and a window too small for a saved
//! width shows it narrower without forgetting it. In the sidebar, a line
//! splits Channels from Direct messages the same way (Up and Down), saved
//! as a fraction of its height (`sidebar-channels-fraction-v1`), each list
//! keeping at least 96 px. A child of `app`.

use super::App;
use gtk::{glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;

const MIN: i32 = 220;
const MAX: i32 = 440;
const SIDEBAR: i32 = 280;
const MEMBERS: i32 = 220;
/// Room the conversation keeps beside the sidebar, and beside the members.
const BESIDE_SIDEBAR: i32 = 380;
const BESIDE_MEMBERS: i32 = 320;
/// Arrow keys move a line this far.
const STEP: i32 = 10;
/// The line between panes (`paned.caper-split > separator`).
const LINE: i32 = 1;
const SIDEBAR_KEY: &str = "sidebar-width-v1";
const MEMBERS_KEY: &str = "members-width-v1";
const LISTS_KEY: &str = "sidebar-channels-fraction-v1";

fn saved(app: &App, key: &str, default: i32) -> i32 {
    app.prefs
        .get::<i32>(key)
        .filter(|width| (MIN..=MAX).contains(width))
        .unwrap_or(default)
}

/// The shortest either sidebar list can be in a split `height` tall.
fn list_min(height: i32) -> i32 {
    if height >= 240 { 96 } else { height / 4 }
}

/// The channel list's height in a split `height` tall.
fn lists_position(position: i32, height: i32) -> i32 {
    let min = list_min(height);
    position.clamp(min, (height - min).max(min))
}

/// The widest the sidebar can be in a split `total` wide.
fn sidebar_max(total: i32) -> i32 {
    (total - BESIDE_SIDEBAR).clamp(MIN, MAX)
}

/// The widest the member list can be beside a conversation; `total` is
/// both together.
fn members_max(total: i32) -> i32 {
    (total - BESIDE_MEMBERS).clamp(MIN, MAX)
}

impl App {
    pub(super) fn connect_panes(self: &Rc<Self>) {
        let split = &self.ui.split;
        split.set_position(saved(self, SIDEBAR_KEY, SIDEBAR));
        let app = Rc::downgrade(self);
        split.connect_position_notify(move |split| {
            let Some(app) = app.upgrade() else {
                return;
            };
            let total = split.width();
            // A narrow window's Browse fills the split; its width isn't one.
            if total <= 0 || app.is_narrow() {
                return;
            }
            let position = split.position();
            let clamped = position.clamp(MIN, sidebar_max(total));
            if clamped != position {
                split.set_position(clamped);
            } else if total >= clamped + BESIDE_SIDEBAR {
                // Only a width the window had room for is remembered.
                app.prefs.set(SIDEBAR_KEY, clamped);
            }
        });
        keys(split, |split, key| {
            let next = match key {
                Key::Shorter => split.position() - STEP,
                Key::Wider => split.position() + STEP,
                Key::Min => MIN,
                Key::Max => MAX,
            };
            split.set_position(next.clamp(MIN, sidebar_max(split.width())));
        });
        reset_on_double_click(split, |split| split.set_position(SIDEBAR));

        let content = &self.ui.content_split;
        let app = Rc::downgrade(self);
        content.connect_position_notify(move |content| {
            let Some(app) = app.upgrade() else {
                return;
            };
            let total = content.width();
            if total <= 0
                || app.is_narrow()
                || !app.ui.members_panel.is_visible()
                || !app.panes_placed.get()
            {
                return;
            }
            let width = total - content.position() - LINE;
            let clamped = width.clamp(MIN, members_max(total));
            if clamped != width {
                content.set_position(total - clamped - LINE);
            } else if total >= clamped + BESIDE_MEMBERS {
                app.prefs.set(MEMBERS_KEY, clamped);
            }
        });
        keys(content, |content, key| {
            let total = content.width();
            let width = total - content.position() - LINE;
            let next = match key {
                // Left moves the line left: the member list widens.
                Key::Shorter => width + STEP,
                Key::Wider => width - STEP,
                Key::Min => MIN,
                Key::Max => MAX,
            };
            content.set_position(total - next.clamp(MIN, members_max(total)) - LINE);
        });
        reset_on_double_click(content, |content| {
            content.set_position(content.width() - MEMBERS - LINE)
        });
        // The member list's width is placed once the split has one, and
        // again whenever it comes back.
        let app = Rc::downgrade(self);
        self.ui.members_panel.connect_visible_notify(move |panel| {
            if let Some(app) = app.upgrade()
                && panel.is_visible()
            {
                app.place_members();
            }
        });
        self.place_members();

        // Channels above Direct messages. GTK keeps the line where it was,
        // proportionally, as the sidebar's height changes; only a move at a
        // steady height is the person's, and saved.
        let lists = &self.ui.sidebar_lists;
        let fraction = self
            .prefs
            .get::<f64>(LISTS_KEY)
            .filter(|fraction| fraction.is_finite() && *fraction > 0.0 && *fraction < 1.0)
            .unwrap_or(0.5);
        // Nothing is saved before the saved fraction is placed: GTK's own
        // first position isn't the person's.
        let placed = Rc::new(Cell::new(false));
        let height = Rc::new(Cell::new(0));
        // A resize that leaves the line in place still changes the height.
        let seen = height.clone();
        lists.connect_max_position_notify(move |lists| seen.set(lists.height() - LINE));
        let app = Rc::downgrade(self);
        let ready = placed.clone();
        let steady = height.clone();
        lists.connect_position_notify(move |lists| {
            let Some(app) = app.upgrade() else {
                return;
            };
            let total = lists.height() - LINE;
            if total <= 0 || !ready.get() {
                return;
            }
            let position = lists.position();
            let clamped = lists_position(position, total);
            if clamped != position {
                lists.set_position(clamped);
            } else if steady.replace(total) == total {
                app.prefs
                    .set(LISTS_KEY, f64::from(position) / f64::from(total));
            }
        });
        keys(lists, |lists, key| {
            let total = lists.height() - LINE;
            let next = match key {
                Key::Shorter => lists.position() - STEP,
                Key::Wider => lists.position() + STEP,
                Key::Min => 0,
                Key::Max => total,
            };
            lists.set_position(lists_position(next, total));
        });
        reset_on_double_click(lists, |lists| {
            lists.set_position((lists.height() - LINE) / 2);
        });
        lists.add_tick_callback(move |lists, _| {
            let total = lists.height() - LINE;
            if total <= 0 {
                return glib::ControlFlow::Continue;
            }
            #[allow(clippy::cast_possible_truncation)]
            let position = (fraction * f64::from(total)).round() as i32;
            lists.set_position(lists_position(position, total));
            height.set(total);
            placed.set(true);
            glib::ControlFlow::Break
        });
    }

    /// Puts the sidebar back at its saved width, as a window widens past
    /// the narrow layout.
    /// It waits a frame, for the split's wide width.
    pub(super) fn place_sidebar(self: &Rc<Self>) {
        let app = Rc::downgrade(self);
        self.ui.split.add_tick_callback(move |split, _| {
            if let Some(app) = app.upgrade() {
                split.set_position(saved(&app, SIDEBAR_KEY, SIDEBAR));
            }
            glib::ControlFlow::Break
        });
    }

    pub(super) fn place_members(self: &Rc<Self>) {
        self.panes_placed.set(false);
        let app = Rc::downgrade(self);
        self.ui.content_split.add_tick_callback(move |content, _| {
            let Some(app) = app.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let total = content.width();
            if total <= 0 {
                return glib::ControlFlow::Continue;
            }
            let width = saved(&app, MEMBERS_KEY, MEMBERS).min(members_max(total));
            content.set_position(total - width - LINE);
            app.panes_placed.set(true);
            glib::ControlFlow::Break
        });
    }
}

/// What a key does to a focused line: Left/Up, Right/Down, Home and End.
enum Key {
    Shorter,
    Wider,
    Min,
    Max,
}

/// The focused line (F8) moves 10 px per arrow key, and to either end with
/// Home and End, in place of GTK's 1 px steps.
fn keys(paned: &gtk::Paned, apply: impl Fn(&gtk::Paned, Key) + 'static) {
    let controller = gtk::EventControllerKey::new();
    controller.set_propagation_phase(gtk::PropagationPhase::Capture);
    let target = paned.downgrade();
    controller.connect_key_pressed(move |_, key, _, modifiers| {
        // Only the paned itself has focus while its line does.
        let Some(paned) = target.upgrade().filter(|paned| paned.has_focus()) else {
            return glib::Propagation::Proceed;
        };
        if !modifiers.is_empty() {
            return glib::Propagation::Proceed;
        }
        let vertical = paned.orientation() == gtk::Orientation::Vertical;
        let action = match key {
            gtk::gdk::Key::Left | gtk::gdk::Key::KP_Left if !vertical => Key::Shorter,
            gtk::gdk::Key::Right | gtk::gdk::Key::KP_Right if !vertical => Key::Wider,
            gtk::gdk::Key::Up | gtk::gdk::Key::KP_Up if vertical => Key::Shorter,
            gtk::gdk::Key::Down | gtk::gdk::Key::KP_Down if vertical => Key::Wider,
            gtk::gdk::Key::Home | gtk::gdk::Key::KP_Home => Key::Min,
            gtk::gdk::Key::End | gtk::gdk::Key::KP_End => Key::Max,
            _ => return glib::Propagation::Proceed,
        };
        apply(&paned, action);
        glib::Propagation::Stop
    });
    paned.add_controller(controller);
}

/// Double-clicking the line between panes puts it back where it started.
fn reset_on_double_click(paned: &gtk::Paned, reset: impl Fn(&gtk::Paned) + 'static) {
    let click = gtk::GestureClick::new();
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let target = paned.downgrade();
    click.connect_pressed(move |_, presses, x, y| {
        if let Some(paned) = target.upgrade()
            && presses == 2
            && ((if paned.orientation() == gtk::Orientation::Vertical {
                y
            } else {
                x
            }) - f64::from(paned.position()))
            .abs()
                <= 4.0
        {
            reset(&paned);
        }
    });
    paned.add_controller(click);
}
