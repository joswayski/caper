//! The resizable channel sidebar and member list, as on the egui desktop:
//! each is 220–440 px, saved per device (`sidebar-width-v1`,
//! `members-width-v1`). Drag the line between panes, or focus it (F8) and
//! use the arrow keys (10 px), Home and End; double-click resets it. The
//! conversation keeps room beside them, and a window too small for a saved
//! width shows it narrower without forgetting it. A child of `app`.

use super::App;
use gtk::{glib, prelude::*};
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

fn saved(app: &App, key: &str, default: i32) -> i32 {
    app.prefs
        .get::<i32>(key)
        .filter(|width| (MIN..=MAX).contains(width))
        .unwrap_or(default)
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
            if total <= 0 {
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
            if total <= 0 || !app.ui.members_panel.is_visible() || !app.panes_placed.get() {
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
    }

    fn place_members(self: &Rc<Self>) {
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

/// What a key does to a focused line: Left, Right, Home and End.
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
        let action = match key {
            gtk::gdk::Key::Left | gtk::gdk::Key::KP_Left => Key::Shorter,
            gtk::gdk::Key::Right | gtk::gdk::Key::KP_Right => Key::Wider,
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
    click.connect_pressed(move |_, presses, x, _| {
        if let Some(paned) = target.upgrade()
            && presses == 2
            && (x - f64::from(paned.position())).abs() <= 4.0
        {
            reset(&paned);
        }
    });
    paned.add_controller(click);
}
