//! The update notice and decorative SVG layers used in its review.

use eframe::egui;

/// Match UpdateButtonShine on macOS: a still prismatic edge and one short
/// light sweep every eight seconds. Never animate the label or hit target.
fn shine(ui: &egui::Ui, response: &egui::Response) {
    if !ui.is_enabled() {
        return;
    }
    let rect = response.rect;
    let colors = [
        [167.0, 128.0, 242.0],
        [104.0, 175.0, 236.0],
        [99.0, 210.0, 193.0],
        [215.0, 205.0, 130.0],
        [229.0, 139.0, 183.0],
    ];
    let mut corners = Vec::new();
    egui::epaint::tessellator::path::rounded_rectangle(
        &mut corners,
        rect,
        egui::epaint::CornerRadiusF32::same(8.0),
    );
    // The stroke colors interpolate between vertices, so sample the long
    // straight edges as well as the corners to retain every rainbow stop.
    let mut outline = Vec::new();
    for (&a, &b) in corners.iter().zip(corners.iter().cycle().skip(1)) {
        let steps = (a.distance(b) / 8.0).ceil().max(1.0) as usize;
        for step in 0..steps {
            outline.push(a.lerp(b, step as f32 / steps as f32));
        }
    }
    // Preserve the button's own keyboard focus indication.
    if !response.has_focus() {
        for (width, alpha) in [(4.0, 31), (1.0, 191)] {
            let stroke = egui::epaint::PathStroke::new_uv(width, move |_, point| {
                let t = ((point.x - rect.left()) / rect.width()).clamp(0.0, 1.0) * 4.0;
                let index = (t as usize).min(3);
                let rgb = std::array::from_fn::<_, 3, _>(|channel| {
                    egui::lerp(
                        colors[index][channel]..=colors[index + 1][channel],
                        t - index as f32,
                    )
                    .round() as u8
                });
                egui::Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], alpha)
            })
            .inside();
            ui.painter()
                .add(egui::Shape::closed_line(outline.clone(), stroke));
        }
    }
    if !crate::looping(ui) {
        return;
    }
    let duration = f64::from(crate::motion::SLOW) * 6.0;
    let time = ui.input(|input| input.time);
    let start = ui.ctx().data_mut(|data| {
        *data.get_temp_mut_or_insert_with(response.id.with("shine-start"), || time)
    });
    let phase = (time - start).rem_euclid(8.0);
    if phase >= duration {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(8.0 - phase));
        return;
    }
    let progress = crate::motion::ease_out((phase / duration) as f32);
    let center = rect.left() - 52.0 + (rect.width() + 160.0) * progress;
    let mut mesh = egui::Mesh::default();
    // Clip the band to rounded corners in its geometry, not the rectangular
    // painter clip, so no light leaks outside the button as it enters/leaves.
    let height = rect.height();
    for y in [
        0.0,
        2.0,
        4.0,
        8.0,
        height - 8.0,
        height - 4.0,
        height - 2.0,
        height,
    ] {
        let corner = (8.0 - y.min(height - y)).max(0.0);
        let inset = 8.0 - (64.0 - corner * corner).sqrt();
        let left = rect.left() + inset;
        let right = rect.right() - inset;
        let x = center - (y - height / 2.0) * 18.0_f32.to_radians().tan();
        for column in [
            left,
            (x - 28.0).clamp(left, right),
            x.clamp(left, right),
            (x + 28.0).clamp(left, right),
            right,
        ] {
            let alpha = (1.0 - (column - x).abs() / 28.0).max(0.0) * 41.0;
            mesh.colored_vertex(
                egui::pos2(column, rect.top() + y),
                egui::Color32::from_white_alpha(alpha.round() as u8),
            );
        }
    }
    for row in 0..7 {
        for column in 0..4 {
            let i = row * 5 + column;
            mesh.add_triangle(i, i + 1, i + 5);
            mesh.add_triangle(i + 1, i + 6, i + 5);
        }
    }
    ui.painter().add(mesh);
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(33));
}

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
    shine(ui, &response);
    ui.add_space(8.0);
    response.on_hover_text(format!(
        "Caper {version} is available. Review the update before restarting."
    ))
}
