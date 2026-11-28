//! A file staged in a composer: preparing, uploading, uploaded or failed.
//! Clients keep their own thumbnails and widgets beside it; the upload's
//! state, its status words and when it must go up again live here.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crate::compress::Compression;
use crate::model::{Attachment, AttachmentKind};
use crate::uploads::{Prepared, Scratch};
use crate::worker::{Command, UploadUpdate};

/// Uploads the server has not had sent with a message expire after a day;
/// an hour's margin uploads them again before that.
const FRESH: Duration = Duration::from_secs(23 * 60 * 60);

/// How long a removed file waits, still uploading or uploaded, for the
/// same file to be added to the same composer again.
pub const REMOVAL_GRACE: Duration = Duration::from_secs(30);

/// What recognises the same picked file added again: its canonical path,
/// size and modification time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceKey {
    path: PathBuf,
    size: u64,
    modified: Option<SystemTime>,
}

/// The file at `path` as a `SourceKey`, or None when it cannot be read.
pub fn source_key(path: &Path) -> Option<SourceKey> {
    let path = std::fs::canonicalize(path).ok()?;
    let metadata = std::fs::metadata(&path).ok()?;
    Some(SourceKey {
        size: metadata.len(),
        modified: metadata.modified().ok(),
        path,
    })
}

/// Removed files kept for `REMOVAL_GRACE`, by composer target and source:
/// adding the same file there again restores one, and the rest are released
/// as Remove releases a file once their time runs out.
pub struct Parked<K, T> {
    entries: Vec<(K, SourceKey, Instant, T)>,
}

impl<K, T> Default for Parked<K, T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<K: PartialEq, T> Parked<K, T> {
    pub fn park(&mut self, target: K, source: SourceKey, item: T, now: Instant) {
        self.entries
            .push((target, source, now + REMOVAL_GRACE, item));
    }

    /// The file parked for `target` from `source`, which leaves the park.
    pub fn restore(&mut self, target: &K, source: &SourceKey) -> Option<T> {
        let i = self
            .entries
            .iter()
            .position(|(k, s, ..)| k == target && s == source)?;
        Some(self.entries.remove(i).3)
    }

    /// The files whose grace ended by `now`, for the caller to release.
    pub fn expired(&mut self, now: Instant) -> Vec<T> {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|(_, _, due, _)| *due <= now);
        self.entries = kept;
        gone.into_iter().map(|(.., item)| item).collect()
    }

    /// Every parked file, for the caller to release at once.
    pub fn take_all(&mut self) -> Vec<T> {
        std::mem::take(&mut self.entries)
            .into_iter()
            .map(|(.., item)| item)
            .collect()
    }

    /// How long until the next file's grace ends, or None with none parked.
    pub fn next_due(&self, now: Instant) -> Option<Duration> {
        self.entries
            .iter()
            .map(|(_, _, due, _)| due.saturating_duration_since(now))
            .min()
    }

    /// Parked files still take their upload's events.
    pub fn items_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.iter_mut().map(|(.., item)| item)
    }
}

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
    /// The picked file on disk as it was added, which removing it parks
    /// under; None for pastes, captures and recordings.
    pub source_key: Option<SourceKey>,
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
    /// Another device's file, from a synced draft (`saved_drafts`): it shows
    /// from its URLs, sends by its ID, and removing it never cancels it.
    pub synced: bool,
}

/// What a staged file's busy ring shows.
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
            source_key: None,
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
            synced: false,
        }
    }

    /// A file from a synced draft: ready, shown and sent by its ID, with no
    /// local copy until Edit downloads one.
    pub fn synced(
        key: u64,
        channel: String,
        thread_root_id: Option<String>,
        attachment: Attachment,
    ) -> Self {
        let mut upload = Self::uploaded(key, channel, thread_root_id, attachment);
        upload.synced = true;
        upload
    }

    /// Whether this device holds the file itself (picked, pasted, recorded
    /// or downloaded), so it can be prepared and uploaded again.
    pub fn has_source(&self) -> bool {
        self.local_path.is_some() || self.prepared.is_some() || self.source_scratch.is_some()
    }

    /// Where a synced file's tile picture loads from: its preview, or an
    /// image's own URL.
    pub fn thumbnail_url(&self) -> Option<&str> {
        let attachment = self.attachment.as_ref()?;
        attachment.preview_url.as_deref().or_else(|| {
            (attachment.kind == AttachmentKind::Image && !attachment.animated)
                .then_some(attachment.url.as_deref())
                .flatten()
        })
    }

    /// A recording staged in a composer as a voice message: already
    /// prepared, so it uploads as it is, and not yet started.
    pub fn recorded(
        key: u64,
        channel: String,
        thread_root_id: Option<String>,
        prepared: Arc<Prepared>,
    ) -> Self {
        let mut upload = Self::picked(key, channel, thread_root_id, prepared.name.clone(), None);
        upload.kind = AttachmentKind::Audio;
        upload.source_size = prepared.size;
        upload.stored_size = Some(prepared.size);
        upload.uploading = false;
        upload.prepared = Some(prepared);
        upload
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

    /// What removing this file parks it under: only an unedited picked
    /// file has one; anything else is released at once.
    pub fn parkable(&self) -> Option<SourceKey> {
        self.source_key.clone().filter(|_| {
            self.edit.is_none() && self.source_scratch.is_none() && self.voice().is_none()
        })
    }

    /// The uploaded file as the message sends it, with the composer's
    /// spoiler choice.
    pub fn send_attachment(&self) -> Option<Attachment> {
        self.attachment.clone().map(|mut attachment| {
            attachment.spoiler = self.spoiler;
            attachment
        })
    }

    /// A file that never uploaded, or one of this device's that has (nearly)
    /// expired. A file with no local copy (another device's) is never
    /// uploaded again.
    pub fn needs_upload(&self) -> bool {
        self.attachment.is_none()
            || self.has_source()
                && (self.upload_expired || self.uploaded_at.is_some_and(|at| at.elapsed() >= FRESH))
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
        // A synced file whose download for editing failed: Retry clears the
        // failure; it still sends by its ID.
        if self.attachment.is_some() && !self.has_source() {
            self.error = None;
            return None;
        }
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
    /// busy ring shows while unfinished: packing first when that reports a
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
                // Nothing sent yet reads as waiting, so the ring turns instead of sitting empty.
                if self.progress > 0.0 {
                    Progress::Known(self.progress)
                } else {
                    Progress::Unknown
                },
            )
        }
    }
}

/// A composer's file as the clients keep it, around the shared upload.
pub trait Staged {
    fn upload(&self) -> &Upload;
    fn upload_mut(&mut self) -> &mut Upload;
    /// Whether it sits in a composer (and so in its draft), rather than
    /// sending on its own.
    fn in_composer(&self) -> bool {
        true
    }
}

impl Staged for Upload {
    fn upload(&self) -> &Upload {
        self
    }
    fn upload_mut(&mut self) -> &mut Upload {
        self
    }
}

/// Applies a synced draft's files (`saved_drafts::Applied::files`) to the
/// tiles of its composer (`channel`, `root`): the server's files, in its
/// order and with its spoiler marks, reusing tiles that already hold them
/// and making the rest with `synced` (`Upload::synced`); then this device's
/// files still preparing, uploading or failed. Returns the tiles the server
/// no longer lists, for the caller to release as Remove does (a synced one
/// is only dropped). Other composers' tiles are left as they are.
pub fn adopt<T: Staged>(
    tiles: &mut Vec<T>,
    channel: &str,
    root: Option<&str>,
    server: &[Attachment],
    mut synced: impl FnMut(&Attachment) -> T,
) -> Vec<T> {
    let (mine, others): (Vec<T>, Vec<T>) = std::mem::take(tiles).into_iter().partition(|tile| {
        let upload = tile.upload();
        tile.in_composer() && upload.channel == channel && upload.thread_root_id.as_deref() == root
    });
    let mut mine: Vec<Option<T>> = mine.into_iter().map(Some).collect();
    *tiles = others;
    for attachment in server {
        let held = mine.iter_mut().find(|tile| {
            tile.as_ref().is_some_and(|tile| {
                tile.upload()
                    .attachment
                    .as_ref()
                    .is_some_and(|file| file.id == attachment.id)
            })
        });
        match held.and_then(Option::take) {
            Some(mut tile) => {
                tile.upload_mut().spoiler = attachment.spoiler;
                tiles.push(tile);
            }
            None => tiles.push(synced(attachment)),
        }
    }
    let mut dropped = Vec::new();
    for tile in mine.into_iter().flatten() {
        if tile.upload().attachment.is_none() {
            tiles.push(tile);
        } else {
            dropped.push(tile);
        }
    }
    dropped
}

/// Whether the attachment editor can open a synced file once downloaded:
/// still images it decodes, and videos with FFmpeg present.
pub fn synced_editable(attachment: &Attachment) -> Option<crate::attachment_edit::Kind> {
    use crate::attachment_edit::Kind;
    match attachment.kind {
        AttachmentKind::Image if !attachment.animated => match attachment.content_type.as_str() {
            "image/jpeg" | "image/png" | "image/webp" | "image/bmp" | "image/tiff" => {
                Some(Kind::Image)
            }
            "image/heic" | "image/heif" | "image/avif" if crate::ffmpeg::binary().is_some() => {
                Some(Kind::Image)
            }
            _ => None,
        },
        AttachmentKind::Video
            if !attachment.animated && !attachment.voice && crate::ffmpeg::binary().is_some() =>
        {
            Some(Kind::Video)
        }
        _ => None,
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
        assert_eq!(file.status(), ("Sending 0%".into(), Progress::Unknown));
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
        let mut file = Upload::picked(
            1,
            "c".into(),
            None,
            "photo.png".into(),
            Some("photo.png".into()),
        );
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
    fn parked_files_restore_for_their_own_target_until_the_grace_ends() {
        let scratch = Scratch::new("png");
        std::fs::write(scratch.path(), b"picture").unwrap();
        let key = source_key(scratch.path()).unwrap();
        assert_eq!(key.size, 7);
        assert_eq!(source_key(Path::new("/no/such/file.png")), None);
        let other = SourceKey {
            path: "/elsewhere.png".into(),
            ..key.clone()
        };
        let now = Instant::now();
        let mut parked = Parked::default();
        parked.park(("c", None), key.clone(), 1, now);
        parked.park(("c", Some("root")), key.clone(), 2, now);
        assert_eq!(parked.next_due(now), Some(REMOVAL_GRACE));
        // Another file, or the same file in another composer, is not it.
        assert_eq!(parked.restore(&("c", None), &other), None);
        assert_eq!(parked.restore(&("d", None), &key), None);
        assert_eq!(parked.restore(&("c", None), &key), Some(1));
        assert_eq!(parked.restore(&("c", None), &key), None);
        // Parked files still take their upload's events.
        assert_eq!(parked.items_mut().collect::<Vec<_>>(), [&mut 2]);
        // A file parked again later waits its own full grace.
        let later = now + Duration::from_secs(10);
        parked.park(("c", None), key.clone(), 3, later);
        let ended = now + REMOVAL_GRACE;
        assert!(parked.expired(ended - Duration::from_millis(1)).is_empty());
        assert_eq!(parked.expired(ended), [2]);
        assert_eq!(parked.next_due(ended), Some(Duration::from_secs(10)));
        assert_eq!(parked.take_all(), [3]);
        assert_eq!(parked.next_due(ended), None);
    }

    #[test]
    fn only_unedited_picked_files_park() {
        let scratch = Scratch::new("txt");
        std::fs::write(scratch.path(), b"notes").unwrap();
        let mut file = Upload::picked(1, "c".into(), None, "notes.txt".into(), None);
        assert_eq!(file.parkable(), None);
        file.source_key = source_key(scratch.path());
        assert!(file.parkable().is_some());
        file.edit = Some(Default::default());
        assert_eq!(file.parkable(), None);
        file.edit = None;
        file.source_scratch = Some(Arc::new(Scratch::new("png")));
        assert_eq!(file.parkable(), None);
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
