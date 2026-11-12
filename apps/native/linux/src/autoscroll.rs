//! Middle-click autoscroll over a message history, as desktop's
//! `autoscroll.rs` (and Windows apps and browsers): press the middle button
//! to drop an anchor, then move the pointer away from it to scroll at a
//! speed that grows with the distance. A plain middle click keeps scrolling
//! until the next click, Escape, wheel or the window going to the
//! background; holding the button and moving stops on release.

use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Pointer travel from the anchor that does not scroll yet.
const DEAD_ZONE: f64 = 12.0;
/// Points per second of scrolling for each point beyond the dead zone.
const SPEED: f64 = 8.0;
/// The anchor marker's radius.
const MARKER: f64 = 13.0;

#[derive(Default)]
struct State {
    /// Where autoscroll started, in the overlay, while it is active.
    anchor: Option<(f64, f64)>,
    /// The pointer, in the overlay.
    pointer: (f64, f64),
    middle_held: bool,
    /// The held middle button left the dead zone, so its release ends it.
    dragged: bool,
    last_frame: Option<i64>,
    ticking: Option<gtk::TickCallbackId>,
}

/// Points per second for a pointer this far below (positive) or above the
/// anchor: positive scrolls toward newer messages.
fn scroll_speed(offset: f64) -> f64 {
    (offset.abs() - DEAD_ZONE).max(0.0) * SPEED * offset.signum()
}

/// Gives `scroller` middle-click autoscroll, with its anchor marker drawn
/// in `overlay`, which holds the scroller.
pub fn attach(scroller: &gtk::ScrolledWindow, overlay: &gtk::Overlay) {
    let state: Rc<RefCell<State>> = Rc::default();
    let marker = gtk::DrawingArea::new();
    marker.set_content_width((MARKER * 2.0 + 2.0) as i32);
    marker.set_content_height((MARKER * 2.0 + 2.0) as i32);
    marker.set_halign(gtk::Align::Start);
    marker.set_valign(gtk::Align::Start);
    marker.set_can_target(false);
    marker.set_visible(false);
    marker.set_draw_func(|_, cr, width, height| {
        let (x, y) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
        // Surface fill, border ring and a text-coloured dot with arrows.
        cr.arc(x, y, MARKER, 0.0, std::f64::consts::TAU);
        cr.set_source_rgb(
            0x15 as f64 / 255.0,
            0x17 as f64 / 255.0,
            0x19 as f64 / 255.0,
        );
        let _ = cr.fill_preserve();
        cr.set_line_width(1.0);
        cr.set_source_rgb(
            0x34 as f64 / 255.0,
            0x38 as f64 / 255.0,
            0x3b as f64 / 255.0,
        );
        let _ = cr.stroke();
        cr.set_source_rgb(
            0xf3 as f64 / 255.0,
            0xf4 as f64 / 255.0,
            0xf5 as f64 / 255.0,
        );
        cr.arc(x, y, 2.0, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
        for direction in [-1.0, 1.0] {
            cr.move_to(x, y + 9.0 * direction);
            cr.line_to(x + 4.0, y + 5.0 * direction);
            cr.line_to(x - 4.0, y + 5.0 * direction);
            cr.close_path();
            let _ = cr.fill();
        }
    });
    overlay.add_overlay(&marker);
    // While a plain middle click's autoscroll runs, the history's rows stop
    // taking the pointer, so the scroller's cursor shows the direction
    // instead of the text cursor of whatever row is under it.
    let cover = scroller.clone();

    let stop = {
        let (state, cover, marker) = (state.clone(), cover.downgrade(), marker.downgrade());
        Rc::new(move || {
            let mut state = state.borrow_mut();
            if state.anchor.take().is_none() {
                return;
            }
            state.middle_held = false;
            state.dragged = false;
            state.last_frame = None;
            if let Some(id) = state.ticking.take() {
                id.remove();
            }
            if let Some(marker) = marker.upgrade() {
                marker.set_visible(false);
            }
            if let Some(cover) = cover.upgrade() {
                if let Some(rows) = cover.child() {
                    rows.set_can_target(true);
                }
                cover.set_cursor(None);
            }
        })
    };
    // The cursor shows which way it scrolls.
    let point = {
        let state = state.clone();
        move |cover: &gtk::ScrolledWindow| {
            let state = state.borrow();
            let Some((_, anchor)) = state.anchor else {
                return;
            };
            let offset = state.pointer.1 - anchor;
            cover.set_cursor_from_name(Some(if offset.abs() <= DEAD_ZONE {
                "ns-resize"
            } else if offset < 0.0 {
                "n-resize"
            } else {
                "s-resize"
            }));
        }
    };

    // Raw buttons, before the history's own widgets (a click gesture would
    // give up on a held drag): the middle one starts it, or after a drag
    // ends it on release; any press while it runs only ends it, and its
    // release goes nowhere.
    let buttons = gtk::EventControllerLegacy::new();
    buttons.set_propagation_phase(gtk::PropagationPhase::Capture);
    let swallow = Rc::new(Cell::new(None::<u32>));
    {
        let (state, stop, marker, cover, scroller, point) = (
            state.clone(),
            stop.clone(),
            marker.downgrade(),
            cover.downgrade(),
            scroller.downgrade(),
            point.clone(),
        );
        buttons.connect_event(move |controller, event| {
            let (pressed, button) = match event.downcast_ref::<gdk::ButtonEvent>() {
                Some(button) if event.event_type() == gdk::EventType::ButtonPress => {
                    (true, button.button())
                }
                Some(button) if event.event_type() == gdk::EventType::ButtonRelease => {
                    (false, button.button())
                }
                _ => return glib::Propagation::Proceed,
            };
            let (Some(overlay), Some(scroller), Some(cover), Some(marker)) = (
                controller.widget(),
                scroller.upgrade(),
                cover.upgrade(),
                marker.upgrade(),
            ) else {
                return glib::Propagation::Proceed;
            };
            if !pressed {
                if swallow.get() == Some(button) {
                    swallow.set(None);
                    return glib::Propagation::Stop;
                }
                if button != gdk::BUTTON_MIDDLE || state.borrow().anchor.is_none() {
                    return glib::Propagation::Proceed;
                }
                let dragged = {
                    let mut state = state.borrow_mut();
                    state.middle_held = false;
                    state.dragged
                };
                if dragged {
                    stop();
                } else {
                    // Shown once the press is over: a widget appearing
                    // under a held button breaks GTK's tracking of it.
                    // After this release is dispatched, so GTK's tracking of
                    // the press under it stays balanced.
                    let (state, point) = (state.clone(), point.clone());
                    glib::idle_add_local_once(move || {
                        if state.borrow().anchor.is_some() {
                            if let Some(rows) = cover.child() {
                                rows.set_can_target(false);
                            }
                            point(&cover);
                        }
                    });
                }
                return glib::Propagation::Proceed;
            }
            if state.borrow().anchor.is_some() {
                stop();
                swallow.set(Some(button));
                return glib::Propagation::Stop;
            }
            if button != gdk::BUTTON_MIDDLE {
                return glib::Propagation::Proceed;
            }
            // The press in the overlay's coordinates.
            let Some(native) = overlay.native() else {
                return glib::Propagation::Proceed;
            };
            let Some((x, y)) = event.position() else {
                return glib::Propagation::Proceed;
            };
            let (dx, dy) = native.surface_transform();
            let Some(at) = native.compute_point(
                &overlay,
                &gtk::graphene::Point::new((x - dx) as f32, (y - dy) as f32),
            ) else {
                return glib::Propagation::Proceed;
            };
            let (x, y) = (f64::from(at.x()), f64::from(at.y()));
            // Only over the history itself, not its scrollbar.
            let inside = overlay
                .compute_point(&scroller, &gtk::graphene::Point::new(x as f32, y as f32))
                .is_some_and(|at| scroller.contains(f64::from(at.x()), f64::from(at.y())));
            if !inside {
                return glib::Propagation::Proceed;
            }
            {
                let mut state = state.borrow_mut();
                state.anchor = Some((x, y));
                state.pointer = (x, y);
                state.middle_held = true;
                state.dragged = false;
                state.last_frame = None;
            }
            marker.set_margin_start((x - MARKER - 1.0).round().max(0.0) as i32);
            marker.set_margin_top((y - MARKER - 1.0).round().max(0.0) as i32);
            marker.set_visible(true);
            let ticking = state.clone();
            let id = scroller.add_tick_callback(move |scroller, clock| {
                let now = clock.frame_time();
                let mut state = ticking.borrow_mut();
                let Some((_, anchor)) = state.anchor else {
                    return glib::ControlFlow::Break;
                };
                let dt = state
                    .last_frame
                    .map_or(0.0, |last| (now - last) as f64 / 1_000_000.0)
                    .clamp(0.0, 0.1);
                state.last_frame = Some(now);
                let speed = scroll_speed(state.pointer.1 - anchor);
                drop(state);
                if speed != 0.0 {
                    let adjustment = scroller.vadjustment();
                    let end = adjustment.upper() - adjustment.page_size();
                    adjustment.set_value(
                        (adjustment.value() + speed * dt).clamp(adjustment.lower(), end),
                    );
                }
                glib::ControlFlow::Continue
            });
            state.borrow_mut().ticking = Some(id);
            // The middle button means nothing to the history's widgets;
            // letting it through keeps GTK's press tracking balanced.
            glib::Propagation::Proceed
        });
    }
    overlay.add_controller(buttons);

    let motion = gtk::EventControllerMotion::new();
    {
        let (state, cover) = (state.clone(), cover.downgrade());
        motion.connect_motion(move |_, x, y| {
            {
                let mut state = state.borrow_mut();
                state.pointer = (x, y);
                if let Some(anchor) = state.anchor
                    && state.middle_held
                    && ((x - anchor.0).powi(2) + (y - anchor.1).powi(2)).sqrt() > DEAD_ZONE
                {
                    state.dragged = true;
                }
            }
            if let Some(cover) = cover.upgrade() {
                point(&cover);
            }
        });
    }
    overlay.add_controller(motion);

    // The wheel takes over from it.
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    {
        let stop = stop.clone();
        wheel.connect_scroll(move |_, _, _| {
            stop();
            glib::Propagation::Proceed
        });
    }
    overlay.add_controller(wheel);

    // Escape, wherever focus is, and the window going to the background.
    let installed = Cell::new(false);
    scroller.connect_realize(move |scroller| {
        let Some(window) = scroller.root().and_downcast::<gtk::Window>() else {
            return;
        };
        if installed.replace(true) {
            return;
        }
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let (escape, state) = (stop.clone(), state.clone());
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape && state.borrow().anchor.is_some() {
                escape();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
        let away = stop.clone();
        window.connect_is_active_notify(move |window| {
            if !window.is_active() {
                away();
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_grows_past_the_dead_zone_toward_the_pointer() {
        assert_eq!(scroll_speed(0.0), 0.0);
        assert_eq!(scroll_speed(DEAD_ZONE), 0.0);
        assert_eq!(scroll_speed(-DEAD_ZONE), 0.0);
        // Below the anchor scrolls down (toward newer), above scrolls up.
        assert_eq!(scroll_speed(DEAD_ZONE + 10.0), 80.0);
        assert_eq!(scroll_speed(-(DEAD_ZONE + 10.0)), -80.0);
    }
}
