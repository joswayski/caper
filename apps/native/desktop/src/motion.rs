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

/// `--motion-fast`, `--motion-base` and `--motion-slow` in seconds.
pub const FAST: f32 = 0.14;
pub const BASE: f32 = 0.2;
pub const SLOW: f32 = 0.26;

/// `--ease-out`, `cubic-bezier(0.16, 1, 0.3, 1)`: quick to start, long to settle.
pub fn ease_out(t: f32) -> f32 {
    egui::emath::easing::exponential_out(t.clamp(0.0, 1.0))
}

fn enabled_id() -> Id {
    Id::new("caper-motion")
}

/// Headless tests draw final states unless they turn motion on.
fn enabled(context: &Context) -> bool {
    context
        .data(|data| data.get_temp(enabled_id()))
        .unwrap_or(!cfg!(test))
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

fn settle_range(
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

/// How long a row counts as arriving: long enough for its entrance, short
/// enough that a row drawn again later never replays it.
pub const ARRIVAL: f64 = 1.0;

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
    use super::{ARRIVAL, Arrivals, BASE, enable, entrance};
    use eframe::egui;

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
