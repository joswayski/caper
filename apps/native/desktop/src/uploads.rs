//! Sending attachments: prepare a file (compressing stills with the server's
//! settings, see `compress`), reserve its exact size, PUT the bytes and any
//! preview straight to storage, confirm, and return the description sent
//! with `attachmentIds` (docs/media.md, "Uploads and attachments").
//! Desktop has no bundled video transcoder: videos upload as the original,
//! with dimensions and duration read from MP4/QuickTime headers and their
//! metadata boxes blanked (`metadata`), so location never leaves the device.

use crate::api::Api;
use crate::compress::{self, Compression};
use crate::model::{Attachment, AttachmentKind};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const MAX_ATTACHMENTS: usize = 10;
/// Stills larger than this upload unchanged instead of being read for decoding.
const MAX_IMAGE_READ_BYTES: u64 = 200 * 1024 * 1024;
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

pub enum Body {
    Memory(Vec<u8>),
    /// Streamed from disk so large files never sit in memory, with metadata
    /// boxes blanked on the way.
    File {
        path: PathBuf,
        patches: Vec<crate::metadata::Patch>,
    },
}

pub struct Prepared {
    pub name: String,
    pub content_type: String,
    pub kind: AttachmentKind,
    /// The picked file's size, sent as `sourceByteSize`.
    pub source_size: u64,
    /// What is stored: the compressed bytes or the unchanged original.
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
        body: Body::File {
            path: path.to_owned(),
            patches: Vec::new(),
        },
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
        if let Body::File { patches, .. } = &mut prepared.body {
            *patches = crate::metadata::mp4_metadata_patches(&mut file);
        }
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
/// `progress` receives 0–1 across preview and stored bytes.
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
    let request = reservation(channel, &prepared);
    let Prepared {
        name,
        size,
        body,
        preview,
        ..
    } = prepared;
    // Open first so an unreadable file never holds a reservation.
    let source: Box<dyn Read + Send> = match body {
        Body::Memory(bytes) => Box::new(std::io::Cursor::new(bytes)),
        // Exactly the reserved size, which the URL signs: a file that grew
        // since is cut, one that shrank fails verification.
        Body::File { path, patches } => Box::new(crate::metadata::Patched::new(
            std::fs::File::open(path)
                .map_err(|_| UploadError::new(format!("Could not read {name}.")))?
                .take(size),
            patches,
        )),
    };
    let reserved = api.create_asset(token, request)?;
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
    let preview_size = preview.as_ref().map_or(0, Vec::len) as u64;
    let total = (size + preview_size).max(1) as f32;
    if let Some(preview) = preview {
        // The reservation counted the preview, so it must arrive.
        let put = presigned(&reserved["previewUpload"]).ok_or_else(invalid)?;
        cancelled(&cancel)?;
        let body = Progress {
            inner: std::io::Cursor::new(preview),
            sent: 0,
            report: {
                let progress = progress.clone();
                Arc::new(move |sent| progress(sent as f32 / total))
            },
            cancel: cancel.clone(),
        };
        api.put_presigned(
            &put.url,
            &put.headers,
            reqwest::blocking::Body::sized(body, preview_size),
        )
        .map_err(|error| cancelled(&cancel).err().unwrap_or(error))?;
    }
    cancelled(&cancel)?;
    let report: Arc<dyn Fn(u64) + Send + Sync> = {
        let progress = progress.clone();
        Arc::new(move |sent| progress((preview_size + sent) as f32 / total))
    };
    let body = reqwest::blocking::Body::sized(
        Progress {
            inner: source,
            sent: 0,
            report,
            cancel: cancel.clone(),
        },
        size,
    );
    api.put_presigned(&upload.url, &upload.headers, body)
        .map_err(|error| cancelled(&cancel).err().unwrap_or(error))?;
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
            "kind": "file",
            "upload": {"method": "PUT", "url": format!("{base}/r2/original/asset0001?X-Amz-Signature=s"), "headers": {
                "content-type": "application/octet-stream"
            }},
            "storage": {"used": 23, "limit": 1000}
        })
        .to_string()
    }

    fn completed(kind: &str, content_type: &str, name: &str, size: usize) -> String {
        json!({
            "id": "asset0001", "kind": kind, "contentType": content_type,
            "name": name, "size": size
        })
        .to_string()
    }

    type Reporter = Arc<dyn Fn(f32) + Send + Sync>;

    fn recorder() -> (Arc<Mutex<Vec<f32>>>, Reporter) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let progress: Arc<dyn Fn(f32) + Send + Sync> = {
            let seen = seen.clone();
            Arc::new(move |value| seen.lock().unwrap().push(value))
        };
        (seen, progress)
    }

    /// Storage PUTs carry exactly the returned headers and the exact length,
    /// and never a Caper credential.
    fn assert_no_credentials(put: &Request) {
        assert!(!put.headers.contains_key("authorization"));
        assert!(!put.headers.contains_key("x-caper-chat-token"));
        assert!(!put.headers.contains_key("cookie"));
        assert!(!put.headers.contains_key("transfer-encoding"));
    }

    #[test]
    fn compressed_still_uploads_preview_then_file_and_retries_complete() {
        // A large few-colour screenshot: lossless WebP (smaller than its
        // exact palette PNG) with a preview.
        let scratch = Scratch::new("still");
        let image = image::RgbaImage::from_fn(1280, 720, |x, y| {
            image::Rgba(if (x / 40 + y / 40) % 2 == 0 {
                [12, 13, 15, 255]
            } else {
                [243, 244, 245, 255]
            })
        });
        let mut source = Vec::new();
        image::DynamicImage::ImageRgba8(image.clone())
            .write_to(
                &mut std::io::Cursor::new(&mut source),
                image::ImageFormat::Bmp,
            )
            .unwrap();
        let path = scratch.file("Screenshot.bmp", &source);
        let prepared = prepare_path(&path, &Compression::default()).unwrap();
        assert_eq!(prepared.content_type, "image/webp");
        assert_eq!(prepared.name, "Screenshot.webp");
        assert_eq!(prepared.kind, AttachmentKind::Image);
        assert_eq!(prepared.source_size, source.len() as u64);
        assert!(prepared.size < prepared.source_size);
        assert_eq!(
            prepared.thumbnail.as_ref().unwrap().dimensions(),
            (720, 405)
        );
        let Body::Memory(stored) = &prepared.body else {
            panic!("compressed stills upload from memory");
        };
        let stored = stored.clone();
        let preview = prepared.preview.clone().unwrap();
        let (base, server) = serve(|base| {
            let reserved = json!({
                "id": "asset0001",
                "kind": "image",
                "upload": {"method": "PUT", "url": format!("{base}/r2/original/asset0001?X-Amz-Signature=s"), "headers": {
                    "content-type": "image/webp",
                    "content-disposition": "inline; filename=\"Screenshot.webp\""
                }},
                "previewUpload": {"method": "PUT", "url": format!("{base}/r2/preview/asset0001?X-Amz-Signature=p"), "headers": {
                    "content-type": "image/jpeg"
                }},
                "storage": {"used": 23, "limit": 1000}
            });
            vec![
                (201, reserved.to_string()),
                (200, String::new()),
                (200, String::new()),
                (409, r#"{"error":"upload not finished"}"#.into()),
                (
                    200,
                    completed("image", "image/webp", "Screenshot.webp", stored.len()),
                ),
            ]
        });
        let api = Api::new(&base).unwrap();
        let (seen, progress) = recorder();
        let attachment = upload_with_delays(
            &api,
            "account-token",
            "chan00000001",
            prepared,
            progress,
            Arc::new(AtomicBool::new(false)),
            &[Duration::from_millis(1); 2],
        )
        .unwrap();
        assert_eq!(attachment.id, "asset0001");
        assert_eq!(attachment.kind, AttachmentKind::Image);
        assert_eq!(attachment.status, crate::model::AttachmentStatus::Ready);
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
                "POST /api/assets/asset0001/complete HTTP/1.1",
            ]
        );
        let reservation: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            reservation,
            json!({
                "channelId": "chan00000001", "filename": "Screenshot.webp", "contentType": "image/webp",
                "byteSize": stored.len(), "sourceByteSize": source.len(),
                "width": 1280, "height": 720,
                "preview": {"contentType": "image/jpeg", "byteSize": preview.len()}
            })
        );
        for api_request in [&requests[0], &requests[3], &requests[4]] {
            assert_eq!(
                api_request.headers.get("authorization").map(String::as_str),
                Some("Bearer account-token")
            );
        }
        let put_preview = &requests[1];
        assert_eq!(put_preview.body, preview);
        assert_eq!(put_preview.headers["content-type"], "image/jpeg");
        assert_eq!(
            put_preview.headers["content-length"],
            preview.len().to_string()
        );
        let original = &requests[2];
        assert_eq!(original.body, stored);
        assert_eq!(original.headers["content-type"], "image/webp");
        assert_eq!(original.headers["content-length"], stored.len().to_string());
        assert_eq!(
            original.headers["content-disposition"],
            "inline; filename=\"Screenshot.webp\""
        );
        for put in [put_preview, original] {
            assert_no_credentials(put);
        }
        // The stored bytes are exactly the source pixels.
        assert_eq!(
            image::load_from_memory(&original.body).unwrap().to_rgba8(),
            image
        );
        let progress = seen.lock().unwrap();
        assert_eq!(progress.last(), Some(&1.0));
        assert!(progress.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[test]
    fn videos_stream_from_disk_without_location_and_with_header_metadata() {
        let scratch = Scratch::new("video");
        let mut mvhd = vec![0; 100];
        mvhd[12..16].copy_from_slice(&1000_u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&2500_u32.to_be_bytes());
        let mp4_box = |kind: &[u8; 4], body: &[u8]| {
            let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            bytes.extend_from_slice(kind);
            bytes.extend_from_slice(body);
            bytes
        };
        let mut file = mp4_box(b"ftyp", b"isom\0\0\0\0");
        file.extend(mp4_box(b"mdat", &[5; 333]));
        let mut moov = mp4_box(b"mvhd", &mvhd);
        moov.extend(mp4_box(
            b"udta",
            &mp4_box(b"\xa9xyz", b"+40.6892-074.0445/"),
        ));
        file.extend(mp4_box(b"moov", &moov));
        let path = scratch.file("clip.mp4", &file);
        let prepared = prepare_path(&path, &Compression::default()).unwrap();
        assert_eq!(prepared.content_type, "video/mp4");
        assert_eq!(prepared.kind, AttachmentKind::Video);
        let Body::File { patches, .. } = &prepared.body else {
            panic!("videos stream from disk");
        };
        assert_eq!(patches.len(), 1, "moov/udta is blanked");
        assert_eq!(prepared.duration_ms, Some(2500));
        assert!(prepared.preview.is_none());
        let (base, server) = serve(|base| {
            vec![
                (201, reserved(base)),
                (200, String::new()),
                (200, completed("video", "video/mp4", "clip.mp4", file.len())),
            ]
        });
        let api = Api::new(&base).unwrap();
        upload(
            &api,
            "token",
            "chan",
            prepared,
            Arc::new(|_| {}),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let requests = server.join().unwrap();
        let reservation: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(
            reservation,
            json!({
                "channelId": "chan", "filename": "clip.mp4", "contentType": "video/mp4",
                "byteSize": file.len(), "sourceByteSize": file.len(), "durationMs": 2500
            })
        );
        // Same length and samples; the location box is now zero-filled `free`.
        let sent = &requests[1].body;
        assert_eq!(sent.len(), file.len());
        assert_eq!(sent[..16 + 8 + 333], file[..16 + 8 + 333]);
        assert!(!sent.windows(7).any(|window| window == b"+40.689"));
        assert!(sent.windows(4).any(|window| window == b"free"));
        assert_eq!(
            compress::probe_mp4(&mut std::io::Cursor::new(sent)).duration_ms,
            Some(2500)
        );
        assert_no_credentials(&requests[1]);
        assert_eq!(
            prepare_path(&scratch.file("empty.txt", b""), &Compression::default()).err(),
            Some("empty.txt is empty.".into())
        );
        assert_eq!(
            prepare_path(&scratch.0, &Compression::default())
                .err()
                .map(|error| error.ends_with("is not a file.")),
            Some(true)
        );
    }

    #[test]
    fn complete_gives_up_after_its_retries_and_reports_size_mismatch() {
        let scratch = Scratch::new("complete");
        let path = scratch.file("a.bin", b"x");
        let (base, server) = serve(|base| {
            vec![
                (201, reserved(base)),
                (200, String::new()),
                (409, r#"{"error":"upload not finished"}"#.into()),
                (409, r#"{"error":"upload not finished"}"#.into()),
                (201, reserved(base)),
                (200, String::new()),
                (
                    422,
                    r#"{"error":"file does not match its declared size or type"}"#.into(),
                ),
            ]
        });
        let api = Api::new(&base).unwrap();
        let attempt = || {
            upload_with_delays(
                &api,
                "token",
                "chan",
                prepare_path(&path, &Compression::default()).unwrap(),
                Arc::new(|_| {}),
                Arc::new(AtomicBool::new(false)),
                &[Duration::from_millis(1)],
            )
            .unwrap_err()
        };
        let pending = attempt();
        assert_eq!(pending.status, Some(409));
        assert_eq!(pending.message, "upload not finished");
        let mismatch = attempt();
        assert_eq!(mismatch.status, Some(422));
        assert_eq!(server.join().unwrap().len(), 7);
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
                (None, false),
            )
            .unwrap();
        assert_eq!(sent.content.attachments.len(), 1);
        api.send(
            Some("token"),
            "chat",
            "chan",
            "client",
            "hi",
            &[],
            (None, false),
        )
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
                    json!({"used": 1, "limit": 2, "compression": {"imageQuality": 70, "paletteColors": 64}})
                        .to_string(),
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
        let settings = api.asset_usage("token").unwrap();
        assert_eq!(settings.image_quality, 70);
        assert_eq!(settings.palette_colors, 64);
        assert_eq!(settings.image_max_edge, 4096);
        // Without settings the defaults apply.
        assert_eq!(api.asset_usage("token").unwrap(), Compression::default());
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
        assert_eq!(size_label(1_677_722, Some(146_432)), "1.6 MB → 143 KB");
        assert_eq!(size_label(2048, Some(2048)), "2.0 KB");
        assert_eq!(size_label(2048, None), "2.0 KB");
    }
}
