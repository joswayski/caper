//! Files in a composer: each prepares and uploads as soon as it is added
//! (`caper_client::worker::Command::Upload`), and only Send writes the
//! message. A composer's files belong to its channel and, for a thread's
//! composer, to that thread.

use crate::images::{Icon, Images};
use caper_client::Rgba;
use caper_client::attachment_edit::Kind;
use caper_client::drafts::{Progress, Upload};
use caper_client::model::AttachmentKind;
use caper_client::uploads::{Scratch, format_bytes};
use gtk::{glib, pango, prelude::*};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// The thumbnail a chip shows.
const CHIP_EDGE: i32 = 48;
/// The busy ring over the thumbnail: its diameter and stroke.
const RING: i32 = 28;
const RING_STROKE: f64 = 3.0;

/// Where a busy ring without a fraction has turned to, once a second;
/// still with reduced motion.
fn spin() -> f64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    if !crate::images::motion() {
        return 0.0;
    }
    let ms = START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis()
        % 1000;
    ms as f64 / 1000.0
}

/// A busy file's ring: a white track at 25% and the fraction clockwise
/// from the top, or a quarter turning while there is none. Decoration: the
/// chip's status line says the same.
fn ring(progress: Progress) -> gtk::DrawingArea {
    let ring = gtk::DrawingArea::new();
    ring.set_content_width(RING);
    ring.set_content_height(RING);
    ring.set_can_target(false);
    ring.set_halign(gtk::Align::Center);
    ring.set_valign(gtk::Align::Center);
    ring.set_draw_func(move |_, cr, width, height| {
        use std::f64::consts::{FRAC_PI_2, TAU};
        let (cx, cy) = (f64::from(width) / 2.0, f64::from(height) / 2.0);
        let r = (f64::from(RING) - RING_STROKE) / 2.0;
        cr.set_line_width(RING_STROKE);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.25);
        cr.arc(cx, cy, r, 0.0, TAU);
        let _ = cr.stroke();
        let (from, sweep) = match progress {
            Progress::Known(fraction) => (0.0, f64::from(fraction.clamp(0.0, 1.0))),
            Progress::Unknown => (spin(), 0.25),
        };
        if sweep > 0.0 {
            let start = from * TAU - FRAC_PI_2;
            cr.set_source_rgb(244.0 / 255.0, 244.0 / 255.0, 246.0 / 255.0);
            cr.arc(cx, cy, r, start, start + sweep * TAU);
            let _ = cr.stroke();
        }
    });
    if progress == Progress::Unknown && crate::images::motion() {
        ring.add_tick_callback(|ring, _| {
            ring.queue_draw();
            glib::ControlFlow::Continue
        });
    }
    ring
}

pub struct DraftFile {
    /// The upload's state, shared with the other clients.
    pub upload: Upload,
    /// The chip's picture: the editor's render until the prepared file's own.
    pub thumbnail: Option<Rgba>,
    /// Whether the attachment editor can open it.
    pub editable: Option<Kind>,
}

impl std::ops::Deref for DraftFile {
    type Target = Upload;
    fn deref(&self) -> &Upload {
        &self.upload
    }
}

impl std::ops::DerefMut for DraftFile {
    fn deref_mut(&mut self) -> &mut Upload {
        &mut self.upload
    }
}

impl caper_client::drafts::Staged for DraftFile {
    fn upload(&self) -> &Upload {
        &self.upload
    }
    fn upload_mut(&mut self) -> &mut Upload {
        &mut self.upload
    }
}

impl DraftFile {
    /// Another device's file from a synced draft: ready, its picture loads
    /// from its URL (`Command::DraftThumbnail`), and Edit downloads it.
    pub fn synced(
        key: u64,
        channel: String,
        root: Option<String>,
        attachment: caper_client::model::Attachment,
    ) -> Self {
        Self {
            editable: caper_client::drafts::synced_editable(&attachment),
            upload: Upload::synced(key, channel, root, attachment),
            thumbnail: None,
        }
    }

    pub fn new(
        key: u64,
        channel: String,
        root: Option<String>,
        path: PathBuf,
        scratch: Option<Arc<Scratch>>,
    ) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        let mut upload = Upload::picked(key, channel, root, name, Some(path.clone()));
        upload.source_key = scratch
            .is_none()
            .then(|| caper_client::drafts::source_key(&path))
            .flatten();
        upload.source_scratch = scratch;
        upload.source_size = std::fs::metadata(&path).map_or(0, |metadata| metadata.len());
        Self {
            upload,
            thumbnail: None,
            editable: caper_client::attachment_edit::editable(&path),
        }
    }

    /// A recording kept as a voice message: prepared already, and not yet
    /// uploading (`restart` starts it).
    pub fn recorded(
        key: u64,
        channel: String,
        root: Option<String>,
        prepared: Arc<caper_client::uploads::Prepared>,
    ) -> Self {
        Self {
            upload: Upload::recorded(key, channel, root, prepared),
            thumbnail: None,
            editable: None,
        }
    }

    /// Stops its upload; the worker cancels the reservation.
    pub fn stop(&self) {
        self.upload.abandon();
    }

    /// The chip's second line, and its progress while unfinished.
    pub fn status(&self) -> (String, Progress) {
        let (status, progress) = self.upload.status();
        if status.is_empty() {
            (
                caper_client::uploads::size_label(self.source_size, self.stored_size),
                progress,
            )
        } else {
            (status, progress)
        }
    }
}

/// Whether `paths` would all fit, and the error to show when some do not.
pub fn room(attached: usize, adding: usize) -> (usize, Option<String>) {
    let max = caper_client::uploads::MAX_ATTACHMENTS;
    let room = max.saturating_sub(attached);
    if room == 0 {
        return (0, Some(format!("You can attach up to {max} files.")));
    }
    let error = (adding > room).then(|| {
        format!(
            "Only {room} more file{} can be attached.",
            if room == 1 { "" } else { "s" }
        )
    });
    (room.min(adding), error)
}

/// What a chip's controls ask for.
pub enum ChipAction {
    /// Opens the file in the attachment editor.
    Edit(u64),
    Remove(u64),
    Retry(u64),
    Spoiler(u64, bool),
}

/// The chips for `files`, filling `container`.
pub fn chips(
    container: &gtk::Box,
    images: &Images,
    files: &[&DraftFile],
    handler: &Rc<dyn Fn(ChipAction)>,
) {
    // Rebuilt on every change: only files new to it arrive.
    let mut shown = std::collections::HashSet::new();
    while let Some(child) = container.first_child() {
        shown.insert(child.widget_name());
        container.remove(&child);
    }
    let place = |chip: &gtk::Widget, key: u64| {
        let name = format!("file-{key}");
        if !shown.contains(name.as_str()) {
            chip.add_css_class("arriving");
        }
        chip.set_widget_name(&name);
        container.append(chip);
    };
    for file in files {
        // A staged recording: a compact voice chip beside the files.
        if let Some(prepared) = file.voice() {
            let key = file.key;
            let uploading = file.attachment.is_none() && file.error.is_none();
            let (remove, retry) = (handler.clone(), handler.clone());
            let chip = crate::playback::chip(
                images,
                prepared,
                uploading.then_some(file.progress),
                file.error.as_deref(),
                move || remove(ChipAction::Remove(key)),
                move || retry(ChipAction::Retry(key)),
            );
            chip.set_valign(gtk::Align::Start);
            place(chip.upcast_ref(), key);
            continue;
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("chip");
        row.add_css_class("draft-chip");
        if file.error.is_some() {
            row.add_css_class("failed");
        }
        let preview = gtk::Box::new(gtk::Orientation::Vertical, 0);
        preview.add_css_class("chip-preview");
        preview.set_size_request(CHIP_EDGE, CHIP_EDGE);
        preview.set_homogeneous(true);
        preview.set_overflow(gtk::Overflow::Hidden);
        match file.thumbnail.as_ref().and_then(crate::media::texture) {
            Some(texture) => {
                // An image keeps its own size request; this one draws at the chip's.
                let image = gtk::Image::from_paintable(Some(&texture));
                image.set_pixel_size(CHIP_EDGE);
                preview.append(&image);
            }
            None => {
                let icon = images.icon_image(Icon::FileText, "#b9bcbe", 20);
                icon.set_halign(gtk::Align::Center);
                icon.set_valign(gtk::Align::Center);
                icon.set_vexpand(false);
                preview.append(&icon);
            }
        }
        if file.editable.is_some() {
            // The picture opens the editor, as on the desktop.
            preview.set_cursor_from_name(Some("pointer"));
            preview.set_tooltip_text(Some("Edit"));
            let click = gtk::GestureClick::new();
            let (handler, key) = (handler.clone(), file.key);
            click.connect_released(move |gesture, _, _, _| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                handler(ChipAction::Edit(key));
            });
            preview.add_controller(click);
        }
        // While it prepares or uploads, the picture dims under a centred ring.
        let picture = gtk::Overlay::new();
        picture.set_child(Some(&preview));
        picture.set_valign(gtk::Align::Center);
        let (status, progress) = file.status();
        if file.uploading {
            let scrim = gtk::Box::new(gtk::Orientation::Vertical, 0);
            scrim.add_css_class("tile-scrim");
            scrim.set_can_target(false);
            picture.add_overlay(&scrim);
            picture.add_overlay(&ring(progress));
        }
        row.append(&picture);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_valign(gtk::Align::Center);
        let name = crate::ui::label(&file.name, &["file-name"]);
        name.set_ellipsize(pango::EllipsizeMode::Middle);
        name.set_max_width_chars(18);
        text.append(&name);
        let status = crate::ui::label(
            &status,
            if file.error.is_some() {
                &["error", "small"]
            } else {
                &["muted", "small"]
            },
        );
        status.set_ellipsize(pango::EllipsizeMode::End);
        status.set_max_width_chars(22);
        text.append(&status);
        row.append(&text);
        let key = file.key;
        let button = |icon: Icon, name: &str, action: ChipAction| {
            let button: gtk::Button = crate::app::named("GtkButton", name);
            button.set_child(Some(&images.icon_image(icon, "#b9bcbe", 14)));
            button.set_tooltip_text(Some(name));
            button.add_css_class("flat");
            button.add_css_class("tool");
            button.set_valign(gtk::Align::Center);
            let (handler, action) = (handler.clone(), std::cell::Cell::new(Some(action)));
            button.connect_clicked(move |_| {
                if let Some(action) = action.take() {
                    handler(action);
                }
            });
            button
        };
        // Remove and Edit: a white glyph (and word) on a dark face.
        let face_button = |icon: Icon, word: Option<&str>, name: &str, action: ChipAction| {
            let button: gtk::Button = crate::app::named("GtkButton", name);
            button.set_tooltip_text(Some(name));
            let face = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            face.add_css_class("tile-face");
            face.append(&images.icon_image(icon, "#f4f4f6", 12));
            if let Some(word) = word {
                face.append(&crate::ui::label(word, &["tile-word"]));
            }
            button.set_child(Some(&face));
            let (handler, action) = (handler.clone(), std::cell::Cell::new(Some(action)));
            button.connect_clicked(move |_| {
                if let Some(action) = action.take() {
                    handler(action);
                }
            });
            button
        };
        if file.editable.is_some() {
            let edit = face_button(
                Icon::Pencil,
                Some("Edit"),
                &format!("Edit {}", file.name),
                ChipAction::Edit(key),
            );
            edit.add_css_class("tile-pill");
            edit.set_valign(gtk::Align::Center);
            row.append(&edit);
        }
        if file.error.is_some() {
            row.append(&button(Icon::Retry, "Retry upload", ChipAction::Retry(key)));
        }
        let media = matches!(file.kind, AttachmentKind::Image | AttachmentKind::Video);
        if media || file.spoiler {
            row.append(&button(
                if file.spoiler {
                    Icon::EyeOff
                } else {
                    Icon::Eye
                },
                if file.spoiler {
                    "Remove spoiler"
                } else {
                    "Mark as spoiler"
                },
                ChipAction::Spoiler(key, !file.spoiler),
            ));
        }
        row.set_tooltip_text(Some(&format!(
            "{} · {}",
            file.name,
            format_bytes(file.stored_size.unwrap_or(file.source_size))
        )));
        // Remove sits over the chip's top-right corner, usable while busy.
        let chip = gtk::Overlay::new();
        chip.set_child(Some(&row));
        chip.set_valign(gtk::Align::Start);
        let remove = face_button(
            Icon::Close,
            None,
            &format!("Remove {}", file.name),
            ChipAction::Remove(key),
        );
        remove.add_css_class("tile-remove");
        remove.set_halign(gtk::Align::End);
        remove.set_valign(gtk::Align::Start);
        chip.add_overlay(&remove);
        place(chip.upcast_ref(), file.key);
    }
    container.set_visible(!files.is_empty());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_composer_holds_at_most_ten_files() {
        assert_eq!(room(0, 3), (3, None));
        assert_eq!(
            room(8, 3),
            (2, Some("Only 2 more files can be attached.".into()))
        );
        assert_eq!(
            room(9, 2),
            (1, Some("Only 1 more file can be attached.".into()))
        );
        assert_eq!(
            room(10, 1),
            (0, Some("You can attach up to 10 files.".into()))
        );
    }
}
