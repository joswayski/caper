//! Still images: lossless WebP for graphics, AVIF for photos, or the
//! original (metadata stripped) when re-encoding does not pay off.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::detect::ImageFormat;
use crate::process::{Ctx, Error, Kind, Outcome};
use crate::settings::Settings;
use crate::tools::{self, VipsHeader};
use crate::video;

/// WebP's dimension limit.
const WEBP_MAX: u32 = 16_383;
/// Lossless WebP below this many bits per pixel marks a screenshot/graphic,
/// which gets 4:4:4 AVIF so sharp coloured edges survive.
const GRAPHIC_BPP: f64 = 4.0;
/// `cwebp -z` 3 costs ~20% more than 1 but stores screenshots ~15% smaller;
/// above 25 MP (noisy photos, where lossless rarely wins) use 1 to stay fast.
const LOSSLESS_LARGE_PIXELS: u64 = 25_000_000;
const PREVIEW_MAX_BYTES: u64 = 512 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    LosslessWebp,
    Avif,
    KeepOriginal,
}

/// Inputs to the storage decision (sizes in bytes).
#[derive(Clone, Copy, Debug)]
pub struct Facts {
    pub upload_size: u64,
    pub lossless: Option<u64>,
    pub avif: Option<u64>,
    /// The original may be stored as-is with metadata stripped: a JPEG, PNG,
    /// WebP or AVIF that is upright (orientation 1/absent), was not resized,
    /// and (for AVIF) carries no EXIF/XMP.
    pub keepable: bool,
}

#[allow(clippy::cast_precision_loss)]
fn savings_percent(original: u64, candidate: u64) -> f64 {
    if original == 0 {
        return 0.0;
    }
    (original as f64 - candidate as f64) / original as f64 * 100.0
}

/// The compression rules from `docs/media.md`:
/// lossless WebP when it is at most `ratio` × the AVIF size; otherwise AVIF,
/// unless AVIF saves less than the minimum and the original can be kept.
/// A lossless result that is no smaller than a keepable original keeps the
/// original instead (it is the same pixels in fewer bytes).
pub fn choose(facts: &Facts, settings: &Settings) -> Option<Choice> {
    #[allow(clippy::cast_precision_loss)]
    let lossless_wins = match (facts.lossless, facts.avif) {
        (Some(lossless), Some(avif)) => {
            lossless as f64 <= settings.image_lossless_ratio * avif as f64
        }
        (Some(_), None) => true,
        (None, Some(_)) => false,
        (None, None) => return facts.keepable.then_some(Choice::KeepOriginal),
    };
    if lossless_wins {
        let lossless = facts.lossless.unwrap_or(u64::MAX);
        return Some(if facts.keepable && lossless >= facts.upload_size {
            Choice::KeepOriginal
        } else {
            Choice::LosslessWebp
        });
    }
    let avif = facts.avif.unwrap_or(u64::MAX);
    Some(
        if facts.keepable
            && savings_percent(facts.upload_size, avif) < settings.image_min_savings_percent
        {
            Choice::KeepOriginal
        } else {
            Choice::Avif
        },
    )
}

/// `cwebp -z` effort for an image of `pixels`.
pub fn lossless_effort(pixels: u64) -> u8 {
    if pixels > LOSSLESS_LARGE_PIXELS { 1 } else { 3 }
}

/// AVIF chroma subsampling: 4:4:4 for graphics, 4:2:0 for photos.
#[allow(clippy::cast_precision_loss)]
pub fn chroma(lossless: Option<u64>, pixels: u64) -> &'static str {
    match lossless {
        Some(bytes) if pixels > 0 && (bytes as f64 * 8.0 / pixels as f64) < GRAPHIC_BPP => "444",
        _ => "420",
    }
}

fn arg(path: &Path, options: &str) -> OsString {
    let mut s = path.as_os_str().to_owned();
    s.push(options);
    s
}

/// Upright copy of the first frame as PNG (ICC profile only), downscaled
/// to `max_edge` when set.
async fn normalise(
    source: &Path,
    header: &VipsHeader,
    out: &Path,
    max_edge: u32,
    deadline: Option<Instant>,
) -> Result<bool, Error> {
    let (w, h) = (header.width().unwrap_or(0), header.height().unwrap_or(0));
    let longer = w.max(h);
    let saved = arg(out, "[keep=icc,compression=1]");
    if max_edge > 0 && longer > max_edge {
        let size = max_edge.to_string();
        tools::run(
            "vips",
            [
                OsString::from("thumbnail"),
                source.as_os_str().to_owned(),
                saved,
                size.clone().into(),
                "--height".into(),
                size.into(),
                "--size".into(),
                "down".into(),
            ],
            deadline,
        )
        .await?;
        Ok(true)
    } else {
        tools::run(
            "vips",
            [
                OsString::from("autorot"),
                source.as_os_str().to_owned(),
                saved,
            ],
            deadline,
        )
        .await?;
        Ok(false)
    }
}

/// WebP preview: longer edge ≤ `edge`, never upscaled, ≤ 512 KiB, sRGB.
pub async fn preview(source: &Path, ctx: &Ctx<'_>, name: &str) -> Option<(PathBuf, u32, u32)> {
    let edge = ctx.settings().preview_edge.to_string();
    let scaled = ctx.path(&format!("{name}-src.png"));
    let out = ctx.path(&format!("{name}.webp"));
    let made = tools::run(
        "vips",
        [
            OsString::from("thumbnail"),
            source.as_os_str().to_owned(),
            arg(&scaled, "[keep=none,compression=1]"),
            edge.clone().into(),
            "--height".into(),
            edge.into(),
            "--size".into(),
            "down".into(),
            "--export-profile".into(),
            "srgb".into(),
        ],
        ctx.deadline,
    )
    .await;
    if let Err(error) = made {
        tracing::warn!(error = %error, "preview thumbnail failed");
        return None;
    }
    let header = tools::vipsheader(&scaled, ctx.deadline).await.ok()?;
    for quality in ["80", "65", "50"] {
        let encoded = tools::run(
            "cwebp",
            [
                OsString::from("-quiet"),
                "-mt".into(),
                "-q".into(),
                quality.into(),
                "-m".into(),
                "4".into(),
                "-metadata".into(),
                "none".into(),
                scaled.as_os_str().to_owned(),
                "-o".into(),
                out.as_os_str().to_owned(),
            ],
            ctx.deadline,
        )
        .await;
        if let Err(error) = encoded {
            tracing::warn!(error = %error, "preview encode failed");
            return None;
        }
        if std::fs::metadata(&out).ok()?.len() <= PREVIEW_MAX_BYTES {
            return Some((out, header.width()?, header.height()?));
        }
    }
    None
}

async fn lossless_webp(input: &Path, out: &Path, pixels: u64, ctx: &Ctx<'_>) -> Option<u64> {
    let result = tools::run(
        "cwebp",
        [
            OsString::from("-quiet"),
            "-mt".into(),
            "-lossless".into(),
            "-z".into(),
            lossless_effort(pixels).to_string().into(),
            "-metadata".into(),
            "icc".into(),
            input.as_os_str().to_owned(),
            "-o".into(),
            out.as_os_str().to_owned(),
        ],
        ctx.deadline,
    )
    .await;
    match result {
        Ok(_) => std::fs::metadata(out).ok().map(|m| m.len()),
        Err(error) => {
            tracing::warn!(error = %error, "lossless WebP encode failed");
            None
        }
    }
}

async fn avif(input: &Path, out: &Path, chroma: &str, ctx: &Ctx<'_>) -> Option<u64> {
    let s = ctx.settings();
    let quality = s.image_avif_quality.to_string();
    let result = tools::run(
        "avifenc",
        [
            OsString::from("-q"),
            quality.clone().into(),
            "--qalpha".into(),
            quality.into(),
            "-s".into(),
            s.image_avif_speed.to_string().into(),
            "-y".into(),
            chroma.into(),
            "-d".into(),
            "8".into(),
            "-j".into(),
            "all".into(),
            "--ignore-exif".into(),
            "--ignore-xmp".into(),
            input.as_os_str().to_owned(),
            out.as_os_str().to_owned(),
        ],
        ctx.deadline,
    )
    .await;
    match result {
        Ok(_) => std::fs::metadata(out).ok().map(|m| m.len()),
        Err(error) => {
            tracing::warn!(error = %error, "AVIF encode failed");
            None
        }
    }
}

pub async fn process(
    input: &Path,
    format: ImageFormat,
    animated_hint: bool,
    ctx: &Ctx<'_>,
) -> Result<Outcome, Error> {
    let (source, header) = match tools::vipsheader(input, ctx.deadline).await {
        Ok(header) => (input.to_path_buf(), header),
        Err(error) => {
            // libvips without a loader for this format (e.g. BMP without
            // ImageMagick): decode the first frame with ffmpeg instead.
            tracing::info!(error = %error, "libvips cannot read the image; decoding with ffmpeg");
            let decoded = ctx.path("decoded.png");
            tools::run(
                "ffmpeg",
                [
                    OsString::from("-hide_banner"),
                    "-loglevel".into(),
                    "error".into(),
                    "-nostdin".into(),
                    "-y".into(),
                    "-i".into(),
                    input.as_os_str().to_owned(),
                    "-frames:v".into(),
                    "1".into(),
                    decoded.as_os_str().to_owned(),
                ],
                ctx.deadline,
            )
            .await?;
            let header = tools::vipsheader(&decoded, ctx.deadline).await?;
            (decoded, header)
        }
    };
    let animated = match format {
        ImageFormat::Gif | ImageFormat::WebP => header.pages() > 1,
        ImageFormat::Png | ImageFormat::Avif => animated_hint,
        _ => false,
    };
    if animated {
        match video::animated(input, format, &header, ctx).await {
            Ok(outcome) => return Ok(outcome),
            Err(error) => {
                tracing::warn!(error = %error, "animation could not be converted; storing the first frame");
            }
        }
    }
    still(input, &source, format, &header, ctx).await
}

async fn still(
    input: &Path,
    source: &Path,
    format: ImageFormat,
    header: &VipsHeader,
    ctx: &Ctx<'_>,
) -> Result<Outcome, Error> {
    let settings = ctx.settings();
    let normalised = ctx.path("normalised.png");
    let resized = normalise(
        source,
        header,
        &normalised,
        settings.image_max_edge,
        ctx.deadline,
    )
    .await?;
    let normal = tools::vipsheader(&normalised, ctx.deadline).await?;
    let (width, height) = (
        normal.width().ok_or("no width")?,
        normal.height().ok_or("no height")?,
    );
    let pixels = u64::from(width) * u64::from(height);

    let lossless_path = ctx.path("lossless.webp");
    let lossless = if width <= WEBP_MAX && height <= WEBP_MAX {
        lossless_webp(&normalised, &lossless_path, pixels, ctx).await
    } else {
        None
    };
    let avif_path = ctx.path("result.avif");
    let avif = avif(&normalised, &avif_path, chroma(lossless, pixels), ctx).await;

    let upload_size = std::fs::metadata(input)?.len();
    let keepable = format.keepable()
        && source == input
        && !resized
        && header.orientation() == 1
        && !(format == ImageFormat::Avif && (header.has("exif-data") || header.has("xmp-data")));
    let facts = Facts {
        upload_size,
        lossless,
        avif,
        keepable,
    };
    let choice = choose(&facts, settings).ok_or("no image encoder succeeded")?;
    tracing::info!(
        ?choice,
        upload_size,
        ?lossless,
        ?avif,
        keepable,
        "image decision"
    );

    let (result, content_type) = match choice {
        Choice::LosslessWebp => (lossless_path, "image/webp"),
        Choice::Avif => (avif_path, "image/avif"),
        Choice::KeepOriginal => (keep(input, format, ctx).await?, format.content_type()),
    };
    let preview = preview(&normalised, ctx, "preview")
        .await
        .map(|(path, _, _)| path);
    Ok(Outcome::new(Kind::Image, content_type, result, ctx.job)?
        .size(Some(width), Some(height))
        .preview(preview))
}

/// The original with EXIF/XMP/text metadata removed (colour profile kept).
async fn keep(input: &Path, format: ImageFormat, ctx: &Ctx<'_>) -> Result<PathBuf, Error> {
    match format {
        ImageFormat::Jpeg => {
            let out = ctx.path("kept.jpg");
            tools::run(
                "jpegtran",
                [
                    OsString::from("-copy"),
                    "icc".into(),
                    "-optimize".into(),
                    "-outfile".into(),
                    out.as_os_str().to_owned(),
                    input.as_os_str().to_owned(),
                ],
                ctx.deadline,
            )
            .await?;
            Ok(out)
        }
        ImageFormat::Png => {
            let out = ctx.path("kept.png");
            let (from, to) = (input.to_path_buf(), out.clone());
            tokio::task::spawn_blocking(move || strip_png(&from, &to)).await??;
            Ok(out)
        }
        ImageFormat::WebP => {
            let out = ctx.path("kept.webp");
            let (from, to) = (input.to_path_buf(), out.clone());
            tokio::task::spawn_blocking(move || strip_webp(&from, &to)).await??;
            Ok(out)
        }
        // Kept only when libvips found no EXIF/XMP.
        ImageFormat::Avif => Ok(input.to_path_buf()),
        _ => Err("format cannot be kept".into()),
    }
}

/// PNG chunks that affect how pixels look (everything else — `eXIf`,
/// `tEXt`, `iTXt`, `zTXt`, `tIME`, private chunks — is dropped).
const PNG_KEEP: [&[u8; 4]; 12] = [
    b"IHDR", b"PLTE", b"IDAT", b"IEND", b"tRNS", b"iCCP", b"sRGB", b"gAMA", b"cHRM", b"sBIT",
    b"pHYs", b"cICP",
];

pub fn strip_png(input: &Path, output: &Path) -> io::Result<()> {
    let mut reader = BufReader::with_capacity(1 << 20, File::open(input)?);
    let mut writer = BufWriter::with_capacity(1 << 20, File::create(output)?);
    let mut signature = [0; 8];
    reader.read_exact(&mut signature)?;
    if &signature != b"\x89PNG\r\n\x1a\n" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "not a PNG"));
    }
    writer.write_all(&signature)?;
    loop {
        let mut header = [0; 8];
        reader.read_exact(&mut header)?;
        let len = u64::from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]));
        let kind: [u8; 4] = [header[4], header[5], header[6], header[7]];
        let mut chunk = (&mut reader).take(len + 4);
        if PNG_KEEP.contains(&&kind) {
            writer.write_all(&header)?;
            let copied = io::copy(&mut chunk, &mut writer)?;
            if copied != len + 4 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        } else {
            io::copy(&mut chunk, &mut io::sink())?;
        }
        if &kind == b"IEND" {
            break;
        }
    }
    writer
        .into_inner()
        .map_err(io::IntoInnerError::into_error)?
        .sync_all()
}

/// Removes `EXIF` and `XMP ` chunks from a WebP and clears their VP8X flags.
pub fn strip_webp(input: &Path, output: &Path) -> io::Result<()> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "not a WebP");
    let mut file = File::open(input)?;
    let total = file.metadata()?.len();
    let mut header = [0; 12];
    file.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WEBP" {
        return Err(invalid());
    }
    let riff_end = (u64::from(u32::from_le_bytes([
        header[4], header[5], header[6], header[7],
    ])) + 8)
        .min(total);
    // (fourcc, payload offset, padded payload length)
    let mut chunks = Vec::new();
    let mut offset = 12;
    while offset + 8 <= riff_end {
        let mut chunk = [0; 8];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut chunk)?;
        let len = u64::from(u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]));
        let padded = len + (len & 1);
        if offset + 8 + padded > riff_end {
            return Err(invalid());
        }
        chunks.push((
            [chunk[0], chunk[1], chunk[2], chunk[3]],
            offset + 8,
            len,
            padded,
        ));
        offset += 8 + padded;
    }
    chunks.retain(|(kind, ..)| kind != b"EXIF" && kind != b"XMP ");
    let body: u64 = chunks.iter().map(|(_, _, _, padded)| 8 + padded).sum();
    let riff_size = u32::try_from(4 + body).map_err(|_| invalid())?;
    let mut writer = BufWriter::with_capacity(1 << 20, File::create(output)?);
    writer.write_all(b"RIFF")?;
    writer.write_all(&riff_size.to_le_bytes())?;
    writer.write_all(b"WEBP")?;
    for (kind, start, len, padded) in chunks {
        writer.write_all(&kind)?;
        writer.write_all(&u32::try_from(len).map_err(|_| invalid())?.to_le_bytes())?;
        file.seek(SeekFrom::Start(start))?;
        if &kind == b"VP8X" {
            let mut payload = vec![0; usize::try_from(padded).map_err(|_| invalid())?];
            file.read_exact(&mut payload)?;
            if let Some(flags) = payload.first_mut() {
                *flags &= !(0x08 | 0x04); // EXIF, XMP
            }
            writer.write_all(&payload)?;
        } else {
            let copied = io::copy(&mut (&mut file).take(padded), &mut writer)?;
            if copied != padded {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
        }
    }
    writer
        .into_inner()
        .map_err(io::IntoInnerError::into_error)?
        .sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(lossless: Option<u64>, avif: Option<u64>, keepable: bool) -> Facts {
        Facts {
            upload_size: 1000,
            lossless,
            avif,
            keepable,
        }
    }

    #[test]
    fn lossless_wins_within_the_ratio() {
        let s = Settings::default();
        assert_eq!(
            choose(&facts(Some(130), Some(100), true), &s),
            Some(Choice::LosslessWebp)
        );
        assert_eq!(
            choose(&facts(Some(131), Some(100), false), &s),
            Some(Choice::Avif)
        );
        assert_eq!(
            choose(&facts(Some(300), None, false), &s),
            Some(Choice::LosslessWebp)
        );
    }

    #[test]
    fn avif_needs_minimum_savings_to_replace_a_keepable_original() {
        let s = Settings::default();
        // 850 saves exactly 15%: AVIF.
        assert_eq!(
            choose(&facts(Some(5000), Some(850), true), &s),
            Some(Choice::Avif)
        );
        // 851 saves less than 15%: keep the original.
        assert_eq!(
            choose(&facts(Some(5000), Some(851), true), &s),
            Some(Choice::KeepOriginal)
        );
        // HEIC/TIFF/BMP, rotated or resized images always convert.
        assert_eq!(
            choose(&facts(Some(5000), Some(990), false), &s),
            Some(Choice::Avif)
        );
        assert_eq!(
            choose(&facts(Some(5000), Some(2000), false), &s),
            Some(Choice::Avif)
        );
    }

    #[test]
    fn lossless_larger_than_a_keepable_original_keeps_it() {
        let s = Settings::default();
        assert_eq!(
            choose(&facts(Some(1000), Some(900), true), &s),
            Some(Choice::KeepOriginal)
        );
        assert_eq!(
            choose(&facts(Some(1000), Some(900), false), &s),
            Some(Choice::LosslessWebp)
        );
    }

    #[test]
    fn encoder_failures() {
        let s = Settings::default();
        assert_eq!(
            choose(&facts(None, Some(500), true), &s),
            Some(Choice::Avif)
        );
        assert_eq!(
            choose(&facts(None, None, true), &s),
            Some(Choice::KeepOriginal)
        );
        assert_eq!(choose(&facts(None, None, false), &s), None);
    }

    #[test]
    fn ratio_setting_is_respected() {
        let s = Settings {
            image_lossless_ratio: 2.0,
            ..Settings::default()
        };
        assert_eq!(
            choose(&facts(Some(200), Some(100), false), &s),
            Some(Choice::LosslessWebp)
        );
    }

    #[test]
    fn effort_and_chroma() {
        assert_eq!(lossless_effort(12_000_000), 3);
        assert_eq!(lossless_effort(50_000_000), 1);
        // 0.5 bpp screenshot → 4:4:4; 12 bpp photo → 4:2:0.
        assert_eq!(chroma(Some(500_000), 8_000_000), "444");
        assert_eq!(chroma(Some(12_000_000), 8_000_000), "420");
        assert_eq!(chroma(None, 8_000_000), "420");
    }

    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&[1, 2, 3, 4]);
        out
    }

    /// Input/output paths in a scratch directory unique to one test.
    fn temp(test: &str) -> (PathBuf, PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("caper-media-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        (dir.join("in"), dir.join("out"), dir)
    }

    #[test]
    fn png_metadata_is_stripped() {
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in [
            (b"IHDR", &b"0123456789abc"[..]),
            (b"iCCP", b"profile"),
            (b"eXIf", b"MM\0*gps"),
            (b"tEXt", b"Comment\0secret"),
            (b"IDAT", b"pixels"),
            (b"IEND", b""),
        ] {
            png.extend(png_chunk(kind, data));
        }
        let (input, output, dir) = temp("strip-png");
        std::fs::write(&input, &png).unwrap();
        strip_png(&input, &output).unwrap();
        let out = std::fs::read(&output).unwrap();
        let mut expected = b"\x89PNG\r\n\x1a\n".to_vec();
        for (kind, data) in [
            (b"IHDR", &b"0123456789abc"[..]),
            (b"iCCP", b"profile"),
            (b"IDAT", b"pixels"),
            (b"IEND", b""),
        ] {
            expected.extend(png_chunk(kind, data));
        }
        assert_eq!(out, expected);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn riff_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = kind.to_vec();
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn riff(chunks: &[Vec<u8>]) -> Vec<u8> {
        let body: Vec<u8> = chunks.concat();
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&u32::try_from(body.len() + 4).unwrap().to_le_bytes());
        out.extend_from_slice(b"WEBP");
        out.extend(body);
        out
    }

    #[test]
    fn webp_metadata_is_stripped() {
        let vp8x_with = [0x08 | 0x04 | 0x10, 0, 0, 0, 1, 0, 0, 1, 0, 0];
        let vp8x_without = [0x10, 0, 0, 0, 1, 0, 0, 1, 0, 0];
        let input_bytes = riff(&[
            riff_chunk(b"VP8X", &vp8x_with),
            riff_chunk(b"ALPH", b"abc"),
            riff_chunk(b"VP8L", b"pixels"),
            riff_chunk(b"EXIF", b"MM\0*gps!"),
            riff_chunk(b"XMP ", b"<x/>"),
        ]);
        let (input, output, dir) = temp("strip-webp");
        std::fs::write(&input, input_bytes).unwrap();
        strip_webp(&input, &output).unwrap();
        let expected = riff(&[
            riff_chunk(b"VP8X", &vp8x_without),
            riff_chunk(b"ALPH", b"abc"),
            riff_chunk(b"VP8L", b"pixels"),
        ]);
        assert_eq!(std::fs::read(&output).unwrap(), expected);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
