//! Per-person drafts. Files upload eagerly; only explicit Send writes a message.
use crate::{
    api::Api,
    compress::{self, Compression},
    model, uploads,
};
use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, atomic::AtomicBool};

type Uploaded = Result<(String, model::Attachment, std::time::Instant), String>;

struct Upload {
    cancel: Arc<AtomicBool>,
    result: Arc<OnceLock<Uploaded>>,
}

impl Drop for Upload {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

#[derive(Clone)]
pub struct File {
    pub path: PathBuf,
    /// Pasted images belong to the draft, including while a worker reads them.
    pub scratch: Option<Arc<uploads::Scratch>>,
    pub attachment: Option<model::Attachment>,
    uploaded_at: Option<std::time::Instant>,
    upload: Option<Arc<Upload>>,
    pub spoiler: bool,
    /// Bytes on disk when staged, for the composer chip.
    pub size: u64,
    /// A local image/video preview, filled off the UI thread by [File::thumbnail].
    pub preview: Arc<OnceLock<Option<egui::ColorImage>>>,
    pub texture: Option<egui::TextureHandle>,
    preview_requested: bool,
    /// Whether the file can be cropped, drawn on and so on before sending.
    pub editable: Option<crate::attachment_edit::Kind>,
    /// Applied to the original before preparing and uploading.
    pub edit: Option<crate::attachment_edit::Edit>,
    /// A recorded voice message (`voice_note`), uploaded exactly as prepared.
    pub voice: Option<Arc<uploads::Prepared>>,
}

impl File {
    pub fn picked(path: PathBuf) -> Self {
        Self::staged(path, None)
    }

    /// A voice message sent on its own from the profile card: never staged
    /// in the draft, compressed or edited.
    pub fn voice(prepared: Arc<uploads::Prepared>) -> Self {
        Self {
            size: prepared.size,
            voice: Some(prepared),
            ..Self::staged(PathBuf::new(), None)
        }
    }

    fn staged(path: PathBuf, scratch: Option<Arc<uploads::Scratch>>) -> Self {
        Self {
            size: std::fs::metadata(&path).map_or(0, |metadata| metadata.len()),
            editable: crate::attachment_edit::editable(&path),
            path,
            scratch,
            attachment: None,
            uploaded_at: None,
            upload: None,
            spoiler: false,
            preview: Arc::default(),
            texture: None,
            preview_requested: false,
            edit: None,
            voice: None,
        }
    }

    /// A new edit (`None` returns to the original), shown in a fresh
    /// preview: the editor's render when it has one, else decoded again.
    pub fn set_edit(
        &mut self,
        edit: Option<crate::attachment_edit::Edit>,
        preview: Option<egui::ColorImage>,
    ) {
        self.upload = None;
        self.attachment = None;
        self.edit = edit;
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

    pub fn name(&self) -> String {
        if let Some(voice) = &self.voice {
            voice.name.clone()
        } else if self.scratch.is_some() {
            "Pasted image.png".into()
        } else {
            self.path
                .file_name()
                .map_or_else(|| "file".into(), |name| name.to_string_lossy().into_owned())
        }
    }

    /// The server's asset for this file, uploaded eagerly or on a send, so
    /// removing a staged file can cancel it.
    pub fn uploaded_id(&self) -> Option<String> {
        self.attachment
            .as_ref()
            .map(|file| file.id.clone())
            .or_else(
                || match self.upload.as_ref().and_then(|upload| upload.result.get()) {
                    Some(Ok((_, attachment, _))) => Some(attachment.id.clone()),
                    _ => None,
                },
            )
    }

    pub fn upload_status(&self) -> (&str, bool) {
        match self.upload.as_ref().and_then(|upload| upload.result.get()) {
            Some(Err(error)) => (error, true),
            Some(Ok(_)) => ("", false),
            None if self.upload.is_some() => ("Uploading…", false),
            None => ("", false),
        }
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
        if !self.editable() {
            return;
        }
        for file in &mut self.files {
            if file.upload.is_some() || file.attachment.is_some() {
                continue;
            }
            let cancel = Arc::new(AtomicBool::new(false));
            let result = Arc::new(OnceLock::new());
            let mut send = Send {
                person: person.into(),
                username: username.into(),
                name: String::new(),
                client_id: String::new(),
                text: String::new(),
                files: vec![file.clone()],
                settings: Some(settings.clone()),
                cancel: cancel.clone(),
                retrying: false,
                open_only: false,
                voice_only: false,
                direct: None,
            };
            file.upload = Some(Arc::new(Upload {
                cancel,
                result: result.clone(),
            }));
            context.request_repaint();
            let (api, token, context) = (api.clone(), token.to_owned(), context.clone());
            std::thread::spawn(move || {
                let uploaded = send
                    .run_inner(&api, &token, &|_| {}, true)
                    .map(|direct| {
                        (
                            direct.id,
                            send.files[0].attachment.clone().expect("uploaded file"),
                            std::time::Instant::now(),
                        )
                    })
                    .map_err(|error| error.message);
                let _ = result.set(uploaded);
                context.request_repaint();
            });
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

pub struct Send {
    pub person: String,
    pub username: String,
    pub name: String,
    pub client_id: String,
    pub text: String,
    pub files: Vec<File>,
    pub settings: Option<Compression>,
    pub cancel: Arc<AtomicBool>,
    pub retrying: bool,
    /// Explicit navigation resolves the DM but never uploads or writes a message.
    pub open_only: bool,
    /// A voice message sent alone as soon as recording stopped (the "send
    /// at once" setting): it keeps the card's draft instead of clearing it.
    pub voice_only: bool,
    pub direct: Option<model::DirectConversation>,
}

#[derive(Debug)]
pub struct Failure {
    pub message: String,
    pub uncertain: bool,
}

impl Send {
    /// Rechecks destination safety on Send, waits for eager uploads, and
    /// reuses uploaded IDs after a failed write.
    pub fn run(
        &mut self,
        api: &Api,
        token: &str,
        status: &dyn Fn(String),
    ) -> Result<model::DirectConversation, Failure> {
        self.run_inner(api, token, status, false)
    }

    fn run_inner(
        &mut self,
        api: &Api,
        token: &str,
        status: &dyn Fn(String),
        upload_only: bool,
    ) -> Result<model::DirectConversation, Failure> {
        let cancelled = || {
            if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                Err(Failure {
                    message: "Send cancelled.".into(),
                    uncertain: self.retrying,
                })
            } else {
                Ok(())
            }
        };
        cancelled()?;
        // No pre-POST failure can disprove an earlier uncertain write.
        let retrying = self.retrying;
        let lookup_failure = |error: crate::api::ApiError| Failure {
            uncertain: retrying,
            message: error.to_string(),
        };
        self.direct = api
            .direct_conversations(token)
            .map_err(lookup_failure)?
            .conversations
            .into_iter()
            .find(|direct| direct.peer.id == self.person);
        cancelled()?;
        let direct = match self.direct.clone() {
            Some(direct) => direct,
            None => api
                .create_direct(token, &self.username)
                .map_err(lookup_failure)?,
        };
        if direct.peer.id != self.person {
            return Err(Failure {
                message: "This person's username changed. Reopen their profile before sending."
                    .into(),
                uncertain: retrying,
            });
        }
        self.direct = Some(direct.clone());
        if self.open_only {
            return Ok(direct);
        }
        if direct.status == model::DirectStatus::Incoming {
            return Err(Failure {
                message: "Open conversation and accept this request before sending.".into(),
                uncertain: retrying,
            });
        }
        if direct.blocked {
            return Err(Failure {
                message: "Unblock this person to message them.".into(),
                uncertain: retrying,
            });
        }
        cancelled()?;
        let session = if upload_only {
            None
        } else {
            Some(
                api.chat_session(Some(token), &self.name)
                    .map_err(lookup_failure)?,
            )
        };
        for (index, file) in self.files.iter_mut().enumerate() {
            cancelled()?;
            if let Some(upload) = file.upload.take() {
                while upload.result.get().is_none() {
                    cancelled()?;
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                if let Some(Ok((destination, attachment, completed))) = upload.result.get()
                    && destination == &direct.id
                {
                    file.attachment = Some(attachment.clone());
                    file.uploaded_at = Some(*completed);
                }
                // A failed eager attempt is retried by explicit Send, never every frame.
            }
            if !retrying
                && file
                    .uploaded_at
                    .is_some_and(|at| at.elapsed().as_secs() >= 23 * 60 * 60)
            {
                if let Some(attachment) = file.attachment.take() {
                    api.cancel_asset(token, &attachment.id);
                }
                file.uploaded_at = None;
            }
            if file.attachment.is_some() {
                continue;
            }
            let settings = self.settings.as_ref().ok_or_else(|| Failure {
                message: "File uploads are not ready. Try again shortly.".into(),
                uncertain: retrying,
            })?;
            status(if file.voice.is_some() {
                "Uploading voice message…".into()
            } else {
                format!("Preparing and uploading {} ({})…", file.name(), index + 1)
            });
            let prepared = match (&file.voice, &file.edit) {
                (Some(voice), _) => Ok(voice.as_ref().clone()),
                (None, Some(edit)) => uploads::prepare_edited_path(
                    &file.path,
                    &file.name(),
                    edit,
                    settings,
                    &|_| {},
                    &self.cancel,
                ),
                (None, None) => {
                    uploads::prepare_path_with(&file.path, settings, &|_| {}, &self.cancel)
                }
            };
            let mut prepared = prepared.map_err(|message| Failure {
                message,
                uncertain: retrying,
            })?;
            if file.scratch.is_some() {
                prepared.name =
                    crate::compress::renamed("Pasted image.png", &prepared.content_type);
            }
            file.attachment = Some(
                uploads::upload(
                    api,
                    token,
                    &direct.id,
                    prepared,
                    Arc::new(|_| {}),
                    self.cancel.clone(),
                )
                .map_err(|error| Failure {
                    message: error.message,
                    uncertain: retrying,
                })?,
            );
            file.uploaded_at = Some(std::time::Instant::now());
        }
        cancelled()?;
        if upload_only {
            return Ok(direct);
        }
        let session = session.expect("explicit send session");
        status("Sending…".into());
        let ids = self
            .files
            .iter()
            .filter_map(|file| file.attachment.as_ref().map(|a| a.id.clone()))
            .collect::<Vec<_>>();
        let spoiler_ids = self
            .files
            .iter()
            .filter(|file| file.spoiler)
            .filter_map(|file| file.attachment.as_ref().map(|a| a.id.clone()))
            .collect::<Vec<_>>();
        let message = api
            .send(
                Some(token),
                &session.token,
                &direct.id,
                &self.client_id,
                &self.text,
                &ids,
                &spoiler_ids,
                (None, false),
                None,
                None,
            )
            .map_err(|error| Failure {
                // A definitive 4xx refuses the write. Transport/5xx responses
                // may have lost the acknowledgement after committing it.
                uncertain: error.status.is_none_or(|status| status.is_server_error()),
                message: error.to_string(),
            })?;
        if message.channel_id != direct.id
            || message.client_message_id != self.client_id
            || message.author.id != session.author.id
        {
            return Err(Failure {
                message: "Could not confirm message delivery. Retry sends the same message safely."
                    .into(),
                uncertain: true,
            });
        }
        Ok(direct)
    }
}

/// egui-winit 0.33 consumes the paste key-down even when the clipboard has no
/// text. Its key-up still arrives. Supply a paste event then, but never duplicate
/// a text/file-list paste already delivered on key-down.
#[derive(Default)]
pub struct PasteShortcut {
    delivered: bool,
}

impl PasteShortcut {
    pub fn update(&mut self, events: &mut Vec<egui::Event>) {
        let mut paste = false;
        for event in events.iter() {
            match event {
                egui::Event::Paste(_) => self.delivered = true,
                egui::Event::Key {
                    key,
                    pressed: false,
                    modifiers,
                    ..
                } if *key == egui::Key::V
                    || *key == egui::Key::Paste
                    || *key == egui::Key::Insert =>
                {
                    let shortcut = (*key == egui::Key::V && modifiers.command)
                        || *key == egui::Key::Paste
                        || (*key == egui::Key::Insert
                            && modifiers.shift
                            && cfg!(target_os = "windows"));
                    paste |= shortcut && !self.delivered;
                    self.delivered = false;
                }
                egui::Event::WindowFocused(false) => self.delivered = false,
                _ => {}
            }
        }
        if paste {
            events.push(egui::Event::Paste(String::new()));
        }
    }
}

/// Prefer copied files to an image (file managers may advertise both), and
/// leave ordinary text pastes to egui. Unsupported clipboard formats are a no-op.
pub fn paste_files() -> Result<Vec<File>, String> {
    let Ok(mut clipboard) = arboard::Clipboard::new() else {
        return Ok(Vec::new());
    };
    if let Ok(paths) = clipboard.get().file_list()
        && !paths.is_empty()
    {
        return Ok(paths.into_iter().map(File::picked).collect());
    }
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

    const DIRECT: &str = r#"{"id":"dm0000000002","peer":{"id":"alex-id","username":"alex","displayName":"Alex"},"lastSeq":"0","readSeq":"0","status":"outgoing"}"#;

    #[test]
    fn image_only_paste_shortcuts_arrive_once_without_duplicating_text_paste() {
        let release = |command| egui::Event::Key {
            key: egui::Key::V,
            physical_key: Some(egui::Key::V),
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers {
                command,
                ctrl: command,
                ..Default::default()
            },
        };
        let mut shortcut = PasteShortcut::default();
        // The integration swallowed an image-only key-down. Its release must paste.
        let mut events = vec![release(true)];
        shortcut.update(&mut events);
        assert_eq!(
            events,
            vec![release(true), egui::Event::Paste(String::new())]
        );
        // A file manager may provide both files and text, delivering Paste on key-down.
        let mut events = vec![egui::Event::Paste("file:///tmp/notes.txt".into())];
        shortcut.update(&mut events);
        assert_eq!(events.len(), 1);
        let mut events = vec![release(true)];
        shortcut.update(&mut events);
        assert_eq!(events, vec![release(true)]);
        // A subsequent image-only paste still works, including in one frame.
        let mut events = vec![release(true)];
        shortcut.update(&mut events);
        assert_eq!(events.len(), 2);
        let mut events = vec![egui::Event::Paste("normal text".into()), release(true)];
        shortcut.update(&mut events);
        assert_eq!(events.len(), 2);
        let mut events = vec![release(false)];
        shortcut.update(&mut events);
        assert_eq!(
            events,
            vec![release(false)],
            "typing v never reads the clipboard"
        );
    }

    fn operation(files: Vec<File>) -> Send {
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
        send.files = draft.files.clone();
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
    fn profile_send_uploads_once_and_uncertain_retry_keeps_the_exact_payload() {
        let scratch = uploads::Scratch::new("txt");
        std::fs::write(scratch.path(), b"receipt").unwrap();
        let mut operation = operation(vec![File::picked(scratch.path().to_owned())]);
        operation.files[0].spoiler = true;
        let client_id = operation.client_id.clone();
        let (api, requests) = server(move |base| {
            vec![
            (200, json!({"conversations":[]}).to_string()),
            (201, DIRECT.into()),
            (200, json!({"token":"chat","author":{"id":"owner","name":"Owner","isGuest":false}}).to_string()),
            (201, json!({"id":"asset1","upload":{"method":"PUT","url":format!("{base}/storage/asset1"),"headers":{}}}).to_string()),
            (200, String::new()),
            (200, json!({"id":"asset1","kind":"file","contentType":"text/plain","name":"receipt.txt","size":7}).to_string()),
            (503, json!({"error":"Acknowledgement lost"}).to_string()),
            (503, json!({"error":"Temporary lookup failure"}).to_string()),
            (200, format!("{{\"conversations\":[{DIRECT}]}}")),
            (200, json!({"id":"m1","channelId":"dm0000000002","seq":"1","createdAt":"2026-10-11T00:00:00Z","clientMessageId":client_id,"author":{"id":"owner","name":"Owner","isGuest":false},"content":{"version":1,"type":"text","text":"Receipt 🎉"}}).to_string()),
        ]
        });
        let error = operation.run(&api, "account", &|_| {}).unwrap_err();
        assert!(error.uncertain);
        assert_eq!(operation.files[0].attachment.as_ref().unwrap().id, "asset1");
        operation.retrying = true;
        assert!(
            operation
                .run(&api, "account", &|_| {})
                .unwrap_err()
                .uncertain
        );
        assert_eq!(
            operation.run(&api, "account", &|_| {}).unwrap().id,
            "dm0000000002"
        );
        let seen = requests.join().unwrap();
        assert_eq!(seen[4].body, b"receipt");
        let reservation: Value = serde_json::from_slice(&seen[3].body).unwrap();
        assert_eq!(reservation["channelId"], "dm0000000002");
        assert_eq!(reservation["byteSize"], 7);
        assert_eq!(
            seen[6].body, seen[9].body,
            "retry must not change text, IDs or attachments"
        );
        let sent: Value = serde_json::from_slice(&seen[9].body).unwrap();
        assert_eq!(
            sent,
            json!({"clientMessageId":operation.client_id,"text":"Receipt 🎉","attachmentIds":["asset1"],"spoilerAttachmentIds":["asset1"]})
        );
        assert_eq!(
            seen.iter()
                .filter(|request| request.line == "POST /api/assets HTTP/1.1")
                .count(),
            1
        );
    }

    #[test]
    fn voice_messages_upload_as_recorded_and_send_alone() {
        let prepared = crate::voice_note::prepare(&[4_000; 48_000], None).unwrap();
        let size = prepared.size;
        let mut operation = operation(vec![File::voice(Arc::new(prepared))]);
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

    #[test]
    fn edited_files_upload_the_edit_and_unappliable_edits_send_nothing() {
        use crate::attachment_edit::{Crop, Edit};
        let picture = uploads::Scratch::new("png");
        image::RgbaImage::from_fn(64, 32, |x, _| {
            image::Rgba(if x < 32 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            })
        })
        .save(picture.path())
        .unwrap();
        let left = Edit {
            crop: Crop {
                x: 0.0,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
            ..Edit::default()
        };
        let mut file = File::picked(picture.path().to_owned());
        assert_eq!(file.editable, Some(crate::attachment_edit::Kind::Image));
        file.set_edit(Some(left.clone()), None);
        let mut edited = operation(vec![file]);
        let client_id = edited.client_id.clone();
        let (api, requests) = server(move |base| {
            vec![
                (200, format!("{{\"conversations\":[{DIRECT}]}}")),
                (200, json!({"token":"chat","author":{"id":"owner","name":"Owner","isGuest":false}}).to_string()),
                (201, json!({"id":"asset1","upload":{"method":"PUT","url":format!("{base}/storage/asset1"),"headers":{}}}).to_string()),
                (200, String::new()),
                (200, json!({"id":"asset1","kind":"image","contentType":"image/png","name":"left.png","size":1}).to_string()),
                (200, json!({"id":"m1","channelId":"dm0000000002","seq":"1","createdAt":"2026-10-11T00:00:00Z","clientMessageId":client_id,"author":{"id":"owner","name":"Owner","isGuest":false},"content":{"version":1,"type":"text","text":"Receipt 🎉"}}).to_string()),
            ]
        });
        edited.run(&api, "account", &|_| {}).unwrap();
        let seen = requests.join().unwrap();
        let reservation: Value = serde_json::from_slice(&seen[2].body).unwrap();
        assert_eq!(
            (
                reservation["width"].as_u64(),
                reservation["height"].as_u64()
            ),
            (Some(32), Some(32))
        );
        let stored = image::load_from_memory(&seen[3].body).unwrap().to_rgba8();
        assert_eq!(stored.dimensions(), (32, 32));
        assert!(
            stored.pixels().all(|pixel| pixel.0 == [255, 0, 0, 255]),
            "only the kept half"
        );
        // An edit on something that cannot take it fails before any reservation.
        let notes = uploads::Scratch::new("txt");
        std::fs::write(notes.path(), b"notes").unwrap();
        let mut file = File::picked(notes.path().to_owned());
        assert_eq!(file.editable, None);
        file.set_edit(Some(left), None);
        let mut refused = operation(vec![file]);
        let (api, requests) = server(|_| {
            vec![
                (200, format!("{{\"conversations\":[{DIRECT}]}}")),
                (
                    200,
                    json!({"token":"chat","author":{"id":"owner","name":"Owner","isGuest":false}})
                        .to_string(),
                ),
            ]
        });
        let error = refused.run(&api, "account", &|_| {}).unwrap_err();
        assert_eq!(error.message, crate::attachment_edit::FAILED);
        assert_eq!(
            requests.join().unwrap().len(),
            2,
            "nothing is reserved or sent"
        );
    }

    #[test]
    fn incoming_and_blocked_directs_refuse_before_creation_upload_or_send() {
        for (status, blocked, expected) in [
            (
                "incoming",
                false,
                "Open conversation and accept this request before sending.",
            ),
            ("accepted", true, "Unblock this person to message them."),
        ] {
            for (retrying, upload_only) in [(false, false), (true, false), (false, true)] {
                let (api, requests) = server(move |_| {
                    vec![(200, json!({"conversations":[{
                "id":"dm0000000002","peer":{"id":"alex-id","username":"alex","displayName":"Alex"},"lastSeq":"0","readSeq":"0","status":status,"blocked":blocked
            }]}).to_string())]
                });
                let mut operation = operation(vec![File::picked(PathBuf::from("not-read.txt"))]);
                operation.retrying = retrying;
                let error = operation
                    .run_inner(&api, "account", &|_| {}, upload_only)
                    .unwrap_err();
                assert_eq!(error.message, expected);
                assert_eq!(error.uncertain, retrying);
                assert_eq!(operation.text, "Receipt 🎉");
                assert_eq!(operation.files.len(), 1);
                assert!(operation.files[0].attachment.is_none());
                assert_eq!(requests.join().unwrap()[0].line, "GET /api/dms HTTP/1.1");
            }
        }
    }

    #[test]
    fn eager_upload_refuses_changed_recipient_and_cancelled_drafts() {
        let (api, requests) = server(|_| {
            vec![
                (200, json!({"conversations":[]}).to_string()),
                (201, DIRECT.replace("alex-id", "other-id")),
            ]
        });
        let mut send = operation(vec![File::picked("not-read.txt".into())]);
        let error = send.run_inner(&api, "account", &|_| {}, true).unwrap_err();
        assert!(error.message.contains("username changed"));
        assert_eq!(requests.join().unwrap().len(), 2);
        send.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            send.run_inner(&api, "account", &|_| {}, true)
                .unwrap_err()
                .message,
            "Send cancelled."
        );
    }

    #[test]
    fn mismatched_confirmations_and_later_lookup_refusals_keep_retry_payload() {
        for field in ["channelId", "clientMessageId", "author"] {
            let mut operation = operation(Vec::new());
            let client_id = operation.client_id.clone();
            let (api, requests) = server(move |_| {
                let confirmed = json!({
                    "id":"m1","channelId":"dm0000000002","seq":"1",
                    "createdAt":"2026-10-11T00:00:00Z","clientMessageId":client_id,
                    "author":{"id":"owner","name":"Owner","isGuest":false},
                    "content":{"version":1,"type":"text","text":"Receipt 🎉"}
                });
                let mut wrong = confirmed.clone();
                match field {
                    "channelId" => wrong["channelId"] = json!("dm0000000003"),
                    "clientMessageId" => {
                        wrong["clientMessageId"] = json!(uuid::Uuid::new_v4().to_string())
                    }
                    "author" => wrong["author"]["id"] = json!("someone-else"),
                    _ => unreachable!(),
                }
                vec![
                    (200, format!("{{\"conversations\":[{DIRECT}]}}")),
                    (200, json!({"token":"chat","author":{"id":"owner","name":"Owner","isGuest":false}}).to_string()),
                    (200, wrong.to_string()),
                    (401, json!({"error":"Sign in again"}).to_string()),
                    (200, format!("{{\"conversations\":[{DIRECT}]}}")),
                    (200, confirmed.to_string()),
                ]
            });
            let error = operation.run(&api, "account", &|_| {}).unwrap_err();
            assert!(error.uncertain, "wrong {field} is not confirmation");
            operation.retrying = true;
            let error = operation.run(&api, "account", &|_| {}).unwrap_err();
            assert!(error.uncertain, "lookup 401 cannot disprove the prior POST");
            assert_eq!(
                operation.run(&api, "account", &|_| {}).unwrap().id,
                "dm0000000002"
            );
            let seen = requests.join().unwrap();
            assert_eq!(seen[2].body, seen[5].body);
            let sent: Value = serde_json::from_slice(&seen[5].body).unwrap();
            assert_eq!(sent["clientMessageId"], operation.client_id);
        }
    }
}
