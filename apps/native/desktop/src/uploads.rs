//! Sending attachments: prepare a file, reserve it, PUT the bytes straight to
//! storage, confirm, and return the description sent with `attachmentIds`.
//! Mirrors `apps/web/src/chat/uploads.ts` (`prepareFile`, `uploadPrepared`).
//! Desktop has no bundled video transcoder: videos upload unchanged, with
//! dimensions and duration read from MP4/QuickTime headers when present.

use crate::api::Api;
use crate::compress::{self, Compression};
use crate::model::{Attachment, AttachmentKind};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Stills larger than this upload unchanged instead of being read for decoding.
const MAX_IMAGE_READ_BYTES: u64 = 200 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadError {
    pub message: String,
    pub storage_full: bool,
}

impl UploadError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            storage_full: false,
        }
    }
}

pub enum Body {
    Memory(Vec<u8>),
    /// Streamed from disk so large files never sit in memory.
    File(PathBuf),
}

pub struct Prepared {
    pub name: String,
    pub content_type: String,
    pub kind: AttachmentKind,
    pub source_size: u64,
    pub size: u64,
    pub body: Body,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    /// Always JPEG, at most 512 KiB.
    pub preview: Option<Vec<u8>>,
    pub thumbnail: Option<image::RgbaImage>,
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "file".into())
}

/// Read, classify and compress one picked or dropped file. Runs off the UI thread.
pub fn prepare_path(path: &Path, settings: &Compression) -> Result<Prepared, String> {
    let name = file_name(path);
    let metadata = std::fs::metadata(path).map_err(|_| format!("Could not read {name}."))?;
    if !metadata.is_file() {
        return Err(format!("{name} is not a file."));
    }
    let size = metadata.len();
    if size == 0 {
        return Err(format!("{name} is empty."));
    }
    let mut file = std::fs::File::open(path).map_err(|_| format!("Could not read {name}."))?;
    let mut head = [0; 64];
    let read = file.read(&mut head).unwrap_or(0);
    let content_type = compress::content_type_for(&name, &head[..read]);
    if content_type.starts_with("image/") && size <= MAX_IMAGE_READ_BYTES {
        let bytes = std::fs::read(path).map_err(|_| format!("Could not read {name}."))?;
        return Ok(prepare_bytes(&name, bytes, settings));
    }
    let mut prepared = Prepared {
        kind: compress::attachment_kind(&content_type),
        name,
        content_type,
        source_size: size,
        size,
        body: Body::File(path.to_owned()),
        width: None,
        height: None,
        duration_ms: None,
        preview: None,
        thumbnail: None,
    };
    if matches!(
        prepared.content_type.as_str(),
        "video/mp4" | "video/quicktime"
    ) {
        let info = compress::probe_mp4(&mut file);
        prepared.width = info.width;
        prepared.height = info.height;
        prepared.duration_ms = info.duration_ms;
    }
    Ok(prepared)
}

/// Prepare in-memory bytes (stills are compressed with the server settings).
pub fn prepare_bytes(name: &str, bytes: Vec<u8>, settings: &Compression) -> Prepared {
    let head = &bytes[..bytes.len().min(64)];
    let content_type = compress::content_type_for(name, head);
    let source_size = bytes.len() as u64;
    if content_type.starts_with("image/") {
        let image = compress::prepare_image(name, bytes, &content_type, settings);
        return Prepared {
            kind: compress::attachment_kind(&image.content_type),
            name: image.name,
            content_type: image.content_type,
            source_size,
            size: image.bytes.len() as u64,
            body: Body::Memory(image.bytes),
            width: image.width,
            height: image.height,
            duration_ms: None,
            preview: image.preview,
            thumbnail: image.thumbnail,
        };
    }
    Prepared {
        kind: compress::attachment_kind(&content_type),
        name: name.into(),
        content_type,
        source_size,
        size: source_size,
        body: Body::Memory(bytes),
        width: None,
        height: None,
        duration_ms: None,
        preview: None,
        thumbnail: None,
    }
}

/// The reservation request (`POST /api/assets`).
pub fn reservation(channel: &str, prepared: &Prepared) -> Value {
    let mut body = json!({
        "channelId": channel,
        "filename": prepared.name,
        "contentType": prepared.content_type,
        "byteSize": prepared.size,
    });
    if prepared.source_size > 0 {
        body["sourceByteSize"] = json!(prepared.source_size);
    }
    // The API accepts 1..=32768 pixels and up to 24 hours.
    let dimension = |value: Option<u32>| value.filter(|value| (1..=32_768).contains(value));
    if let (Some(width), Some(height)) = (dimension(prepared.width), dimension(prepared.height)) {
        body["width"] = json!(width);
        body["height"] = json!(height);
    }
    if let Some(duration) = prepared
        .duration_ms
        .filter(|duration| *duration <= 24 * 60 * 60 * 1000)
    {
        body["durationMs"] = json!(duration);
    }
    if let Some(preview) = &prepared.preview {
        body["preview"] = json!({"contentType": "image/jpeg", "byteSize": preview.len()});
    }
    body
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

/// Reserve, upload straight to storage (preview first), then confirm.
/// `progress` receives 0–1 across preview and original bytes.
pub fn upload(
    api: &Api,
    token: &str,
    channel: &str,
    prepared: Prepared,
    progress: Arc<dyn Fn(f32) + Send + Sync>,
    cancel: Arc<AtomicBool>,
) -> Result<Attachment, UploadError> {
    let invalid = || UploadError::new("The upload service returned an invalid response.");
    cancelled(&cancel)?;
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
    let preview_size = prepared.preview.as_ref().map_or(0, Vec::len) as u64;
    let total = (prepared.size + preview_size).max(1) as f32;
    if let (Some(preview), Some(put)) = (&prepared.preview, presigned(&reserved["previewUpload"])) {
        cancelled(&cancel)?;
        let body = Progress {
            inner: std::io::Cursor::new(preview.clone()),
            sent: 0,
            report: Arc::new(|_| {}),
            cancel: cancel.clone(),
        };
        api.put_presigned(
            &put.url,
            &put.headers,
            reqwest::blocking::Body::sized(body, preview_size),
        )?;
    }
    cancelled(&cancel)?;
    let report: Arc<dyn Fn(u64) + Send + Sync> = {
        let progress = progress.clone();
        Arc::new(move |sent| progress((preview_size + sent) as f32 / total))
    };
    let body = match prepared.body {
        Body::Memory(bytes) => reqwest::blocking::Body::sized(
            Progress {
                inner: std::io::Cursor::new(bytes),
                sent: 0,
                report,
                cancel: cancel.clone(),
            },
            prepared.size,
        ),
        Body::File(path) => {
            let file = std::fs::File::open(&path)
                .map_err(|_| UploadError::new(format!("Could not read {}.", prepared.name)))?;
            // Exactly the reserved size: a file that grew since is cut, one
            // that shrank fails verification.
            reqwest::blocking::Body::sized(
                Progress {
                    inner: file.take(prepared.size),
                    sent: 0,
                    report,
                    cancel: cancel.clone(),
                },
                prepared.size,
            )
        }
    };
    api.put_presigned(&upload.url, &upload.headers, body)
        .map_err(|error| {
            if cancel.load(Ordering::Relaxed) {
                UploadError::new("Upload cancelled.")
            } else {
                error
            }
        })?;
    cancelled(&cancel)?;
    let completed = api.complete_asset(token, &id)?;
    let attachment = Attachment::parse(&completed)
        .filter(|attachment| attachment.id == id)
        .ok_or_else(invalid)?;
    progress(1.0);
    Ok(attachment)
}

/// "X MB → Y KB" when compression saved space, otherwise the stored size.
pub fn size_label(source: u64, stored: Option<u64>) -> String {
    match stored {
        Some(stored) if stored < source => {
            format!("{} → {}", format_bytes(source), format_bytes(stored))
        }
        Some(stored) => format_bytes(stored),
        None => format_bytes(source),
    }
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

    fn text_file_with_preview() -> Prepared {
        let mut prepared = prepare_bytes(
            "notes.txt",
            b"hello attachments".to_vec(),
            &Compression::default(),
        );
        prepared.preview = Some(vec![0xff, 0xd8, 0xff, 1, 2, 3]);
        prepared.width = Some(1920);
        prepared.height = Some(1080);
        prepared
    }

    #[test]
    fn upload_reserves_puts_preview_then_original_and_completes() {
        let (base, server) = serve(|base| {
            let reserved = json!({
                "id": "asset0001",
                "kind": "file",
                "upload": {"method": "PUT", "url": format!("{base}/r2/original/asset0001?X-Amz-Signature=s"), "headers": {
                    "content-type": "text/plain",
                    "content-disposition": "attachment; filename=\"notes.txt\""
                }},
                "previewUpload": {"method": "PUT", "url": format!("{base}/r2/preview/asset0001?X-Amz-Signature=p"), "headers": {
                    "content-type": "image/jpeg"
                }},
                "storage": {"used": 23, "limit": 1000}
            });
            let completed = json!({
                "id": "asset0001", "kind": "file", "contentType": "text/plain",
                "name": "notes.txt", "size": 17
            });
            vec![
                (201, reserved.to_string()),
                (200, String::new()),
                (200, String::new()),
                (200, completed.to_string()),
            ]
        });
        let api = Api::new(&base).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let progress: Arc<dyn Fn(f32) + Send + Sync> = {
            let seen = seen.clone();
            Arc::new(move |value| seen.lock().unwrap().push(value))
        };
        let attachment = upload(
            &api,
            "account-token",
            "chan00000001",
            text_file_with_preview(),
            progress,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(attachment.id, "asset0001");
        assert_eq!(attachment.kind, AttachmentKind::File);
        let requests = server.join().unwrap();
        let lines: Vec<_> = requests
            .iter()
            .map(|request| request.line.as_str())
            .collect();
        assert_eq!(
            lines,
            [
                "POST /api/assets HTTP/1.1",
                "PUT /r2/preview/asset0001?X-Amz-Signature=p HTTP/1.1",
                "PUT /r2/original/asset0001?X-Amz-Signature=s HTTP/1.1",
                "POST /api/assets/asset0001/complete HTTP/1.1",
            ]
        );
        let reservation: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            reservation,
            json!({
                "channelId": "chan00000001", "filename": "notes.txt", "contentType": "text/plain",
                "byteSize": 17, "sourceByteSize": 17, "width": 1920, "height": 1080,
                "preview": {"contentType": "image/jpeg", "byteSize": 6}
            })
        );
        for api_request in [&requests[0], &requests[3]] {
            assert_eq!(
                api_request.headers.get("authorization").map(String::as_str),
                Some("Bearer account-token")
            );
        }
        // Storage PUTs carry exactly the returned headers and the exact
        // length, and never a Caper credential.
        let preview = &requests[1];
        assert_eq!(preview.body, [0xff, 0xd8, 0xff, 1, 2, 3]);
        assert_eq!(preview.headers["content-type"], "image/jpeg");
        assert_eq!(preview.headers["content-length"], "6");
        let original = &requests[2];
        assert_eq!(original.body, b"hello attachments");
        assert_eq!(original.headers["content-type"], "text/plain");
        assert_eq!(original.headers["content-length"], "17");
        assert_eq!(
            original.headers["content-disposition"],
            "attachment; filename=\"notes.txt\""
        );
        for put in [preview, original] {
            assert!(!put.headers.contains_key("authorization"));
            assert!(!put.headers.contains_key("x-caper-chat-token"));
            assert!(!put.headers.contains_key("transfer-encoding"));
        }
        let progress = seen.lock().unwrap();
        assert_eq!(progress.last(), Some(&1.0));
        assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn storage_full_and_server_errors_are_explicit() {
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
                prepare_bytes("a.txt", b"x".to_vec(), &Compression::default()),
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
        let api = Api::new("http://127.0.0.1:9").unwrap();
        let cancelled = upload(
            &api,
            "token",
            "chan",
            prepare_bytes("a.txt", b"x".to_vec(), &Compression::default()),
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
    fn send_includes_attachment_ids_only_when_present() {
        let message = |text: &str| {
            json!({
                "id": "m1", "channelId": "chan", "seq": "1", "createdAt": "2026-10-03T00:00:00Z",
                "clientMessageId": "client", "author": {"id": "u", "name": "U", "isGuest": false},
                "content": {"version": 1, "type": "text", "text": text, "attachments": [
                    {"id": "asset0001", "kind": "image", "contentType": "image/png", "name": "a.png", "size": 4,
                     "url": "https://cdn.caper.chat/original/asset0001?exp=1&sig=s"}
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
                        "b": {"url": "javascript:alert(1)"},
                        "c": {"nope": true}
                    }})
                    .to_string(),
                ),
                (
                    200,
                    json!({"used": 1, "limit": 2, "compression": {"imageQuality": 70}}).to_string(),
                ),
                (503, r#"{"error":"uploads are not configured"}"#.into()),
            ]
        });
        let api = Api::new(&base).unwrap();
        let urls = api
            .attachment_urls("token", &["a".into(), "b".into(), "c".into()])
            .unwrap();
        assert_eq!(urls.keys().collect::<Vec<_>>(), ["a"]);
        assert_eq!(api.asset_usage("token").unwrap().image_quality, 70);
        assert!(api.asset_usage("token").is_err());
        let requests = server.join().unwrap();
        assert_eq!(requests[0].line, "POST /api/assets/urls HTTP/1.1");
        assert_eq!(
            serde_json::from_slice::<Value>(&requests[0].body).unwrap(),
            json!({"ids": ["a", "b", "c"]})
        );
        assert_eq!(requests[1].line, "GET /api/assets/usage HTTP/1.1");
    }

    #[test]
    fn video_and_large_files_stream_from_disk_with_header_metadata() {
        let directory = std::env::temp_dir().join(format!("caper-upload-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("clip.mp4");
        std::fs::write(&path, b"\0\0\0\x10ftypisom\0\0\0\0").unwrap();
        let prepared = prepare_path(&path, &Compression::default()).unwrap();
        assert_eq!(prepared.content_type, "video/mp4");
        assert_eq!(prepared.kind, AttachmentKind::Video);
        assert!(matches!(prepared.body, Body::File(_)));
        assert_eq!(prepared.size, 16);
        let empty = directory.join("empty.txt");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(
            prepare_path(&empty, &Compression::default()).err(),
            Some("empty.txt is empty.".into())
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn sizes_read_like_the_web_client() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(143 * 1024), "143 KB");
        assert_eq!(format_bytes(1_677_722), "1.6 MB");
        assert_eq!(size_label(1_677_722, Some(146_432)), "1.6 MB → 143 KB");
        assert_eq!(size_label(2048, Some(2048)), "2.0 KB");
        assert_eq!(size_label(2048, None), "2.0 KB");
    }
}
