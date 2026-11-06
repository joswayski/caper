//! A file staged in a composer: preparing, uploading, uploaded or failed.
//! Clients keep their own thumbnails and widgets beside it; the upload's
//! state, its status words and when it must go up again live here.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::compress::Compression;
use crate::model::{Attachment, AttachmentKind};
use crate::uploads::{Prepared, Scratch};
use crate::worker::{Command, UploadUpdate};

/// Uploads the server has not had sent with a message expire after a day;
/// an hour's margin uploads them again before that.
const FRESH: Duration = Duration::from_secs(23 * 60 * 60);

pub struct Upload {
    /// Matches upload events; saving an edit restarts the upload with a new one.
    pub key: u64,
    /// Stays the same for the chip while uploads restart.
    pub slot: u64,
    pub spoiler: bool,
    pub channel: String,
    /// None belongs to the conversation composer, Some to this thread only.
    pub thread_root_id: Option<String>,
    pub name: String,
    /// The picked file's name: an edited upload keeps it with its new extension.
    pub source_name: String,
    pub kind: AttachmentKind,
    /// The selected original, available for preview before upload completes.
    pub local_path: Option<PathBuf>,
    /// Applied to the original whenever it is prepared, including retries.
    pub edit: Option<crate::attachment_edit::Edit>,
    /// Keep a pasted image alive through preparation, failed uploads and retries.
    pub source_scratch: Option<Arc<Scratch>>,
    /// The picked file's size; 0 until the file was read.
    pub source_size: u64,
    /// What is stored after compression, once prepared.
    pub stored_size: Option<u64>,
    /// Video compression before upload, 0–1, while it runs.
    pub compressing: Option<f32>,
    pub progress: f32,
    pub error: Option<String>,
    pub attachment: Option<Attachment>,
    pub uploaded_at: Option<Instant>,
    /// The server no longer has this upload, so it must be uploaded again.
    /// A flag rather than an `uploaded_at` a day in the past: `Instant` counts
    /// from boot, so subtracting a day panics on a machine up less than a day.
    pub upload_expired: bool,
    pub uploading: bool,
    pub prepared: Option<Arc<Prepared>>,
    pub cancel: Arc<AtomicBool>,
}

/// What a staged file's progress ring shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Progress {
    Known(f32),
    /// Preparing or confirming: there is no fraction to show.
    Unknown,
}

impl Upload {
    /// A picked, dropped or pasted file whose upload starts now.
    pub fn picked(
        key: u64,
        channel: String,
        thread_root_id: Option<String>,
        name: String,
        local_path: Option<PathBuf>,
    ) -> Self {
        Self {
            key,
            slot: key,
            spoiler: false,
            channel,
            thread_root_id,
            source_name: name.clone(),
            name,
            kind: AttachmentKind::File,
            local_path,
            edit: None,
            source_scratch: None,
            source_size: 0,
            stored_size: None,
            compressing: None,
            progress: 0.0,
            error: None,
            attachment: None,
            uploaded_at: None,
            upload_expired: false,
            uploading: true,
            prepared: None,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A file already on the server, such as one a rejected message returns
    /// to the composer.
    pub fn uploaded(
        key: u64,
        channel: String,
        thread_root_id: Option<String>,
        attachment: Attachment,
    ) -> Self {
        let mut upload = Self::picked(key, channel, thread_root_id, attachment.name.clone(), None);
        upload.spoiler = attachment.spoiler;
        upload.kind = attachment.kind;
        upload.source_size = attachment.size;
        upload.stored_size = Some(attachment.size);
        upload.progress = 1.0;
        upload.uploading = false;
        upload.attachment = Some(attachment);
        upload
    }

    /// A recorded voice message staged in the composer.
    pub fn voice(&self) -> Option<&Arc<Prepared>> {
        self.prepared
            .as_ref()
            .filter(|prepared| prepared.voice.is_some())
    }

    /// The uploaded file as the message sends it, with the composer's
    /// spoiler choice.
    pub fn send_attachment(&self) -> Option<Attachment> {
        self.attachment.clone().map(|mut attachment| {
            attachment.spoiler = self.spoiler;
            attachment
        })
    }

    pub fn needs_upload(&self) -> bool {
        self.attachment.is_none()
            || self.upload_expired
            || self.uploaded_at.is_some_and(|at| at.elapsed() >= FRESH)
    }

    /// Gives up the upload: its worker stops at the next chance.
    pub fn abandon(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// The command that prepares and uploads this file. Prepared bytes
    /// already carry the edit; without them it is applied again.
    pub fn command(&self, token: String, settings: Compression) -> Command {
        Command::Upload {
            key: self.key,
            token,
            channel: self.channel.clone(),
            path: self.local_path.clone().unwrap_or_default(),
            name: self.source_name.clone(),
            edit: self.edit.clone(),
            source_scratch: self.source_scratch.clone(),
            prepared: self.prepared.clone(),
            settings,
            cancel: self.cancel.clone(),
        }
    }

    /// Starts this file again, returning the upload it replaces, which the
    /// caller cancels on the server. None when it is uploading or still fresh.
    pub fn restart(&mut self) -> Option<Option<Attachment>> {
        if self.uploading || !self.needs_upload() {
            return None;
        }
        self.error = None;
        self.progress = 0.0;
        self.uploading = true;
        Some(self.attachment.take())
    }

    /// Applies a worker update, returning the sender's own decoded copy
    /// when one arrives.
    pub fn apply(&mut self, update: UploadUpdate) -> Option<crate::Rgba> {
        match update {
            UploadUpdate::Retained(prepared) => self.prepared = Some(prepared),
            UploadUpdate::Prepared {
                name,
                kind,
                source_size,
                stored_size,
            } => {
                self.name = name;
                self.kind = kind;
                self.source_size = source_size;
                self.stored_size = Some(stored_size);
                self.compressing = None;
            }
            UploadUpdate::Compressing(fraction) => {
                self.compressing = Some(fraction.max(self.compressing.unwrap_or(0.0)));
            }
            UploadUpdate::Thumbnail(image) => return Some(image),
            UploadUpdate::Progress(progress) => self.progress = progress.max(self.progress),
            UploadUpdate::Finished(Ok(attachment)) => {
                self.progress = 1.0;
                self.uploaded_at = Some(Instant::now());
                self.upload_expired = false;
                self.uploading = false;
                self.error = None;
                self.attachment = Some(attachment);
            }
            UploadUpdate::Finished(Err(error)) => {
                self.uploading = false;
                // An edit that could not be applied says so; nothing was sent.
                self.error = Some(if error.message == crate::attachment_edit::FAILED {
                    error.message
                } else {
                    "Failed to upload.".into()
                });
            }
        }
        None
    }

    /// The status for the file's label and hover text, and the progress its
    /// Remove ring shows while unfinished: packing first when that reports a
    /// fraction, then sending.
    pub fn status(&self) -> (String, Progress) {
        if let Some(error) = &self.error {
            (error.clone(), Progress::Unknown)
        } else if self.attachment.is_some() {
            (String::new(), Progress::Known(1.0))
        } else if let Some(fraction) = self.compressing.filter(|_| self.stored_size.is_none()) {
            (
                format!("Packing {}%", (fraction * 100.0).round() as u32),
                Progress::Known(fraction),
            )
        } else if self.stored_size.is_none() {
            ("Preparing…".into(), Progress::Unknown)
        } else if self.progress >= 1.0 {
            ("Confirming…".into(), Progress::Unknown)
        } else {
            (
                format!("Sending {}%", (self.progress * 100.0).round() as u32),
                Progress::Known(self.progress),
            )
        }
    }
}

/// The line above a composer's files when some failed, or None.
pub fn failed_notice(failed: usize) -> Option<String> {
    match failed {
        0 => None,
        1 => Some("1 file failed to upload. Send will retry it, or you can remove it.".into()),
        n => Some(format!(
            "{n} files failed to upload. Send will retry them, or you can remove them."
        )),
    }
}

/// The error when more files were chosen than the composer has `room` for.
pub fn room_error(room: usize) -> String {
    if room == 0 {
        return format!(
            "You can attach up to {} files.",
            crate::uploads::MAX_ATTACHMENTS
        );
    }
    format!(
        "Only {room} more file{} can be attached.",
        if room == 1 { "" } else { "s" }
    )
}

/// The short type label a file without a preview shows: its extension in
/// capitals when it looks like one, else "File".
pub fn kind_label(name: &str) -> String {
    std::path::Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .filter(|ext| {
            !ext.is_empty() && ext.len() <= 5 && ext.chars().all(|ch| ch.is_ascii_alphanumeric())
        })
        .map_or_else(|| "File".to_owned(), str::to_ascii_uppercase)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uploads::UploadError;

    fn attachment(id: &str) -> Attachment {
        Attachment {
            spoiler: false,
            id: id.into(),
            kind: AttachmentKind::Image,
            content_type: "image/png".into(),
            name: "photo.png".into(),
            size: 2048,
            width: None,
            height: None,
            duration_ms: None,
            preview: false,
            url: None,
            preview_url: None,
            unavailable: false,
            status: crate::model::AttachmentStatus::Ready,
            animated: false,
            voice: false,
            waveform: None,
            transcript: None,
        }
    }

    #[test]
    fn status_walks_packing_preparing_sending_confirming_and_failure() {
        let mut file = Upload::picked(1, "c".into(), None, "clip.mp4".into(), None);
        assert_eq!(file.status(), ("Preparing…".into(), Progress::Unknown));
        file.apply(UploadUpdate::Compressing(0.25));
        // Packing never goes backwards.
        file.apply(UploadUpdate::Compressing(0.1));
        assert_eq!(file.status(), ("Packing 25%".into(), Progress::Known(0.25)));
        file.apply(UploadUpdate::Prepared {
            name: "clip.mp4".into(),
            kind: AttachmentKind::Video,
            source_size: 9000,
            stored_size: 4000,
        });
        assert_eq!(file.kind, AttachmentKind::Video);
        file.apply(UploadUpdate::Progress(0.6));
        file.apply(UploadUpdate::Progress(0.4));
        assert_eq!(file.status(), ("Sending 60%".into(), Progress::Known(0.6)));
        file.apply(UploadUpdate::Progress(1.0));
        assert_eq!(file.status(), ("Confirming…".into(), Progress::Unknown));
        file.apply(UploadUpdate::Finished(Err(UploadError::new("HTTP 500"))));
        assert_eq!(file.status().0, "Failed to upload.");
        assert!(!file.uploading);
        file.apply(UploadUpdate::Finished(Err(UploadError::new(
            crate::attachment_edit::FAILED,
        ))));
        assert_eq!(file.status().0, crate::attachment_edit::FAILED);
    }

    #[test]
    fn restart_only_retries_failed_or_expired_files_and_hands_back_the_old_upload() {
        let mut file = Upload::picked(1, "c".into(), None, "photo.png".into(), None);
        // Still uploading: nothing to restart.
        assert!(file.restart().is_none());
        file.apply(UploadUpdate::Finished(Ok(attachment("a1"))));
        assert!(!file.needs_upload());
        assert!(file.restart().is_none());
        file.upload_expired = true;
        assert_eq!(
            file.restart().map(|old| old.map(|a| a.id)),
            Some(Some("a1".into()))
        );
        assert!(file.uploading && file.attachment.is_none() && file.progress == 0.0);
        file.apply(UploadUpdate::Finished(Ok(attachment("a2"))));
        assert!(!file.upload_expired && !file.needs_upload());
    }

    #[test]
    fn sent_attachments_carry_the_composer_spoiler_choice() {
        let mut file = Upload::uploaded(3, "c".into(), Some("root".into()), attachment("a"));
        assert!(!file.uploading && !file.needs_upload());
        assert_eq!(file.status(), (String::new(), Progress::Known(1.0)));
        file.spoiler = true;
        assert!(file.send_attachment().is_some_and(|a| a.spoiler));
    }

    #[test]
    fn notices_and_labels() {
        assert_eq!(failed_notice(0), None);
        assert!(failed_notice(1).is_some_and(|text| text.starts_with("1 file failed")));
        assert!(failed_notice(3).is_some_and(|text| text.starts_with("3 files failed")));
        assert_eq!(room_error(0), "You can attach up to 10 files.");
        assert_eq!(room_error(1), "Only 1 more file can be attached.");
        assert_eq!(room_error(4), "Only 4 more files can be attached.");
        assert_eq!(kind_label("notes.pdf"), "PDF");
        assert_eq!(kind_label("archive.tar.gz"), "GZ");
        assert_eq!(kind_label("README"), "File");
        assert_eq!(kind_label("weird.ext-ension"), "File");
    }
}
