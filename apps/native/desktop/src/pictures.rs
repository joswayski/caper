//! Custom profile pictures and space icons (`avatarAsset`/`avatarUrl` on
//! people, `iconAsset`/`iconUrl` on spaces; the shared pictures contract in
//! docs/media.md): preparing a picked image for upload, and a bounded
//! cache that shows the pictures wherever a mascot or space initial is drawn.
//!
//! Pictures are cached by asset id, never by URL: signed URLs re-sign daily.
//! Fetching and decoding run off the UI thread at display size (at most
//! [`DISPLAY_EDGE`] pixels, cropped square), and an animated GIF keeps its
//! composited frames and advances them on one texture, as inline GIF
//! attachments do (`attachments`). Animations over the frame or memory
//! budget show one still instead of a truncated loop, reduced motion shows
//! the first frame, and animations that are no longer drawn release their
//! frames. Until a picture is ready, or when it fails, callers draw the
//! saved mascot or the space initial.

use crate::api::Api;
use crate::uploads::UploadError;
use eframe::egui;
use image::{DynamicImage, ImageDecoder};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

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
pub const INVALID: &str = "That picture couldn't be used. Try another one.";
pub const UNAVAILABLE: &str = "Picture uploads aren’t available right now.";

/// Decoded pictures fit this square: crisp at 64 points on a 2x display.
pub const DISPLAY_EDGE: u32 = 128;
/// Below this edge an animation would be blurry: show a still instead.
const MIN_ANIMATED_EDGE: u32 = 48;
const MAX_FRAMES: usize = 600;
/// Retained frame pixels for one animation (128 frames at 128 px).
const FRAME_BUDGET: usize = 8 * 1024 * 1024;
/// Retained frame pixels across every animation.
const TOTAL_BUDGET: usize = 64 * 1024 * 1024;
/// Animations not drawn for this long keep only their first frame.
const IDLE: Duration = Duration::from_secs(20);
/// An animation released to stay within [`TOTAL_BUDGET`] while it was still
/// drawn waits this long before loading again.
const RELOAD_AFTER: Duration = Duration::from_secs(60);
/// Decoded pictures kept; each texture is at most 64 KiB.
const MAX_ENTRIES: usize = 400;
const MAX_FETCHES: usize = 4;
/// A load that failed on the network or a server error (a laptop waking,
/// say) tries the same URL again after this, doubling up to [`RETRY_MAX`].
/// Undecodable pictures and other 4xx answers wait for a re-signed URL.
const RETRY_FIRST: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(5 * 60);

pub type Frames = Vec<(egui::ColorImage, Duration)>;

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

// ---------------------------------------------------------------------------
// Preparing a picked picture for upload.

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

#[cfg(test)]
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

fn square(image: &image::RgbaImage, edge: u32) -> egui::ColorImage {
    let (x, y, side) = square_crop(image.width(), image.height());
    let view = image::imageops::crop_imm(image, x, y, side, side);
    let scaled = if side > edge {
        image::imageops::thumbnail(&*view, edge, edge)
    } else {
        view.to_image()
    };
    egui::ColorImage::from_rgba_unmultiplied(
        [scaled.width() as usize, scaled.height() as usize],
        scaled.as_raw(),
    )
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
        total += image.pixels.len() * 4;
        // The header undercounted: a still, not a truncated loop.
        if total > FRAME_BUDGET || frames.len() >= MAX_FRAMES {
            return None;
        }
        frames.push((image, delay));
    }
    (frames.len() > 1).then_some(frames)
}

fn decode_still(bytes: &[u8]) -> Option<egui::ColorImage> {
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

// ---------------------------------------------------------------------------
// The display cache.

struct Animation {
    frames: Frames,
    started: Instant,
    shown: usize,
    bytes: usize,
}

impl Animation {
    fn new(frames: Frames, started: Instant) -> Self {
        let bytes = frames.iter().map(|(image, _)| image.pixels.len() * 4).sum();
        Self {
            frames,
            started,
            shown: 0,
            bytes,
        }
    }

    /// The frame `elapsed` into the loop, and how long until the next one.
    fn frame_at(&self, elapsed: Duration) -> (usize, Duration) {
        let total: u128 = self.frames.iter().map(|(_, delay)| delay.as_nanos()).sum();
        if total == 0 {
            return (0, Duration::from_millis(100));
        }
        let mut time = elapsed.as_nanos() % total;
        for (index, (_, delay)) in self.frames.iter().enumerate() {
            let length = delay.as_nanos();
            if time < length {
                return (index, Duration::from_nanos((length - time) as u64));
            }
            time -= length;
        }
        (0, Duration::from_millis(100))
    }
}

#[derive(Default)]
struct Entry {
    /// The picture, or an animation's current frame.
    texture: Option<egui::TextureHandle>,
    animation: Option<Animation>,
    /// When frames were released, and whether that was for the total budget
    /// (which delays loading them again) rather than disuse.
    released: Option<(Instant, bool)>,
    /// The URL being fetched.
    loading: Option<String>,
    /// The URL that failed; another (re-signed) URL tries again.
    failed: Option<String>,
    /// After a transient failure: when the same URL may load again, and the
    /// wait that set it.
    retry: Option<(Instant, Duration)>,
    used: Option<Instant>,
}

struct Request {
    asset: String,
    url: String,
    animate: bool,
}

struct Fetched {
    generation: u64,
    asset: String,
    url: String,
    frames: Option<Frames>,
    /// The download failed in a way worth trying again.
    transient: bool,
}

/// Network failures (no status) and server errors are worth retrying.
fn transient(status: Option<u16>) -> bool {
    status.is_none_or(|status| status >= 500 || matches!(status, 408 | 429))
}

/// Decoded pictures by asset id. Shared through the egui context so every
/// avatar painter reaches it; cleared on logout.
pub struct Pictures {
    entries: HashMap<String, Entry>,
    waiting: VecDeque<Request>,
    in_flight: usize,
    animated_bytes: usize,
    sender: Sender<Fetched>,
    receiver: Receiver<Fetched>,
    /// `None` in tests and fixtures, which never fetch.
    api: Option<Api>,
    generation: u64,
}

impl Default for Pictures {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            entries: HashMap::new(),
            waiting: VecDeque::new(),
            in_flight: 0,
            animated_bytes: 0,
            sender,
            receiver,
            api: None,
            generation: 0,
        }
    }
}

const TEXTURE: egui::TextureOptions = egui::TextureOptions::LINEAR;

impl Pictures {
    fn clear(&mut self) {
        self.generation += 1;
        self.entries.clear();
        self.waiting.clear();
        self.in_flight = 0;
        self.animated_bytes = 0;
    }

    /// The texture to draw now, if any, and when the next animation frame is
    /// due. Starts a load when the picture is missing, or when a released
    /// animation is drawn again and the budget has room.
    fn texture(
        &mut self,
        source: Source<'_>,
        playing: bool,
        now: Instant,
    ) -> (Option<egui::TextureId>, Option<Duration>) {
        if !self.entries.contains_key(source.asset) {
            self.entries
                .insert(source.asset.to_owned(), Entry::default());
        }
        let room = self.animated_bytes + FRAME_BUDGET <= TOTAL_BUDGET;
        let entry = self.entries.get_mut(source.asset).expect("inserted above");
        entry.used = Some(now);
        let mut request = None;
        if entry.loading.is_none()
            && let Some(url) = source.url
        {
            let due = entry.retry.is_some_and(|(at, _)| now >= at);
            let missing = entry.texture.is_none() && (entry.failed.as_deref() != Some(url) || due);
            let reload = playing
                && room
                && entry.texture.is_some()
                && entry.animation.is_none()
                && entry.released.is_some_and(|(at, budget)| {
                    !budget || now.saturating_duration_since(at) >= RELOAD_AFTER
                });
            if missing || reload {
                entry.loading = Some(url.to_owned());
                request = Some(Request {
                    asset: source.asset.to_owned(),
                    url: url.to_owned(),
                    animate: playing,
                });
            }
        }
        let mut wait = None;
        let texture = entry.texture.as_mut().map(|texture| {
            if let Some(animation) = &mut entry.animation {
                let (index, next) = if playing {
                    animation.frame_at(now.saturating_duration_since(animation.started))
                } else {
                    (0, Duration::MAX)
                };
                if animation.shown != index {
                    texture.set(animation.frames[index].0.clone(), TEXTURE);
                    animation.shown = index;
                }
                if playing {
                    wait = Some(next);
                }
            }
            texture.id()
        });
        // A transient failure repaints when it may load again.
        if texture.is_none()
            && request.is_none()
            && entry.loading.is_none()
            && entry
                .failed
                .as_deref()
                .is_some_and(|failed| source.url == Some(failed))
            && let Some((at, _)) = entry.retry
        {
            wait = Some(at.saturating_duration_since(now));
        }
        if let Some(request) = request {
            self.waiting.push_back(request);
        }
        (texture, wait)
    }

    /// Show `frames` for `asset` now: an upload's own picture, or a fixture.
    fn store(&mut self, context: &egui::Context, asset: &str, frames: Frames, now: Instant) {
        let Some(first) = frames.first().map(|(image, _)| image.clone()) else {
            return;
        };
        let entry = self.entries.entry(asset.to_owned()).or_default();
        if let Some(old) = entry.animation.take() {
            self.animated_bytes -= old.bytes;
        }
        match &mut entry.texture {
            Some(texture) => texture.set(first, TEXTURE),
            None => {
                entry.texture =
                    Some(context.load_texture(format!("picture-{asset}"), first, TEXTURE));
            }
        }
        if frames.len() > 1 {
            let animation = Animation::new(frames, now);
            self.animated_bytes += animation.bytes;
            entry.animation = Some(animation);
        }
        entry.released = None;
        entry.failed = None;
        entry.retry = None;
        entry.loading = None;
        entry.used.get_or_insert(now);
    }

    /// Keep only an animation's first frame.
    fn release(&mut self, asset: &str, now: Instant, budget: bool) {
        let Some(entry) = self.entries.get_mut(asset) else {
            return;
        };
        let Some(animation) = entry.animation.take() else {
            return;
        };
        self.animated_bytes -= animation.bytes;
        if animation.shown != 0
            && let Some(texture) = &mut entry.texture
        {
            texture.set(animation.frames[0].0.clone(), TEXTURE);
        }
        entry.released = Some((now, budget));
    }

    /// Apply finished loads, start queued ones, and bound memory.
    fn poll(&mut self, context: &egui::Context, now: Instant) {
        let results: Vec<_> = self.receiver.try_iter().collect();
        for fetched in results {
            if fetched.generation != self.generation {
                continue;
            }
            self.in_flight = self.in_flight.saturating_sub(1);
            let Some(entry) = self.entries.get_mut(&fetched.asset) else {
                continue;
            };
            if entry.loading.as_deref() != Some(fetched.url.as_str()) {
                continue;
            }
            entry.loading = None;
            match fetched.frames {
                Some(frames) if !frames.is_empty() => {
                    self.store(context, &fetched.asset, frames, now);
                }
                // A failed reload keeps its still and stops trying.
                _ if entry.texture.is_some() => entry.released = None,
                _ => {
                    entry.retry = fetched.transient.then(|| {
                        let wait = entry
                            .retry
                            .filter(|_| entry.failed.as_ref() == Some(&fetched.url))
                            .map_or(RETRY_FIRST, |(_, wait)| (wait * 2).min(RETRY_MAX));
                        (now + wait, wait)
                    });
                    entry.failed = Some(fetched.url);
                }
            }
        }
        while self.in_flight < MAX_FETCHES
            && let Some(request) = self.waiting.pop_front()
        {
            self.in_flight += 1;
            // Tests and fixtures: the entry stays loading.
            let Some(api) = self.api.clone() else {
                continue;
            };
            let sender = self.sender.clone();
            let context = context.clone();
            let generation = self.generation;
            std::thread::spawn(move || {
                let (frames, transient) = match api.fetch_media(&request.url) {
                    Ok(bytes) => (decode(&bytes, request.animate), false),
                    Err(status) => (None, transient(status)),
                };
                let _ = sender.send(Fetched {
                    generation,
                    asset: request.asset,
                    url: request.url,
                    frames,
                    transient,
                });
                context.request_repaint();
            });
        }
        // Animations no longer drawn keep their first frame.
        let idle: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.animation.is_some()
                    && entry
                        .used
                        .is_none_or(|used| now.saturating_duration_since(used) >= IDLE)
            })
            .map(|(asset, _)| asset.clone())
            .collect();
        for asset in idle {
            self.release(&asset, now, false);
        }
        // Then the least recently drawn, until all fit.
        while self.animated_bytes > TOTAL_BUDGET {
            let Some(asset) = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.animation.is_some())
                .min_by_key(|(_, entry)| entry.used)
                .map(|(asset, _)| asset.clone())
            else {
                break;
            };
            self.release(&asset, now, true);
        }
        if self.entries.len() > MAX_ENTRIES {
            let mut used: Vec<(Option<Instant>, String)> = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.loading.is_none())
                .map(|(asset, entry)| (entry.used, asset.clone()))
                .collect();
            used.sort();
            for (_, asset) in used.into_iter().take(self.entries.len() - MAX_ENTRIES) {
                if let Some(entry) = self.entries.remove(&asset)
                    && let Some(animation) = entry.animation
                {
                    self.animated_bytes -= animation.bytes;
                }
            }
        }
    }
}

/// The context's picture cache.
#[derive(Clone, Default)]
pub struct Shared(Arc<Mutex<Pictures>>);

impl Shared {
    pub fn of(context: &egui::Context) -> Self {
        let id = egui::Id::new("caper-pictures");
        context.data_mut(|data| data.get_temp_mut_or_default::<Self>(id).clone())
    }

    fn lock(&self) -> MutexGuard<'_, Pictures> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Where pictures are fetched from; `None` (fixtures) never fetches.
    pub fn install(&self, api: Option<Api>) {
        self.lock().api = api;
    }

    /// Forget every picture (logout); loads in flight are ignored.
    pub fn clear(&self) {
        self.lock().clear();
    }

    pub fn poll(&self, context: &egui::Context) {
        self.lock().poll(context, Instant::now());
    }

    /// Show these frames for `asset` without a download.
    pub fn insert(&self, context: &egui::Context, asset: &str, frames: Frames) {
        self.lock().store(context, asset, frames, Instant::now());
    }

    /// The decoded texture for `asset`, if one is cached.
    #[cfg(test)]
    pub fn texture_id(&self, asset: &str) -> Option<egui::TextureId> {
        self.lock()
            .entries
            .get(asset)?
            .texture
            .as_ref()
            .map(egui::TextureHandle::id)
    }
}

/// Animations play unless the system asks for less motion or the window is
/// in the background (then the first frame shows and nothing repaints), as
/// inline GIFs do.
pub fn playing(ui: &egui::Ui) -> bool {
    crate::looping(ui) && ui.style().animation_time > 0.0
}

/// The texture to draw for `source` in `ui`, loading it if needed; `None`
/// means draw the mascot or initial instead.
pub fn texture(ui: &egui::Ui, source: Source<'_>) -> Option<egui::TextureId> {
    let shared = Shared::of(ui.ctx());
    let mut pictures = shared.lock();
    let queued = pictures.waiting.len();
    let (texture, wait) = pictures.texture(source, playing(ui), Instant::now());
    // A new load starts at the next frame's poll; fetches repaint when done.
    let queued = pictures.waiting.len() > queued;
    drop(pictures);
    if let Some(wait) = wait {
        ui.ctx().request_repaint_after(wait);
    }
    if queued {
        ui.ctx().request_repaint();
    }
    texture
}

/// Draw `source` covering `rect` with `corner` rounding. Returns `false`
/// (drawing nothing) while it loads or after it failed.
pub fn paint(
    ui: &egui::Ui,
    rect: egui::Rect,
    corner: impl Into<egui::CornerRadius>,
    source: Source<'_>,
) -> bool {
    match texture(ui, source) {
        Some(texture) => {
            egui::Image::new((texture, rect.size()))
                .corner_radius(corner)
                .paint_at(ui, rect);
            true
        }
        None => false,
    }
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
    fn display_decoding_crops_square_and_honors_animate() {
        let bytes = gif(&[([255, 0, 0, 255], 80), ([0, 0, 255, 255], 800)], 400, 200);
        let frames = decode(&bytes, true).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0.size, [128, 128]);
        assert_eq!(frames[0].1, Duration::from_millis(80));
        assert_eq!(frames[1].0.pixels[0], egui::Color32::BLUE);
        // Reduced motion decodes only the first frame.
        let still = decode(&bytes, false).unwrap();
        assert_eq!(still.len(), 1);
        assert_eq!(still[0].0.pixels[0], egui::Color32::RED);
        assert!(decode(b"nope", true).is_none());
    }

    fn texture_color(context: &egui::Context, texture: egui::TextureId) -> egui::Color32 {
        let delta = context.tex_manager().write().take_delta();
        let update = &delta
            .set
            .iter()
            .rev()
            .find(|(id, _)| *id == texture)
            .unwrap()
            .1;
        let egui::ImageData::Color(image) = &update.image;
        image.pixels[0]
    }

    #[test]
    fn cached_by_asset_animated_unless_reduced_motion_and_failures_retry_new_urls() {
        let context = egui::Context::default();
        let mut pictures = Pictures::default();
        let start = Instant::now();
        let url = |day: u32| format!("https://cdn.caper.chat/assets/a1?exp={day}&sig=x");
        let first = url(1);
        let source = Source {
            asset: "a1",
            url: Some(&first),
        };
        // Nothing yet: the caller draws the mascot, and a load is queued.
        assert_eq!(pictures.texture(source, true, start), (None, None));
        assert_eq!(pictures.waiting.len(), 1);
        pictures.poll(&context, start);
        assert_eq!(pictures.texture(source, true, start), (None, None));
        assert!(pictures.waiting.is_empty(), "one load per asset");
        let bytes = gif(&[([255, 0, 0, 255], 80), ([0, 0, 255, 255], 800)], 4, 4);
        pictures
            .sender
            .send(Fetched {
                generation: pictures.generation,
                asset: "a1".into(),
                url: first.clone(),
                frames: decode(&bytes, true),
                transient: false,
            })
            .unwrap();
        pictures.poll(&context, start);
        // A re-signed URL reuses the cached picture.
        let second = url(2);
        let resigned = Source {
            asset: "a1",
            url: Some(&second),
        };
        let (texture, wait) = pictures.texture(resigned, true, start);
        let texture = texture.unwrap();
        assert_eq!(wait, Some(Duration::from_millis(80)));
        assert!(pictures.waiting.is_empty());
        assert_eq!(texture_color(&context, texture), egui::Color32::RED);
        let (_, wait) = pictures.texture(resigned, true, start + Duration::from_millis(100));
        assert_eq!(texture_color(&context, texture), egui::Color32::BLUE);
        assert_eq!(wait, Some(Duration::from_millis(780)));
        // Reduced motion: the first frame, and no repaint schedule.
        let (_, wait) = pictures.texture(resigned, false, start + Duration::from_millis(100));
        assert_eq!(texture_color(&context, texture), egui::Color32::RED);
        assert_eq!(wait, None);
        // A failed load keeps the fallback until a different URL arrives.
        let other = Source {
            asset: "a2",
            url: Some(&first),
        };
        pictures.texture(other, true, start);
        pictures.poll(&context, start);
        pictures
            .sender
            .send(Fetched {
                generation: pictures.generation,
                asset: "a2".into(),
                url: first.clone(),
                frames: None,
                transient: false,
            })
            .unwrap();
        pictures.poll(&context, start);
        assert_eq!(pictures.texture(other, true, start), (None, None));
        assert!(pictures.waiting.is_empty(), "the same URL never retries");
        let fresh = Source {
            asset: "a2",
            url: Some(&second),
        };
        assert_eq!(pictures.texture(fresh, true, start), (None, None));
        assert_eq!(pictures.waiting.len(), 1, "a fresh URL retries");
        // A network failure retries the same URL, backing off to 5 minutes.
        pictures.poll(&context, start);
        let mut at = start;
        for wait in [30, 60, 120, 240, 300, 300] {
            pictures
                .sender
                .send(Fetched {
                    generation: pictures.generation,
                    asset: "a2".into(),
                    url: second.clone(),
                    frames: None,
                    transient: true,
                })
                .unwrap();
            pictures.poll(&context, at);
            let wait = Duration::from_secs(wait);
            assert_eq!(
                pictures.texture(fresh, true, at),
                (None, Some(wait)),
                "the fallback shows and a repaint is due when it may retry"
            );
            assert!(pictures.waiting.is_empty());
            at += wait;
            assert_eq!(pictures.texture(fresh, true, at), (None, None));
            assert_eq!(pictures.waiting.len(), 1, "retried after {wait:?}");
            pictures.poll(&context, at);
        }
        // Logout forgets everything; late results are ignored.
        pictures.clear();
        pictures
            .sender
            .send(Fetched {
                generation: pictures.generation - 1,
                asset: "a1".into(),
                url: second.clone(),
                frames: decode(&bytes, true),
                transient: false,
            })
            .unwrap();
        pictures.poll(&context, start);
        assert_eq!(pictures.entries.len(), 0);
    }

    #[test]
    fn idle_and_over_budget_animations_keep_their_first_frame() {
        let context = egui::Context::default();
        let mut pictures = Pictures::default();
        let start = Instant::now();
        // Eight 128 px frames: 512 KiB each animation.
        let frames = || -> Frames {
            [egui::Color32::RED, egui::Color32::BLUE]
                .into_iter()
                .cycle()
                .take(8)
                .map(|color| {
                    (
                        egui::ColorImage::filled([128, 128], color),
                        Duration::from_millis(100),
                    )
                })
                .collect()
        };
        pictures.store(&context, "idle", frames(), start);
        let url = "https://cdn.caper.chat/assets/idle?exp=1&sig=x";
        let idle = Source {
            asset: "idle",
            url: Some(url),
        };
        let (texture, _) = pictures.texture(idle, true, start + Duration::from_millis(150));
        let texture = texture.unwrap();
        assert_eq!(texture_color(&context, texture), egui::Color32::BLUE);
        pictures.poll(&context, start + IDLE + Duration::from_secs(1));
        assert!(pictures.entries["idle"].animation.is_none());
        assert_eq!(pictures.animated_bytes, 0);
        assert_eq!(
            texture_color(&context, texture),
            egui::Color32::RED,
            "released animations rest on their first frame"
        );
        // Drawn again: the still shows while the frames load again.
        let later = start + IDLE + Duration::from_secs(2);
        assert_eq!(pictures.texture(idle, true, later).0, Some(texture));
        assert_eq!(pictures.waiting.len(), 1);
        // Too many animations at once: the least recently drawn give way.
        let count = TOTAL_BUDGET / (128 * 128 * 4 * 8) + 3;
        assert!(count < MAX_ENTRIES);
        for index in 0..count {
            pictures.store(&context, &format!("a{index}"), frames(), later);
            pictures.entries.get_mut(&format!("a{index}")).unwrap().used =
                Some(later + Duration::from_millis(index as u64));
        }
        pictures.poll(&context, later + Duration::from_millis(count as u64));
        assert!(pictures.animated_bytes <= TOTAL_BUDGET);
        assert!(pictures.entries["a0"].animation.is_none());
        assert!(
            pictures.entries[&format!("a{}", count - 1)]
                .animation
                .is_some()
        );
    }
}
