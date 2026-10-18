use crate::egui::{self, emath::Numeric};
use std::ops::RangeInclusive;

/// Native slider semantics and fine keyboard/accessibility steps, plus a
/// pointer-only magnetic home position. Defaults are supplied by each caller.
pub fn default_slider<T: Numeric>(
    ui: &mut egui::Ui,
    value: &mut T,
    range: RangeInclusive<T>,
    default: T,
    percent: bool,
) -> egui::Response {
    let (min, max, home) = (
        range.start().to_f64(),
        range.end().to_f64(),
        default.to_f64(),
    );
    let width = ui.spacing().slider_width;
    let thickness = ui
        .text_style_height(&egui::TextStyle::Body)
        .max(ui.spacing().interact_size.y);
    let radius = thickness / 2.5
        * match ui.visuals().handle_shape {
            egui::style::HandleShape::Circle => 1.0,
            egui::style::HandleShape::Rect { aspect_ratio } => aspect_ratio,
        };
    let pointer_down = ui.input(|input| input.pointer.primary_down());
    let mut slider = egui::Slider::from_get_set(min..=max, |next| {
        if let Some(next) = next
            // egui writes the old value on every render to clamp it. Do not
            // magnetize those writes when a pointer is down on another widget.
            && next != value.to_f64()
        {
            *value = T::from_f64(
                if pointer_down && (next - home).abs() <= (max - min) * 0.02 {
                    home
                } else {
                    next.round()
                },
            );
        }
        value.to_f64()
    })
    .smart_aim(false)
    .show_value(percent)
    .suffix(if percent { "%" } else { "" })
    .trailing_fill(true);
    // Snap before integer rounding, so even small ranges get the full zone.
    // Keyboard/accessibility still have an explicit one-unit native step.
    if !pointer_down {
        slider = slider.step_by(1.0);
    }
    let response = ui.add(slider);
    // The optional value editor sits after the track. Do not include it in the
    // notch's coordinate range; egui reserves half a handle at either end.
    let x = response.rect.left()
        + radius
        + (width - 2.0 * radius) * ((home - min) / (max - min)) as f32;
    let y = response.rect.center().y + 6.0;
    ui.painter().line_segment(
        [egui::pos2(x, y), egui::pos2(x, y + 4.0)],
        egui::Stroke::new(2.0, ui.visuals().weak_text_color()),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        value: &mut f64,
        range: RangeInclusive<f64>,
        home: f64,
        events: Vec<egui::Event>,
    ) -> egui::Response {
        let mut response = None;
        let _ = ctx.run(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.spacing_mut().slider_width = 300.0;
                    response = Some(default_slider(ui, value, range.clone(), home, false));
                });
            },
        );
        response.unwrap()
    }

    #[test]
    fn pointer_magnet_and_keyboard_fine_steps_for_each_default() {
        for (min, max, home) in [
            (0.0, 200.0, 100.0),
            (0.0, 100.0, 25.0),
            (2.0, 40.0, 6.0),
            (14.0, 96.0, 32.0),
        ] {
            let ctx = egui::Context::default();
            let mut value = home - 1.0;
            let response = frame(&ctx, &mut value, min..=max, home, vec![]);
            assert_eq!(
                value,
                home - 1.0,
                "rendering must not normalize saved values"
            );
            let outside = egui::pos2(0.0, 0.0);
            frame(
                &ctx,
                &mut value,
                min..=max,
                home,
                vec![egui::Event::PointerButton {
                    pos: outside,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert_eq!(
                value,
                home - 1.0,
                "an unrelated pointer press must not snap"
            );
            frame(
                &ctx,
                &mut value,
                min..=max,
                home,
                vec![egui::Event::PointerButton {
                    pos: outside,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            let radius = response.rect.height() / 2.5
                * match ctx.style().visuals.handle_shape {
                    egui::style::HandleShape::Circle => 1.0,
                    egui::style::HandleShape::Rect { aspect_ratio } => aspect_ratio,
                };
            let position = |next: f64| {
                egui::pos2(
                    response.rect.left()
                        + radius
                        + (300.0 - radius * 2.0) * ((next - min) / (max - min)) as f32,
                    response.rect.center().y,
                )
            };
            let near = position(home + (max - min) * 0.01);
            frame(
                &ctx,
                &mut value,
                min..=max,
                home,
                vec![
                    egui::Event::PointerMoved(near),
                    egui::Event::PointerButton {
                        pos: near,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            assert_eq!(value, home, "pointer near explicit default should snap");
            let far = position(home + (max - min) * 0.08);
            frame(
                &ctx,
                &mut value,
                min..=max,
                home,
                vec![egui::Event::PointerMoved(far)],
            );
            assert!(value > home + (max - min) * 0.02, "drag must escape magnet");
            frame(
                &ctx,
                &mut value,
                min..=max,
                home,
                vec![egui::Event::PointerButton {
                    pos: far,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            value = home;
            ctx.memory_mut(|memory| memory.request_focus(response.id));
            frame(
                &ctx,
                &mut value,
                min..=max,
                home,
                vec![egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            assert_eq!(
                value,
                home + 1.0,
                "keyboard must escape default in one fine step"
            );
        }
    }
}
