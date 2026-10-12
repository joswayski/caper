//! Local, per-person profile drafts. Only explicit Send or Open contacts the API.
use crate::{api::Api, compress::Compression, model, uploads};
use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, atomic::AtomicBool};

#[derive(Clone)]
pub struct File {
    pub path: PathBuf,
    /// Pasted images belong to the draft, including while a worker reads them.
    pub scratch: Option<Arc<uploads::Scratch>>,
    pub attachment: Option<model::Attachment>,
    /// Bytes on disk when staged, for the composer chip.
    pub size: u64,
    /// An image's decoded preview, filled off the UI thread by [File::thumbnail].
    pub preview: Arc<OnceLock<Option<egui::ColorImage>>>,
    pub texture: Option<egui::TextureHandle>,
    preview_requested: bool,
}

impl File {
    pub fn picked(path: PathBuf) -> Self {
        Self::staged(path, None)
    }

    fn staged(path: PathBuf, scratch: Option<Arc<uploads::Scratch>>) -> Self {
        Self {
            size: std::fs::metadata(&path).map_or(0, |metadata| metadata.len()),
            path,
            scratch,
            attachment: None,
            preview: Arc::default(),
            texture: None,
            preview_requested: false,
        }
    }

    /// The preview texture once decoded. The first call decodes an image
    /// file's thumbnail in the background; other files keep the file icon.
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
        if self.preview.get().is_some() || !is_image {
            return;
        }
        let (path, preview, context) = (self.path.clone(), self.preview.clone(), context.clone());
        std::thread::spawn(move || {
            let image = std::fs::read(&path)
                .ok()
                .and_then(|bytes| crate::attachments::decode_thumbnail(&bytes));
            let _ = preview.set(image);
            context.request_repaint();
        });
    }

    pub fn name(&self) -> String {
        if self.scratch.is_some() {
            "Pasted image.png".into()
        } else {
            self.path
                .file_name()
                .map_or_else(|| "file".into(), |name| name.to_string_lossy().into_owned())
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
}

impl Draft {
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
    pub direct: Option<model::DirectConversation>,
}

#[derive(Debug)]
pub struct Failure {
    pub message: String,
    pub uncertain: bool,
}

impl Send {
    /// Resolves only on explicit Send/Open. Reuses uploaded IDs after a
    /// failed write and uses the same compression/reservation/PUT pipeline.
    pub fn run(
        &mut self,
        api: &Api,
        token: &str,
        status: &dyn Fn(String),
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
        let session = api
            .chat_session(Some(token), &self.name)
            .map_err(lookup_failure)?;
        for (index, file) in self.files.iter_mut().enumerate() {
            cancelled()?;
            if file.attachment.is_some() {
                continue;
            }
            let settings = self.settings.as_ref().ok_or_else(|| Failure {
                message: "File uploads are not ready. Try again shortly.".into(),
                uncertain: retrying,
            })?;
            status(format!(
                "Preparing and uploading {} ({})…",
                file.name(),
                index + 1
            ));
            let mut prepared =
                uploads::prepare_path_with(&file.path, settings, &|_| {}, &self.cancel).map_err(
                    |message| Failure {
                        message,
                        uncertain: retrying,
                    },
                )?;
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
        }
        cancelled()?;
        status("Sending…".into());
        let ids = self
            .files
            .iter()
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
                (None, false),
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
    fn profile_send_uploads_once_and_uncertain_retry_keeps_the_exact_payload() {
        let scratch = uploads::Scratch::new("txt");
        std::fs::write(scratch.path(), b"receipt").unwrap();
        let mut operation = operation(vec![File::picked(scratch.path().to_owned())]);
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
            json!({"clientMessageId":operation.client_id,"text":"Receipt 🎉","attachmentIds":["asset1"]})
        );
        assert_eq!(
            seen.iter()
                .filter(|request| request.line == "POST /api/assets HTTP/1.1")
                .count(),
            1
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
            for retrying in [false, true] {
                let (api, requests) = server(move |_| {
                    vec![(200, json!({"conversations":[{
                "id":"dm0000000002","peer":{"id":"alex-id","username":"alex","displayName":"Alex"},"lastSeq":"0","readSeq":"0","status":status,"blocked":blocked
            }]}).to_string())]
                });
                let mut operation = operation(vec![File::picked(PathBuf::from("not-read.txt"))]);
                operation.retrying = retrying;
                let error = operation.run(&api, "account", &|_| {}).unwrap_err();
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
