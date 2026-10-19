//! The composer's attach panel, as on web (`chat/AttachmentPicker.tsx`): a
//! popup above Attach with the draft's capacity, Photos and videos, and
//! Files. It still opens at the attachment limit, where the choices nudge the
//! capacity line instead of opening a dialog that couldn't add anything.

use crate::{BORDER, MUTED, TERRACOTTA, TERRACOTTA_BRIGHT, TEXT, motion, uploads};
use eframe::egui::{self, RichText};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachChoice {
    /// The system dialog, filtered to photos and videos first.
    Media,
    /// The system dialog for any file.
    Files,
    /// Files copied to the clipboard (profile messages).
    Paste,
}

/// The capacity line, worded as on every client.
pub fn label(attached: usize) -> String {
    let max = uploads::MAX_ATTACHMENTS;
    let count = attached.min(max);
    if count == 0 {
        format!("Up to {max} files")
    } else {
        format!("{count} of {max} attached")
    }
}

/// Shown once full: how to make room.
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
    egui::Popup::menu(button).width(248.0).show(|ui| {
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
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("Attach").strong().size(13.0).color(TEXT));
        });
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            // The row rests 4 points in, so the shake can move it either way.
            ui.add_space(4.0 + offset);
            let (rect, _) = ui.allocate_exact_size(
                egui::vec2(uploads::MAX_ATTACHMENTS as f32 * 9.0 - 3.0, 10.0),
                egui::Sense::hover(),
            );
            for index in 0..uploads::MAX_ATTACHMENTS {
                let fill = if full {
                    TERRACOTTA
                } else if index < attached {
                    TEXT
                } else {
                    BORDER
                };
                ui.painter().circle_filled(
                    egui::pos2(rect.min.x + 3.0 + index as f32 * 9.0, rect.center().y),
                    3.0,
                    fill,
                );
            }
            ui.label(RichText::new(label(attached)).size(12.0).color(if full {
                TERRACOTTA_BRIGHT
            } else {
                MUTED
            }));
        });
        if let Some(hint) = hint(attached) {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.label(RichText::new(hint).size(12.0).color(MUTED));
            });
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
                .min_size(egui::vec2(232.0, 36.0)),
            )
        };
        let mut picked = None;
        if row(
            ui,
            egui::Image::new(egui::include_image!("../resources/icons/images.svg")),
            "Photos and videos",
        )
        .clicked()
        {
            picked = Some(AttachChoice::Media);
        }
        if row(
            ui,
            egui::Image::new(egui::include_image!("../resources/icons/file-text.svg")),
            "Files",
        )
        .clicked()
        {
            picked = Some(AttachChoice::Files);
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
    fn capacity_matches_web_wording() {
        assert_eq!(label(0), "Up to 10 files");
        assert_eq!(label(3), "3 of 10 attached");
        assert_eq!(label(12), "10 of 10 attached");
        assert_eq!(hint(9), None);
        assert_eq!(hint(10), Some("Remove a file to add another."));
    }

    #[test]
    fn nudge_settles_back_in_place() {
        assert_eq!(nudge_offset(-1.0), 0.0);
        assert_eq!(nudge_offset(motion::SLOW), 0.0);
        assert!(nudge_offset(motion::SLOW / 8.0) > 3.9);
        assert!(nudge_offset(motion::SLOW * 3.0 / 8.0) < -3.9);
    }
}
