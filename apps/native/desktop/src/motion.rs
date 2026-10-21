//! Entrances for what appears in the app, as on web (`apps/web/src/motion.css`):
//! dialogs, menus, cards, the thread panel, arriving messages and composer
//! files fade and settle into place instead of appearing at once. Nothing
//! animates on the way out: closing stays instant.
//!
//! Shapes are adjusted after they are painted, so hit areas stay where the
//! settled content will be; the offsets are a few points and gone within
//! `SLOW`. Each entrance repaints only until it lands.

use eframe::egui::{
    self, Color32, Context, Id, LayerId, Pos2, Shape, Vec2, emath::TSTransform, layers::ShapeIdx,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

static REDUCED: AtomicBool = AtomicBool::new(false);

/// The system asks for less motion (web's `prefers-reduced-motion`): slides
/// and entrances show their resting pose at once.
pub fn reduced() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

/// Refreshes the system's setting every two seconds off the UI thread and
/// repaints only when it changes. GNOME's "Animations" (`enable-animations`), KDE's animation
/// speed set to instant, and Windows' "Animation effects" (`MinAnimate`) count.
pub fn watch_reduced(context: &Context) {
    let context = context.clone();
    std::thread::spawn(move || {
        loop {
            let reduced = system_reduces_motion();
            if REDUCED.swap(reduced, Ordering::Relaxed) != reduced {
                context.request_repaint();
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

#[cfg(target_os = "linux")]
fn system_reduces_motion() -> bool {
    let gnome = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .is_some_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "false");
    let kde = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| std::path::Path::new(&home).join(".config"))
        })
        .and_then(|config| std::fs::read_to_string(config.join("kdeglobals")).ok())
        .is_some_and(|globals| kde_animations_off(&globals));
    gnome || kde
}

#[cfg(target_os = "windows")]
fn system_reduces_motion() -> bool {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey("Control Panel\\Desktop\\WindowMetrics")
        .and_then(|key| key.get_value::<String, _>("MinAnimate"))
        .is_ok_and(|value| value.trim() == "0")
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn system_reduces_motion() -> bool {
    false
}

/// `AnimationDurationFactor=0` in `kdeglobals`' `[KDE]` group: "Instant".
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
fn kde_animations_off(globals: &str) -> bool {
    let mut group = "";
    for line in globals.lines().map(str::trim) {
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            group = name;
        } else if group == "KDE"
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "AnimationDurationFactor"
        {
            return value
                .trim()
                .parse::<f32>()
                .is_ok_and(|factor| factor <= 0.0);
        }
    }
    false
}

/// `--motion-fast`, `--motion-base` and `--motion-slow` in seconds.
pub const FAST: f32 = 0.14;
pub const BASE: f32 = 0.2;
pub const SLOW: f32 = 0.26;

/// `--ease-out`, `cubic-bezier(0.16, 1, 0.3, 1)`: quick to start, long to settle.
pub fn ease_out(t: f32) -> f32 {
    egui::emath::easing::exponential_out(t.clamp(0.0, 1.0))
}

/// A slow breath for something that waits: opacity eases (a cosine, so in
/// and out) from 1 down to `low` and back once every `period` seconds.
pub fn breath(time: f64, period: f64, low: f32) -> f32 {
    let phase = (time.rem_euclid(period) / period * std::f64::consts::TAU).cos() as f32;
    low + (1.0 - low) * (0.5 + 0.5 * phase)
}

/// The opacity of a [`breath`] while it shows, repainting for the next
/// step; still at 1 with reduced motion (and in headless tests).
pub fn breathing(context: &Context, period: f64, low: f32) -> f32 {
    if !enabled(context) {
        return 1.0;
    }
    context.request_repaint_after(std::time::Duration::from_millis(33));
    breath(context.input(|input| input.time), period, low)
}

fn enabled_id() -> Id {
    Id::new("caper-motion")
}

/// Headless tests draw final states unless they turn motion on.
fn enabled(context: &Context) -> bool {
    context
        .data(|data| data.get_temp(enabled_id()))
        .unwrap_or(!cfg!(test) && !reduced())
}

#[cfg(test)]
pub fn enable(context: &Context) {
    context.data_mut(|data| data.insert_temp(enabled_id(), true));
}

/// Eased progress, 0 to 1, of an entrance that started at `start` (egui input
/// time). Requests a repaint until it lands.
pub fn progress(context: &Context, start: f64, duration: f32) -> f32 {
    if !enabled(context) {
        return 1.0;
    }
    let elapsed = (context.input(|input| input.time) - start) as f32;
    let t = (elapsed / duration).clamp(0.0, 1.0);
    if t < 1.0 {
        context.request_repaint();
    }
    ease_out(t)
}

/// Progress of something drawn on every pass while it shows: it starts again
/// whenever `id` was not drawn on the previous pass.
pub fn entrance(context: &Context, id: Id, duration: f32) -> f32 {
    let pass = context.cumulative_pass_nr();
    let now = context.input(|input| input.time);
    let start = context.data_mut(|data| {
        let (seen, start) = data.get_temp_mut_or(id, (pass, now));
        if *seen + 1 < pass {
            *start = now;
        }
        *seen = pass;
        *start
    });
    progress(context, start, duration)
}

/// `value` approached linearly over `duration` from where it last was, as
/// web's progress bars do.
pub fn glide(context: &Context, id: Id, value: f32, duration: f32) -> f32 {
    if !enabled(context) {
        return value;
    }
    context.animate_value_with_time(id, value, duration)
}

/// Moves by what remains of `offset` at `progress`.
pub fn rise(progress: f32, offset: Vec2) -> TSTransform {
    TSTransform::from_translation(offset * (1.0 - progress))
}

/// Scales about `center`, from `from` at the start to 1 when it lands.
pub fn grow(progress: f32, from: f32, center: Pos2) -> TSTransform {
    let scale = from + (1.0 - from) * progress;
    TSTransform::from_translation(center.to_vec2())
        * TSTransform::from_scaling(scale)
        * TSTransform::from_translation(-center.to_vec2())
}

/// Where the next shape on `ui`'s layer goes, for `settle` afterwards.
pub fn mark(ui: &egui::Ui) -> usize {
    ui.ctx().graphics(|graphics| {
        graphics
            .get(ui.layer_id())
            .map_or(0, |shapes| shapes.next_idx().0)
    })
}

/// Fades the shapes painted on `layer` since `from` and moves them.
pub fn settle(
    context: &Context,
    layer: LayerId,
    from: usize,
    opacity: f32,
    transform: TSTransform,
) {
    settle_range(context, layer, from..usize::MAX, opacity, transform);
}

/// Like [`settle`] for the shapes in `range` only.
pub fn settle_range(
    context: &Context,
    layer: LayerId,
    range: std::ops::Range<usize>,
    opacity: f32,
    transform: TSTransform,
) {
    if opacity >= 1.0 && transform == TSTransform::IDENTITY {
        return;
    }
    context.graphics_mut(|graphics| {
        let shapes = graphics.entry(layer);
        for index in range.start..range.end.min(shapes.next_idx().0) {
            shapes.mutate_shape(ShapeIdx(index), |clipped| {
                clipped.shape.transform(transform);
                fade(&mut clipped.shape, opacity);
            });
        }
    });
}

/// Like [`settle`] for shapes in `range`, but their clip rectangles travel
/// with them: content a finger drags past its frame is carried, not cut off.
pub fn carry(
    context: &Context,
    layer: LayerId,
    range: std::ops::Range<usize>,
    opacity: f32,
    transform: TSTransform,
) {
    if opacity >= 1.0 && transform == TSTransform::IDENTITY {
        return;
    }
    context.graphics_mut(|graphics| {
        let shapes = graphics.entry(layer);
        for index in range.start..range.end.min(shapes.next_idx().0) {
            shapes.mutate_shape(ShapeIdx(index), |clipped| {
                clipped.shape.transform(transform);
                clipped.clip_rect = transform * clipped.clip_rect;
                fade(&mut clipped.shape, opacity);
            });
        }
    });
}

fn fade(shape: &mut Shape, opacity: f32) {
    if opacity >= 1.0 {
        return;
    }
    match shape {
        Shape::Vec(shapes) => shapes.iter_mut().for_each(|shape| fade(shape, opacity)),
        Shape::Text(text) => text.opacity_factor *= opacity,
        _ => egui::epaint::shape_transform::adjust_colors(shape, move |color| {
            if *color != Color32::PLACEHOLDER {
                *color = color.gamma_multiply(opacity);
            }
        }),
    }
}

/// Draws `content`, settling what it painted on `ui`'s layer into place
/// `offset` away from where it started.
pub fn arrive<R>(
    ui: &mut egui::Ui,
    progress: f32,
    offset: Vec2,
    content: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    if progress >= 1.0 {
        return content(ui);
    }
    let from = mark(ui);
    let inner = content(ui);
    settle(
        ui.ctx(),
        ui.layer_id(),
        from,
        progress,
        rise(progress, offset),
    );
    inner
}

/// When the `Area` (window, modal, menu or card) on `layer` last became visible.
pub fn shown_at(context: &Context, layer: LayerId) -> Option<f64> {
    egui::AreaState::load(context, layer.id).and_then(|state| state.last_became_visible_at)
}

/// A card, menu or window rising `distance` points into place over
/// `duration` after it opens. With `fade` it fades in too, for areas shown
/// with `fade_in(false)`; egui already fades the others (its menus).
pub fn overlay(context: &Context, layer: LayerId, duration: f32, distance: f32, fade: bool) {
    let Some(start) = shown_at(context, layer) else {
        return;
    };
    let progress = progress(context, start, duration);
    let opacity = if fade { progress } else { 1.0 };
    settle(
        context,
        layer,
        0,
        opacity,
        rise(progress, egui::vec2(0.0, distance)),
    );
}

/// An `egui::Modal` egui fades in itself: its dialog rises 8 points over
/// `BASE` while the backdrop (the layer's first shape) stays put.
pub fn modal(context: &Context, layer: LayerId) {
    let Some(start) = shown_at(context, layer) else {
        return;
    };
    let dialog = progress(context, start, BASE);
    settle(context, layer, 1, 1.0, rise(dialog, egui::vec2(0.0, 8.0)));
}

/// A full-window `egui::Modal` (the attachment preview): its contents fade
/// in and grow from 98% about `center` over `BASE` while the backdrop (the
/// layer's first shape) stays put. Closing stays instant.
pub fn modal_zoom(context: &Context, layer: LayerId, center: Pos2) {
    let Some(start) = shown_at(context, layer) else {
        return;
    };
    let progress = progress(context, start, BASE);
    settle(context, layer, 1, progress, grow(progress, 0.98, center));
}

/// An opacity easing towards `shown` (1) or hidden (0) over `BASE`,
/// decelerating from wherever it was when `shown` last changed, so a quick
/// toggle back never jumps. `state` keeps where it started, where it is
/// going and when; the first call shows the resting state at once.
pub fn fade_toward(context: &Context, state: &mut Option<(f32, bool, f64)>, shown: bool) -> f32 {
    let target = if shown { 1.0 } else { 0.0 };
    let Some((from, to, start)) = *state else {
        *state = Some((target, shown, f64::NEG_INFINITY));
        return target;
    };
    let toward = if to { 1.0 } else { 0.0 };
    let current = from + (toward - from) * progress(context, start, BASE);
    if to == shown {
        return current;
    }
    // Turned around: start again from here (at once without motion).
    let now = context.input(|input| input.time);
    *state = Some((current, shown, now));
    current + (target - current) * progress(context, now, BASE)
}

/// How long a row counts as arriving: long enough for its entrance, short
/// enough that a row drawn again later never replays it.
pub const ARRIVAL: f64 = 1.0;

#[derive(Clone, Default)]
struct Displacement {
    event: Option<f64>,
    until: f64,
    shift: f32,
    rows: HashMap<String, (f32, f32, f64, u64, f32)>,
}

/// Paint-only FLIP compensation. Scroll offsets and virtual-list measurements
/// remain physical and immediate. Keep the window open across measuring passes.
pub fn begin_displacement(
    context: &Context,
    id: Id,
    event: Option<f64>,
    watching: bool,
    live: bool,
) {
    let now = context.input(|input| input.time);
    let pass = context.cumulative_pass_nr();
    let allowed = live
        && enabled(context)
        && !context.input(|input| {
            input.raw_scroll_delta != Vec2::ZERO
                || input.pointer.is_decidedly_dragging()
                || [
                    egui::Key::PageUp,
                    egui::Key::PageDown,
                    egui::Key::Home,
                    egui::Key::End,
                ]
                .iter()
                .any(|key| input.key_pressed(*key))
        });
    context.data_mut(|data| {
        let state = data.get_temp_mut_or_default::<Displacement>(id);
        state.shift = 0.0;
        state.rows.retain(|_, row| row.3 + 1 >= pass);
        if !allowed {
            state.rows.clear();
            state.until = 0.0;
        } else if watching
            && event != state.event
            && event.is_some_and(|at| now - at < f64::from(SLOW))
        {
            state.until = now + f64::from(SLOW);
        }
        state.event = event;
        data.insert_temp(Id::new("displacement-active"), (id, allowed));
    });
}

pub fn end_displacement(context: &Context) {
    context.data_mut(|data| data.remove::<(Id, bool)>(Id::new("displacement-active")));
}

/// Carry the whole timeline, including dividers, from its last painted position.
/// A single transform keeps newly inserted content clear of the retained rows.
pub fn displaced<R>(ui: &mut egui::Ui, key: &str, content: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let top = ui.cursor().top();
    let from = mark(ui);
    let inner = content(ui);
    let now = ui.input(|input| input.time);
    let pass = ui.ctx().cumulative_pass_nr();
    let active = ui
        .ctx()
        .data(|data| data.get_temp::<(Id, bool)>(Id::new("displacement-active")));
    if let Some((id, true)) = active {
        let offset = ui.ctx().data_mut(|data| {
            let state = data.get_temp_mut_or_default::<Displacement>(id);
            let row = state.rows.entry(key.to_owned()).or_insert((
                top,
                state.shift,
                now,
                pass,
                state.shift,
            ));
            if now <= state.until && (top - row.0).abs() > 0.1 {
                // Retarget from the last actually painted pose, not an imagined
                // intermediate frame toward a provisional layout measurement.
                let from = row.4 + row.0 - top;
                *row = (top, from, now, pass, from);
            } else {
                if (top - row.0).abs() > 0.1 {
                    row.1 = 0.0;
                }
                row.0 = top;
            }
            row.3 = pass;
            row.4 = row.1 * (1.0 - ease_out(((now - row.2) / f64::from(SLOW)) as f32));
            state.shift = row.4;
            state.shift
        });
        if offset.abs() > 0.01 {
            ui.ctx().request_repaint();
            settle(
                ui.ctx(),
                ui.layer_id(),
                from,
                1.0,
                TSTransform::from_translation(egui::vec2(0.0, offset)),
            );
        }
    }
    inner
}

struct List {
    key: String,
    ids: Vec<String>,
    live: bool,
}

/// Remembers which messages arrived while the reader watched the live end of
/// a list, so only those animate in (web's `chat/arrivals.ts`). The first
/// render of a channel or thread, older pages, jumps into history and newer
/// pages loaded towards the live end all appear in place. A pending send
/// keeps its client message ID once confirmed, so it animates once.
#[derive(Default)]
pub struct Arrivals {
    lists: HashMap<&'static str, List>,
    arrived: HashMap<String, f64>,
}

impl Arrivals {
    pub fn latest(&self, scope: &'static str) -> Option<f64> {
        self.lists
            .get(scope)?
            .ids
            .iter()
            .filter_map(|id| self.arrived.get(id).copied())
            .max_by(f64::total_cmp)
    }

    /// Call on every pass with the list's client message IDs, oldest first.
    /// `live` is true when the list ends at the newest message. Both this and
    /// the previous pass must be live, so a newer page loaded on the way back
    /// to the live end does not count. `now` is egui input time.
    pub fn observe<'a>(
        &mut self,
        scope: &'static str,
        key: &str,
        ids: impl IntoIterator<Item = &'a str>,
        live: bool,
        now: f64,
    ) {
        let ids: Vec<&str> = ids.into_iter().collect();
        let unchanged = self.lists.get(scope).is_some_and(|list| {
            list.key == key
                && list.live == live
                && list.ids.iter().map(String::as_str).eq(ids.iter().copied())
        });
        if unchanged {
            return;
        }
        let next = List {
            key: key.to_owned(),
            ids: ids.iter().map(|id| (*id).to_owned()).collect(),
            live,
        };
        let Some(previous) = self
            .lists
            .insert(scope, next)
            .filter(|previous| previous.key == key && previous.live && live)
        else {
            return;
        };
        let Some(last) = previous.ids.last() else {
            return;
        };
        // Only rows after the previous newest row are arrivals; prepended history is not.
        let Some(tail) = ids.iter().rposition(|id| id == last) else {
            return;
        };
        for id in &ids[tail + 1..] {
            if !previous.ids.iter().any(|seen| seen == id) {
                self.arrived.entry((*id).to_owned()).or_insert(now);
            }
        }
        self.arrived.retain(|_, at| now - *at <= ARRIVAL);
    }

    /// When the row with this client message ID arrived, while it still counts.
    pub fn arrived(&self, id: &str, now: f64) -> Option<f64> {
        self.arrived
            .get(id)
            .copied()
            .filter(|at| now - at <= ARRIVAL)
    }
}

#[cfg(test)]
mod tests {
    use super::{ARRIVAL, Arrivals, BASE, breath, breathing, enable, entrance, kde_animations_off};
    use eframe::egui;

    #[test]
    fn a_breath_eases_from_full_to_low_and_back_every_period() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        assert!(close(breath(0.0, 2.0, 0.4), 1.0));
        assert!(close(breath(0.5, 2.0, 0.4), 0.7));
        assert!(close(breath(1.0, 2.0, 0.4), 0.4));
        assert!(close(breath(1.5, 2.0, 0.4), 0.7));
        assert!(close(breath(2.0, 2.0, 0.4), 1.0));
        assert!(close(breath(7.0, 2.0, 0.4), 0.4), "it loops");
        // In-out: slow near either end, fastest halfway.
        let step = |t: f64| (breath(t + 0.05, 2.0, 0.4) - breath(t, 2.0, 0.4)).abs();
        assert!(step(0.0) < step(0.475) && step(0.95) < step(0.475));
        let mut previous = 1.0;
        for tenth in 1..=10 {
            let value = breath(f64::from(tenth) / 10.0, 2.0, 0.4);
            assert!(value < previous && value >= 0.4);
            previous = value;
        }
    }

    #[test]
    fn breathing_is_still_without_motion() {
        for (motion, expected) in [(false, 1.0), (true, 0.4)] {
            let context = egui::Context::default();
            let mut alpha = 0.0;
            let _ = context.run(
                egui::RawInput {
                    time: Some(1.0),
                    ..Default::default()
                },
                |context| {
                    if motion {
                        enable(context);
                    }
                    alpha = breathing(context, 2.0, 0.4);
                },
            );
            assert!((alpha - expected).abs() < 1e-4);
        }
    }

    fn displaced_at(
        context: &egui::Context,
        time: f64,
        top: f32,
        event: Option<f64>,
        live: bool,
        motion: bool,
    ) -> f32 {
        let mut painted = 0.0;
        let _ = context.run(
            egui::RawInput {
                time: Some(time),
                ..Default::default()
            },
            |context| {
                if motion {
                    enable(context);
                }
                // ScrollArea can temporarily report not-at-end while it measures
                // the append. That must not cancel an already-started animation.
                let watching = live && event.is_none_or(|at| time <= at);
                super::begin_displacement(context, egui::Id::new("list"), event, watching, live);
                egui::CentralPanel::default().show(context, |ui| {
                    ui.add_space(top);
                    let from = super::mark(ui);
                    super::displaced(ui, "timeline", |ui| {
                        ui.painter().rect_filled(
                            egui::Rect::from_min_size(ui.cursor().min, egui::vec2(20.0, 20.0)),
                            0.0,
                            egui::Color32::WHITE,
                        );
                    });
                    painted = context.graphics(|graphics| {
                        graphics
                            .get(ui.layer_id())
                            .unwrap()
                            .all_entries()
                            .nth(from)
                            .unwrap()
                            .shape
                            .visual_bounding_rect()
                            .top()
                    });
                });
                super::end_displacement(context);
            },
        );
        painted
    }

    #[test]
    fn live_displacement_preserves_pose_then_eases_to_physical_position() {
        let context = egui::Context::default();
        let before = displaced_at(&context, 1.0, 100.0, None, true, true);
        let first = displaced_at(&context, 1.1, 40.0, Some(1.1), true, true);
        assert!((first - before).abs() < 0.01);
        let middle = displaced_at(&context, 1.23, 40.0, Some(1.1), true, true);
        assert!(middle < before - 30.0 && middle > before - 60.0);
        let landed = displaced_at(&context, 1.4, 40.0, Some(1.1), true, true);
        assert!((landed - (before - 60.0)).abs() < 0.01);
        // Confirmation keeps the same event and does not restart displacement.
        assert_eq!(
            displaced_at(&context, 1.5, 40.0, Some(1.1), true, true),
            landed
        );
    }

    #[test]
    fn history_and_disabled_motion_move_immediately() {
        for (live, motion) in [(false, true), (true, false)] {
            let context = egui::Context::default();
            let before = displaced_at(&context, 1.0, 100.0, None, live, motion);
            let after = displaced_at(&context, 1.1, 40.0, Some(1.1), live, motion);
            assert!((after - (before - 60.0)).abs() < 0.01);
        }
    }

    #[test]
    fn measurement_passes_and_burst_arrivals_keep_the_current_painted_pose() {
        let context = egui::Context::default();
        let before = displaced_at(&context, 1.0, 100.0, None, true, true);
        displaced_at(&context, 1.1, 70.0, Some(1.1), true, true);
        // A second measuring pass at the same time changes the physical pose.
        assert_eq!(
            displaced_at(&context, 1.1, 40.0, Some(1.1), true, true),
            before
        );
        let painted = displaced_at(&context, 1.2, 40.0, Some(1.1), true, true);
        assert_eq!(
            displaced_at(&context, 1.2, 10.0, Some(1.2), true, true),
            painted
        );
        // Leaving the live end cancels even an in-flight compensation.
        let immediate = displaced_at(&context, 1.21, 10.0, Some(1.2), false, true);
        assert!((immediate - (before - 90.0)).abs() < 0.01);
    }

    #[test]
    fn corrected_short_list_measurement_does_not_overshoot_then_reverse() {
        let context = egui::Context::default();
        let before = displaced_at(&context, 1.0, 200.0, None, true, true);
        // The first pass temporarily includes both bottom padding and a stale
        // scroll offset. The next frame clamps the offset back to zero.
        assert_eq!(
            displaced_at(&context, 1.1, 6.0, Some(1.1), true, true),
            before
        );
        assert_eq!(
            displaced_at(&context, 1.15, 103.0, Some(1.1), true, true),
            before
        );
        let middle = displaced_at(&context, 1.2, 103.0, Some(1.1), true, true);
        assert!(middle < before && middle > before - 97.0);
        let final_position = displaced_at(&context, 1.5, 103.0, Some(1.1), true, true);
        assert!((final_position - (before - 97.0)).abs() < 0.01);
    }

    #[test]
    fn layout_changes_without_a_live_append_are_immediate() {
        let context = egui::Context::default();
        let before = displaced_at(&context, 1.0, 100.0, None, true, true);
        let after = displaced_at(&context, 1.1, 40.0, None, true, true);
        assert!((after - (before - 60.0)).abs() < 0.01);
    }

    #[test]
    fn kde_instant_animations_reduce_motion() {
        assert!(kde_animations_off(
            "[General]\nfoo=1\n[KDE]\nAnimationDurationFactor=0\n"
        ));
        assert!(!kde_animations_off("[KDE]\nAnimationDurationFactor=0.5\n"));
        assert!(!kde_animations_off("[Other]\nAnimationDurationFactor=0\n"));
        assert!(!kde_animations_off(""));
    }

    /// Runs one pass at `time` seconds and returns the entrance's progress.
    fn entrance_at(context: &egui::Context, time: f64, motion: bool) -> f32 {
        let input = egui::RawInput {
            time: Some(time),
            ..Default::default()
        };
        let mut progress = 0.0;
        let _ = context.run(input, |context| {
            if motion {
                enable(context);
            }
            progress = entrance(context, egui::Id::new("entering"), BASE);
        });
        progress
    }

    #[test]
    fn entrances_start_hidden_and_land_within_their_duration() {
        let context = egui::Context::default();
        assert_eq!(entrance_at(&context, 1.0, true), 0.0);
        let midway = entrance_at(&context, 1.0 + f64::from(BASE) / 2.0, true);
        assert!(
            midway > 0.5 && midway < 1.0,
            "eased out, not linear: {midway}"
        );
        assert_eq!(entrance_at(&context, 1.0 + f64::from(BASE), true), 1.0);
    }

    #[test]
    fn headless_tests_draw_final_states() {
        assert_eq!(entrance_at(&egui::Context::default(), 1.0, false), 1.0);
    }

    fn has(arrivals: &Arrivals, id: &str, now: f64) -> bool {
        arrivals.arrived(id, now).is_some()
    }

    #[test]
    fn does_not_animate_a_channels_first_render_or_a_channel_switch() {
        let mut arrivals = Arrivals::default();
        arrivals.observe("channel", "a", ["1", "2"], true, 0.0);
        assert!(!has(&arrivals, "2", 0.0));
        arrivals.observe("channel", "b", ["3", "4"], true, 0.0);
        assert!(!has(&arrivals, "4", 0.0));
    }

    #[test]
    fn animates_rows_appended_at_the_live_end() {
        let mut arrivals = Arrivals::default();
        arrivals.observe("channel", "a", ["1", "2"], true, 0.0);
        arrivals.observe("channel", "a", ["1", "2", "3"], true, 0.0);
        assert!(has(&arrivals, "3", 0.0));
        assert!(!has(&arrivals, "2", 0.0));
    }

    #[test]
    fn does_not_animate_an_empty_lists_first_messages() {
        let mut arrivals = Arrivals::default();
        arrivals.observe("channel", "a", [], true, 0.0);
        arrivals.observe("channel", "a", ["1", "2"], true, 0.0);
        assert!(!has(&arrivals, "1", 0.0));
    }

    #[test]
    fn ignores_older_pages_and_newer_pages_loaded_towards_the_live_end() {
        let mut arrivals = Arrivals::default();
        arrivals.observe("channel", "a", ["5", "6"], false, 0.0);
        arrivals.observe("channel", "a", ["3", "4", "5", "6"], false, 0.0);
        arrivals.observe("channel", "a", ["3", "4", "5", "6", "7", "8"], true, 0.0);
        assert!(
            !["3", "4", "7", "8"]
                .iter()
                .any(|id| has(&arrivals, id, 0.0))
        );
        arrivals.observe(
            "channel",
            "a",
            ["1", "2", "3", "4", "5", "6", "7", "8"],
            true,
            0.0,
        );
        assert!(!has(&arrivals, "1", 0.0));
    }

    #[test]
    fn animates_a_confirmed_send_once_keeping_its_client_message_id() {
        let mut arrivals = Arrivals::default();
        arrivals.observe("channel", "a", ["1"], true, 0.0);
        arrivals.observe("channel", "a", ["1", "sent"], true, 0.0);
        assert!(has(&arrivals, "sent", 0.0));
        let later = ARRIVAL + 0.001;
        arrivals.observe("channel", "a", ["1", "sent"], true, later);
        assert!(!has(&arrivals, "sent", later));
    }

    #[test]
    fn keeps_channel_and_thread_lists_apart() {
        let mut arrivals = Arrivals::default();
        arrivals.observe("channel", "a", ["1"], true, 0.0);
        arrivals.observe("thread", "root", ["r1"], true, 0.0);
        arrivals.observe("thread", "root", ["r1", "r2"], true, 0.0);
        arrivals.observe("channel", "a", ["1"], true, 0.0);
        assert!(has(&arrivals, "r2", 0.0));
    }
}
