//! Custom profile pictures and space icons (`avatarAsset`/`avatarUrl` on
//! people, `iconAsset`/`iconUrl` on spaces; the shared pictures contract in
//! docs/media.md): preparing a picked image for upload, uploading or removing
//! it, and decoding it for display. Showing pictures is the UI's.

use crate::api::Api;
use crate::host::Rgba;
use crate::uploads::UploadError;
use image::{DynamicImage, ImageDecoder};
use serde_json::{Value, json};
use std::time::Duration;

pub const INVALID: &str = "That picture couldn't be used. Try another one.";
pub const UNAVAILABLE: &str = "Picture uploads aren’t available right now.";
/// GIFs upload unchanged up to this size (the API's limit); bigger ones are
/// shrunk to fit it.
pub const MAX_GIF_BYTES: usize = 5 * 1024 * 1024;
/// Every other picture is re-encoded to at most this size.
pub const MAX_STILL_BYTES: usize = 1024 * 1024;
/// The API's largest picture edge.
pub const MAX_DIMENSION: u32 = 1024;
/// Re-encoded pictures are squares of at most this edge.
pub const UPLOAD_EDGE: u32 = 512;
/// Larger files are not read: no picture needs them.
pub const MAX_SOURCE_BYTES: u64 = 100 * 1024 * 1024;
/// Lossy WebP qualities tried in turn until the picture fits.
const QUALITIES: [u8; 3] = [90, 80, 70];

/// What the image picker offers. HEIC and AVIF are listed so choosing one
/// explains why it can't be used here.
pub const PICKER_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "jpe", "jfif", "gif", "webp", "bmp", "tif", "tiff", "heic", "heif",
    "avif",
];

/// Output edges for a shrunk GIF, largest first: the first that fits wins.
const SHRINK_EDGES: [u32; 5] = [512, 384, 256, 192, 128];
pub const UNREADABLE: &str = "Caper can’t read that file. Choose a PNG, JPEG, WebP or GIF.";
pub const UNSUPPORTED: &str =
    "HEIC and AVIF pictures can’t be used on this computer. Choose a PNG, JPEG, WebP or GIF.";
/// Decoded pictures fit this square: crisp at 64 points on a 2x display.
pub const DISPLAY_EDGE: u32 = 128;
/// Below this edge an animation would be blurry: show a still instead.
pub const MIN_ANIMATED_EDGE: u32 = 48;
pub const MAX_FRAMES: usize = 600;
/// Retained frame pixels for one animation (128 frames at 128 px).
pub const FRAME_BUDGET: usize = 8 * 1024 * 1024;
/// Display frames and how long each shows; a still is one frame.
pub type Frames = Vec<(Rgba, Duration)>;

/// What a person or space shows instead of the mascot or initial.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source<'a> {
    /// The stable cache key; changes whenever the picture changes.
    pub asset: &'a str,
    /// Absent when the server signs no CDN URLs: only a cached picture shows.
    pub url: Option<&'a str>,
}

impl<'a> Source<'a> {
    /// `None` without an asset: no custom picture.
    pub fn new(asset: &'a Option<String>, url: &'a Option<String>) -> Option<Self> {
        asset
            .as_deref()
            .filter(|asset| !asset.is_empty())
            .map(|asset| Self {
                asset,
                url: url.as_deref().filter(|url| !url.is_empty()),
            })
    }
}

/// Which picture an upload or removal changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Avatar,
    /// A space's icon, by space id.
    SpaceIcon(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prepared {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
    pub width: u32,
    pub height: u32,
}

impl Prepared {
    /// `avatar.<ext>` or `space-icon.<ext>`.
    pub fn filename(&self, target: &Target) -> String {
        let stem = match target {
            Target::Avatar => "avatar",
            Target::SpaceIcon(_) => "space-icon",
        };
        let extension = match self.content_type {
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/png" => "png",
            _ => "jpg",
        };
        format!("{stem}.{extension}")
    }
}

/// Why a picture change failed, worded for the settings row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub message: String,
    /// The server has no picture storage (`503`): the controls step back.
    pub unavailable: bool,
}

impl Failure {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            unavailable: false,
        }
    }
}

/// The API's picture errors as sentences (contract "Uploading a picture").
pub fn failure(status: Option<u16>, code: Option<&str>, message: &str, target: &Target) -> Failure {
    let text = match (code, status) {
        (Some("picture_type"), _) => "Choose a PNG, JPEG, WebP or GIF picture.",
        (Some("picture_too_large"), _) | (_, Some(413)) => {
            "That picture is too large. Try another one."
        }
        (Some("picture_rate_limited"), _) | (_, Some(429)) => {
            "You’ve changed pictures too often today. Try again later."
        }
        (Some("picture_invalid"), _) | (_, Some(422)) => INVALID,
        (_, Some(400)) => "Pictures can be up to 1024 × 1024.",
        (_, Some(404)) => match target {
            Target::Avatar => "That’s no longer available.",
            Target::SpaceIcon(_) => "Only the space owner can change its icon.",
        },
        (_, Some(409)) => "The upload didn’t finish. Try again.",
        (_, Some(503)) => UNAVAILABLE,
        _ => message,
    };
    Failure {
        message: text.into(),
        unavailable: status == Some(503),
    }
}

pub fn upload_failure(error: &UploadError, target: &Target) -> Failure {
    failure(error.status, error.code.as_deref(), &error.message, target)
}

pub fn api_failure(error: &crate::api::ApiError, target: &Target) -> Failure {
    failure(
        error.status.map(|status| status.as_u16()),
        error.code.as_deref(),
        &error.message,
        target,
    )
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviousPicture {
    pub avatar_asset: String,
    pub avatar_url: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub pictures: Vec<PreviousPicture>,
    pub next_cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// Preparing a picked picture for upload.

/// GIFs are recognised by their magic bytes, never by extension.
pub fn is_gif(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

/// A GIF's logical screen width and height (bytes 6–9, little-endian).
pub fn gif_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if !is_gif(bytes) || bytes.len() < 10 {
        return None;
    }
    let width = u16::from_le_bytes([bytes[6], bytes[7]]);
    let height = u16::from_le_bytes([bytes[8], bytes[9]]);
    Some((u32::from(width), u32::from(height)))
}

/// How many images a GIF holds, read from its block structure without
/// decompressing anything. A file cut short counts the images before the cut.
pub fn gif_frame_count(bytes: &[u8]) -> Option<usize> {
    fn table(packed: u8) -> usize {
        if packed & 0x80 == 0 {
            0
        } else {
            3 << ((packed & 7) + 1)
        }
    }
    fn sub_blocks(bytes: &[u8], mut at: usize) -> Option<usize> {
        loop {
            let length = usize::from(*bytes.get(at)?);
            at += 1;
            if length == 0 {
                return Some(at);
            }
            at += length;
        }
    }
    if !is_gif(bytes) {
        return None;
    }
    let mut at = 13 + table(*bytes.get(10)?);
    let mut frames = 0;
    loop {
        let next = match bytes.get(at) {
            Some(0x21) => sub_blocks(bytes, at + 2),
            Some(0x2c) => {
                let packed = bytes.get(at + 9);
                let start = packed.map(|packed| at + 10 + table(*packed) + 1);
                let end = start.and_then(|start| sub_blocks(bytes, start));
                if end.is_some() {
                    frames += 1;
                }
                end
            }
            // The trailer, or anything unexpected: stop counting.
            _ => None,
        };
        match next {
            Some(next) => at = next,
            None => return (frames > 0).then_some(frames),
        }
    }
}

/// The centred square crop of a `width` × `height` image: `(x, y, side)`.
pub fn square_crop(width: u32, height: u32) -> (u32, u32, u32) {
    let side = width.min(height);
    ((width - side) / 2, (height - side) / 2, side)
}

/// HEIF-family files (HEIC, AVIF) start with an ISO `ftyp` box; the bundled
/// decoders read none of them.
fn heif_family(bytes: &[u8]) -> bool {
    bytes.get(4..8) == Some(b"ftyp")
}

/// Constrain the visible editor frame to a pixel-square region about its center.
pub fn square_frame(
    crop: crate::attachment_edit::Crop,
    width: u32,
    height: u32,
) -> crate::attachment_edit::Crop {
    let side = (crop.width * f64::from(width)).min(crop.height * f64::from(height));
    let w = side / f64::from(width);
    let h = side / f64::from(height);
    crate::attachment_edit::Crop {
        x: crop.x + (crop.width - w) / 2.0,
        y: crop.y + (crop.height - h) / 2.0,
        width: w,
        height: h,
    }
}

pub fn prepare_edited(
    bytes: Vec<u8>,
    edit: Option<&crate::attachment_edit::Edit>,
) -> Result<Prepared, String> {
    let Some(edit) = edit else {
        return prepare(bytes);
    };
    let (image, icc) = crate::compress::decode_upright(&bytes).ok_or(UNREADABLE)?;
    let icc = icc.filter(|icc| icc.get(16..20) == Some(b"RGB "));
    let rendered =
        crate::attachment_edit::render(&image, edit).ok_or(crate::attachment_edit::FAILED)?;
    let edge = rendered.width().min(rendered.height()).min(UPLOAD_EDGE);
    encode(
        &rendered.resize_exact(edge, edge, image::imageops::FilterType::Lanczos3),
        icc.as_deref(),
    )
}

/// Prepare picked bytes under the picture upload policy. Unedited GIFs retain
/// animation; stills become square, metadata-free images of at most 512 px.
pub fn prepare(bytes: Vec<u8>) -> Result<Prepared, String> {
    if is_gif(&bytes) {
        let (width, height) = gif_dimensions(&bytes)
            .filter(|(width, height)| *width > 0 && *height > 0)
            .ok_or(INVALID)?;
        if bytes.len() > MAX_GIF_BYTES || width > MAX_DIMENSION || height > MAX_DIMENSION {
            return shrink_gif(&bytes, MAX_GIF_BYTES);
        }
        return Ok(Prepared {
            bytes,
            content_type: "image/gif",
            width,
            height,
        });
    }
    let Some((image, icc)) = crate::compress::decode_upright(&bytes) else {
        return Err(if heif_family(&bytes) {
            UNSUPPORTED
        } else {
            UNREADABLE
        }
        .into());
    };
    let (x, y, side) = square_crop(image.width(), image.height());
    if side == 0 {
        return Err(INVALID.into());
    }
    let edge = side.min(UPLOAD_EDGE);
    let square = image.crop_imm(x, y, side, side);
    let square = if edge == side {
        square
    } else {
        square.resize_exact(edge, edge, image::imageops::FilterType::Lanczos3)
    };
    // Re-encodings keep an RGB colour profile so colours match.
    let icc = icc.filter(|icc| icc.get(16..20) == Some(b"RGB "));
    encode(&square, icc.as_deref())
}

fn encode(image: &DynamicImage, icc: Option<&[u8]>) -> Result<Prepared, String> {
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let opaque = rgba.pixels().all(|pixel| pixel[3] == 255);
    let rgb = opaque.then(|| image.to_rgb8());
    for quality in QUALITIES {
        let pixels = rgb.as_ref().map_or(rgba.as_raw(), |rgb| rgb.as_raw());
        let encoded = match crate::compress::encode_lossy_webp(
            pixels, width, height, !opaque, quality, icc,
        ) {
            Ok(bytes) => Some((bytes, "image/webp")),
            Err(_) => crate::compress::encode_jpeg(
                &crate::compress::composite_onto_white(&rgba),
                quality,
                icc,
            )
            .ok()
            .map(|bytes| (bytes, "image/jpeg")),
        };
        if let Some((bytes, content_type)) = encoded
            && bytes.len() <= MAX_STILL_BYTES
        {
            return Ok(Prepared {
                bytes,
                content_type,
                width,
                height,
            });
        }
    }
    Err(INVALID.into())
}

/// `POST /api/assets` for a picture: no channel, preview or source size.
pub fn reservation(target: &Target, prepared: &Prepared) -> Value {
    let mut body = json!({
        "purpose": match target {
            Target::Avatar => "avatar",
            Target::SpaceIcon(_) => "spaceIcon",
        },
        "filename": prepared.filename(target),
        "contentType": prepared.content_type,
        "byteSize": prepared.bytes.len(),
        "width": prepared.width,
        "height": prepared.height,
    });
    if let Target::SpaceIcon(space) = target {
        body["spaceId"] = json!(space);
    }
    body
}

/// Shrink a GIF too large to upload unchanged, as the web's `gif-shrink.ts`
/// does: every composited frame cropped to a centred square and scaled down,
/// keeping its timing. 512 px first, then smaller edges, then fewer frames
/// (each lasting as long as the ones it replaces).
pub fn shrink_gif(bytes: &[u8], budget: usize) -> Result<Prepared, String> {
    let (width, height) = gif_dimensions(bytes)
        .filter(|(width, height)| *width > 0 && *height > 0)
        .ok_or(INVALID)?;
    let short = width.min(height);
    let mut edges: Vec<u32> = SHRINK_EDGES
        .into_iter()
        .filter(|edge| *edge <= short)
        .collect();
    if edges.is_empty() {
        edges.push(short);
    }
    let frames = gif_frame_count(bytes).unwrap_or(1);
    let (mut edge, mut step) = (edges[0], 1);
    loop {
        let output = encode_shrunk(bytes, edge, step)?;
        if output.len() <= budget {
            return Ok(Prepared {
                bytes: output,
                content_type: "image/gif",
                width: edge,
                height: edge,
            });
        }
        // Jump straight to the edge the size suggests, else halve the frames.
        let target = f64::from(edge) * (budget as f64 / output.len() as f64).sqrt();
        let below: Vec<u32> = edges
            .iter()
            .copied()
            .filter(|candidate| *candidate < edge)
            .collect();
        if let Some(smaller) = below
            .iter()
            .copied()
            .find(|candidate| f64::from(*candidate) <= target)
            .or(below.last().copied())
        {
            edge = smaller;
        } else if step * 2 < frames {
            step *= 2;
        } else {
            return Err("That GIF is too large. Try another one.".into());
        }
    }
}

/// An `edge`-pixel square GIF of every `step`th composited frame.
fn encode_shrunk(bytes: &[u8], edge: u32, step: usize) -> Result<Vec<u8>, String> {
    use image::AnimationDecoder;
    use image::codecs::gif::{GifDecoder, GifEncoder, Repeat};
    let mut decoder = GifDecoder::new(std::io::Cursor::new(bytes)).map_err(|_| UNREADABLE)?;
    decoder.set_limits(limits()).map_err(|_| UNREADABLE)?;
    let mut output = Vec::new();
    {
        let mut encoder = GifEncoder::new_with_speed(&mut output, 10);
        encoder.set_repeat(Repeat::Infinite).map_err(|_| INVALID)?;
        // A kept frame waits until the frames it stands in for add their time.
        let mut pending: Option<(image::RgbaImage, u32)> = None;
        let flush = |encoder: &mut GifEncoder<&mut Vec<u8>>,
                     (image, ms): (image::RgbaImage, u32)| {
            encoder
                .encode_frame(image::Frame::from_parts(
                    image,
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(ms, 1),
                ))
                .map_err(|_| INVALID.to_string())
        };
        for (index, frame) in decoder.into_frames().enumerate() {
            let frame = frame.map_err(|_| UNREADABLE)?;
            let (numer, denom) = frame.delay().numer_denom_ms();
            let ms = numer / denom.max(1);
            if index % step != 0 {
                if let Some((_, total)) = pending.as_mut() {
                    *total += ms;
                }
                continue;
            }
            if let Some(ready) = pending.take() {
                flush(&mut encoder, ready)?;
            }
            let buffer = frame.into_buffer();
            let (x, y, side) = square_crop(buffer.width(), buffer.height());
            let square = image::imageops::crop_imm(&buffer, x, y, side, side).to_image();
            let scaled =
                image::imageops::resize(&square, edge, edge, image::imageops::FilterType::Triangle);
            pending = Some((scaled, ms));
        }
        match pending.take() {
            Some(ready) => flush(&mut encoder, ready)?,
            None => return Err(UNREADABLE.into()),
        }
    }
    Ok(output)
}

/// A successful change: the account or space as the server now has it.
#[derive(Debug)]
pub enum Applied {
    Account(crate::model::Account),
    Space(crate::model::Space),
}

impl Applied {
    /// The picture now applied, if any.
    pub fn asset(&self) -> Option<&str> {
        match self {
            Self::Account(account) => account.avatar_asset.as_deref(),
            Self::Space(space) => space.icon_asset.as_deref(),
        }
    }
}

/// A finished change, with the uploaded picture decoded for display when the
/// server applied it, so it shows without a download.
pub struct Changed {
    pub applied: Applied,
    pub local: Option<(String, Frames)>,
}

/// Read, prepare, upload and apply a picked picture. Runs off the UI thread.
pub fn upload(
    api: &Api,
    token: &str,
    target: &Target,
    path: &std::path::Path,
    animate: bool,
    edit: Option<&crate::attachment_edit::Edit>,
) -> Result<Changed, Failure> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|_| Failure::new("Could not read that file."))?;
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::new("Could not read that file."))?;
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err(Failure::new("That file is too large to use as a picture."));
    }
    let prepared = prepare_edited(bytes, edit).map_err(Failure::new)?;
    let (id, completed) = crate::uploads::upload_picture(
        api,
        token,
        reservation(target, &prepared),
        prepared.bytes.clone(),
    )
    .map_err(|error| upload_failure(&error, target))?;
    let applied = match target {
        Target::Avatar => serde_json::from_value(completed["account"].clone())
            .map(Applied::Account)
            .ok(),
        Target::SpaceIcon(space) => {
            serde_json::from_value::<crate::model::Space>(completed["space"].clone())
                .ok()
                .filter(|applied| applied.id == *space)
                .map(Applied::Space)
        }
    }
    .ok_or_else(|| Failure::new("The upload service returned an invalid response."))?;
    if *target == Target::Avatar {
        api.reset_chat_session();
    }
    // A late repeated completion returns the current picture without
    // applying this one: only show local frames for the applied upload.
    let local = (applied.asset() == Some(id.as_str()))
        .then(|| decode(&prepared.bytes, animate))
        .flatten()
        .map(|frames| (id, frames));
    Ok(Changed { applied, local })
}

pub fn select(
    api: &Api,
    token: &str,
    asset: &str,
    expected: Option<&str>,
) -> Result<Changed, Failure> {
    let account = api.select_avatar(token, asset, expected).map_err(|error| {
        if error.status.is_some_and(|status| status.as_u16() == 409) {
            Failure::new(
                "Your picture changed elsewhere. Close and reopen Previous pictures to try again.",
            )
        } else {
            api_failure(&error, &Target::Avatar)
        }
    })?;
    api.reset_chat_session();
    Ok(Changed {
        applied: Applied::Account(account),
        local: None,
    })
}

/// `DELETE` the picture: back to the saved mascot or the space initial.
pub fn remove(api: &Api, token: &str, target: &Target) -> Result<Changed, Failure> {
    let applied = match target {
        Target::Avatar => api.remove_avatar(token).map(Applied::Account),
        Target::SpaceIcon(space) => api.remove_space_icon(token, space).map(Applied::Space),
    }
    .map_err(|error| api_failure(&error, target))?;
    if *target == Target::Avatar {
        api.reset_chat_session();
    }
    Ok(Changed {
        applied,
        local: None,
    })
}

// ---------------------------------------------------------------------------
// Decoding for display.

/// The square edge an animation of `frames` frames (from a `side`-pixel
/// square) is kept at within [`FRAME_BUDGET`], or `None` for a still.
pub fn animated_edge(frames: usize, side: u32) -> Option<u32> {
    if !(2..=MAX_FRAMES).contains(&frames) || side == 0 {
        return None;
    }
    let fit = ((FRAME_BUDGET / (4 * frames)) as f64).sqrt().floor() as u32;
    let edge = DISPLAY_EDGE.min(fit).min(side);
    (edge >= MIN_ANIMATED_EDGE.min(side)).then_some(edge)
}

fn square(image: &image::RgbaImage, edge: u32) -> Rgba {
    let (x, y, side) = square_crop(image.width(), image.height());
    let view = image::imageops::crop_imm(image, x, y, side, side);
    let scaled = if side > edge {
        image::imageops::thumbnail(&*view, edge, edge)
    } else {
        view.to_image()
    };
    Rgba::from_image(scaled)
}

/// Decode a picture for display: every composited frame of an animated GIF
/// when `animate` and the budget allow, otherwise one still. Squares of at
/// most [`DISPLAY_EDGE`] pixels.
pub fn decode(bytes: &[u8], animate: bool) -> Option<Frames> {
    if animate
        && is_gif(bytes)
        && let Some(frames) = decode_animation(bytes)
    {
        return Some(frames);
    }
    decode_still(bytes).map(|image| vec![(image, Duration::ZERO)])
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8_192);
    limits.max_image_height = Some(8_192);
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits
}

fn decode_animation(bytes: &[u8]) -> Option<Frames> {
    use image::AnimationDecoder;
    let count = gif_frame_count(bytes)?;
    let (width, height) = gif_dimensions(bytes)?;
    let edge = animated_edge(count, width.min(height))?;
    let mut decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    decoder.set_limits(limits()).ok()?;
    let mut frames = Vec::new();
    let mut total = 0;
    for frame in decoder.into_frames() {
        let frame = frame.ok()?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = Duration::from_millis(u64::from(numerator / denominator.max(1)));
        // Browsers play tiny delays at 100 ms.
        let delay = if delay < Duration::from_millis(20) {
            Duration::from_millis(100)
        } else {
            delay
        };
        let image = square(&frame.into_buffer(), edge);
        total += image.pixels.len();
        // The header undercounted: a still, not a truncated loop.
        if total > FRAME_BUDGET || frames.len() >= MAX_FRAMES {
            return None;
        }
        frames.push((image, delay));
    }
    (frames.len() > 1).then_some(frames)
}

fn decode_still(bytes: &[u8]) -> Option<Rgba> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    reader.limits(limits());
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    image.apply_orientation(orientation);
    Some(square(&image.to_rgba8(), DISPLAY_EDGE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gif(frames: &[([u8; 4], u32)], width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            encoder
                .set_repeat(image::codecs::gif::Repeat::Infinite)
                .unwrap();
            for (color, delay) in frames {
                encoder
                    .encode_frame(image::Frame::from_parts(
                        image::RgbaImage::from_pixel(width, height, image::Rgba(*color)),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(*delay, 1),
                    ))
                    .unwrap();
            }
        }
        bytes
    }

    #[test]
    fn gif_headers_give_magic_dimensions_and_frame_counts() {
        let bytes = gif(&[([255, 0, 0, 255], 80), ([0, 0, 255, 255], 120)], 300, 200);
        assert!(is_gif(&bytes));
        assert_eq!(gif_dimensions(&bytes), Some((300, 200)));
        assert_eq!(gif_frame_count(&bytes), Some(2));
        // Little-endian u16s at bytes 6–9.
        let mut header = b"GIF89a".to_vec();
        header.extend([0x00, 0x04, 0x01, 0x04]);
        assert_eq!(gif_dimensions(&header), Some((1024, 1025)));
        assert_eq!(gif_dimensions(b"GIF87a\x01"), None, "cut short");
        assert!(!is_gif(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(gif_dimensions(b"PNG.....xxxx"), None);
        // A file cut inside its second image still counts the first.
        assert_eq!(gif_frame_count(&bytes[..bytes.len() - 20]), Some(1));
        assert_eq!(gif_frame_count(b"GIF89a"), None);
    }

    #[test]
    fn square_crops_are_centred_on_the_short_edge() {
        assert_eq!(square_crop(1200, 800), (200, 0, 800));
        assert_eq!(square_crop(800, 1200), (0, 200, 800));
        assert_eq!(square_crop(301, 300), (0, 0, 300));
        assert_eq!(square_crop(64, 64), (0, 0, 64));
        assert_eq!(square_crop(0, 10), (0, 5, 0));
    }

    #[test]
    fn edited_picture_upload_keeps_the_selected_square_not_the_center() {
        use crate::attachment_edit::{Crop, Edit};
        let image = image::RgbaImage::from_fn(120, 60, |x, _| {
            if x < 60 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        });
        let mut bytes = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        for (x, channel) in [(0.0, 0), (0.5, 2)] {
            let crop = square_frame(
                Crop {
                    x,
                    y: 0.0,
                    width: 0.5,
                    height: 1.0,
                },
                120,
                60,
            );
            let edit = Edit {
                crop,
                ..Edit::default()
            };
            let prepared = prepare_edited(bytes.get_ref().clone(), Some(&edit)).unwrap();
            let saved = image::load_from_memory(&prepared.bytes).unwrap().to_rgb8();
            assert_eq!(saved.dimensions(), (60, 60));
            for pixel in saved.pixels() {
                assert!(
                    pixel[channel] > 230,
                    "selected region survives upload encoding"
                );
                assert!(
                    pixel[2 - channel] < 25,
                    "excluded region must not leak into the result"
                );
            }
        }
        let animation = gif(&[([255, 0, 0, 255], 80), ([0, 0, 255, 255], 120)], 60, 60);
        assert_eq!(
            prepare_edited(animation.clone(), None).unwrap().bytes,
            animation
        );
        assert_ne!(
            prepare_edited(animation, Some(&Edit::default()))
                .unwrap()
                .content_type,
            "image/gif"
        );
    }

    #[test]
    fn gifs_upload_unchanged_within_the_limits() {
        let bytes = gif(&[([255, 0, 0, 255], 80), ([0, 0, 255, 255], 80)], 48, 32);
        let prepared = prepare(bytes.clone()).unwrap();
        assert_eq!(
            prepared.bytes, bytes,
            "the original bytes keep the animation"
        );
        assert_eq!(prepared.content_type, "image/gif");
        assert_eq!((prepared.width, prepared.height), (48, 32));
        assert_eq!(prepared.filename(&Target::Avatar), "avatar.gif");
        // Over 5 MiB: shrunk (to its 32 px short edge), never refused.
        let mut large = bytes;
        large.resize(MAX_GIF_BYTES + 1, 0);
        let shrunk = prepare(large).unwrap();
        assert_eq!(
            (shrunk.content_type, shrunk.width, shrunk.height),
            ("image/gif", 32, 32)
        );
        assert_eq!(gif_frame_count(&shrunk.bytes), Some(2));
    }

    #[test]
    fn oversized_gifs_shrink_to_a_512_px_square_keeping_frames_and_timing() {
        use image::AnimationDecoder;
        // Over 1024 px wide: a 512 px square, centre-cropped, every frame and delay.
        let wide = gif(
            &[
                ([255, 0, 0, 255], 80),
                ([0, 0, 255, 255], 120),
                ([0, 255, 0, 255], 40),
            ],
            1100,
            700,
        );
        let prepared = prepare(wide).unwrap();
        assert_eq!((prepared.width, prepared.height), (512, 512));
        assert_eq!(gif_dimensions(&prepared.bytes), Some((512, 512)));
        let decoder =
            image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&prepared.bytes)).unwrap();
        let frames: Vec<_> = decoder.into_frames().map(Result::unwrap).collect();
        let delays: Vec<u32> = frames
            .iter()
            .map(|frame| frame.delay().numer_denom_ms().0)
            .collect();
        assert_eq!(delays, [80, 120, 40]);
        assert_eq!(frames[1].buffer().get_pixel(256, 256).0, [0, 0, 255, 255]);
        // A tight budget: smaller, then fewer frames lasting as long.
        let source = gif(
            &[
                ([255, 0, 0, 255], 50),
                ([0, 0, 255, 255], 50),
                ([0, 255, 0, 255], 50),
                ([9, 9, 9, 255], 50),
            ],
            600,
            600,
        );
        assert_eq!(shrink_gif(&source, MAX_GIF_BYTES).unwrap().width, 512);
        let tight = shrink_gif(&source, 700).unwrap();
        assert!(tight.bytes.len() <= 700);
        assert_eq!(gif_frame_count(&tight.bytes), Some(2));
        assert_eq!(tight.width, 128);
        let decoder =
            image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&tight.bytes)).unwrap();
        let total: u32 = decoder
            .into_frames()
            .map(|frame| frame.unwrap().delay().numer_denom_ms().0)
            .sum();
        assert_eq!(total, 200, "the animation keeps its length");
    }

    #[test]
    fn stills_become_upright_square_webp_of_at_most_512_px() {
        let mut source = Vec::new();
        // A wide JPEG whose centre is red and whose sides are blue.
        DynamicImage::ImageRgb8(image::RgbImage::from_fn(1500, 900, |x, _| {
            if (300..1200).contains(&x) {
                image::Rgb([200, 20, 20])
            } else {
                image::Rgb([20, 20, 200])
            }
        }))
        .write_to(
            &mut std::io::Cursor::new(&mut source),
            image::ImageFormat::Jpeg,
        )
        .unwrap();
        let prepared = prepare(source).unwrap();
        assert_eq!(prepared.content_type, "image/webp");
        assert_eq!((prepared.width, prepared.height), (512, 512));
        assert!(prepared.bytes.len() <= MAX_STILL_BYTES);
        assert_eq!(
            prepared.filename(&Target::SpaceIcon("s".into())),
            "space-icon.webp"
        );
        let decoded = image::load_from_memory(&prepared.bytes).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (512, 512));
        // The crop kept the centre: no blue sides.
        for x in [0, 256, 511] {
            let pixel = decoded.get_pixel(x, 256);
            assert!(pixel[0] > 150 && pixel[2] < 80, "{pixel:?} at {x}");
        }
        // Small pictures are never enlarged.
        let mut small = Vec::new();
        DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            40,
            90,
            image::Rgba([1, 2, 3, 128]),
        ))
        .write_to(
            &mut std::io::Cursor::new(&mut small),
            image::ImageFormat::Png,
        )
        .unwrap();
        let prepared = prepare(small).unwrap();
        assert_eq!((prepared.width, prepared.height), (40, 40));
        assert_eq!(prepared.content_type, "image/webp");
    }

    #[test]
    fn unreadable_and_heif_pictures_explain_themselves() {
        assert_eq!(prepare(b"not a picture".to_vec()), Err(UNREADABLE.into()));
        let mut heic = vec![0, 0, 0, 24];
        heic.extend(b"ftypheic\0\0\0\0mif1heic");
        assert_eq!(prepare(heic), Err(UNSUPPORTED.into()));
        let mut avif = vec![0, 0, 0, 24];
        avif.extend(b"ftypavif\0\0\0\0mif1avif");
        assert_eq!(prepare(avif), Err(UNSUPPORTED.into()));
    }

    #[test]
    fn reservations_carry_the_purpose_and_space() {
        let prepared = Prepared {
            bytes: vec![0; 48],
            content_type: "image/webp",
            width: 512,
            height: 512,
        };
        assert_eq!(
            reservation(&Target::Avatar, &prepared),
            json!({"purpose":"avatar","filename":"avatar.webp","contentType":"image/webp","byteSize":48,"width":512,"height":512})
        );
        assert_eq!(
            reservation(&Target::SpaceIcon("space0000001".into()), &prepared),
            json!({"purpose":"spaceIcon","spaceId":"space0000001","filename":"space-icon.webp","contentType":"image/webp","byteSize":48,"width":512,"height":512})
        );
    }

    #[test]
    fn animations_fit_the_frame_budget_or_fall_back_to_a_still() {
        assert_eq!(animated_edge(10, 480), Some(128));
        assert_eq!(animated_edge(10, 64), Some(64), "never enlarged");
        assert_eq!(animated_edge(128, 1024), Some(128));
        let edge = animated_edge(300, 1024).unwrap();
        assert!((MIN_ANIMATED_EDGE..128).contains(&edge));
        assert!(300 * (edge as usize).pow(2) * 4 <= FRAME_BUDGET);
        assert_eq!(animated_edge(MAX_FRAMES + 1, 1024), None);
        assert_eq!(animated_edge(1, 1024), None, "one frame is a still");
    }

    #[test]
    fn api_errors_map_to_sentences() {
        let space = Target::SpaceIcon("s".into());
        let message = |status, code| failure(status, code, "server text", &space).message;
        assert_eq!(
            message(Some(400), Some("picture_type")),
            "Choose a PNG, JPEG, WebP or GIF picture."
        );
        assert_eq!(
            message(Some(413), Some("picture_too_large")),
            "That picture is too large. Try another one."
        );
        assert_eq!(message(Some(422), Some("picture_invalid")), INVALID);
        assert_eq!(
            message(Some(429), Some("picture_rate_limited")),
            "You’ve changed pictures too often today. Try again later."
        );
        assert_eq!(
            message(Some(404), None),
            "Only the space owner can change its icon."
        );
        assert_eq!(message(Some(500), None), "server text");
        let unavailable = failure(Some(503), None, "x", &Target::Avatar);
        assert!(unavailable.unavailable);
        assert_eq!(unavailable.message, UNAVAILABLE);
        assert!(!failure(Some(422), None, "x", &Target::Avatar).unavailable);
    }
}
