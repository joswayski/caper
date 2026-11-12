//! The update notice's decorative button edge, as desktop's
//! `update_art.rs`.

use gtk::{glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

/// "Update available" with the desktop's shine (desktop/src/update_art.rs,
/// UpdateButtonShine on macOS): a still five-colour edge and a short light
/// sweep every eight seconds, drawn over the button by a layer that takes
/// no input, so the label and hit target never move. No edge while the
/// button shows keyboard focus (its focus ring stays), nothing while it is
/// disabled, and no sweep when animations are off (reduced motion) or the
/// window is in the background. The overlay shows with the button.
pub fn shine(button: &gtk::Button) -> gtk::Overlay {
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("update-dock-host");
    overlay.set_child(Some(button));
    button
        .bind_property("visible", &overlay, "visible")
        .sync_create()
        .build();
    let layer = gtk::DrawingArea::new();
    layer.set_can_target(false);
    overlay.add_overlay(&layer);
    let start = Rc::new(Cell::new(None::<Instant>));
    let (weak, since) = (button.downgrade(), start.clone());
    layer.set_draw_func(move |layer, cr, width, height| {
        let Some(button) = weak.upgrade() else {
            return;
        };
        if !button.is_sensitive() {
            return;
        }
        let sweep = since
            .get()
            .filter(|_| sweeping(layer))
            .map(|start| start.elapsed().as_secs_f64() % SHINE_PERIOD)
            .filter(|phase| *phase < SHINE_SWEEP)
            .map(|phase| ease_out(phase / SHINE_SWEEP));
        draw_shine(
            cr,
            f64::from(width),
            f64::from(height),
            !button.has_visible_focus(),
            sweep,
        );
    });
    let focus = gtk::EventControllerFocus::new();
    for signal in ["enter", "leave"] {
        let layer = layer.downgrade();
        focus.connect_local(signal, false, move |_| {
            if let Some(layer) = layer.upgrade() {
                layer.queue_draw();
            }
            None
        });
    }
    button.add_controller(focus);
    button.connect_sensitive_notify({
        let layer = layer.downgrade();
        move |_| {
            if let Some(layer) = layer.upgrade() {
                layer.queue_draw();
            }
        }
    });
    // Repaints only while a sweep crosses; between them a timer waits.
    let ticking: Rc<Cell<Option<gtk::TickCallbackId>>> = Rc::default();
    let timer: Rc<Cell<Option<glib::SourceId>>> = Rc::default();
    layer.connect_map({
        let (start, ticking, timer) = (start.clone(), ticking.clone(), timer.clone());
        move |layer| {
            start.set(Some(Instant::now()));
            sweep_after(layer, &start, &ticking, &timer);
        }
    });
    layer.connect_unmap(move |_| {
        if let Some(id) = ticking.take() {
            id.remove();
        }
        if let Some(id) = timer.take() {
            id.remove();
        }
    });
    overlay
}

/// One sweep's length (`motion::SLOW` × 6) and how often one starts.
const SHINE_SWEEP: f64 = 0.26 * 6.0;
const SHINE_PERIOD: f64 = 8.0;

/// `--ease-out` (desktop's exponential out).
fn ease_out(t: f64) -> f64 {
    if t >= 1.0 {
        1.0
    } else {
        1.0 - 2f64.powf(-10.0 * t.clamp(0.0, 1.0))
    }
}

/// Sweeps play with animations on and the window in front.
fn sweeping(layer: &gtk::DrawingArea) -> bool {
    crate::images::motion()
        && layer
            .root()
            .and_downcast::<gtk::Window>()
            .is_some_and(|window| window.is_active())
}

/// Waits for the next sweep, then repaints every frame while it crosses.
fn sweep_after(
    layer: &gtk::DrawingArea,
    start: &Rc<Cell<Option<Instant>>>,
    ticking: &Rc<Cell<Option<gtk::TickCallbackId>>>,
    timer: &Rc<Cell<Option<glib::SourceId>>>,
) {
    let Some(started) = start.get() else {
        return;
    };
    let phase = started.elapsed().as_secs_f64() % SHINE_PERIOD;
    if phase < SHINE_SWEEP && sweeping(layer) {
        let (start, ticking_done, timer) = (start.clone(), ticking.clone(), timer.clone());
        let id = layer.add_tick_callback(move |layer, _| {
            layer.queue_draw();
            let phase = start.get().map_or(SHINE_PERIOD, |start| {
                start.elapsed().as_secs_f64() % SHINE_PERIOD
            });
            if phase < SHINE_SWEEP {
                return glib::ControlFlow::Continue;
            }
            ticking_done.set(None);
            sweep_after(layer, &start, &ticking_done, &timer);
            glib::ControlFlow::Break
        });
        ticking.set(Some(id));
        return;
    }
    let wait = std::time::Duration::from_secs_f64(SHINE_PERIOD - phase);
    let (layer, start, ticking, again) = (
        layer.downgrade(),
        start.clone(),
        ticking.clone(),
        timer.clone(),
    );
    timer.set(Some(glib::timeout_add_local_once(wait, move || {
        again.set(None);
        if let Some(layer) = layer.upgrade() {
            sweep_after(&layer, &start, &ticking, &again);
        }
    })));
}

/// The edge (when `edge`) and a sweep at `sweep` (0 to 1) over a
/// `width` × `height` button with 8 px corners, scaled with the window's
/// zoom (the dock, padding and border included, is 46 px tall at 100%).
fn draw_shine(cr: &gtk::cairo::Context, width: f64, height: f64, edge: bool, sweep: Option<f64>) {
    const COLORS: [[f64; 3]; 5] = [
        [167.0, 128.0, 242.0],
        [104.0, 175.0, 236.0],
        [99.0, 210.0, 193.0],
        [215.0, 205.0, 130.0],
        [229.0, 139.0, 183.0],
    ];
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let zoom = height / 46.0;
    let rounded = |inset: f64| {
        let radius = (8.0 * zoom - inset).max(0.0);
        let (x0, y0, x1, y1) = (inset, inset, width - inset, height - inset);
        cr.new_sub_path();
        cr.arc(x1 - radius, y0 + radius, radius, -FRAC_PI_2, 0.0);
        cr.arc(x1 - radius, y1 - radius, radius, 0.0, FRAC_PI_2);
        cr.arc(x0 + radius, y1 - radius, radius, FRAC_PI_2, PI);
        cr.arc(x0 + radius, y0 + radius, radius, PI, 3.0 * FRAC_PI_2);
        cr.close_path();
    };
    use std::f64::consts::{FRAC_PI_2, PI};
    if edge {
        // Strokes inside the edge: a soft 4 px glow under a 1 px line.
        for (stroke, alpha) in [(4.0 * zoom, 31.0), (zoom, 191.0)] {
            let gradient = gtk::cairo::LinearGradient::new(0.0, 0.0, width, 0.0);
            for (index, [r, g, b]) in COLORS.iter().enumerate() {
                gradient.add_color_stop_rgba(
                    index as f64 / 4.0,
                    r / 255.0,
                    g / 255.0,
                    b / 255.0,
                    alpha / 255.0,
                );
            }
            rounded(stroke / 2.0);
            cr.set_line_width(stroke);
            let _ = cr.set_source(&gradient);
            let _ = cr.stroke();
        }
    }
    let Some(progress) = sweep else {
        return;
    };
    // A band 56 px wide slanted 18°, from off the left edge to off the
    // right, clipped to the button's corners.
    let band = 28.0 * zoom;
    let center = -52.0 * zoom + (width + 160.0 * zoom) * progress;
    let slant = 18f64.to_radians().tan();
    let light = gtk::cairo::LinearGradient::new(center - band, 0.0, center + band, 0.0);
    light.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.0);
    light.add_color_stop_rgba(0.5, 1.0, 1.0, 1.0, 41.0 / 255.0);
    light.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 0.0);
    // Pattern space measures across the slant: x + (y - height / 2) tan 18°.
    light.set_matrix(gtk::cairo::Matrix::new(
        1.0,
        0.0,
        slant,
        1.0,
        -slant * height / 2.0,
        0.0,
    ));
    rounded(0.0);
    let _ = cr.set_source(&light);
    let _ = cr.fill();
}
