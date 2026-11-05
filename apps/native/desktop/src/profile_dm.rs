//! Per-person drafts. Files upload eagerly; only explicit Send writes a message.
use crate::{
    api::Api,
    compress::{self, Compression},
    uploads,
};
#[cfg(test)]
pub use caper_client::profile_dm::Failure;
pub use caper_client::profile_dm::Send;
use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

/// A staged file with its local preview, which only the UI draws.
#[derive(Clone)]
pub struct File {
    pub file: caper_client::profile_dm::File,
    /// A local image/video preview, filled off the UI thread by [File::thumbnail].
    pub preview: Arc<OnceLock<Option<egui::ColorImage>>>,
    pub texture: Option<egui::TextureHandle>,
    preview_requested: bool,
}

impl std::ops::Deref for File {
    type Target = caper_client::profile_dm::File;

    fn deref(&self) -> &Self::Target {
        &self.file
    }
}

impl std::ops::DerefMut for File {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.file
    }
}

impl From<caper_client::profile_dm::File> for File {
    fn from(file: caper_client::profile_dm::File) -> Self {
        Self {
            file,
            preview: Arc::default(),
            texture: None,
            preview_requested: false,
        }
    }
}

/// The files a send returns, with the previews they were staged with: a
/// returned file keeps the preview of the draft file in its place.
pub fn returned(previous: Vec<File>, files: Vec<caper_client::profile_dm::File>) -> Vec<File> {
    let mut previous = previous.into_iter();
    files
        .into_iter()
        .map(
            |file| match previous.next().filter(|staged| staged.path == file.path) {
                Some(staged) => File { file, ..staged },
                None => File::from(file),
            },
        )
        .collect()
}

impl File {
    pub fn picked(path: PathBuf) -> Self {
        caper_client::profile_dm::File::picked(path).into()
    }

    /// A voice message sent on its own from the profile card: never staged
    /// in the draft, compressed or edited.
    pub fn voice(prepared: Arc<uploads::Prepared>) -> Self {
        caper_client::profile_dm::File::voice(prepared).into()
    }

    fn staged(path: PathBuf, scratch: Option<Arc<uploads::Scratch>>) -> Self {
        caper_client::profile_dm::File::staged(path, scratch).into()
    }

    /// A new edit (`None` returns to the original), shown in a fresh
    /// preview: the editor's render when it has one, else decoded again.
    pub fn set_edit(
        &mut self,
        edit: Option<crate::attachment_edit::Edit>,
        preview: Option<egui::ColorImage>,
    ) {
        self.file.set_edit(edit);
        self.texture = None;
        self.preview_requested = preview.is_some();
        self.preview =
            Arc::new(preview.map_or_else(OnceLock::new, |preview| OnceLock::from(Some(preview))));
    }

    /// The preview texture once decoded. Images and video posters are decoded
    /// in the background, edited as they will be sent; unsupported files keep
    /// the file icon.
    pub fn thumbnail(&mut self, context: &egui::Context) -> Option<&egui::TextureHandle> {
        if !self.preview_requested {
            self.preview_requested = true;
            self.load_preview(context);
        }
        if self.texture.is_none()
            && let Some(Some(image)) = self.preview.get()
        {
            self.texture = Some(context.load_texture(
                format!("profile-file-{}", self.path.display()),
                image.clone(),
                egui::TextureOptions::LINEAR,
            ));
        }
        self.texture.as_ref()
    }

    fn load_preview(&self, context: &egui::Context) {
        let is_image = self.scratch.is_some() || image::ImageFormat::from_path(&self.path).is_ok();
        let is_video = compress::type_from_name(&self.name()).starts_with("video/");
        if self.preview.get().is_some() || (!is_image && !is_video) {
            return;
        }
        let (path, preview, context) = (self.path.clone(), self.preview.clone(), context.clone());
        let edit = self.edit.clone();
        // Pasted files must outlive a draft removed while decoding.
        let scratch = self.scratch.clone();
        std::thread::spawn(move || {
            let _scratch = scratch;
            // The edited picture, as it will be sent.
            let edited = |image: image::DynamicImage, edit: &crate::attachment_edit::Edit| {
                let image = crate::attachment_edit::render(&image, edit)?;
                let edge = crate::attachments::THUMBNAIL_EDGE;
                let image = image.thumbnail(edge, edge).to_rgba8();
                Some(egui::ColorImage::from_rgba_unmultiplied(
                    [image.width() as usize, image.height() as usize],
                    image.as_raw(),
                ))
            };
            let image = if is_video {
                crate::ffmpeg::binary().and_then(|binary| {
                    let probe = crate::ffmpeg::probe(binary, &path).ok()?;
                    let poster = crate::ffmpeg::poster(binary, &path, &probe, 320)?;
                    match &edit {
                        Some(edit) => edited(image::DynamicImage::ImageRgb8(poster), edit),
                        None => Some(egui::ColorImage::from_rgb(
                            [poster.width() as usize, poster.height() as usize],
                            poster.as_raw(),
                        )),
                    }
                })
            } else {
                std::fs::read(&path).ok().and_then(|bytes| match &edit {
                    Some(edit) => crate::compress::decode_upright(&bytes)
                        .and_then(|(image, _)| edited(image, edit)),
                    None => crate::attachments::decode_thumbnail(&bytes),
                })
            };
            let _ = preview.set(image);
            context.request_repaint();
        });
    }
}

#[derive(Default)]
pub struct Draft {
    pub text: String,
    pub files: Vec<File>,
    pub sending: bool,
    /// An uncertain write freezes its payload until the same ID is confirmed.
    pub retry_id: Option<String>,
    pub error: Option<String>,
    pub status: Option<String>,
    /// A voice message that did not send: Retry resends it (same ID, the
    /// uploaded file reused) and Discard drops it. The text draft stays.
    pub voice_retry: Option<Box<Send>>,
}

impl Draft {
    /// One recorded voice message per message.
    pub fn has_voice(&self) -> bool {
        self.files.iter().any(|file| file.voice.is_some())
    }

    pub fn needs_upload(&self) -> bool {
        self.editable()
            && self
                .files
                .iter()
                .any(|file| file.upload.is_none() && file.attachment.is_none())
    }

    pub fn cancel_uploads(&self) {
        for file in &self.files {
            if let Some(upload) = &file.upload {
                upload
                    .cancel
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }

    /// Each file owns its worker; removing/editing it invalidates its result.
    pub fn upload_files(
        &mut self,
        api: &Api,
        token: &str,
        (person, username): (&str, &str),
        settings: &Compression,
        context: &egui::Context,
    ) {
        self.upload_files_to(api, token, (person, username, None), settings, context);
    }

    pub fn upload_activity_files(
        &mut self,
        api: &Api,
        token: &str,
        item: &crate::activity::Item,
        settings: &Compression,
        context: &egui::Context,
    ) {
        self.upload_files_to(
            api,
            token,
            (&item.draft_key(), &item.actor_name, Some(item.clone())),
            settings,
            context,
        );
    }

    fn upload_files_to(
        &mut self,
        api: &Api,
        token: &str,
        (person, username, target): (&str, &str, Option<crate::activity::Item>),
        settings: &Compression,
        context: &egui::Context,
    ) {
        if !self.editable() {
            return;
        }
        for file in &mut self.files {
            if file.upload.is_some() || file.attachment.is_some() {
                continue;
            }
            let host = crate::host::Repaint(context.clone());
            file.file.upload_eagerly(
                api,
                token,
                (person, username, target.clone()),
                settings,
                &host,
            );
        }
    }

    pub fn editable(&self) -> bool {
        !self.sending && self.retry_id.is_none()
    }

    pub fn ready(&self) -> bool {
        !self.sending && (!self.text.trim().is_empty() || !self.files.is_empty())
    }

    pub fn add_files(&mut self, files: Vec<File>) {
        if !self.editable() || files.is_empty() {
            return;
        }
        let room = uploads::MAX_ATTACHMENTS.saturating_sub(self.files.len());
        self.error = (files.len() > room)
            .then(|| format!("You can attach up to {} files.", uploads::MAX_ATTACHMENTS));
        self.files.extend(files.into_iter().take(room));
    }

    pub fn begin(&mut self) -> Result<String, String> {
        if !self.ready() {
            return Err("Write a message or attach a file first.".into());
        }
        if self.text.chars().count() > 4_000 {
            return Err("Messages can be at most 4,000 characters.".into());
        }
        if self
            .text
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            return Err("Messages cannot contain control characters.".into());
        }
        let id = self
            .retry_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.sending = true;
        self.error = None;
        self.status = Some("Sending…".into());
        Ok(id)
    }
}

// Only actual attachment representations: never text/HTML, selection metadata,
// or file-transfer protocols. Copied arbitrary files use arboard's file list.
// GIF/PNG are Windows registered format names; request the offered spelling.
fn attachment_format(format: &str) -> Option<(&'static str, u8)> {
    Some(match format.to_ascii_lowercase().as_str() {
        "image/gif" | "gif" => ("gif", 0),
        "image/png" | "png" => ("png", 1),
        "image/jpeg" => ("jpg", 2),
        "image/webp" => ("webp", 2),
        "image/avif" => ("avif", 2),
        "image/heic" => ("heic", 2),
        "image/bmp" => ("bmp", 2),
        "image/tiff" => ("tiff", 2),
        "video/mp4" => ("mp4", 2),
        "video/webm" => ("webm", 2),
        "video/quicktime" => ("mov", 2),
        "audio/mpeg" => ("mp3", 2),
        "audio/mp4" => ("m4a", 2),
        "audio/ogg" => ("ogg", 2),
        "audio/wav" | "audio/x-wav" => ("wav", 2),
        "audio/flac" => ("flac", 2),
        "application/pdf" => ("pdf", 2),
        "application/zip" => ("zip", 2),
        "application/octet-stream" => ("bin", 3),
        _ => return None,
    })
}

fn encoded_attachment(
    formats: Vec<String>,
    mut read: impl FnMut(&str) -> Option<Vec<u8>>,
) -> Result<Option<File>, String> {
    let mut formats: Vec<_> = formats
        .into_iter()
        .filter_map(|format| attachment_format(&format).map(|kind| (format, kind)))
        .collect();
    formats.sort_by_key(|(_, (_, priority))| *priority);
    for (format, (extension, _)) in formats {
        let Some(bytes) = read(&format).filter(|bytes| !bytes.is_empty()) else {
            continue;
        };
        let scratch = Arc::new(uploads::Scratch::new(extension));
        std::fs::write(scratch.path(), bytes)
            .map_err(|_| "Could not save the clipboard attachment.")?;
        return Ok(Some(File::staged(scratch.path().to_owned(), Some(scratch))));
    }
    Ok(None)
}

fn raw_clipboard_attachment() -> Result<Option<File>, String> {
    use clipboard_rs::{Clipboard, ClipboardContext};
    let Ok(clipboard) = ClipboardContext::new() else {
        return Ok(None);
    };
    encoded_attachment(
        clipboard.available_formats().unwrap_or_default(),
        |format| clipboard.get_buffer(format).ok(),
    )
}

/// Prefer copied files to an image (file managers may advertise both), and
/// leave ordinary text pastes to egui. Unsupported clipboard formats are a no-op.
pub fn paste_files() -> Result<Vec<File>, String> {
    let mut clipboard = arboard::Clipboard::new().ok();
    if let Some(clipboard) = clipboard.as_mut()
        && let Ok(paths) = clipboard.get().file_list()
        && !paths.is_empty()
    {
        return Ok(paths.into_iter().map(File::picked).collect());
    }
    if let Some(file) = raw_clipboard_attachment()? {
        return Ok(vec![file]);
    }
    let Some(mut clipboard) = clipboard else {
        return Ok(Vec::new());
    };
    let Ok(image) = clipboard.get_image() else {
        return Ok(Vec::new());
    };
    let width = u32::try_from(image.width).map_err(|_| "Clipboard image is too large.")?;
    let height = u32::try_from(image.height).map_err(|_| "Clipboard image is too large.")?;
    let pixels = image::RgbaImage::from_raw(width, height, image.bytes.into_owned())
        .ok_or("Could not read the clipboard image.")?;
    let scratch = Arc::new(uploads::Scratch::new("png"));
    pixels
        .save_with_format(scratch.path(), image::ImageFormat::Png)
        .map_err(|_| "Could not save the clipboard image.")?;
    Ok(vec![File::staged(scratch.path().to_owned(), Some(scratch))])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::sync::atomic::AtomicBool;

    const DIRECT: &str = r#"{"id":"dm0000000002","peer":{"id":"alex-id","username":"alex","displayName":"Alex"},"lastSeq":"0","readSeq":"0","status":"outgoing"}"#;

    fn animated_clipboard_gif() -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                        2,
                        2,
                        image::Rgba(color),
                    )))
                    .unwrap();
            }
        }
        bytes
    }

    #[test]
    fn encoded_paste_preserves_gif_and_prefers_it_to_png() {
        let gif = animated_clipboard_gif();
        for formats in [["image/png", "image/gif"], ["PNG", "GIF"]] {
            let file = encoded_attachment(formats.map(String::from).to_vec(), |format| {
                Some(if format.to_ascii_lowercase().contains("gif") {
                    gif.clone()
                } else {
                    b"flattened PNG".to_vec()
                })
            })
            .unwrap()
            .unwrap();
            assert_eq!(file.path.extension().unwrap(), "gif");
            assert_eq!(std::fs::read(&file.path).unwrap(), gif);
            let path = file.path.clone();
            drop(file);
            assert!(!path.exists(), "draft owns and cleans up its encoded bytes");
        }
    }

    #[test]
    fn encoded_paste_skips_unreadable_and_empty_formats() {
        let file = encoded_attachment(
            ["image/png", "image/gif", "application/pdf"]
                .map(String::from)
                .to_vec(),
            |format| match format {
                "image/gif" => None,
                "image/png" => Some(Vec::new()),
                "application/pdf" => Some(b"%PDF-original".to_vec()),
                _ => unreachable!(),
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(file.path.extension().unwrap(), "pdf");
        assert_eq!(std::fs::read(file.path.clone()).unwrap(), b"%PDF-original");
    }

    #[test]
    fn encoded_paste_does_not_turn_text_metadata_or_file_protocols_into_files() {
        let formats = [
            "text/plain",
            "text/html",
            "HTML Format",
            "text/uri-list",
            "x-special/gnome-copied-files",
            "application/x-kde-cutselection",
            "TARGETS",
            "TIMESTAMP",
            "SAVE_TARGETS",
            "FileGroupDescriptorW",
            "FileContents",
            "image/not-a-format",
        ];
        assert!(
            encoded_attachment(formats.map(String::from).to_vec(), |_| Some(
                b"metadata".to_vec()
            ))
            .unwrap()
            .is_none()
        );
    }

    #[test]
    #[ignore = "requires an isolated real desktop clipboard; replaces its contents"]
    fn real_clipboard_encoded_paste_and_text() {
        use clipboard_rs::{Clipboard, ClipboardContent, ClipboardContext};
        let clipboard = ClipboardContext::new().unwrap();
        let gif = animated_clipboard_gif();
        clipboard
            .set(vec![
                ClipboardContent::Other("image/png".into(), b"png alternative".to_vec()),
                ClipboardContent::Other("image/gif".into(), gif.clone()),
            ])
            .unwrap();
        let files = paste_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path.extension().unwrap(), "gif");
        assert_eq!(std::fs::read(&files[0].path).unwrap(), gif);
        let png = uploads::Scratch::new("png");
        image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]))
            .save_with_format(png.path(), image::ImageFormat::Png)
            .unwrap();
        let bytes = std::fs::read(png.path()).unwrap();
        clipboard.set_buffer("image/png", bytes.clone()).unwrap();
        let files = paste_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path.extension().unwrap(), "png");
        assert_eq!(std::fs::read(&files[0].path).unwrap(), bytes);
        for (format, extension) in [
            ("application/pdf", "pdf"),
            ("application/zip", "zip"),
            ("application/octet-stream", "bin"),
        ] {
            let bytes = vec![0, 255, 19, 82, 0, 12];
            clipboard.set_buffer(format, bytes.clone()).unwrap();
            let files = paste_files().unwrap();
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].path.extension().unwrap(), extension);
            assert_eq!(std::fs::read(&files[0].path).unwrap(), bytes);
        }
        let copied = uploads::Scratch::new("arbitrary");
        std::fs::write(copied.path(), b"arbitrary copied file").unwrap();
        clipboard
            .set(vec![
                ClipboardContent::Files(vec![copied.path().to_str().unwrap().into()]),
                ClipboardContent::Other("image/gif".into(), gif),
            ])
            .unwrap();
        let files = paste_files().unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, copied.path());
        assert!(files[0].scratch.is_none());
        clipboard.set_text("ordinary text".into()).unwrap();
        assert!(paste_files().unwrap().is_empty());
        assert_eq!(clipboard.get_text().unwrap(), "ordinary text");
        clipboard
            .set_html("<img src='https://example.invalid/a.gif'>".into())
            .unwrap();
        assert!(paste_files().unwrap().is_empty());
    }

    fn operation(files: Vec<caper_client::profile_dm::File>) -> Send {
        Send {
            person: "alex-id".into(),
            username: "alex".into(),
            name: "Owner".into(),
            client_id: uuid::Uuid::new_v4().to_string(),
            text: "Receipt 🎉".into(),
            files,
            settings: Some(Compression::default()),
            cancel: Arc::new(AtomicBool::new(false)),
            retrying: false,
            open_only: false,
            voice_only: false,
            direct: None,
        }
    }

    struct Request {
        line: String,
        body: Vec<u8>,
    }

    /// A loopback API/storage server records the public HTTP contract.
    fn server(
        replies: impl FnOnce(&str) -> Vec<(u16, String)> + std::marker::Send + 'static,
    ) -> (Api, std::thread::JoinHandle<Vec<Request>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let api = Api::new(&base).unwrap();
        let thread = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, reply) in replies(&base) {
                let (stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut length = 0;
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    if let Some(value) = header.to_ascii_lowercase().strip_prefix("content-length:")
                    {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                seen.push(Request {
                    line: line.trim_end().into(),
                    body,
                });
                write!(reader.get_mut(), "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}", reply.len()).unwrap();
            }
            seen
        });
        (api, thread)
    }

    #[test]
    #[ignore = "needs CAPER_FFMPEG and a fixture FFmpeg with lavfi"]
    fn staged_videos_get_a_thumbnail_before_upload() {
        let video = uploads::Scratch::new("mp4");
        let status = crate::ffmpeg::command(&crate::ffmpeg::fixture_binary())
            .args(["-y", "-f", "lavfi", "-i", "color=c=red:s=64x32:d=1"])
            .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
            .arg(video.path())
            .output()
            .unwrap();
        assert!(status.status.success(), "fixture generated");
        let context = egui::Context::default();
        let mut file = File::picked(video.path().to_owned());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while file.thumbnail(&context).is_none() {
            assert!(std::time::Instant::now() < deadline, "video poster decoded");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let texture = file.thumbnail(&context).unwrap();
        assert_eq!(texture.size(), [64, 32]);
        assert!(file.attachment.is_none(), "preview needs no upload");
        // An edit without the editor's render shows the poster as it will be sent.
        let left = crate::attachment_edit::Edit {
            crop: crate::attachment_edit::Crop {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
            ..crate::attachment_edit::Edit::default()
        };
        file.set_edit(Some(left), None);
        while file.thumbnail(&context).is_none() {
            assert!(
                std::time::Instant::now() < deadline,
                "edited poster decoded"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let [width, height] = file.thumbnail(&context).unwrap().size();
        assert_eq!(width, height, "the kept left half is square");
    }

    #[test]
    fn staged_images_get_a_thumbnail_and_other_files_keep_the_icon() {
        let context = egui::Context::default();
        let image = uploads::Scratch::new("png");
        image::RgbaImage::from_pixel(64, 32, image::Rgba([182, 77, 50, 255]))
            .save(image.path())
            .unwrap();
        let mut file = File::picked(image.path().to_owned());
        assert!(file.size > 0, "the chip shows the staged size");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while file.thumbnail(&context).is_none() {
            assert!(std::time::Instant::now() < deadline, "thumbnail decoded");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(file.thumbnail(&context).unwrap().size(), [64, 32]);
        let text = uploads::Scratch::new("txt");
        std::fs::write(text.path(), b"notes").unwrap();
        let mut file = File::picked(text.path().to_owned());
        assert!(file.thumbnail(&context).is_none());
        assert!(
            file.preview.get().is_none(),
            "no decode is attempted for other files"
        );
    }

    #[test]
    fn eager_upload_does_not_send_and_explicit_send_reuses_it() {
        let scratch = uploads::Scratch::new("txt");
        std::fs::write(scratch.path(), b"receipt").unwrap();
        let mut draft = Draft::default();
        draft.add_files(vec![File::picked(scratch.path().to_owned())]);
        let mut send = operation(Vec::new());
        let client_id = send.client_id.clone();
        let (api, requests) = server(move |base| {
            vec![
            (200, format!("{{\"conversations\":[{DIRECT}]}}")),
            (201, json!({"id":"asset1","upload":{"method":"PUT","url":format!("{base}/storage/asset1"),"headers":{}}}).to_string()),
            (200, String::new()),
            (200, json!({"id":"asset1","kind":"file","contentType":"text/plain","name":"receipt.txt","size":7}).to_string()),
            (200, format!("{{\"conversations\":[{DIRECT}]}}")),
            (200, json!({"token":"chat","author":{"id":"owner","name":"Owner","isGuest":false}}).to_string()),
            (200, json!({"id":"m1","channelId":"dm0000000002","seq":"1","createdAt":"2026-10-11T00:00:00Z","clientMessageId":client_id,"author":{"id":"owner","name":"Owner","isGuest":false},"content":{"version":1,"type":"text","text":"Receipt 🎉"}}).to_string()),
        ]
        });
        draft.upload_files(
            &api,
            "account",
            ("alex-id", "alex"),
            &Compression::default(),
            &egui::Context::default(),
        );
        let upload = draft.files[0].upload.as_ref().unwrap().clone();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while upload.result.get().is_none() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(upload.result.get().unwrap().is_ok());
        send.files = draft.files.iter().map(|file| file.file.clone()).collect();
        send.run(&api, "account", &|_| {}).unwrap();
        let seen = requests.join().unwrap();
        assert_eq!(seen[1].line, "POST /api/assets HTTP/1.1");
        assert_eq!(seen[4].line, "GET /api/dms HTTP/1.1");
        assert_eq!(
            seen.len(),
            7,
            "no eager session/message or duplicate upload"
        );
        let sent: Value = serde_json::from_slice(&seen[6].body).unwrap();
        assert_eq!(sent["attachmentIds"], json!(["asset1"]));
        draft.files[0].set_edit(None, None);
        assert!(draft.files[0].upload.is_none());
        assert!(draft.files[0].attachment.is_none());
        draft.cancel_uploads();
    }

    #[test]
    fn voice_messages_upload_as_recorded_and_send_alone() {
        let prepared = crate::voice_note::prepare(&[4_000; 48_000], None).unwrap();
        let size = prepared.size;
        let mut operation = operation(vec![caper_client::profile_dm::File::voice(Arc::new(
            prepared,
        ))]);
        operation.text = String::new();
        operation.voice_only = true;
        assert_eq!(operation.files[0].name(), "voice-message.wav");
        let client_id = operation.client_id.clone();
        let (api, requests) = server(move |base| {
            vec![
            (200, json!({"conversations":[]}).to_string()),
            (201, DIRECT.into()),
            (200, json!({"token":"chat","author":{"id":"owner","name":"Owner","isGuest":false}}).to_string()),
            (201, json!({"id":"voice1","upload":{"method":"PUT","url":format!("{base}/storage/voice1"),"headers":{}}}).to_string()),
            (200, String::new()),
            (200, json!({"id":"voice1","kind":"audio","contentType":"audio/mp4","name":"Voice message.m4a","size":900,"durationMs":1000,"voice":true,"transcript":{"status":"pending"}}).to_string()),
            (200, json!({"id":"m1","channelId":"dm0000000002","seq":"1","createdAt":"2026-10-11T00:00:00Z","clientMessageId":client_id,"author":{"id":"owner","name":"Owner","isGuest":false},"content":{"version":1,"type":"text","text":""}}).to_string()),
        ]
        });
        let mut statuses = std::sync::Mutex::new(Vec::new());
        let direct = operation
            .run(&api, "account", &|status| {
                statuses.lock().unwrap().push(status)
            })
            .unwrap();
        assert_eq!(direct.id, "dm0000000002");
        assert!(
            statuses
                .get_mut()
                .unwrap()
                .contains(&"Uploading voice message…".into())
        );
        let attachment = operation.files[0].attachment.as_ref().unwrap();
        assert!(attachment.voice, "the completed description is kept");
        let seen = requests.join().unwrap();
        let reservation: Value = serde_json::from_slice(&seen[3].body).unwrap();
        assert_eq!(reservation["channelId"], "dm0000000002");
        assert_eq!(reservation["voice"], true);
        assert_eq!(reservation["durationMs"], 1000);
        assert_eq!(reservation["byteSize"], size);
        assert!(reservation["waveform"].is_string());
        assert_eq!(seen[4].body.len() as u64, size, "uploaded as recorded");
        let sent: Value = serde_json::from_slice(&seen[6].body).unwrap();
        assert_eq!(
            sent,
            json!({"clientMessageId":operation.client_id,"text":"","attachmentIds":["voice1"]})
        );
    }
}
