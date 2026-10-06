//! End-to-end runs of `caper-media-worker process` on generated inputs.
//!
//! These need ffmpeg/ffprobe, libvips (`vips`, `vipsheader`), cwebp,
//! img2webp, avifenc and jpegtran on PATH, so they are `#[ignore]`d by
//! default. Run them with:
//!
//! ```sh
//! cargo test -p caper-media-worker -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const BIN: &str = env!("CARGO_BIN_EXE_caper-media-worker");

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("caper-media-it-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tool(program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{program} is required for this test: {e}"));
    assert!(
        output.status.success(),
        "{program} {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn ffmpeg(args: &[&str]) {
    let mut all = vec!["-hide_banner", "-loglevel", "error", "-y"];
    all.extend_from_slice(args);
    tool("ffmpeg", &all);
}

fn s(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// Runs the local mode and returns (finish JSON, stored file path).
fn process(dir: &Dir, input: &Path, filename: &str) -> (Value, PathBuf) {
    process_with(dir, input, filename, None)
}

/// Like [`process`], with a settings JSON override.
fn process_with(
    dir: &Dir,
    input: &Path,
    filename: &str,
    settings: Option<&str>,
) -> (Value, PathBuf) {
    let out = dir.path("out");
    let _ = std::fs::remove_dir_all(&out);
    let mut command = Command::new(BIN);
    command.args(["process", s(input), s(&out), "--filename", filename]);
    if let Some(settings) = settings {
        let path = dir.path("settings.json");
        std::fs::write(&path, settings).unwrap();
        command.args(["--settings", s(&path)]);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "process failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let log = String::from_utf8_lossy(&output.stderr);
    for line in log
        .lines()
        .filter(|l| l.contains("decision") || l.contains("WARN"))
    {
        println!("  {line}");
    }
    let finish: Value = serde_json::from_slice(&output.stdout).unwrap();
    let stored = out.join(finish["filename"].as_str().unwrap());
    let input_size = std::fs::metadata(input).unwrap().len();
    println!(
        "{filename}: {input_size} bytes → {} {} bytes{} ({})",
        finish["contentType"].as_str().unwrap(),
        finish["byteSize"],
        if finish.get("contentEncoding").is_some() {
            " gzip"
        } else {
            ""
        },
        finish["filename"].as_str().unwrap()
    );
    assert_eq!(
        finish["byteSize"].as_u64().unwrap(),
        std::fs::metadata(&stored).unwrap().len()
    );
    if let Some(preview) = finish.get("preview") {
        let file = out.join("preview.webp");
        let size = std::fs::metadata(&file).unwrap().len();
        assert_eq!(preview["byteSize"].as_u64(), Some(size));
        assert!(size <= 512 * 1024);
        let header = tool("vipsheader", &[s(&file)]);
        let (w, h) = dims(&header);
        assert!(w.max(h) <= 640, "preview too large: {header}");
    }
    (finish, stored)
}

fn dims(vipsheader: &str) -> (u32, u32) {
    // "file: 640x480 uchar, 3 bands, srgb, …"
    let size = vipsheader
        .split(": ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    let (w, h) = size.split_once('x').unwrap();
    (w.parse().unwrap(), h.parse().unwrap())
}

fn probe(path: &Path) -> Value {
    let out = tool(
        "ffprobe",
        &[
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
            s(path),
        ],
    );
    serde_json::from_str(&out).unwrap()
}

/// Whether two images decode to identical pixels.
fn identical(dir: &Dir, a: &Path, b: &Path) -> bool {
    let diff = dir.path("diff.v");
    let abs = dir.path("abs.v");
    tool("vips", &["subtract", s(a), s(b), s(&diff)]);
    tool("vips", &["abs", s(&diff), s(&abs)]);
    tool("vips", &["max", s(&abs)])
        .trim()
        .parse::<f64>()
        .unwrap()
        == 0.0
}

/// Photo-like content: smooth random colour fields plus fine sensor noise.
fn photo_png(dir: &Dir, name: &str, w: u32, h: u32) -> PathBuf {
    let mut bands = Vec::new();
    for band in 0..3 {
        let coarse = dir.path(&format!("coarse{band}.v"));
        let up = dir.path(&format!("up{band}.v"));
        let fine = dir.path(&format!("fine{band}.v"));
        let sum = dir.path(&format!("sum{band}.v"));
        tool(
            "vips",
            &[
                "gaussnoise",
                s(&coarse),
                &(w / 10).to_string(),
                &(h / 10).to_string(),
                "--sigma",
                "60",
                "--mean",
                "128",
            ],
        );
        tool(
            "vips",
            &["resize", s(&coarse), s(&up), "10", "--kernel", "cubic"],
        );
        tool(
            "vips",
            &[
                "gaussnoise",
                s(&fine),
                &w.to_string(),
                &h.to_string(),
                "--sigma",
                "4",
                "--mean",
                "0",
            ],
        );
        tool("vips", &["add", s(&up), s(&fine), s(&sum)]);
        bands.push(sum);
    }
    let joined = dir.path("joined.v");
    let list = bands.iter().map(|b| s(b)).collect::<Vec<_>>().join(" ");
    tool("vips", &["bandjoin", &list, s(&joined)]);
    let out = dir.path(name);
    tool("vips", &["cast", s(&joined), s(&out), "uchar"]);
    out
}

/// Big-endian EXIF (TIFF) block with an orientation and a GPS latitude.
fn exif_app1(orientation: u16) -> Vec<u8> {
    let mut t = Vec::new();
    t.extend_from_slice(b"MM\0*");
    t.extend_from_slice(&8u32.to_be_bytes());
    // IFD0 at 8: 2 entries.
    let ifd0_len = 2 + 2 * 12 + 4;
    let gps_ifd = 8 + ifd0_len;
    t.extend_from_slice(&2u16.to_be_bytes());
    // Orientation SHORT 1
    t.extend_from_slice(&0x0112u16.to_be_bytes());
    t.extend_from_slice(&3u16.to_be_bytes());
    t.extend_from_slice(&1u32.to_be_bytes());
    t.extend_from_slice(&orientation.to_be_bytes());
    t.extend_from_slice(&[0, 0]);
    // GPSInfo LONG 1 → offset
    t.extend_from_slice(&0x8825u16.to_be_bytes());
    t.extend_from_slice(&4u16.to_be_bytes());
    t.extend_from_slice(&1u32.to_be_bytes());
    t.extend_from_slice(&u32::try_from(gps_ifd).unwrap().to_be_bytes());
    t.extend_from_slice(&0u32.to_be_bytes());
    // GPS IFD: LatitudeRef ASCII "N", Latitude RATIONAL×3.
    let gps_len = 2 + 2 * 12 + 4;
    let rationals = gps_ifd + gps_len;
    t.extend_from_slice(&2u16.to_be_bytes());
    t.extend_from_slice(&0x0001u16.to_be_bytes());
    t.extend_from_slice(&2u16.to_be_bytes());
    t.extend_from_slice(&2u32.to_be_bytes());
    t.extend_from_slice(b"N\0\0\0");
    t.extend_from_slice(&0x0002u16.to_be_bytes());
    t.extend_from_slice(&5u16.to_be_bytes());
    t.extend_from_slice(&3u32.to_be_bytes());
    t.extend_from_slice(&u32::try_from(rationals).unwrap().to_be_bytes());
    t.extend_from_slice(&0u32.to_be_bytes());
    for (n, d) in [(40u32, 1u32), (44, 1), (1234, 100)] {
        t.extend_from_slice(&n.to_be_bytes());
        t.extend_from_slice(&d.to_be_bytes());
    }
    let mut app1 = vec![0xff, 0xe1];
    app1.extend_from_slice(&u16::try_from(2 + 6 + t.len()).unwrap().to_be_bytes());
    app1.extend_from_slice(b"Exif\0\0");
    app1.extend(t);
    app1
}

fn jpeg_with_exif(dir: &Dir, source: &Path, name: &str, orientation: u16) -> PathBuf {
    let plain = dir.path(&format!("plain-{name}"));
    tool(
        "vips",
        &["copy", s(source), &format!("{}[Q=92,keep=none]", s(&plain))],
    );
    let bytes = std::fs::read(&plain).unwrap();
    let mut out = bytes[..2].to_vec();
    out.extend(exif_app1(orientation));
    out.extend_from_slice(&bytes[2..]);
    let path = dir.path(name);
    std::fs::write(&path, out).unwrap();
    let header = tool("vipsheader", &["-a", s(&path)]);
    assert!(
        header.contains("GPSLatitude"),
        "test EXIF not readable: {header}"
    );
    path
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn flat_screenshot_becomes_pixel_exact_lossless_webp() {
    let dir = Dir::new("screenshot");
    let input = dir.path("screenshot.png");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=0xF3F4F5:s=1440x900",
        "-vf",
        "drawbox=x=0:y=0:w=240:h=900:c=0x151719:t=fill,drawbox=x=280:y=40:w=1100:h=60:c=0xB64D32:t=fill,drawgrid=w=48:h=32:t=1:c=0x34383B,drawbox=x=300:y=140:w=500:h=300:c=0x637A43:t=4",
        "-frames:v",
        "1",
        s(&input),
    ]);
    let (finish, stored) = process(&dir, &input, "Screenshot 2026-10-06.png");
    assert_eq!(finish["kind"], "image");
    assert_eq!(finish["contentType"], "image/webp");
    assert_eq!(finish["filename"], "Screenshot 2026-10-06.webp");
    assert_eq!(
        (finish["width"].as_u64(), finish["height"].as_u64()),
        (Some(1440), Some(900))
    );
    assert!(finish["preview"].is_object());
    assert!(
        identical(&dir, &input, &stored),
        "lossless WebP must be pixel-exact"
    );
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn photo_png_becomes_avif() {
    let dir = Dir::new("photo");
    let input = photo_png(&dir, "photo.png", 1600, 1200);
    let (finish, _) = process(&dir, &input, "photo.png");
    assert_eq!(finish["contentType"], "image/avif");
    assert_eq!(finish["filename"], "photo.avif");
    assert!(finish["byteSize"].as_u64().unwrap() < std::fs::metadata(&input).unwrap().len() / 2);
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn transparent_png_keeps_alpha() {
    let dir = Dir::new("alpha");
    let input = dir.path("logo.png");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "color=c=black@0.0:s=400x300,format=rgba",
        "-vf",
        "drawbox=x=50:y=50:w=200:h=120:c=0xB64D32@1.0:t=fill,drawbox=x=150:y=100:w=200:h=150:c=0x637A43@0.5:t=fill",
        "-frames:v",
        "1",
        s(&input),
    ]);
    let (finish, stored) = process(&dir, &input, "logo.png");
    assert_eq!(finish["kind"], "image");
    let header = tool("vipsheader", &[s(&stored)]);
    assert!(header.contains("4 bands"), "alpha lost: {header}");
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn jpeg_gps_metadata_is_removed() {
    let dir = Dir::new("gps");
    let photo = photo_png(&dir, "source.png", 1200, 900);
    let input = jpeg_with_exif(&dir, &photo, "IMG_0001.JPG", 1);
    let (finish, stored) = process(&dir, &input, "IMG_0001.JPG");
    assert_eq!(finish["kind"], "image");
    let header = tool("vipsheader", &["-a", s(&stored)]);
    assert!(!header.contains("GPS"), "GPS survived: {header}");
    assert!(!header.contains("exif-data"), "EXIF survived: {header}");
    let bytes = std::fs::read(&stored).unwrap();
    assert!(!bytes.windows(6).any(|w| w == b"Exif\0\0"));
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn rotated_jpeg_is_stored_upright_and_converted() {
    let dir = Dir::new("rotated");
    let photo = photo_png(&dir, "source.png", 1200, 900);
    let input = jpeg_with_exif(&dir, &photo, "portrait.jpg", 6);
    let (finish, stored) = process(&dir, &input, "portrait.jpg");
    assert_ne!(finish["contentType"], "image/jpeg");
    assert_eq!(
        (finish["width"].as_u64(), finish["height"].as_u64()),
        (Some(900), Some(1200))
    );
    let header = tool("vipsheader", &["-a", s(&stored)]);
    assert!(!header.contains("GPS"), "GPS survived: {header}");
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn animated_gif_becomes_looping_mp4() {
    let dir = Dir::new("gif");
    let input = dir.path("party.gif");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=321x241:rate=10",
        "-t",
        "2",
        s(&input),
    ]);
    let (finish, stored) = process(&dir, &input, "party.gif");
    assert_eq!(finish["kind"], "video");
    assert_eq!(finish["contentType"], "video/mp4");
    assert_eq!(finish["animated"], true);
    assert_eq!(finish["filename"], "party.mp4");
    assert!(finish["preview"].is_object());
    let probe = probe(&stored);
    let streams = probe["streams"].as_array().unwrap();
    assert_eq!(streams.len(), 1, "animated MP4 must be silent");
    assert_eq!(streams[0]["codec_name"], "h264");
    assert_eq!(streams[0]["pix_fmt"], "yuv420p");
    assert_eq!(streams[0]["width"].as_u64().unwrap() % 2, 0);
    let duration = finish["durationMs"].as_u64().unwrap();
    assert!((1900..=2100).contains(&duration), "duration {duration}");
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc, img2webp and jpegtran"]
fn animated_webp_keeps_frame_timing() {
    let dir = Dir::new("webp");
    let frames: Vec<PathBuf> = (1..=4).map(|i| dir.path(&format!("f{i}.png"))).collect();
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=96x64:rate=5",
        "-frames:v",
        "4",
        s(&dir.path("f%d.png")),
    ]);
    let input = dir.path("sticker.webp");
    tool(
        "img2webp",
        &[
            "-loop",
            "0",
            "-d",
            "100",
            s(&frames[0]),
            "-d",
            "300",
            s(&frames[1]),
            s(&frames[2]),
            s(&frames[3]),
            "-o",
            s(&input),
        ],
    );
    let (finish, _) = process(&dir, &input, "sticker.webp");
    assert_eq!(finish["contentType"], "video/mp4");
    assert_eq!(finish["animated"], true);
    let duration = finish["durationMs"].as_u64().unwrap();
    assert!((995..=1010).contains(&duration), "duration {duration}");
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn video_becomes_h264_aac_mp4_with_poster() {
    let dir = Dir::new("video");
    let input = dir.path("clip.mov");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=1280x720:rate=30",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=48000",
        "-t",
        "3",
        "-c:v",
        "mpeg4",
        "-q:v",
        "2",
        "-c:a",
        "pcm_s16le",
        "-metadata",
        "location=+40.6892-074.0445/",
        s(&input),
    ]);
    let (finish, stored) = process(&dir, &input, "clip.mov");
    assert_eq!(finish["kind"], "video");
    assert_eq!(finish["contentType"], "video/mp4");
    assert_eq!(finish["filename"], "clip.mp4");
    assert_eq!(finish["animated"], false);
    assert_eq!(
        (finish["width"].as_u64(), finish["height"].as_u64()),
        (Some(1280), Some(720))
    );
    assert!((2900..=3100).contains(&finish["durationMs"].as_u64().unwrap()));
    assert!(finish["preview"].is_object(), "poster preview expected");
    let probe = probe(&stored);
    let codecs: Vec<&str> = probe["streams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["codec_name"].as_str().unwrap())
        .collect();
    assert_eq!(codecs, ["h264", "aac"]);
    assert_eq!(probe["streams"][0]["profile"], "High");
    assert!(
        probe["format"]["tags"].get("location").is_none(),
        "location metadata survived"
    );
    // faststart: the moov box precedes mdat.
    let bytes = std::fs::read(&stored).unwrap();
    let find = |tag: &[u8]| bytes.windows(4).position(|w| w == tag).unwrap();
    assert!(find(b"moov") < find(b"mdat"));
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn rotated_phone_video_reports_displayed_size() {
    let dir = Dir::new("rotation");
    let source = dir.path("source.mp4");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=640x360:rate=30",
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440",
        "-t",
        "2",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:a",
        "aac",
        s(&source),
    ]);
    let input = dir.path("IMG_0002.MOV");
    ffmpeg(&[
        "-display_rotation",
        "90",
        "-i",
        s(&source),
        "-c",
        "copy",
        s(&input),
    ]);
    let (finish, stored) = process(&dir, &input, "IMG_0002.MOV");
    assert_eq!(
        (finish["width"].as_u64(), finish["height"].as_u64()),
        (Some(360), Some(640))
    );
    let probe = probe(&stored);
    assert_eq!(probe["streams"][0]["codec_name"], "h264");
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn wav_becomes_flac() {
    let dir = Dir::new("wav");
    let input = dir.path("notes.wav");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:duration=2",
        "-c:a",
        "pcm_s16le",
        s(&input),
    ]);
    let (finish, stored) = process(&dir, &input, "notes.wav");
    assert_eq!(finish["kind"], "audio");
    assert_eq!(finish["contentType"], "audio/flac");
    assert_eq!(finish["filename"], "notes.flac");
    assert!((1990..=2010).contains(&finish["durationMs"].as_u64().unwrap()));
    // Lossless: identical PCM after decoding.
    let pcm = |path: &Path, out: &str| {
        let raw = dir.path(out);
        ffmpeg(&["-i", s(path), "-f", "s16le", "-c:a", "pcm_s16le", s(&raw)]);
        std::fs::read(raw).unwrap()
    };
    assert_eq!(pcm(&input, "a.raw"), pcm(&stored, "b.raw"));
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn text_is_gzipped_and_round_trips() {
    let dir = Dir::new("text");
    let input = dir.path("notes.txt");
    let text = "Caper keeps conversations close. ".repeat(2000);
    std::fs::write(&input, &text).unwrap();
    let (finish, stored) = process(&dir, &input, "notes.txt");
    assert_eq!(finish["kind"], "file");
    assert_eq!(finish["contentEncoding"], "gzip");
    assert_eq!(finish["filename"], "notes.txt");
    let mut decoded = String::new();
    std::io::Read::read_to_string(
        &mut flate2::read::GzDecoder::new(std::fs::File::open(&stored).unwrap()),
        &mut decoded,
    )
    .unwrap();
    assert_eq!(decoded, text);
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn random_bytes_are_stored_unchanged() {
    let dir = Dir::new("random");
    let input = dir.path("blob.bin");
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let bytes: Vec<u8> = (0..300_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state.to_le_bytes()[3]
        })
        .collect();
    std::fs::write(&input, &bytes).unwrap();
    let (finish, stored) = process(&dir, &input, "blob.bin");
    assert_eq!(finish["kind"], "file");
    assert_eq!(finish["contentType"], "application/octet-stream");
    assert!(finish.get("contentEncoding").is_none());
    assert_eq!(std::fs::read(stored).unwrap(), bytes);
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn corrupt_image_is_stored_as_a_file() {
    let dir = Dir::new("corrupt");
    let input = dir.path("broken.png");
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(std::iter::repeat_n(b'x', 5000));
    std::fs::write(&input, &bytes).unwrap();
    let (finish, _) = process(&dir, &input, "broken.png");
    assert_eq!(finish["kind"], "file");
    assert_eq!(finish["contentType"], "application/octet-stream");
    assert_eq!(finish["filename"], "broken.png");
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn jpeg_is_kept_with_metadata_stripped_when_avif_saves_too_little() {
    let dir = Dir::new("keep");
    let photo = photo_png(&dir, "source.png", 800, 600);
    let input = jpeg_with_exif(&dir, &photo, "small.jpeg", 1);
    // Demand savings AVIF cannot reach, so the original is kept.
    let (finish, stored) = process_with(
        &dir,
        &input,
        "small.jpeg",
        Some(r#"{"imageMinSavingsPercent":100}"#),
    );
    assert_eq!(finish["contentType"], "image/jpeg");
    assert_eq!(finish["filename"], "small.jpeg");
    let bytes = std::fs::read(&stored).unwrap();
    assert!(!bytes.windows(6).any(|w| w == b"Exif\0\0"));
    assert!(
        identical(&dir, &input, &stored),
        "kept JPEG must decode identically"
    );
}

#[test]
#[ignore = "needs ffmpeg, libvips, cwebp, avifenc and jpegtran"]
fn avif_bmp_and_tiff_inputs() {
    let dir = Dir::new("formats");
    let photo = photo_png(&dir, "source.png", 640, 480);
    let avif = dir.path("in.avif");
    tool(
        "avifenc",
        &[
            "-q",
            "70",
            "-s",
            "8",
            "--ignore-exif",
            "--ignore-xmp",
            s(&photo),
            s(&avif),
        ],
    );
    let (finish, stored) = process(&dir, &avif, "camera.avif");
    assert_eq!(finish["contentType"], "image/avif");
    // Re-encoding a lossy AVIF at q90 does not pay off: kept byte-for-byte.
    assert_eq!(
        std::fs::read(&stored).unwrap(),
        std::fs::read(&avif).unwrap()
    );

    for (name, ext) in [("in.bmp", "bmp"), ("in.tif", "tif")] {
        let input = dir.path(name);
        tool("vips", &["copy", s(&photo), s(&input)]);
        let (finish, _) = process(&dir, &input, &format!("scan.{ext}"));
        let content_type = finish["contentType"].as_str().unwrap();
        assert!(
            content_type == "image/avif" || content_type == "image/webp",
            "{ext} must always convert, got {content_type}"
        );
    }
}
