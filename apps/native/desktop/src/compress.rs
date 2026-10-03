//! Attachment preparation before upload: content types, still compression and
//! previews. Mirrors `apps/web/src/chat/uploads.ts` and applies the server's
//! `GET /api/assets/usage` compression settings. The API verifies stored bytes
//! independently; this only saves storage and bandwidth.

use crate::model::AttachmentKind;
use image::{DynamicImage, ImageReader, Limits, Rgb, RgbImage, RgbaImage};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::{Cursor, Read, Seek, SeekFrom};

pub const MAX_ATTACHMENTS: usize = 10;
pub const PREVIEW_MAX_BYTES: usize = 512 * 1024;
const PREVIEW_QUALITY: u8 = 80;
/// Decoding enormous stills can exhaust memory; upload those unchanged.
const MAX_COMPRESS_PIXELS: u64 = 50_000_000;
/// Locally shown thumbnails fit the largest inline frame at 2x.
pub const THUMBNAIL_EDGE: u32 = 720;

/// Server-tunable client compression (`compression` in the usage response).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct Compression {
    /// Lossy still quality, 1–100. 100 disables lossy re-encoding.
    pub image_quality: u8,
    /// Longest still edge in pixels; 0 keeps the original size.
    pub image_max_edge: u32,
    /// Stills with at most this many colours are stored as an exact indexed
    /// PNG. 0 disables; at most 256.
    pub palette_colors: u16,
    pub preview_edge: u32,
    /// Desktop has no bundled transcoder, so video settings are not applied.
    pub video_max_height: u32,
    pub video_bitrate_kbps: u32,
    pub audio_bitrate_kbps: u32,
}

impl Default for Compression {
    fn default() -> Self {
        Self {
            image_quality: 92,
            image_max_edge: 4096,
            palette_colors: 256,
            preview_edge: 640,
            video_max_height: 1080,
            video_bitrate_kbps: 4000,
            audio_bitrate_kbps: 128,
        }
    }
}

/// Mirrors `assets::kind` on the API: only these render inline.
pub fn attachment_kind(content_type: &str) -> AttachmentKind {
    match content_type {
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/avif" => {
            AttachmentKind::Image
        }
        "video/mp4" | "video/webm" | "video/quicktime" => AttachmentKind::Video,
        "audio/mpeg" | "audio/mp4" | "audio/x-m4a" | "audio/aac" | "audio/ogg" | "audio/wav"
        | "audio/x-wav" | "audio/webm" | "audio/flac" => AttachmentKind::Audio,
        _ => AttachmentKind::File,
    }
}

/// Mirrors the API's signature check, so a mislabelled file is uploaded as a
/// download instead of being rejected after upload.
pub fn sniff(content_type: &str, bytes: &[u8]) -> bool {
    let at = |offset: usize, magic: &[u8]| bytes.get(offset..offset + magic.len()) == Some(magic);
    let ftyp = at(4, b"ftyp");
    match content_type {
        "image/png" => at(0, b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => at(0, b"\xff\xd8\xff"),
        "image/gif" => at(0, b"GIF87a") || at(0, b"GIF89a"),
        "image/webp" => at(0, b"RIFF") && at(8, b"WEBP"),
        "image/avif" => {
            ftyp && [b"avif", b"avis", b"mif1", b"msf1"]
                .iter()
                .any(|brand| at(8, *brand))
        }
        "video/mp4" | "audio/mp4" | "audio/x-m4a" => ftyp,
        "video/quicktime" => {
            ftyp || [b"moov", b"mdat", b"wide", b"free"]
                .iter()
                .any(|kind| at(4, *kind))
        }
        "video/webm" | "audio/webm" => at(0, b"\x1a\x45\xdf\xa3"),
        "audio/mpeg" | "audio/aac" => {
            at(0, b"ID3") || (bytes.len() > 1 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0)
        }
        "audio/ogg" => at(0, b"OggS"),
        "audio/wav" | "audio/x-wav" => at(0, b"RIFF") && at(8, b"WAVE"),
        "audio/flac" => at(0, b"fLaC"),
        _ => true,
    }
}

/// Desktop files have names, not browser MIME types: guess from the extension,
/// prefer a still's real signature, and fall back to a download when an
/// inline type's signature does not match.
pub fn content_type_for(name: &str, head: &[u8]) -> String {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    let guessed = match extension.as_str() {
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
        "mp3" => "audio/mpeg",
        "m4a" => "audio/x-m4a",
        "aac" => "audio/aac",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "json" => "application/json",
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        _ => "application/octet-stream",
    };
    let signature = ["image/png", "image/jpeg", "image/gif", "image/webp"]
        .into_iter()
        .find(|candidate| sniff(candidate, head));
    if guessed.starts_with("image/")
        && let Some(signature) = signature
    {
        return signature.into();
    }
    if attachment_kind(guessed) != AttachmentKind::File && !sniff(guessed, head) {
        return "application/octet-stream".into();
    }
    guessed.into()
}

/// Re-encode stills except animated or vector formats (web `compressible`).
pub fn compressible(content_type: &str) -> bool {
    content_type.starts_with("image/")
        && !matches!(content_type, "image/gif" | "image/svg+xml" | "image/avif")
}

/// Keep the re-encoded file only when it is meaningfully (10%) smaller, or
/// when the original could not be displayed inline at all.
pub fn keep_compressed(original_type: &str, original_size: u64, compressed_size: u64) -> bool {
    attachment_kind(original_type) != AttachmentKind::Image
        || (compressed_size as f64) < original_size as f64 * 0.9
}

pub fn renamed(name: &str, content_type: &str) -> String {
    let extension = match content_type {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/jpeg" => "jpg",
        _ => return name.into(),
    };
    let stem = match name.rfind('.') {
        Some(dot) if dot > 0 => &name[..dot],
        _ => name,
    };
    format!("{stem}.{extension}")
}

/// Longest edge scaled to `edge`, never enlarged.
pub fn fit_within(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let longest = width.max(height).max(1);
    if edge == 0 || longest <= edge {
        return (width.max(1), height.max(1));
    }
    let scale = f64::from(edge) / f64::from(longest);
    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}

#[derive(Debug, PartialEq, Eq)]
pub enum StillEncoding {
    /// Exact-palette indexed PNG: lossless.
    Palette,
    /// Lossy JPEG at the server quality.
    Jpeg,
    /// Lossy re-encoding disabled (quality 100) and too many colours.
    None,
}

/// Which encoder a still gets for these settings, given whether its distinct
/// colours fit the palette limit.
pub fn still_encoding(settings: &Compression, colors_within_palette: bool) -> StillEncoding {
    if palette_limit(settings) > 0 && colors_within_palette {
        StillEncoding::Palette
    } else if settings.image_quality < 100 {
        StillEncoding::Jpeg
    } else {
        StillEncoding::None
    }
}

fn palette_limit(settings: &Compression) -> u16 {
    settings.palette_colors.min(256)
}

pub struct PreparedImage {
    pub bytes: Vec<u8>,
    pub content_type: String,
    pub name: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// JPEG preview, at most [`PREVIEW_MAX_BYTES`].
    pub preview: Option<Vec<u8>>,
    pub thumbnail: Option<RgbaImage>,
}

fn decode(bytes: &[u8]) -> Option<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(32_768);
    limits.max_image_height = Some(32_768);
    limits.max_alloc = Some(1 << 30);
    reader.limits(limits);
    reader.decode().ok()
}

fn scaled(image: &DynamicImage, edge: u32) -> DynamicImage {
    let (width, height) = fit_within(image.width(), image.height(), edge);
    if (width, height) == (image.width(), image.height()) {
        image.clone()
    } else {
        image.resize_exact(width, height, image::imageops::FilterType::Lanczos3)
    }
}

/// Compress one still and draw its preview and local thumbnail. Undecodable
/// stills upload unchanged.
pub fn prepare_image(
    name: &str,
    bytes: Vec<u8>,
    content_type: &str,
    settings: &Compression,
) -> PreparedImage {
    let mut prepared = PreparedImage {
        bytes,
        content_type: content_type.into(),
        name: name.into(),
        width: None,
        height: None,
        preview: None,
        thumbnail: None,
    };
    let Some(decoded) = decode(&prepared.bytes) else {
        return prepared;
    };
    let pixels = u64::from(decoded.width()) * u64::from(decoded.height());
    prepared.width = Some(decoded.width());
    prepared.height = Some(decoded.height());
    let mut source = decoded;
    if compressible(content_type) && pixels <= MAX_COMPRESS_PIXELS {
        // Re-encoding also drops EXIF metadata such as photo GPS coordinates.
        let resized = scaled(&source, settings.image_max_edge);
        let rgba = resized.to_rgba8();
        let limit = palette_limit(settings);
        let exact = (limit > 0)
            .then(|| exact_indexed_rgba(&rgba, limit))
            .flatten();
        let candidate = match still_encoding(settings, exact.is_some()) {
            StillEncoding::Palette => exact.and_then(|(palette, indices)| {
                encode_indexed_png(rgba.width(), rgba.height(), &palette, &indices)
                    .ok()
                    .map(|bytes| (bytes, "image/png"))
            }),
            StillEncoding::Jpeg => {
                encode_jpeg(&composite_onto_white(&rgba), settings.image_quality)
                    .ok()
                    .map(|bytes| (bytes, "image/jpeg"))
            }
            StillEncoding::None => None,
        };
        if let Some((bytes, encoded_type)) = candidate
            && keep_compressed(
                content_type,
                prepared.bytes.len() as u64,
                bytes.len() as u64,
            )
        {
            prepared.width = Some(rgba.width());
            prepared.height = Some(rgba.height());
            prepared.bytes = bytes;
            prepared.content_type = encoded_type.into();
            prepared.name = renamed(name, encoded_type);
            source = resized;
        }
    }
    let longest = source.width().max(source.height());
    if attachment_kind(&prepared.content_type) == AttachmentKind::Image
        && settings.preview_edge > 0
        && (longest > settings.preview_edge || prepared.bytes.len() > PREVIEW_MAX_BYTES)
    {
        let preview = scaled(&source, settings.preview_edge).to_rgba8();
        prepared.preview = encode_jpeg(&composite_onto_white(&preview), PREVIEW_QUALITY)
            .ok()
            .filter(|bytes| bytes.len() <= PREVIEW_MAX_BYTES);
    }
    prepared.thumbnail = Some(scaled(&source, THUMBNAIL_EDGE).to_rgba8());
    prepared
}

/// Decode a still for local display only (pending rows and draft chips).
pub fn thumbnail(bytes: &[u8]) -> Option<RgbaImage> {
    decode(bytes).map(|image| scaled(&image, THUMBNAIL_EDGE).to_rgba8())
}

// Adapted from joswayski/captures `crates/captures-image/src/encoding.rs`
// (Apache-2.0, same author): full-resolution chroma and ImageMagick tables.
pub fn encode_jpeg(image: &RgbImage, quality: u8) -> Result<Vec<u8>, String> {
    let width =
        u16::try_from(image.width()).map_err(|_| "JPEG width is too large to encode".to_owned())?;
    let height = u16::try_from(image.height())
        .map_err(|_| "JPEG height is too large to encode".to_owned())?;
    let mut bytes = Vec::new();
    let mut encoder = jpeg_encoder::Encoder::new(&mut bytes, quality.clamp(1, 100));
    encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1);
    encoder.set_quantization_tables(
        jpeg_encoder::QuantizationTableType::ImageMagick,
        jpeg_encoder::QuantizationTableType::ImageMagick,
    );
    encoder
        .encode(image.as_raw(), width, height, jpeg_encoder::ColorType::Rgb)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

// From joswayski/captures `encoding.rs`: JPEG has no alpha, so flatten onto
// white rather than black.
pub fn composite_onto_white(image: &RgbaImage) -> RgbImage {
    let mut output = RgbImage::new(image.width(), image.height());
    for (pixel, destination) in image.pixels().zip(output.pixels_mut()) {
        let alpha = u16::from(pixel[3]);
        let inverse = 255 - alpha;
        *destination = Rgb([
            ((u16::from(pixel[0]) * alpha + 255 * inverse) / 255) as u8,
            ((u16::from(pixel[1]) * alpha + 255 * inverse) / 255) as u8,
            ((u16::from(pixel[2]) * alpha + 255 * inverse) / 255) as u8,
        ]);
    }
    output
}

// Adapted from joswayski/captures `crates/captures-image/src/png.rs`
// `exact_indexed_rgba` (Apache-2.0, same author). Returns the first-seen
// palette and one index per pixel when the image has at most `max_colors`
// distinct RGBA values, so the indexed PNG is pixel-exact.
pub fn exact_indexed_rgba(image: &RgbaImage, max_colors: u16) -> Option<(Vec<[u8; 4]>, Vec<u8>)> {
    let limit = usize::from(max_colors);
    let mut map = HashMap::new();
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut indices = Vec::with_capacity(image.width() as usize * image.height() as usize);
    for pixel in image.pixels() {
        // Flat screenshot regions repeat colours: reuse the previous index
        // without hashing, while retaining first-seen palette order.
        if let Some(&index) = indices.last()
            && palette[usize::from(index)] == pixel.0
        {
            indices.push(index);
            continue;
        }
        if let Some(&index) = map.get(&pixel.0) {
            indices.push(index);
            continue;
        }
        if palette.len() >= limit {
            return None;
        }
        let index = u8::try_from(palette.len()).ok()?;
        map.insert(pixel.0, index);
        palette.push(pixel.0);
        indices.push(index);
    }
    Some((palette, indices))
}

// Adapted from joswayski/captures `png.rs` `encode_indexed_png` and
// `captures-history` `mark_png_as_srgb` (Apache-2.0, same author), for png 0.18.
pub fn encode_indexed_png(
    width: u32,
    height: u32,
    palette_colors: &[[u8; 4]],
    indices: &[u8],
) -> Result<Vec<u8>, String> {
    let mut palette = Vec::with_capacity(palette_colors.len().max(1) * 3);
    let mut trns = Vec::with_capacity(palette_colors.len());
    let mut has_transparency = false;
    for color in palette_colors {
        palette.extend_from_slice(&color[..3]);
        trns.push(color[3]);
        has_transparency |= color[3] < 255;
    }
    // The png crate requires a non-empty palette for indexed images.
    if palette.is_empty() {
        palette.extend_from_slice(&[0, 0, 0]);
        trns.push(255);
    }
    let depth = match palette_colors.len() {
        0..=2 => png::BitDepth::One,
        3..=4 => png::BitDepth::Two,
        5..=16 => png::BitDepth::Four,
        _ => png::BitDepth::Eight,
    };
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(depth);
        encoder.set_compression(png::Compression::High);
        // Palette indices are categorical, not intensities: byte-difference
        // filters add noise and inflate the deflate stream.
        encoder.set_filter(png::Filter::NoFilter);
        // Untagged PNGs are treated as generic RGB on some colour-managed displays.
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        encoder.set_source_gamma(png::ScaledFloat::from_scaled(45_455));
        encoder.set_source_chromaticities(png::SourceChromaticities::new(
            (0.3127, 0.3290),
            (0.6400, 0.3300),
            (0.3000, 0.6000),
            (0.1500, 0.0600),
        ));
        encoder.set_palette(palette);
        if has_transparency {
            encoder.set_trns(trns);
        }
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        // PNG stores sub-byte samples most-significant first, padding each row
        // independently.
        let packed;
        let data = if depth == png::BitDepth::Eight {
            indices
        } else {
            let bits = depth as usize;
            let per_byte = 8 / bits;
            packed = indices
                .chunks(width as usize)
                .flat_map(|row| {
                    row.chunks(per_byte).map(|chunk| {
                        chunk.iter().enumerate().fold(0, |byte, (offset, index)| {
                            byte | (index << (8 - bits * (offset + 1)))
                        })
                    })
                })
                .collect::<Vec<u8>>();
            packed.as_slice()
        };
        writer
            .write_image_data(data)
            .map_err(|error| error.to_string())?;
    }
    Ok(bytes)
}

/// Dimensions and duration from an MP4/QuickTime header (`mvhd`, `tkhd`).
/// Reads only box headers and the `moov` box; anything unexpected yields
/// `None`s, and the file is uploaded without them.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct VideoInfo {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
}

pub fn probe_mp4<R: Read + Seek>(reader: &mut R) -> VideoInfo {
    let end = reader.seek(SeekFrom::End(0)).unwrap_or(0);
    let mut position = 0;
    while position + 8 <= end {
        let Some((kind, header, size)) = box_header(reader, position, end) else {
            break;
        };
        if &kind == b"moov" {
            let body = size.saturating_sub(header);
            if body > 64 * 1024 * 1024 {
                break;
            }
            let mut moov = vec![0; body as usize];
            if reader.seek(SeekFrom::Start(position + header)).is_err()
                || reader.read_exact(&mut moov).is_err()
            {
                break;
            }
            return parse_moov(&moov);
        }
        position += size;
    }
    VideoInfo::default()
}

fn box_header<R: Read + Seek>(
    reader: &mut R,
    position: u64,
    end: u64,
) -> Option<([u8; 4], u64, u64)> {
    reader.seek(SeekFrom::Start(position)).ok()?;
    let mut head = [0; 8];
    reader.read_exact(&mut head).ok()?;
    let kind = [head[4], head[5], head[6], head[7]];
    let size = u64::from(u32::from_be_bytes([head[0], head[1], head[2], head[3]]));
    let (header, size) = match size {
        0 => (8, end - position),
        1 => {
            let mut large = [0; 8];
            reader.read_exact(&mut large).ok()?;
            (16, u64::from_be_bytes(large))
        }
        size => (8, size),
    };
    (size >= header && position.checked_add(size)? <= end).then_some((kind, header, size))
}

fn children(bytes: &[u8]) -> impl Iterator<Item = ([u8; 4], &[u8])> {
    let mut offset = 0;
    std::iter::from_fn(move || {
        let head = bytes.get(offset..offset + 8)?;
        let size = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        let kind = [head[4], head[5], head[6], head[7]];
        if size < 8 {
            return None;
        }
        let body = bytes.get(offset + 8..offset.checked_add(size)?)?;
        offset += size;
        Some((kind, body))
    })
}

fn be32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn be64(bytes: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(bytes.get(at..at + 8)?.try_into().ok()?))
}

fn parse_moov(moov: &[u8]) -> VideoInfo {
    let mut info = VideoInfo::default();
    for (kind, body) in children(moov) {
        match &kind {
            b"mvhd" => {
                let (timescale, duration) = if body.first() == Some(&1) {
                    (be32(body, 20), be64(body, 24))
                } else {
                    (be32(body, 12), be32(body, 16).map(u64::from))
                };
                if let (Some(timescale), Some(duration)) = (timescale, duration)
                    && timescale > 0
                    && duration != u64::from(u32::MAX)
                    && duration != u64::MAX
                {
                    let milliseconds = u128::from(duration) * 1000 / u128::from(timescale);
                    info.duration_ms = u64::try_from(milliseconds)
                        .ok()
                        .filter(|value| *value <= 24 * 60 * 60 * 1000);
                }
            }
            b"trak" if info.width.is_none() => {
                if let Some((_, tkhd)) = children(body).find(|(kind, _)| kind == b"tkhd") {
                    // Version 1 widens creation/modification/duration to 64 bits.
                    let matrix = if tkhd.first() == Some(&1) { 52 } else { 40 };
                    let fixed = |at| be32(tkhd, at).map(|value| value >> 16);
                    if let (Some(a), Some(b), Some(width), Some(height)) = (
                        be32(tkhd, matrix),
                        be32(tkhd, matrix + 4),
                        fixed(matrix + 36),
                        fixed(matrix + 40),
                    ) && (1..=32_768).contains(&width)
                        && (1..=32_768).contains(&height)
                    {
                        // A 90/270 degree rotation matrix swaps the display axes.
                        let rotated = a == 0 && b != 0;
                        info.width = Some(if rotated { height } else { width });
                        info.height = Some(if rotated { width } else { height });
                    }
                }
            }
            _ => {}
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn content_types_follow_extensions_and_signatures() {
        assert_eq!(
            content_type_for("a.PNG", b"\x89PNG\r\n\x1a\nrest"),
            "image/png"
        );
        // A still's real signature wins over its extension.
        assert_eq!(
            content_type_for("photo.png", b"\xff\xd8\xff\xe0"),
            "image/jpeg"
        );
        // An inline type whose bytes do not match becomes a download.
        assert_eq!(
            content_type_for("clip.mp4", b"<html>"),
            "application/octet-stream"
        );
        assert_eq!(
            content_type_for("clip.mp4", b"\0\0\0\x18ftypisom"),
            "video/mp4"
        );
        assert_eq!(content_type_for("notes.pdf", b"%PDF"), "application/pdf");
        assert_eq!(
            content_type_for("README", b"text"),
            "application/octet-stream"
        );
        assert_eq!(content_type_for("vector.svg", b"<svg"), "image/svg+xml");
        assert_eq!(attachment_kind("image/svg+xml"), AttachmentKind::File);
        assert_eq!(attachment_kind("audio/x-m4a"), AttachmentKind::Audio);
    }

    #[test]
    fn compression_decisions_match_the_web_client() {
        assert!(compressible("image/png"));
        assert!(compressible("image/heic"));
        assert!(!compressible("image/gif"));
        assert!(!compressible("image/svg+xml"));
        assert!(!compressible("image/avif"));
        assert!(!compressible("video/mp4"));
        // Keep only when at least 10% smaller, unless the original cannot render inline.
        assert!(keep_compressed("image/png", 1000, 899));
        assert!(!keep_compressed("image/png", 1000, 900));
        assert!(keep_compressed("image/heic", 1000, 2000));
        assert_eq!(renamed("shot.final.PNG", "image/jpeg"), "shot.final.jpg");
        assert_eq!(renamed("diagram", "image/png"), "diagram.png");
        assert_eq!(renamed(".hidden", "image/webp"), ".hidden.webp");
        assert_eq!(renamed("clip.mov", "video/mp4"), "clip.mov");
        assert_eq!(fit_within(8000, 4000, 4096), (4096, 2048));
        assert_eq!(fit_within(300, 200, 640), (300, 200));
        assert_eq!(fit_within(300, 200, 0), (300, 200));
        assert_eq!(fit_within(1, 10_000, 640), (1, 640));
        let defaults = Compression::default();
        assert_eq!(still_encoding(&defaults, true), StillEncoding::Palette);
        assert_eq!(still_encoding(&defaults, false), StillEncoding::Jpeg);
        let lossless_only = Compression {
            image_quality: 100,
            ..Compression::default()
        };
        assert_eq!(still_encoding(&lossless_only, false), StillEncoding::None);
        let no_palette = Compression {
            palette_colors: 0,
            ..Compression::default()
        };
        assert_eq!(still_encoding(&no_palette, true), StillEncoding::Jpeg);
    }

    #[test]
    fn usage_compression_settings_decode_with_defaults() {
        let parsed: Compression = serde_json::from_str(
            r#"{"imageQuality":80,"imageMaxEdge":2048,"paletteColors":64,"previewEdge":320,"futureSetting":1}"#,
        )
        .unwrap();
        assert_eq!(parsed.image_quality, 80);
        assert_eq!(parsed.image_max_edge, 2048);
        assert_eq!(parsed.palette_colors, 64);
        assert_eq!(parsed.preview_edge, 320);
        assert_eq!(parsed.video_max_height, 1080);
        let empty: Compression = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, Compression::default());
    }

    fn png_bytes(image: &RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn screenshot_becomes_a_pixel_exact_palette_png() {
        // A UI-like still: flat regions, translucent and transparent pixels,
        // and few colours, with rows whose width does not fill a byte.
        let colors = [
            [12, 13, 15, 255],
            [182, 77, 50, 255],
            [243, 244, 245, 255],
            [99, 122, 67, 128],
            [0, 0, 0, 0],
        ];
        let image = RgbaImage::from_fn(333, 211, |x, y| {
            Rgba(colors[((x / 17 + y / 13) as usize + (x * y) as usize % 3) % colors.len()])
        });
        let original = png_bytes(&image);
        let prepared = prepare_image(
            "screen.png",
            original.clone(),
            "image/png",
            &Compression::default(),
        );
        assert_eq!(prepared.content_type, "image/png");
        assert_eq!(prepared.name, "screen.png");
        assert!(prepared.bytes.len() < original.len() * 9 / 10);
        let reader = png::Decoder::new(Cursor::new(&prepared.bytes))
            .read_info()
            .unwrap();
        assert_eq!(reader.info().color_type, png::ColorType::Indexed);
        let decoded = image::load_from_memory(&prepared.bytes).unwrap().to_rgba8();
        assert_eq!(decoded, image, "palette PNG must be pixel-exact");
        assert_eq!((prepared.width, prepared.height), (Some(333), Some(211)));
        assert!(prepared.preview.is_none(), "small stills need no preview");
    }

    #[test]
    fn indexed_png_packing_round_trips_every_bit_depth() {
        for colors in [1_usize, 2, 3, 4, 5, 16, 17, 256] {
            let palette: Vec<[u8; 4]> = (0..colors)
                .map(|index| {
                    let value = index as u8;
                    [
                        value,
                        255 - value,
                        value / 2,
                        if index % 2 == 0 { 255 } else { value },
                    ]
                })
                .collect();
            for width in [1_u32, 3, 7, 8, 9, 17] {
                let height = 5;
                let indices: Vec<u8> = (0..width * height)
                    .map(|index| (index as usize % colors) as u8)
                    .collect();
                let bytes = encode_indexed_png(width, height, &palette, &indices).unwrap();
                let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
                let expected: Vec<u8> = indices
                    .iter()
                    .flat_map(|&index| palette[usize::from(index)])
                    .collect();
                assert_eq!(
                    decoded.as_raw(),
                    &expected,
                    "{colors} colours, width {width}"
                );
            }
        }
    }

    #[test]
    fn exact_palette_respects_the_colour_limit() {
        let image = RgbaImage::from_fn(6, 2, |x, y| Rgba([(x + y * 6) as u8 % 3, 0, 0, 255]));
        assert!(exact_indexed_rgba(&image, 2).is_none());
        let (palette, indices) = exact_indexed_rgba(&image, 3).unwrap();
        assert_eq!(palette.len(), 3);
        assert_eq!(indices.len(), 12);
    }

    #[test]
    fn photos_become_smaller_jpegs_scaled_with_previews() {
        let image = RgbaImage::from_fn(1600, 900, |x, y| {
            Rgba([
                (x * 7 % 251) as u8,
                (y * 11 % 241) as u8,
                ((x ^ y) % 239) as u8,
                if x < 20 { 0 } else { 255 },
            ])
        });
        let original = png_bytes(&image);
        let settings = Compression {
            image_max_edge: 1200,
            ..Compression::default()
        };
        let prepared = prepare_image("photo.png", original.clone(), "image/png", &settings);
        assert_eq!(prepared.content_type, "image/jpeg");
        assert_eq!(prepared.name, "photo.jpg");
        assert_eq!((prepared.width, prepared.height), (Some(1200), Some(675)));
        assert!(prepared.bytes.len() < original.len() * 9 / 10);
        let decoded = image::load_from_memory(&prepared.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1200, 675));
        // Transparent pixels are flattened onto white, not black.
        assert!(
            decoded
                .to_rgb8()
                .get_pixel(2, 2)
                .0
                .iter()
                .all(|&value| value > 240)
        );
        let preview = image::load_from_memory(prepared.preview.as_ref().unwrap()).unwrap();
        assert_eq!((preview.width(), preview.height()), (640, 360));
        assert!(prepared.preview.unwrap().len() <= PREVIEW_MAX_BYTES);
        assert_eq!(prepared.thumbnail.unwrap().dimensions(), (720, 405));
    }

    #[test]
    fn stills_that_do_not_shrink_or_cannot_decode_upload_unchanged() {
        let tiny = png_bytes(&RgbaImage::from_pixel(2, 2, Rgba([1, 2, 3, 255])));
        let prepared = prepare_image(
            "dot.png",
            tiny.clone(),
            "image/png",
            &Compression::default(),
        );
        assert_eq!(prepared.bytes, tiny);
        assert_eq!(prepared.content_type, "image/png");
        let garbage = b"not an image".to_vec();
        let prepared = prepare_image(
            "x.heic",
            garbage.clone(),
            "image/heic",
            &Compression::default(),
        );
        assert_eq!(prepared.bytes, garbage);
        assert_eq!(prepared.width, None);
        assert!(prepared.thumbnail.is_none());
    }

    fn mp4_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(body);
        bytes
    }

    #[test]
    fn mp4_headers_give_dimensions_duration_and_rotation() {
        let mut mvhd = vec![0; 100];
        mvhd[12..16].copy_from_slice(&600_u32.to_be_bytes());
        mvhd[16..20].copy_from_slice(&3_000_u32.to_be_bytes());
        let tkhd = |rotated: bool| {
            let mut tkhd = vec![0; 84];
            let (a, b): (u32, u32) = if rotated {
                (0, 0x0001_0000)
            } else {
                (0x0001_0000, 0)
            };
            tkhd[40..44].copy_from_slice(&a.to_be_bytes());
            tkhd[44..48].copy_from_slice(&b.to_be_bytes());
            tkhd[76..80].copy_from_slice(&(1920_u32 << 16).to_be_bytes());
            tkhd[80..84].copy_from_slice(&(1080_u32 << 16).to_be_bytes());
            tkhd
        };
        for rotated in [false, true] {
            let mut file = mp4_box(b"ftyp", b"isom\0\0\0\0");
            file.extend(mp4_box(b"mdat", &[0; 32]));
            let mut moov = mp4_box(b"mvhd", &mvhd);
            moov.extend(mp4_box(b"trak", &mp4_box(b"tkhd", &tkhd(rotated))));
            file.extend(mp4_box(b"moov", &moov));
            let info = probe_mp4(&mut Cursor::new(file));
            assert_eq!(info.duration_ms, Some(5_000));
            assert_eq!(
                (info.width, info.height),
                if rotated {
                    (Some(1080), Some(1920))
                } else {
                    (Some(1920), Some(1080))
                }
            );
        }
        assert_eq!(
            probe_mp4(&mut Cursor::new(b"\0\0\0\x08junk".to_vec())),
            VideoInfo::default()
        );
        assert_eq!(
            probe_mp4(&mut Cursor::new(Vec::new())),
            VideoInfo::default()
        );
    }
}
