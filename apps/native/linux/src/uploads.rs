//! Files in a composer: each prepares and uploads as soon as it is added
//! (`caper_client::worker::Command::Upload`), and only Send writes the
//! message. A composer's files belong to its channel and, for a thread's
//! composer, to that thread.

use crate::images::{Icon, Images};
use caper_client::Rgba;
use caper_client::model::{Attachment, AttachmentKind};
use caper_client::uploads::{Prepared, Scratch, format_bytes};
use gtk::{pango, prelude::*};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// The thumbnail a chip shows.
const CHIP_EDGE: i32 = 48;

pub struct DraftFile {
    /// Matches upload events; a retry keeps it.
    pub key: u64,
    pub channel: String,
    /// `None`: the conversation composer; `Some`: that thread's.
    pub root: Option<String>,
    pub name: String,
    pub kind: AttachmentKind,
    pub path: PathBuf,
    /// A pasted image's file, kept while it uploads and for retries.
    pub scratch: Option<Arc<Scratch>>,
    pub source_size: u64,
    pub stored_size: Option<u64>,
    pub compressing: Option<f32>,
    pub progress: f32,
    pub uploading: bool,
    pub error: Option<String>,
    pub attachment: Option<Attachment>,
    pub prepared: Option<Arc<Prepared>>,
    pub thumbnail: Option<Rgba>,
    pub spoiler: bool,
    pub cancel: Arc<AtomicBool>,
}

impl DraftFile {
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
        Self {
            key,
            channel,
            root,
            source_size: std::fs::metadata(&path).map_or(0, |metadata| metadata.len()),
            name,
            kind: AttachmentKind::File,
            path,
            scratch,
            stored_size: None,
            compressing: None,
            progress: 0.0,
            uploading: true,
            error: None,
            attachment: None,
            prepared: None,
            thumbnail: None,
            spoiler: false,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// What it sends: the uploaded file, marked as a spoiler when chosen.
    pub fn send_attachment(&self) -> Option<Attachment> {
        self.attachment.clone().map(|mut attachment| {
            attachment.spoiler = self.spoiler;
            attachment
        })
    }

    /// Stops its upload; the worker cancels the reservation.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The chip's second line.
    pub fn status(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        if let Some(fraction) = self.compressing {
            return format!("Compressing {}%", (fraction * 100.0).round() as u32);
        }
        if self.uploading {
            return format!("Uploading {}%", (self.progress * 100.0).round() as u32);
        }
        caper_client::uploads::size_label(self.source_size, self.stored_size)
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
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    for file in files {
        let chip = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        chip.add_css_class("chip");
        chip.set_valign(gtk::Align::Start);
        if file.error.is_some() {
            chip.add_css_class("failed");
        }
        let preview = gtk::Box::new(gtk::Orientation::Vertical, 0);
        preview.add_css_class("chip-preview");
        preview.set_size_request(CHIP_EDGE, CHIP_EDGE);
        preview.set_homogeneous(true);
        preview.set_overflow(gtk::Overflow::Hidden);
        preview.set_valign(gtk::Align::Center);
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
        chip.append(&preview);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_valign(gtk::Align::Center);
        let name = crate::ui::label(&file.name, &["file-name"]);
        name.set_ellipsize(pango::EllipsizeMode::Middle);
        name.set_max_width_chars(18);
        text.append(&name);
        let status = crate::ui::label(
            &file.status(),
            if file.error.is_some() {
                &["error", "small"]
            } else {
                &["muted", "small"]
            },
        );
        status.set_ellipsize(pango::EllipsizeMode::End);
        status.set_max_width_chars(22);
        text.append(&status);
        if file.uploading {
            let bar = gtk::ProgressBar::new();
            bar.set_fraction(f64::from(file.compressing.unwrap_or(file.progress)));
            bar.add_css_class("upload-progress");
            text.append(&bar);
        }
        chip.append(&text);
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
        if file.error.is_some() {
            chip.append(&button(Icon::Retry, "Retry upload", ChipAction::Retry(key)));
        }
        let media = matches!(file.kind, AttachmentKind::Image | AttachmentKind::Video);
        if media || file.spoiler {
            chip.append(&button(
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
        chip.append(&button(
            Icon::Close,
            &format!("Remove {}", file.name),
            ChipAction::Remove(key),
        ));
        chip.set_tooltip_text(Some(&format!(
            "{} · {}",
            file.name,
            format_bytes(file.stored_size.unwrap_or(file.source_size))
        )));
        container.append(&chip);
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
