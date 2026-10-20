//! Publish only cleaned media, never a client-writable upload object. This is
//! container metadata removal, not anonymisation of visible/audible content or
//! arbitrary documents/archives. Keep colour, orientation and playback timing.
use super::{MAX_CLEANED_BYTES, R2, disposition, sniff, unavailable};
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
/// A pessimistic rate for moving one file through download, cleaning and
/// upload, so large files get time to finish instead of always timing out.
const BYTES_PER_SECOND: u64 = 20_000_000;
const MAX_DEADLINE: Duration = Duration::from_secs(15 * 60);
/// Speech stays clear at this rate in mono; a minute is about half a megabyte.
const VOICE_BITRATE: &str = "64k";
pub(super) const VOICE_CONTENT_TYPE: &str = "audio/mp4";

/// Time allowed to publish `bytes`: a base plus time to move them, about six
/// and a half minutes at the 5 GiB upload limit.
fn deadline(bytes: i64) -> Duration {
    let transfer = u64::try_from(bytes).unwrap_or(0) / BYTES_PER_SECOND;
    (DEADLINE + Duration::from_secs(transfer)).min(MAX_DEADLINE)
}

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
    /// Camera raws, TIFF, JPEG XL containers, JPEG 2000 and Photoshop files.
    /// Their own tags are needed to decode them, so only identifying ones go.
    Tagged(&'static str),
    /// A recorded voice message, re-encoded from any audio container to mono
    /// AAC in MP4 so every client can play it.
    Voice,
}

/// Identifying tags removed from `Format::Tagged` files, and checked for
/// again afterwards. Everything else, including the camera's maker notes that
/// raw decoders need, stays.
const IDENTIFYING: [&str; 8] = [
    "-gps:all",
    "-xmp:all",
    "-iptc:all",
    "-*serial*",
    "-OwnerName",
    "-Artist",
    "-HostComputer",
    "-ImageUniqueID",
];

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
    // A bare JPEG XL codestream has nowhere to keep metadata; only its
    // container does.
    if at(0, b"\xff\x0a") {
        return Ok(None);
    }
    for (matches, extension) in [
        // TIFF and the camera raws built on it (DNG, NEF, CR2, ARW...).
        (at(0, b"II\x2a\0") || at(0, b"MM\0\x2a"), "tif"),
        (at(0, b"IIU\0"), "rw2"),
        (at(0, b"IIRO") || at(0, b"IIRS") || at(0, b"MMOR"), "orf"),
        (at(0, b"FUJIFILMCCD-RAW"), "raf"),
        (at(0, b"II\x1a\0\0\0HEAPCCDR"), "crw"),
        (at(0, b"\0\0\0\x0cJXL \r\n\x87\n"), "jxl"),
        (at(0, b"\0\0\0\x0cjP  "), "jp2"),
        (at(0, b"8BPS"), "psd"),
    ] {
        if matches {
            return Ok(Some(Format::Tagged(extension)));
        }
    }
    if content_type.starts_with("image/")
        || content_type.starts_with("video/")
        || content_type.starts_with("audio/")
        || (at(0, b"BM")
            && head.get(14..18).is_some_and(|size| {
                matches!(
                    u32::from_le_bytes(size.try_into().unwrap()),
                    12 | 16 | 40 | 52 | 56 | 64 | 108 | 124
                )
            }))
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
        Format::Tagged(_) => {
            fs::rename(input, output).await.map_err(|_| unavailable())?;
            command.args(["exiftool", "-config", "", "-overwrite_original"]);
            command.args(IDENTIFYING.map(|tag| format!("{tag}=")));
            command.arg(output);
        }
        Format::Voice => {
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
                    "0:a:0",
                    "-vn",
                    "-ac",
                    "1",
                    "-c:a",
                    "aac",
                    "-b:a",
                    VOICE_BITRATE,
                    "-map_metadata",
                    "-1",
                    "-map_metadata:s",
                    "-1",
                    "-map_chapters",
                    "-1",
                    "-fflags",
                    "+bitexact",
                    "-flags:a",
                    "+bitexact",
                    "-movflags",
                    "+faststart",
                    "-f",
                    "mp4",
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
    // Copying a large file's streams takes time too; the publish deadline
    // still bounds the whole job.
    let wait = Duration::from_secs(65 + max_size / BYTES_PER_SECOND);
    let status = tokio::time::timeout(wait, child.wait())
        .await
        .map_err(|_| rejected())?
        .map_err(|_| unavailable())?;
    if !status.success() {
        return Err(rejected());
    }
    if matches!(format, Format::Tagged(_)) {
        identifying_tags_gone(output).await?;
    }
    Ok(())
}

/// Reads the cleaned file back: a writer that skipped a tag, or a format that
/// keeps it somewhere ExifTool could not rewrite, fails closed.
async fn identifying_tags_gone(path: &Path) -> Result<(), ApiError> {
    let child = Command::new("exiftool")
        .args(["-config", "", "-q", "-q", "-s3"])
        .args(IDENTIFYING)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| unavailable())?;
    let output = tokio::time::timeout(Duration::from_secs(65), child.wait_with_output())
        .await
        .map_err(|_| rejected())?
        .map_err(|_| unavailable())?;
    if output.status.success() && output.stdout.iter().all(u8::is_ascii_whitespace) {
        Ok(())
    } else {
        Err(rejected())
    }
}

/// Reads the exact reserved bytes into a private, automatically removed temp
/// directory. Only the result is PUT to the CDN key; retries cannot publish a
/// staged source and later client PUTs can never overwrite the result.
/// `assembled` marks a staged object built from multipart parts: no client URL
/// can write it any more, so what is checked now is what gets published.
pub(super) async fn publish(
    r2: &R2,
    key: &str,
    expected: i64,
    content_type: &str,
    name: Option<&str>,
    assembled: bool,
    voice: bool,
) -> Result<i64, ApiError> {
    tokio::time::timeout(deadline(expected), async {
        if assembled
            && let Some(size) = copy_unless_media(r2, key, expected, content_type, name).await?
        {
            return Ok(size);
        }
        publish_inner(r2, key, expected, content_type, name, voice).await
    })
    .await
    .map_err(|_| unavailable())?
}

/// Documents and archives need no cleaning, so an assembled one is copied
/// inside the bucket rather than through the pod's scratch disk. Media still
/// goes through `publish_inner`, within the size the disk can clean.
async fn copy_unless_media(
    r2: &R2,
    key: &str,
    expected: i64,
    content_type: &str,
    name: Option<&str>,
) -> Result<Option<i64>, ApiError> {
    let staged = staged(key);
    let head = r2.head_bytes(&staged, 64).await?;
    if !sniff(content_type, &head) {
        return Err(rejected());
    }
    if format(&head, content_type)?.is_some() {
        return if expected > MAX_CLEANED_BYTES {
            Err(media_too_large())
        } else {
            Ok(None)
        };
    }
    let disposition = name.map(disposition);
    let mut headers = vec![("content-type", content_type)];
    if let Some(value) = &disposition {
        headers.push(("content-disposition", value));
    }
    r2.copy(&staged, key, expected, &headers).await?;
    if r2.size(key).await? != Some(expected) {
        return Err(unavailable());
    }
    Ok(Some(expected))
}

fn media_too_large() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "photos, videos and audio can be up to 5 GiB",
    )
    .with_code("media_too_large")
}

async fn publish_inner(
    r2: &R2,
    key: &str,
    expected: i64,
    content_type: &str,
    name: Option<&str>,
    voice: bool,
) -> Result<i64, ApiError> {
    let directory = tempfile::tempdir().map_err(|_| unavailable())?;
    let input = directory.path().join("input");
    let mut file = fs::File::create(&input).await.map_err(|_| unavailable())?;
    let mut response = r2
        .http
        .get(r2.presign("GET", &staged(key), 180, &[]))
        .timeout(deadline(expected))
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
    let format = match format(&head, content_type)? {
        // Only an audio or video container can become a voice message.
        Some(Format::Streams(extension)) if voice => Some((Format::Voice, extension)),
        _ if voice => return Err(rejected()),
        Some(format) => Some((format, "")),
        None => None,
    };
    // The published type: a voice message is always re-encoded to AAC in MP4.
    let content_type = if voice {
        VOICE_CONTENT_TYPE
    } else {
        content_type
    };
    let output = match format {
        Some((format, source)) => {
            let (extension, clean_extension) = match format {
                Format::Image(ext) | Format::Streams(ext) | Format::Tagged(ext) => (ext, ext),
                Format::Voice => (source, "m4a"),
            };
            // Supply our detected type, never the client's filename or declared
            // extension. ExifTool needs the extension for some container writers.
            let typed_input = directory.path().join(format!("source.{extension}"));
            fs::rename(&input, &typed_input)
                .await
                .map_err(|_| unavailable())?;
            let path = directory.path().join(format!("clean.{clean_extension}"));
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
        .timeout(deadline(expected));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raws_and_tagged_images_are_cleaned_and_unknown_media_fails_closed() {
        let mut jpeg_xl = b"\0\0\0\x0cJXL \r\n\x87\n".to_vec();
        jpeg_xl.extend(b"\0\0\0\x14ftypjxl ");
        for (head, extension) in [
            (b"II\x2a\0private".as_slice(), "tif"),
            (b"MM\0\x2aprivate".as_slice(), "tif"),
            (b"IIU\0\x18\0\0\0".as_slice(), "rw2"),
            (b"IIRO\x08\0\0\0".as_slice(), "orf"),
            (b"IIRS\x08\0\0\0".as_slice(), "orf"),
            (b"MMOR\0\0\0\x08".as_slice(), "orf"),
            (b"FUJIFILMCCD-RAW 0201FF383501".as_slice(), "raf"),
            (b"II\x1a\0\0\0HEAPCCDR\x02\0".as_slice(), "crw"),
            (jpeg_xl.as_slice(), "jxl"),
            (
                b"\0\0\0\x0cjP  \r\n\x87\n\0\0\0\x14ftypjp2 ".as_slice(),
                "jp2",
            ),
            (b"8BPS\0\x01\0\0\0\0\0\0".as_slice(), "psd"),
        ] {
            for declared in ["application/octet-stream", "image/x-raw"] {
                assert!(
                    matches!(format(head, declared), Ok(Some(Format::Tagged(found))) if found == extension),
                    "{head:?}"
                );
            }
        }
        // A bare JPEG XL codestream cannot carry metadata.
        assert!(format(b"\xff\x0a\xfa\x7f", "image/jxl").unwrap().is_none());
        // Declared media nothing can clean still fails closed.
        for (head, declared) in [
            (
                b"BM\0\0\0\0\0\0\0\0\0\0\0\0\x28\0\0\0".as_slice(),
                "image/bmp",
            ),
            (b"unknown".as_slice(), "image/x-unknown"),
            (b"unknown".as_slice(), "video/x-unknown"),
        ] {
            assert_eq!(
                format(head, declared).err().map(|error| error.status),
                Some(axum::http::StatusCode::UNPROCESSABLE_ENTITY),
                "{head:?}"
            );
        }
        // Ordinary documents stay plain downloads.
        for head in [
            b"%PDF-1.7".as_slice(),
            b"PK\x03\x04".as_slice(),
            b"plain notes".as_slice(),
        ] {
            assert!(format(head, "application/octet-stream").unwrap().is_none());
        }
    }

    #[test]
    fn publish_deadline_grows_with_size_within_a_bound() {
        assert_eq!(deadline(1), DEADLINE);
        assert_eq!(deadline(200_000_000), DEADLINE + Duration::from_secs(10));
        let largest = deadline(5 * 1024 * 1024 * 1024);
        assert!(largest > Duration::from_secs(380) && largest < MAX_DEADLINE);
        assert_eq!(deadline(i64::MAX), MAX_DEADLINE);
    }
}
