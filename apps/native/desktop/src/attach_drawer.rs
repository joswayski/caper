//! The composer's attach menu, as on web (`chat/AttachmentPicker.tsx`) and
//! Discord: one unfiltered "Upload a file" (and Paste for profile messages).
//! It still opens at the attachment limit, where it says so and the choices
//! nudge that line instead of opening a dialog that couldn't add anything.

use crate::{MUTED, TEXT, motion, uploads};
use eframe::egui::{self, RichText};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachChoice {
    /// The system dialog for any file.
    Upload,
    /// Files copied to the clipboard (profile messages).
    Paste,
}

/// How to make room once full.
pub fn hint(attached: usize) -> Option<&'static str> {
    (attached >= uploads::MAX_ATTACHMENTS).then_some("Remove a file to add another.")
}

/// A short sideways shake over `motion::SLOW` seconds after a refusal.
fn nudge_offset(elapsed: f32) -> f32 {
    if !(0.0..motion::SLOW).contains(&elapsed) {
        return 0.0;
    }
    4.0 * (elapsed / motion::SLOW * 4.0 * std::f32::consts::PI).sin()
}

/// Shows the panel for `button` (opened by clicking it) and returns the
/// chosen action. `paste` adds "Paste from clipboard".
pub fn popup(button: &egui::Response, attached: usize, paste: bool) -> Option<AttachChoice> {
    let full = attached >= uploads::MAX_ATTACHMENTS;
    let nudge_id = button.id.with("attach-nudge");
    let mut choice = None;
    egui::Popup::menu(button).width(200.0).show(|ui| {
        let now = ui.input(|input| input.time);
        let started = ui.data(|data| data.get_temp::<f64>(nudge_id));
        let elapsed = started.map_or(f32::MAX, |start| (now - start) as f32);
        let offset = if motion::reduced() {
            0.0
        } else {
            nudge_offset(elapsed)
        };
        if elapsed < motion::SLOW {
            ui.ctx().request_repaint();
        }
        if let Some(hint) = hint(attached) {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                // The row rests 4 points in, so the shake can move it either way.
                ui.add_space(4.0 + offset);
                ui.label(RichText::new(hint).size(12.0).color(MUTED));
            });
            ui.add_space(2.0);
        }
        ui.add_space(4.0);
        let row = |ui: &mut egui::Ui, image: egui::Image<'static>, text: &str| {
            ui.add(
                egui::Button::image_and_text(
                    image
                        .tint(if full { MUTED } else { TEXT })
                        .fit_to_exact_size(egui::vec2(16.0, 16.0)),
                    RichText::new(text).color(if full { MUTED } else { TEXT }),
                )
                .min_size(egui::vec2(184.0, 32.0)),
            )
        };
        let mut picked = None;
        if row(
            ui,
            egui::Image::new(egui::include_image!("../resources/icons/file-up.svg")),
            "Upload a file",
        )
        .clicked()
        {
            picked = Some(AttachChoice::Upload);
        }
        if paste
            && row(
                ui,
                egui::Image::new(egui::include_image!("../resources/icons/paperclip.svg")),
                "Paste from clipboard",
            )
            .clicked()
        {
            picked = Some(AttachChoice::Paste);
        }
        match picked {
            Some(_) if full => {
                ui.data_mut(|data| data.insert_temp(nudge_id, now));
                ui.ctx().request_repaint();
            }
            Some(picked) => {
                choice = Some(picked);
                ui.close();
            }
            None => {}
        }
    });
    choice
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_shows_only_at_the_limit_as_on_web() {
        assert_eq!(hint(0), None);
        assert_eq!(hint(9), None);
        assert_eq!(hint(10), Some("Remove a file to add another."));
        assert_eq!(hint(12), Some("Remove a file to add another."));
    }

    #[test]
    fn nudge_settles_back_in_place() {
        assert_eq!(nudge_offset(-1.0), 0.0);
        assert_eq!(nudge_offset(motion::SLOW), 0.0);
        assert!(nudge_offset(motion::SLOW / 8.0) > 3.9);
        assert!(nudge_offset(motion::SLOW * 3.0 / 8.0) < -3.9);
    }
}
