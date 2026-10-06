//! Sending attachments: reserve the original's exact size, PUT it straight
//! to storage, confirm, and return the description sent with `attachmentIds`.
//! Clients never compress: the server-side media worker processes every
//! upload (docs/media.md, "Uploads and attachments").

use crate::api::Api;
use crate::model::{Attachment, AttachmentKind};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const MAX_ATTACHMENTS: usize = 10;
/// Waits before re-asking `complete` after a 409 (the upload has not
/// arrived in storage yet).
const COMPLETE_RETRY_DELAYS: [Duration; 4] = [
    Duration::from_millis(500),
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadError {
    pub message: String,
    pub storage_full: bool,
    pub status: Option<u16>,
}

impl UploadError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            storage_full: false,
            status: None,
        }
    }
}

/// One picked or dropped file, uploaded unchanged.
pub struct Prepared {
    pub name: String,
    pub content_type: String,
    pub size: u64,
    pub path: PathBuf,
}

impl Prepared {
    /// For the draft chip only; the server decides the stored kind.
    pub fn kind(&self) -> AttachmentKind {
        kind_for(&self.content_type)
    }
}

pub fn kind_for(content_type: &str) -> AttachmentKind {
    match content_type.split('/').next() {
        Some("image") => AttachmentKind::Image,
        Some("video") => AttachmentKind::Video,
        Some("audio") => AttachmentKind::Audio,
        _ => AttachmentKind::File,
    }
}

/// Desktop files have names, not browser MIME types. The declared type is
/// display-only until processed: the worker detects the real type from the
/// bytes, so a guess from the extension is enough.
pub fn content_type_for(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" | "jfif" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "heic" => "image/heic",
        "heif" => "image/heif",
        "bmp" => "image/bmp",
        "svg" => "image/svg+xml",
        "tif" | "tiff" => "image/tiff",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" | "qt" => "video/quicktime",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/x-m4a",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "aif" | "aiff" => "audio/aiff",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "json" => "application/json",
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        _ => "application/octet-stream",
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "file".into())
}

/// Check one picked or dropped file before anything is reserved.
pub fn prepare_path(path: &Path, max_upload_bytes: Option<u64>) -> Result<Prepared, String> {
    let name = file_name(path);
    let metadata = std::fs::metadata(path).map_err(|_| format!("Could not read {name}."))?;
    if !metadata.is_file() {
        return Err(format!("{name} is not a file."));
    }
    let size = metadata.len();
    if size == 0 {
        return Err(format!("{name} is empty."));
    }
    if let Some(limit) = max_upload_bytes.filter(|limit| size > *limit) {
        return Err(format!(
            "This file is larger than the {} upload limit.",
            format_bytes(limit)
        ));
    }
    Ok(Prepared {
        content_type: content_type_for(&name).into(),
        name,
        size,
        path: path.to_owned(),
    })
}

/// The reservation request (`POST /api/assets`).
pub fn reservation(channel: &str, prepared: &Prepared) -> Value {
    json!({
        "channelId": channel,
        "filename": prepared.name,
        "contentType": prepared.content_type,
        "byteSize": prepared.size,
    })
}

struct PresignedPut {
    url: String,
    headers: BTreeMap<String, String>,
}

fn presigned(value: &Value) -> Option<PresignedPut> {
    if value["method"]
        .as_str()
        .is_some_and(|method| method != "PUT")
    {
        return None;
    }
    let url = value["url"].as_str()?.to_owned();
    let headers = value["headers"]
        .as_object()
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    Some(PresignedPut { url, headers })
}

/// Counts bytes as reqwest reads the body, and stops a removed upload.
struct Progress<R> {
    inner: R,
    sent: u64,
    report: Arc<dyn Fn(u64) + Send + Sync>,
    cancel: Arc<AtomicBool>,
}

impl<R: Read> Read for Progress<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::other("upload cancelled"));
        }
        let read = self.inner.read(buffer)?;
        self.sent += read as u64;
        (self.report)(self.sent);
        Ok(read)
    }
}

fn cancelled(cancel: &AtomicBool) -> Result<(), UploadError> {
    if cancel.load(Ordering::Relaxed) {
        Err(UploadError::new("Upload cancelled."))
    } else {
        Ok(())
    }
}

/// Reserve, stream the original from disk straight to storage, then confirm.
/// `progress` receives 0–1 of the original's bytes. The result is usually
/// `status: processing`; the message can be sent right away.
pub fn upload(
    api: &Api,
    token: &str,
    channel: &str,
    prepared: Prepared,
    progress: Arc<dyn Fn(f32) + Send + Sync>,
    cancel: Arc<AtomicBool>,
) -> Result<Attachment, UploadError> {
    upload_with_delays(
        api,
        token,
        channel,
        prepared,
        progress,
        cancel,
        &COMPLETE_RETRY_DELAYS,
    )
}

fn upload_with_delays(
    api: &Api,
    token: &str,
    channel: &str,
    prepared: Prepared,
    progress: Arc<dyn Fn(f32) + Send + Sync>,
    cancel: Arc<AtomicBool>,
    delays: &[Duration],
) -> Result<Attachment, UploadError> {
    let invalid = || UploadError::new("The upload service returned an invalid response.");
    cancelled(&cancel)?;
    // Open first so an unreadable file never holds a reservation.
    let file = std::fs::File::open(&prepared.path)
        .map_err(|_| UploadError::new(format!("Could not read {}.", prepared.name)))?;
    let reserved = api.create_asset(token, reservation(channel, &prepared))?;
    let id = reserved["id"]
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
        .ok_or_else(invalid)?
        .to_owned();
    let upload = presigned(&reserved["upload"]).ok_or_else(invalid)?;
    cancelled(&cancel)?;
    let total = prepared.size.max(1) as f32;
    let report: Arc<dyn Fn(u64) + Send + Sync> = {
        let progress = progress.clone();
        Arc::new(move |sent| progress(sent as f32 / total))
    };
    // Exactly the reserved size, which the URL signs: a file that grew since
    // is cut, one that shrank fails verification.
    let body = reqwest::blocking::Body::sized(
        Progress {
            inner: file.take(prepared.size),
            sent: 0,
            report,
            cancel: cancel.clone(),
        },
        prepared.size,
    );
    api.put_presigned(&upload.url, &upload.headers, body)
        .map_err(|error| {
            if cancel.load(Ordering::Relaxed) {
                UploadError::new("Upload cancelled.")
            } else {
                error
            }
        })?;
    let mut delays = delays.iter();
    let completed = loop {
        cancelled(&cancel)?;
        match api.complete_asset(token, &id) {
            Err(error) if error.status == Some(409) => match delays.next() {
                Some(delay) => std::thread::sleep(*delay),
                None => return Err(error),
            },
            result => break result?,
        }
    };
    let attachment = Attachment::parse(&completed)
        .filter(|attachment| attachment.id == id)
        .ok_or_else(invalid)?;
    progress(1.0);
    Ok(attachment)
}

/// Web `formatBytes`.
pub fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let units = ["KB", "MB", "GB"];
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < units.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 10.0 {
        format!("{} {}", value.round(), units[unit])
    } else {
        format!("{value:.1} {}", units[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    #[derive(Debug)]
    struct Request {
        line: String,
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
    }

    /// A loopback HTTP/1.1 server answering requests in order and recording
    /// them. Responses are built from the server's own base URL, so presigned
    /// storage URLs can point back at it like a local R2 fake.
    fn serve(
        responses: impl FnOnce(&str) -> Vec<(u16, String)>,
    ) -> (String, std::thread::JoinHandle<Vec<Request>>) {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.local_addr().unwrap());
        let responses = responses(&base);
        let handle = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, body) in responses {
                let (stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let mut headers = BTreeMap::new();
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header == "\r\n" {
                        break;
                    }
                    let (name, value) = header.trim_end().split_once(':').unwrap();
                    headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
                }
                let length = headers
                    .get("content-length")
                    .map_or(0, |length| length.parse().unwrap());
                let mut content = vec![0; length];
                reader.read_exact(&mut content).unwrap();
                write!(
                    reader.get_mut(),
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                requests.push(Request {
                    line: line.trim_end().to_owned(),
                    headers,
                    body: content,
                });
            }
            requests
        });
        (base, handle)
    }

    /// A scratch directory removed when dropped.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "caper-upload-{label}-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn reserved(base: &str) -> String {
        json!({
            "id": "asset0001",
            "kind": "image",
            "upload": {"method": "PUT", "url": format!("{base}/s3/incoming/asset0001?X-Amz-Signature=s"), "headers": {
                "content-type": "image/heic"
            }},
            "storage": {"used": 23, "limit": 1000}
        })
        .to_string()
    }

    fn processing() -> String {
        json!({
            "id": "asset0001", "kind": "image", "contentType": "image/heic",
            "name": "IMG_0001.HEIC", "size": 17, "status": "processing"
        })
        .to_string()
    }

    #[test]
    fn upload_reserves_the_original_puts_it_unchanged_and_retries_complete() {
        let scratch = Scratch::new("flow");
        let path = scratch.file("IMG_0001.HEIC", b"original heic data");
        let prepared = prepare_path(&path, Some(1 << 30)).unwrap();
        assert_eq!(prepared.content_type, "image/heic");
        assert_eq!(prepared.kind(), AttachmentKind::Image);
        let (base, server) = serve(|base| {
            vec![
                (201, reserved(base)),
                (200, String::new()),
                (409, r#"{"error":"upload has not arrived"}"#.into()),
                (409, r#"{"error":"upload has not arrived"}"#.into()),
                (200, processing()),
            ]
        });
        let api = Api::new(&base).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let progress: Arc<dyn Fn(f32) + Send + Sync> = {
            let seen = seen.clone();
            Arc::new(move |value| seen.lock().unwrap().push(value))
        };
        let attachment = upload_with_delays(
            &api,
            "account-token",
            "chan00000001",
            prepared,
            progress,
            Arc::new(AtomicBool::new(false)),
            &[Duration::from_millis(1); 3],
        )
        .unwrap();
        assert_eq!(attachment.id, "asset0001");
        assert_eq!(
            attachment.status,
            crate::model::AttachmentStatus::Processing
        );
        let requests = server.join().unwrap();
        let lines: Vec<_> = requests
            .iter()
            .map(|request| request.line.as_str())
            .collect();
        assert_eq!(
            lines,
            [
                "POST /api/assets HTTP/1.1",
                "PUT /s3/incoming/asset0001?X-Amz-Signature=s HTTP/1.1",
                "POST /api/assets/asset0001/complete HTTP/1.1",
                "POST /api/assets/asset0001/complete HTTP/1.1",
                "POST /api/assets/asset0001/complete HTTP/1.1",
            ]
        );
        // Exactly the original's size and the extension's type, nothing else.
        let reservation: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            reservation,
            json!({
                "channelId": "chan00000001", "filename": "IMG_0001.HEIC",
                "contentType": "image/heic", "byteSize": 18
            })
        );
        for api_request in [&requests[0], &requests[2], &requests[4]] {
            assert_eq!(
                api_request.headers.get("authorization").map(String::as_str),
                Some("Bearer account-token")
            );
        }
        // The storage PUT carries exactly the returned headers, the exact
        // length and the unchanged bytes, and never a Caper credential.
        let put = &requests[1];
        assert_eq!(put.body, b"original heic data");
        assert_eq!(put.headers["content-type"], "image/heic");
        assert_eq!(put.headers["content-length"], "18");
        assert!(!put.headers.contains_key("authorization"));
        assert!(!put.headers.contains_key("x-caper-chat-token"));
        assert!(!put.headers.contains_key("transfer-encoding"));
        assert!(!put.headers.contains_key("cookie"));
        let progress = seen.lock().unwrap();
        assert_eq!(progress.last(), Some(&1.0));
        assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn complete_gives_up_after_its_retries_and_reports_size_mismatch() {
        let scratch = Scratch::new("complete");
        let path = scratch.file("a.bin", b"x");
        let (base, server) = serve(|base| {
            vec![
                (201, reserved(base)),
                (200, String::new()),
                (409, r#"{"error":"upload has not arrived"}"#.into()),
                (409, r#"{"error":"upload has not arrived"}"#.into()),
                (201, reserved(base)),
                (200, String::new()),
                (422, r#"{"error":"uploaded size does not match"}"#.into()),
            ]
        });
        let api = Api::new(&base).unwrap();
        let attempt = || {
            upload_with_delays(
                &api,
                "token",
                "chan",
                prepare_path(&path, None).unwrap(),
                Arc::new(|_| {}),
                Arc::new(AtomicBool::new(false)),
                &[Duration::from_millis(1)],
            )
            .unwrap_err()
        };
        let pending = attempt();
        assert_eq!(pending.status, Some(409));
        assert_eq!(pending.message, "upload has not arrived");
        let mismatch = attempt();
        assert_eq!(mismatch.status, Some(422));
        assert_eq!(server.join().unwrap().len(), 7);
    }

    #[test]
    fn storage_full_and_server_errors_are_explicit() {
        let scratch = Scratch::new("errors");
        let path = scratch.file("a.txt", b"x");
        let (base, server) = serve(|_| {
            vec![
                (
                    413,
                    r#"{"error":"storage limit reached","code":"storage_full"}"#.into(),
                ),
                (413, r#"{"error":"too big"}"#.into()),
                (
                    429,
                    r#"{"error":"uploading too quickly; try again shortly"}"#.into(),
                ),
            ]
        });
        let api = Api::new(&base).unwrap();
        let attempt = || {
            upload(
                &api,
                "token",
                "chan00000001",
                prepare_path(&path, None).unwrap(),
                Arc::new(|_| {}),
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap_err()
        };
        let full = attempt();
        assert!(full.storage_full);
        assert_eq!(full.message, "You’ve used all of your file storage.");
        let large = attempt();
        assert!(!large.storage_full);
        assert_eq!(large.message, "This file is too large to upload.");
        assert_eq!(
            attempt().message,
            "uploading too quickly; try again shortly"
        );
        assert_eq!(server.join().unwrap().len(), 3);
    }

    #[test]
    fn cancelled_uploads_never_reserve_and_bad_storage_urls_are_refused() {
        let scratch = Scratch::new("cancel");
        let api = Api::new("http://127.0.0.1:9").unwrap();
        let cancelled = upload(
            &api,
            "token",
            "chan",
            prepare_path(&scratch.file("a.txt", b"x"), None).unwrap(),
            Arc::new(|_| {}),
            Arc::new(AtomicBool::new(true)),
        )
        .unwrap_err();
        assert_eq!(cancelled.message, "Upload cancelled.");
        let refused = api
            .put_presigned(
                "http://storage.example/original/x",
                &BTreeMap::new(),
                reqwest::blocking::Body::from(vec![1]),
            )
            .unwrap_err();
        assert_eq!(
            refused.message,
            "The upload service returned an invalid storage address."
        );
    }

    #[test]
    fn files_are_checked_against_the_upload_limit_before_reserving() {
        let scratch = Scratch::new("limits");
        let path = scratch.file("clip.mov", &[7; 2048]);
        let prepared = prepare_path(&path, Some(2048)).unwrap();
        assert_eq!(prepared.size, 2048);
        assert_eq!(prepared.content_type, "video/quicktime");
        assert_eq!(
            prepare_path(&path, Some(2047)).err().as_deref(),
            Some("This file is larger than the 2.0 KB upload limit.")
        );
        assert_eq!(
            prepare_path(&scratch.file("empty.txt", b""), None).err(),
            Some("empty.txt is empty.".into())
        );
        assert_eq!(
            prepare_path(&scratch.0, None)
                .err()
                .map(|error| error.ends_with("is not a file.")),
            Some(true)
        );
    }

    #[test]
    fn content_types_come_from_the_extension() {
        assert_eq!(content_type_for("photo.JPG"), "image/jpeg");
        assert_eq!(content_type_for("anim.gif"), "image/gif");
        assert_eq!(content_type_for("scan.tiff"), "image/tiff");
        assert_eq!(content_type_for("take.MOV"), "video/quicktime");
        assert_eq!(content_type_for("memo.wav"), "audio/wav");
        assert_eq!(content_type_for("notes.md"), "text/plain");
        assert_eq!(
            content_type_for("archive.tar.zst"),
            "application/octet-stream"
        );
        assert_eq!(content_type_for("README"), "application/octet-stream");
        assert_eq!(kind_for("application/pdf"), AttachmentKind::File);
        assert_eq!(kind_for("audio/flac"), AttachmentKind::Audio);
    }

    #[test]
    fn send_includes_attachment_ids_only_when_present() {
        let message = |text: &str| {
            json!({
                "id": "m1", "channelId": "chan", "seq": "1", "createdAt": "2026-10-03T00:00:00Z",
                "clientMessageId": "client", "author": {"id": "u", "name": "U", "isGuest": false},
                "content": {"version": 1, "type": "text", "text": text, "attachments": [
                    {"id": "asset0001", "kind": "image", "contentType": "image/png", "name": "a.png", "size": 4,
                     "status": "processing"}
                ]}
            })
            .to_string()
        };
        let (base, server) = serve(|_| vec![(200, message("")), (200, message("hi"))]);
        let api = Api::new(&base).unwrap();
        let sent = api
            .send(
                Some("token"),
                "chat",
                "chan",
                "client",
                "",
                &["asset0001".into()],
            )
            .unwrap();
        assert_eq!(sent.content.attachments.len(), 1);
        api.send(Some("token"), "chat", "chan", "client", "hi", &[])
            .unwrap();
        let requests = server.join().unwrap();
        let first: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            first,
            json!({"clientMessageId": "client", "text": "", "attachmentIds": ["asset0001"]})
        );
        let second: Value = serde_json::from_slice(&requests[1].body).unwrap();
        assert_eq!(second, json!({"clientMessageId": "client", "text": "hi"}));
    }

    #[test]
    fn attachment_url_refresh_and_usage_are_parsed_tolerantly() {
        let (base, server) = serve(|_| {
            vec![
                (
                    200,
                    json!({"urls": {
                        "a": {"url": "https://cdn.caper.chat/original/a?exp=9&sig=s", "previewUrl": "https://cdn.caper.chat/preview/a?exp=9&sig=p"},
                        "processing": {"previewUrl": "https://cdn.caper.chat/preview/p?exp=9&sig=p"},
                        "b": {"url": "javascript:alert(1)"},
                        "c": {"nope": true}
                    }})
                    .to_string(),
                ),
                (
                    200,
                    json!({"used": 1, "limit": 2, "maxUploadBytes": 2147483648_u64}).to_string(),
                ),
                (200, json!({"used": 1, "limit": 2}).to_string()),
                (503, r#"{"error":"uploads are not configured"}"#.into()),
            ]
        });
        let api = Api::new(&base).unwrap();
        let urls = api
            .attachment_urls(
                "token",
                &["a".into(), "processing".into(), "b".into(), "c".into()],
            )
            .unwrap();
        assert_eq!(urls.keys().collect::<Vec<_>>(), ["a", "processing"]);
        assert_eq!(urls["processing"].url, None);
        let usage = api.asset_usage("token").unwrap();
        assert_eq!(usage.max_upload_bytes, Some(2_147_483_648));
        assert_eq!(api.asset_usage("token").unwrap().max_upload_bytes, None);
        assert!(api.asset_usage("token").is_err());
        let requests = server.join().unwrap();
        assert_eq!(requests[0].line, "POST /api/assets/urls HTTP/1.1");
        assert_eq!(requests[1].line, "GET /api/assets/usage HTTP/1.1");
    }

    #[test]
    fn sizes_read_like_the_web_client() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(143 * 1024), "143 KB");
        assert_eq!(format_bytes(1_677_722), "1.6 MB");
        assert_eq!(format_bytes(2_147_483_648), "2.0 GB");
    }
}
