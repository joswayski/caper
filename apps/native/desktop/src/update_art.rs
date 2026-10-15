//! The update notice and decorative SVG layers used in its review.

use eframe::egui;

pub fn paint(ui: &egui::Ui, rect: egui::Rect) {
    egui::Image::new(egui::include_image!(
        "../../../../assets/update-caper/feet.svg"
    ))
    .paint_at(ui, rect);
    egui::Image::new(egui::include_image!(
        "../../../../assets/update-caper/wave.svg"
    ))
    .paint_at(ui, rect);
    egui::Image::new(egui::include_image!(
        "../../../../assets/update-caper/body.svg"
    ))
    .paint_at(ui, rect);
}

/// A labelled, always-visible review action, separate from profile/audio controls.
pub fn dock(ui: &mut egui::Ui, version: &str) -> egui::Response {
    let response = crate::toned(
        ui,
        crate::SECONDARY_TONE,
        true,
        egui::Button::new(crate::bold("Update available").size(12.0))
            .min_size(egui::vec2(ui.available_width(), 36.0)),
    );
    ui.add_space(8.0);
    response.on_hover_text(format!(
        "Caper {version} is available. Review the update before restarting."
    ))
}
