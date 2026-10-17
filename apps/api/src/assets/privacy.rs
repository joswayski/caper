//! Publish only cleaned media, never a client-writable upload object. This is
//! container metadata removal, not anonymisation of visible/audible content or
//! arbitrary documents/archives. Keep colour, orientation and playback timing.
use super::{R2, disposition, sniff, unavailable};
use crate::ApiError;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::Semaphore,
};
use tokio_util::io::ReaderStream;

// A single streamed job bounds scratch disk and subprocess memory per API pod.
pub(super) static JOBS: Semaphore = Semaphore::const_new(1);
const DEADLINE: Duration = Duration::from_secs(120);

pub(super) fn staged(key: &str) -> String {
    format!("staging/{key}")
}

fn rejected() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "this media could not be cleaned safely; try another format",
    )
    .with_code("media_metadata")
}

#[derive(Clone, Copy)]
enum Format {
    Image(&'static str),
    Streams(&'static str),
}

/// Inspect bytes even for application/octet-stream or a misleading filename.
/// Unknown declared media fails closed; ordinary documents remain downloads.
fn format(head: &[u8], content_type: &str) -> Result<Option<Format>, ApiError> {
    let at = |n: usize, s: &[u8]| head.get(n..n + s.len()) == Some(s);
    let image = [
        ("image/jpeg", "jpg"),
        ("image/png", "png"),
        ("image/gif", "gif"),
        ("image/webp", "webp"),
        ("image/avif", "avif"),
    ];
    for (mime, extension) in image {
        if sniff(mime, head) {
            return Ok(Some(Format::Image(extension)));
        }
    }
    if at(4, b"ftyp") {
        if [b"heic", b"heix", b"hevc", b"hevx", b"heif"]
            .iter()
            .any(|brand| at(8, *brand))
        {
            return Ok(Some(Format::Image("heic")));
        }
        return Ok(Some(Format::Streams("mp4")));
    }
    if [b"moov", b"mdat", b"wide", b"free"]
        .iter()
        .any(|tag| at(4, *tag))
    {
        return Ok(Some(Format::Streams("mov")));
    }
    for (matches, extension) in [
        (at(0, b"\x1a\x45\xdf\xa3"), "webm"),
        (at(0, b"fLaC"), "flac"),
        (at(0, b"OggS"), "ogg"),
        (at(0, b"RIFF") && at(8, b"WAVE"), "wav"),
        (at(0, b"RIFF") && at(8, b"AVI "), "avi"),
        (at(0, b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")), "aiff"),
        (
            at(0, b"ID3"),
            if content_type == "audio/aac" {
                "adts"
            } else {
                "mp3"
            },
        ),
    ] {
        if matches {
            return Ok(Some(Format::Streams(extension)));
        }
    }
    if head.len() >= 2 && head[0] == 0xff && head[1] & 0xe0 == 0xe0 {
        return Ok(Some(Format::Streams(if head[1] & 6 == 0 {
            "adts"
        } else {
            "mp3"
        })));
    }
    if content_type.starts_with("image/")
        || content_type.starts_with("video/")
        || content_type.starts_with("audio/")
        || at(0, b"II\x2a\0")
        || at(0, b"MM\0\x2a")
        || at(0, b"BM")
    {
        return Err(rejected());
    }
    Ok(None)
}

/// Fixed arguments and generated paths only. No shell, inherited config,
/// network protocols, original filenames, or subprocess output in logs.
async fn clean(input: &Path, output: &Path, format: Format, max_size: u64) -> Result<(), ApiError> {
    let mut command = Command::new("prlimit");
    command.args([
        "--as=536870912",
        "--data=134217728",
        "--cpu=60",
        &format!("--fsize={max_size}"),
        "--",
    ]);
    match format {
        Format::Image(_) => {
            fs::rename(input, output).await.map_err(|_| unavailable())?;
            command
                .args([
                    "exiftool",
                    "-config",
                    "",
                    "-overwrite_original",
                    "-all=",
                    "-CommonIFD0=",
                    "-tagsfromfile",
                    "@",
                    "-IFD0:Orientation",
                    "-ICC_Profile",
                    "-ColorSpaceTags",
                ])
                .arg(output);
        }
        Format::Streams(extension) => {
            // Only real audio/video: no GPS/data/timecode/subtitle tracks,
            // cover art or attachments. Explicitly disable stream tags too;
            // -map_metadata -1 alone only suppresses global metadata.
            command
                .args([
                    "ffmpeg",
                    "-nostdin",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-protocol_whitelist",
                    "file,pipe",
                    "-i",
                ])
                .arg(input)
                .args([
                    "-map",
                    "0:V?",
                    "-map",
                    "0:a?",
                    "-c",
                    "copy",
                    "-map_metadata",
                    "-1",
                    "-map_metadata:s",
                    "-1",
                    "-map_chapters",
                    "-1",
                    "-fflags",
                    "+bitexact",
                    "-flags:v",
                    "+bitexact",
                    "-flags:a",
                    "+bitexact",
                ]);
            if matches!(extension, "mp4" | "mov") {
                command.args(["-movflags", "+faststart"]);
            }
            // ADTS uses .aac, but the muxer's name is adts.
            command.args(["-f", extension]).arg(output);
        }
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| unavailable())?;
    let status = tokio::time::timeout(Duration::from_secs(65), child.wait())
        .await
        .map_err(|_| rejected())?
        .map_err(|_| unavailable())?;
    if !status.success() {
        return Err(rejected());
    }
    Ok(())
}

/// Reads the exact reserved bytes into a private, automatically removed temp
/// directory. Only the result is PUT to the CDN key; retries cannot publish a
/// staged source and later client PUTs can never overwrite the result.
pub(super) async fn publish(
    r2: &R2,
    key: &str,
    expected: i64,
    content_type: &str,
    name: Option<&str>,
) -> Result<i64, ApiError> {
    tokio::time::timeout(
        DEADLINE,
        publish_inner(r2, key, expected, content_type, name),
    )
    .await
    .map_err(|_| unavailable())?
}

async fn publish_inner(
    r2: &R2,
    key: &str,
    expected: i64,
    content_type: &str,
    name: Option<&str>,
) -> Result<i64, ApiError> {
    let directory = tempfile::tempdir().map_err(|_| unavailable())?;
    let input = directory.path().join("input");
    let mut file = fs::File::create(&input).await.map_err(|_| unavailable())?;
    let mut response = r2
        .http
        .get(r2.presign("GET", &staged(key), 180, &[]))
        .timeout(DEADLINE)
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    let mut length = 0_i64;
    let mut head = Vec::new();
    while let Some(bytes) = response.chunk().await.map_err(|_| unavailable())? {
        length += bytes.len() as i64;
        if length > expected {
            return Err(rejected());
        }
        head.extend(bytes.iter().take(64_usize.saturating_sub(head.len())));
        file.write_all(&bytes).await.map_err(|_| unavailable())?;
    }
    file.flush().await.map_err(|_| unavailable())?;
    drop(file);
    if length != expected || !sniff(content_type, &head) {
        return Err(rejected());
    }
    let format = format(&head, content_type)?;
    let output = match format {
        Some(format) => {
            let extension = match format {
                Format::Image(ext) | Format::Streams(ext) => ext,
            };
            // Supply our detected type, never the client's filename or declared
            // extension. ExifTool needs the extension for some container writers.
            let typed_input = directory.path().join(format!("source.{extension}"));
            fs::rename(&input, &typed_input)
                .await
                .map_err(|_| unavailable())?;
            let path = directory.path().join(format!("clean.{extension}"));
            clean(
                &typed_input,
                &path,
                format,
                expected as u64 + 64 * 1024 * 1024,
            )
            .await?;
            path
        }
        None => input,
    };
    let size = fs::metadata(&output)
        .await
        .map_err(|_| unavailable())?
        .len();
    if size == 0 || size > expected as u64 + 64 * 1024 * 1024 {
        return Err(rejected());
    }
    let mut head = [0; 64];
    let count = fs::File::open(&output)
        .await
        .map_err(|_| unavailable())?
        .read(&mut head)
        .await
        .map_err(|_| unavailable())?;
    if !sniff(content_type, &head[..count]) {
        return Err(rejected());
    }
    let file = fs::File::open(&output).await.map_err(|_| unavailable())?;
    let size_text = size.to_string();
    let mut headers = vec![
        ("content-type", content_type),
        ("content-length", size_text.as_str()),
    ];
    let disposition = name.map(disposition);
    if let Some(value) = &disposition {
        headers.push(("content-disposition", value));
    }
    let mut request = r2
        .http
        .put(r2.presign("PUT", key, 180, &headers))
        .timeout(DEADLINE);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    let response = request
        .body(reqwest::Body::wrap_stream(ReaderStream::new(file)))
        .send()
        .await
        .map_err(|_| unavailable())?;
    if !response.status().is_success() {
        return Err(unavailable());
    }
    Ok(size as i64)
}
