//! Original Caper SVG layers, with a short greeting followed by a long rest.
//! Keep the choreography equivalent to Apple's UpdateMascot in UpdateViews.swift.

use eframe::egui;

fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Travel across the profile, hide depth, waving arm angle and walking bounce.
fn pose(elapsed: f32) -> (f32, f32, f32, f32) {
    let t = elapsed % 24.0;
    let travel = if t < 2.8 {
        0.0
    } else if t < 6.0 {
        ease((t - 2.8) / 3.2)
    } else if t < 7.6 {
        1.0
    } else {
        1.0 - ease((t - 7.6) / 2.0)
    };
    let dip = if t < 0.7 {
        56.0 * (1.0 - ease(t / 0.7))
    } else {
        56.0 * ease((t - 9.6) / 0.8)
    };
    let waving = (0.7..2.8).contains(&t) || (6.0..7.6).contains(&t);
    let walking = (2.8..6.0).contains(&t) || (7.6..9.6).contains(&t);
    (
        travel,
        dip,
        if waving { (t * 10.0).sin() * 0.28 } else { 0.0 },
        if walking {
            (t * 14.0).sin().abs() * -2.0
        } else {
            0.0
        },
    )
}

pub fn paint(ui: &egui::Ui, rect: egui::Rect, wave: f32, step: f32) {
    egui::Image::new(egui::include_image!(
        "../../../../assets/update-caper/feet.svg"
    ))
    .rotate(step * 0.06, egui::vec2(0.5, 0.84))
    .paint_at(ui, rect);
    egui::Image::new(egui::include_image!(
        "../../../../assets/update-caper/wave.svg"
    ))
    .rotate(wave, egui::vec2(0.8, 0.66))
    .paint_at(ui, rect);
    egui::Image::new(egui::include_image!(
        "../../../../assets/update-caper/body.svg"
    ))
    .paint_at(ui, rect);
}

/// A stable keyboard/click target even while Caper is hiding. The tiny dot
/// remains discoverable; audio/profile controls below are never covered.
pub fn dock(ui: &mut egui::Ui, version: &str, reviewing: bool) -> egui::Response {
    let now = ui.input(|input| input.time);
    let active = !reviewing && ui.is_enabled() && crate::looping(ui);
    let start_id = ui.id().with(("update-mascot-start", version));
    let elapsed = ui.ctx().data_mut(|data| {
        if active {
            (now - *data.get_temp_mut_or_insert_with(start_id, || now)) as f32
        } else {
            data.remove::<f64>(start_id);
            0.0
        }
    });
    // Keep the profile and channel list stationary throughout the greeting/rest.
    let height = 56.0;
    let (slot, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let profile_width = (slot.width() - 122.0).max(48.0);
    let target = egui::Rect::from_min_size(slot.min, egui::vec2(profile_width, height));
    let response = ui.interact(target, ui.id().with("update-mascot"), egui::Sense::click());
    response.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::WidgetType::Button,
            ui.is_enabled(),
            "Review Caper update",
        )
    });
    let (travel, mut dip, wave, step) = if active {
        pose(elapsed)
    } else {
        (0.0, 22.0, 0.0, 0.0)
    };
    if response.hovered() || response.has_focus() {
        dip = 0.0;
    }
    let x = slot.left() + 28.0 + travel * (profile_width - 60.0).clamp(0.0, 80.0);
    let rect = egui::Rect::from_min_size(
        egui::pos2(x - 28.0, slot.bottom() - 60.0 + dip + step),
        egui::vec2(60.0, 60.0),
    );
    let mut painter = ui.painter().clone();
    painter.set_clip_rect(slot.intersect(ui.clip_rect()));
    let mut art_ui = ui.new_child(egui::UiBuilder::new().max_rect(slot));
    art_ui.set_clip_rect(slot.intersect(ui.clip_rect()));
    paint(&art_ui, rect, wave, step);
    painter.circle_filled(
        egui::pos2(slot.left() + 14.0, slot.bottom() - 3.0),
        2.5,
        crate::TERRACOTTA_BRIGHT,
    );
    if response.has_focus() {
        painter.rect_stroke(
            target.shrink(1.0),
            8.0,
            egui::Stroke::new(1.0, crate::TERRACOTTA),
            egui::StrokeKind::Inside,
        );
    }
    if active {
        let rest = elapsed % 24.0;
        ui.ctx().request_repaint_after(if rest >= 10.4 {
            std::time::Duration::from_secs_f32(24.0 - rest)
        } else {
            std::time::Duration::from_millis(33)
        });
    }
    response.on_hover_text(format!(
        "Caper {version} is ready. Click to see what’s new."
    ))
}
