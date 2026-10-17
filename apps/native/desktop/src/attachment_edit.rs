//! Edits to an attachment before it is sent: crop, quarter turns, drawing and
//! text, as web's `apps/web/src/chat/edit.ts` (docs/media.md, "Editing
//! attachments before sending"). A draft keeps its original file and this
//! description, so reopening the editor can change any part of it; preparing
//! the upload applies it to the original (`compress::prepare_edited_image`,
//! `uploads::prepare_edited_path`). Geometry is shared with every client
//! through `shared/messages/edit-cases.json`.
//!
//! Marks are painted with tiny-skia (round caps and joins, anti-aliased) and
//! text with the app's own Satoshi Bold outlines, so the exported pixels follow
//! the same rules as the editor's preview.

use image::DynamicImage;
use resvg::tiny_skia;
use serde::Deserialize;
use std::io::Read;
use std::path::Path;

/// An edit that could not be applied fails the upload rather than sending the
/// unedited original: people crop to hide things.
pub const FAILED: &str = "Couldn’t apply your edits.";
/// Markup colours offered before the custom picker; the first is the default.
pub const COLORS: [[u8; 3]; 6] = [
    [0xE5, 0x48, 0x4D],
    [0xF5, 0xC5, 0x42],
    [0x4C, 0xAF, 0x50],
    [0x3B, 0x82, 0xF6],
    [0xFF, 0xFF, 0xFF],
    [0x00, 0x00, 0x00],
];
pub const COLOR_NAMES: [&str; 6] = ["Red", "Yellow", "Green", "Blue", "White", "Black"];
/// Stroke widths and text sizes in on-screen points (min, max, initial),
/// converted to fractions of the picture as they are drawn.
pub const STROKE_SIZES: (f32, f32, f32) = (2.0, 40.0, 6.0);
pub const TEXT_SIZES: (f32, f32, f32) = (14.0, 96.0, 32.0);
pub const MAX_TEXT: usize = 200;
/// The app's Satoshi Bold, for text marks here and in the editor.
pub const SATOSHI_BOLD: &[u8] = include_bytes!("../../../../shared/fonts/cache/Satoshi-Bold.otf");

const EPSILON: f64 = 1e-6;
/// How much of a file is read to tell whether it can be edited.
const HEAD_BYTES: u64 = 64 * 1024;

/// Fractions of the rotated, uncropped image's width and height.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct Crop {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Crop {
    pub const FULL: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    };
}

/// Painted in order over the image. Positions are fractions of the rotated,
/// uncropped image's width (x) and height (y); a stroke's `width` and a
/// text's `size` are fractions of its longer edge.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Mark {
    /// A freehand line through flat x, y pairs; one point is a dot.
    Stroke {
        #[serde(deserialize_with = "hex")]
        color: [u8; 3],
        width: f64,
        points: Vec<f64>,
    },
    /// One line of text centred on x, y; it stays upright when the image turns.
    Text {
        #[serde(deserialize_with = "hex")]
        color: [u8; 3],
        size: f64,
        x: f64,
        y: f64,
        text: String,
    },
}

impl Mark {
    pub fn color(&self) -> [u8; 3] {
        match self {
            Self::Stroke { color, .. } | Self::Text { color, .. } => *color,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Edit {
    /// Clockwise quarter turns of the upright original: 0, 90, 180 or 270.
    pub rotation: u16,
    pub crop: Crop,
    pub marks: Vec<Mark>,
}

impl Default for Edit {
    fn default() -> Self {
        Self {
            rotation: 0,
            crop: Crop::FULL,
            marks: Vec::new(),
        }
    }
}

/// A crop in pixels of the rotated image.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub struct PixelRect {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

impl Edit {
    pub fn is_edited(&self) -> bool {
        let Crop {
            x,
            y,
            width,
            height,
        } = self.crop;
        self.rotation != 0
            || !self.marks.is_empty()
            || x.abs() > EPSILON
            || y.abs() > EPSILON
            || (width - 1.0).abs() > EPSILON
            || (height - 1.0).abs() > EPSILON
    }

    /// Turns the picture a quarter anticlockwise, carrying the crop and marks with it.
    pub fn rotate_left(&self) -> Self {
        let Crop {
            x,
            y,
            width,
            height,
        } = self.crop;
        Self {
            rotation: (self.rotation + 270) % 360,
            crop: Crop {
                x: y,
                y: 1.0 - (x + width),
                width: height,
                height: width,
            },
            marks: self
                .marks
                .iter()
                .map(|mark| match mark {
                    Mark::Stroke {
                        color,
                        width,
                        points,
                    } => Mark::Stroke {
                        color: *color,
                        width: *width,
                        points: (0..points.len())
                            .map(|index| {
                                if index % 2 == 0 {
                                    points.get(index + 1).copied().unwrap_or(0.0)
                                } else {
                                    1.0 - points[index - 1]
                                }
                            })
                            .collect(),
                    },
                    Mark::Text {
                        color,
                        size,
                        x,
                        y,
                        text,
                    } => Mark::Text {
                        color: *color,
                        size: *size,
                        x: *y,
                        y: 1.0 - x,
                        text: text.clone(),
                    },
                })
                .collect(),
        }
    }

    /// The crop in pixels of the upright `width` × `height` original turned by
    /// the rotation. Video crops start and measure on even pixels (4:2:0
    /// chroma); every crop keeps at least one pixel (two for video).
    pub fn crop_pixels(&self, width: u32, height: u32, even: bool) -> PixelRect {
        let (width, height) = rotated_size(width, height, self.rotation);
        let (width, height) = (f64::from(width), f64::from(height));
        let crop = self.crop;
        // Same order as web: clamp to the image, then keep at least a pixel.
        let left = (crop.x * width).round().max(0.0).min(width - 1.0);
        let top = (crop.y * height).round().max(0.0).min(height - 1.0);
        let right = ((crop.x + crop.width) * width)
            .round()
            .min(width)
            .max(left + 1.0);
        let bottom = ((crop.y + crop.height) * height)
            .round()
            .min(height)
            .max(top + 1.0);
        let (left, top, right, bottom) = (left as u32, top as u32, right as u32, bottom as u32);
        if !even {
            return PixelRect {
                left,
                top,
                width: right - left,
                height: bottom - top,
            };
        }
        let left = left - left % 2;
        let top = top - top % 2;
        PixelRect {
            left,
            top,
            width: (right - left - (right - left) % 2).max(2),
            height: (bottom - top - (bottom - top) % 2).max(2),
        }
    }
}

pub fn rotated_size(width: u32, height: u32, rotation: u16) -> (u32, u32) {
    if rotation % 180 == 90 {
        (height, width)
    } else {
        (width, height)
    }
}

/// Keeps a crop inside the image and at least `min` (a fraction) on each side.
pub fn clamp_crop(crop: Crop, min: f64) -> Crop {
    let width = crop.width.max(min).min(1.0);
    let height = crop.height.max(min).min(1.0);
    Crop {
        x: crop.x.max(0.0).min(1.0 - width),
        y: crop.y.max(0.0).min(1.0 - height),
        width,
        height,
    }
}

/// Which part of the crop frame a drag holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Move,
    N,
    NE,
    E,
    SE,
    S,
    SW,
    W,
    NW,
}

impl Handle {
    pub const EDGES: [Self; 8] = [
        Self::NW,
        Self::N,
        Self::NE,
        Self::E,
        Self::SE,
        Self::S,
        Self::SW,
        Self::W,
    ];

    fn west(self) -> bool {
        matches!(self, Self::W | Self::NW | Self::SW)
    }

    fn east(self) -> bool {
        matches!(self, Self::E | Self::NE | Self::SE)
    }

    fn north(self) -> bool {
        matches!(self, Self::N | Self::NE | Self::NW)
    }

    fn south(self) -> bool {
        matches!(self, Self::S | Self::SE | Self::SW)
    }

    /// The handle's anchor on a crop, in the crop's own fractions (0–1).
    pub fn anchor(self) -> (f32, f32) {
        let x = if self.west() {
            0.0
        } else if self.east() {
            1.0
        } else {
            0.5
        };
        let y = if self.north() {
            0.0
        } else if self.south() {
            1.0
        } else {
            0.5
        };
        (x, y)
    }
}

/// A crop dragged by `dx`, `dy` (fractions of the image) from `start` by one
/// of its handles, kept inside the image and at least `min` on each side
/// (web's `dragCrop`).
pub fn drag_crop(start: Crop, handle: Handle, dx: f64, dy: f64, min: (f64, f64)) -> Crop {
    let clamp = |value: f64, low: f64, high: f64| value.max(low).min(low.max(high));
    if handle == Handle::Move {
        return Crop {
            x: clamp(start.x + dx, 0.0, 1.0 - start.width),
            y: clamp(start.y + dy, 0.0, 1.0 - start.height),
            ..start
        };
    }
    let (mut left, mut top) = (start.x, start.y);
    let (mut right, mut bottom) = (start.x + start.width, start.y + start.height);
    if handle.west() {
        left = clamp(left + dx, 0.0, right - min.0);
    }
    if handle.east() {
        right = clamp(right + dx, left + min.0, 1.0);
    }
    if handle.north() {
        top = clamp(top + dy, 0.0, bottom - min.1);
    }
    if handle.south() {
        bottom = clamp(bottom + dy, top + min.1, 1.0);
    }
    Crop {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

/// What a draft's editor opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
}

/// Whether a picked file can be edited before sending: stills the `image`
/// codecs decode, without animation (GIF and animated WebP/PNG would lose
/// every frame but the first), and videos when FFmpeg can re-encode them.
/// HEIC and AVIF do not decode here (`compress`), so they upload unchanged.
pub fn editable(path: &Path) -> Option<Kind> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let mut head = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(HEAD_BYTES)
        .read_to_end(&mut head)
        .ok()?;
    editable_kind(&name, &head, crate::ffmpeg::binary().is_some())
}

pub fn editable_kind(name: &str, head: &[u8], videos: bool) -> Option<Kind> {
    let content_type = crate::compress::content_type_for(name, head);
    match content_type.as_str() {
        "image/jpeg" | "image/bmp" | "image/tiff" => Some(Kind::Image),
        "image/png" if !crate::compress::png_is_animated(head) => Some(Kind::Image),
        "image/webp" if still_webp(head) => Some(Kind::Image),
        video if video.starts_with("video/") && videos => Some(Kind::Video),
        _ => None,
    }
}

/// A simple (VP8/VP8L) WebP, or an extended one whose header says it is not
/// animated. Only the first chunk is needed, unlike `compress::classify`.
fn still_webp(head: &[u8]) -> bool {
    match head.get(12..16) {
        Some(b"VP8 " | b"VP8L") => true,
        Some(b"VP8X") => head.get(20).is_some_and(|flags| flags & 0x02 == 0),
        _ => false,
    }
}

/// Renders an edit onto the upright original at its native resolution: the
/// crop region, turned, with the marks painted over it. Images without alpha
/// stay RGB; an unmarked crop keeps the source's bit depth.
pub fn render(image: &DynamicImage, edit: &Edit) -> Option<DynamicImage> {
    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return None;
    }
    let region = edit.crop_pixels(width, height, false);
    // Crop the original first, where the region lies before turning.
    let (x, y, w, h) = match edit.rotation {
        90 => (
            region.top,
            height - (region.left + region.width),
            region.height,
            region.width,
        ),
        180 => (
            width - (region.left + region.width),
            height - (region.top + region.height),
            region.width,
            region.height,
        ),
        270 => (
            width - (region.top + region.height),
            region.left,
            region.height,
            region.width,
        ),
        _ => (region.left, region.top, region.width, region.height),
    };
    let cropped = image.crop_imm(x, y, w, h);
    let turned = match edit.rotation {
        90 => cropped.rotate90(),
        180 => cropped.rotate180(),
        270 => cropped.rotate270(),
        _ => cropped,
    };
    if edit.marks.is_empty() {
        return Some(turned);
    }
    let alpha = image.color().has_alpha();
    let mut pixels = turned.to_rgba8();
    let (rotated_width, rotated_height) = rotated_size(width, height, edit.rotation);
    let mut marks = tiny_skia::Pixmap::new(region.width, region.height)?;
    paint_marks(
        &mut marks,
        &edit.marks,
        f64::from(rotated_width),
        f64::from(rotated_height),
        tiny_skia::Transform::from_translate(-(region.left as f32), -(region.top as f32)),
    );
    composite(&mut pixels, &marks);
    Some(if alpha {
        DynamicImage::ImageRgba8(pixels)
    } else {
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(pixels).to_rgb8())
    })
}

/// Only the marks, transparent elsewhere, at a video's output size: the
/// `region` of the `width` × `height` rotated picture scaled to `output`.
pub fn overlay(
    edit: &Edit,
    width: u32,
    height: u32,
    region: PixelRect,
    output: (u32, u32),
) -> Option<tiny_skia::Pixmap> {
    let mut pixmap = tiny_skia::Pixmap::new(output.0, output.1)?;
    let transform = tiny_skia::Transform::from_scale(
        output.0 as f32 / region.width.max(1) as f32,
        output.1 as f32 / region.height.max(1) as f32,
    )
    .pre_translate(-(region.left as f32), -(region.top as f32));
    paint_marks(
        &mut pixmap,
        &edit.marks,
        f64::from(width),
        f64::from(height),
        transform,
    );
    Some(pixmap)
}

/// Paints marks given in the rotated picture's pixel space (`width` ×
/// `height`) through `transform`.
fn paint_marks(
    pixmap: &mut tiny_skia::Pixmap,
    marks: &[Mark],
    width: f64,
    height: f64,
    transform: tiny_skia::Transform,
) {
    let edge = width.max(height);
    for mark in marks {
        let [red, green, blue] = mark.color();
        let mut paint = tiny_skia::Paint::default();
        paint.set_color_rgba8(red, green, blue, 255);
        paint.anti_alias = true;
        match mark {
            Mark::Text {
                size, x, y, text, ..
            } => {
                if let Some(path) = text_path(
                    text,
                    (size * edge) as f32,
                    (x * width) as f32,
                    (y * height) as f32,
                ) {
                    pixmap.fill_path(&path, &paint, tiny_skia::FillRule::Winding, transform, None);
                }
            }
            Mark::Stroke {
                width: line,
                points,
                ..
            } => {
                let line = (line * edge) as f32;
                let at = |index: usize| {
                    (
                        (points[index] * width) as f32,
                        (points[index + 1] * height) as f32,
                    )
                };
                if points.len() < 2 || line <= 0.0 {
                    continue;
                }
                if points.len() < 4 {
                    let (x, y) = at(0);
                    if let Some(dot) = tiny_skia::PathBuilder::from_circle(x, y, line / 2.0) {
                        pixmap.fill_path(
                            &dot,
                            &paint,
                            tiny_skia::FillRule::Winding,
                            transform,
                            None,
                        );
                    }
                    continue;
                }
                let mut builder = tiny_skia::PathBuilder::new();
                let (x, y) = at(0);
                builder.move_to(x, y);
                for index in (2..points.len() - 1).step_by(2) {
                    let (x, y) = at(index);
                    builder.line_to(x, y);
                }
                let Some(path) = builder.finish() else {
                    continue;
                };
                let stroke = tiny_skia::Stroke {
                    width: line,
                    line_cap: tiny_skia::LineCap::Round,
                    line_join: tiny_skia::LineJoin::Round,
                    ..Default::default()
                };
                pixmap.stroke_path(&path, &paint, &stroke, transform, None);
            }
        }
    }
}

fn font() -> Option<&'static ab_glyph::FontRef<'static>> {
    static FONT: std::sync::OnceLock<Option<ab_glyph::FontRef<'static>>> =
        std::sync::OnceLock::new();
    FONT.get_or_init(|| ab_glyph::FontRef::try_from_slice(SATOSHI_BOLD).ok())
        .as_ref()
}

/// One line of text at `pixels` per em: its advance width, and its ascent and
/// descent (negative, below the baseline) from the font's metrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMetrics {
    pub width: f32,
    pub ascent: f32,
    pub descent: f32,
}

impl TextMetrics {
    /// The baseline that centres the line on `y`: the middle of its ascent
    /// and descent there.
    pub fn baseline(&self, y: f32) -> f32 {
        y + (self.ascent + self.descent) / 2.0
    }
}

/// Glyphs and their pen positions, with the line's metrics.
fn layout(text: &str, pixels: f32) -> Option<(Vec<(ab_glyph::GlyphId, f32)>, TextMetrics)> {
    use ab_glyph::Font;
    let font = font()?;
    let scale = pixels / font.units_per_em()?;
    let mut glyphs = Vec::new();
    let mut pen = 0.0;
    let mut previous = None;
    for character in text.chars() {
        let id = font.glyph_id(character);
        if let Some(previous) = previous {
            pen += font.kern_unscaled(previous, id) * scale;
        }
        glyphs.push((id, pen));
        pen += font.h_advance_unscaled(id) * scale;
        previous = Some(id);
    }
    Some((
        glyphs,
        TextMetrics {
            width: pen,
            ascent: font.ascent_unscaled() * scale,
            descent: font.descent_unscaled() * scale,
        },
    ))
}

pub fn text_metrics(text: &str, pixels: f32) -> Option<TextMetrics> {
    layout(text, pixels).map(|(_, metrics)| metrics)
}

/// The outline of one line of Satoshi Bold text centred on `x`, `y`.
fn text_path(text: &str, pixels: f32, x: f32, y: f32) -> Option<tiny_skia::Path> {
    use ab_glyph::{Font, OutlineCurve};
    if text.is_empty() || pixels <= 0.0 {
        return None;
    }
    let font = font()?;
    let (glyphs, metrics) = layout(text, pixels)?;
    let scale = pixels / font.units_per_em()?;
    let left = x - metrics.width / 2.0;
    let baseline = metrics.baseline(y);
    let mut builder = tiny_skia::PathBuilder::new();
    for (id, pen) in glyphs {
        let Some(outline) = font.outline(id) else {
            continue;
        };
        // Font units have y up; contours start wherever a curve does not
        // continue the previous one.
        let place =
            |point: ab_glyph::Point| (left + pen + point.x * scale, baseline - point.y * scale);
        let mut end: Option<ab_glyph::Point> = None;
        for curve in &outline.curves {
            let start = match curve {
                OutlineCurve::Line(start, _)
                | OutlineCurve::Quad(start, _, _)
                | OutlineCurve::Cubic(start, _, _, _) => *start,
            };
            if end != Some(start) {
                if end.is_some() {
                    builder.close();
                }
                let (x, y) = place(start);
                builder.move_to(x, y);
            }
            end = Some(match curve {
                OutlineCurve::Line(_, to) => {
                    let (x, y) = place(*to);
                    builder.line_to(x, y);
                    *to
                }
                OutlineCurve::Quad(_, control, to) => {
                    let (cx, cy) = place(*control);
                    let (x, y) = place(*to);
                    builder.quad_to(cx, cy, x, y);
                    *to
                }
                OutlineCurve::Cubic(_, first, second, to) => {
                    let (ax, ay) = place(*first);
                    let (bx, by) = place(*second);
                    let (x, y) = place(*to);
                    builder.cubic_to(ax, ay, bx, by, x, y);
                    *to
                }
            });
        }
        if end.is_some() {
            builder.close();
        }
    }
    builder.finish()
}

/// Source-over of premultiplied marks onto straight-alpha pixels; untouched
/// pixels keep their exact values.
fn composite(pixels: &mut image::RgbaImage, marks: &tiny_skia::Pixmap) {
    for (pixel, mark) in pixels.pixels_mut().zip(marks.pixels()) {
        let alpha = mark.alpha();
        if alpha == 0 {
            continue;
        }
        if alpha == 255 {
            pixel.0 = [mark.red(), mark.green(), mark.blue(), 255];
            continue;
        }
        let source = f32::from(alpha) / 255.0;
        let below = f32::from(pixel[3]) / 255.0 * (1.0 - source);
        let out = source + below;
        let channel = |mark: u8, pixel: u8| {
            ((f32::from(mark) + f32::from(pixel) * below) / out)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        pixel.0 = [
            channel(mark.red(), pixel[0]),
            channel(mark.green(), pixel[1]),
            channel(mark.blue(), pixel[2]),
            (out * 255.0).round().clamp(0.0, 255.0) as u8,
        ];
    }
}

fn hex<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<[u8; 3], D::Error> {
    let text = String::deserialize(deserializer)?;
    parse_hex(&text).ok_or_else(|| serde::de::Error::custom("expected #RRGGBB"))
}

pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let digits = text.strip_prefix('#')?;
    if digits.len() != 6 || !digits.is_ascii() {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&digits[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Cases {
        rotate_left: Vec<RotateCase>,
        crop_pixels: Vec<CropCase>,
        edited: Vec<EditedCase>,
    }

    #[derive(Deserialize)]
    struct RotateCase {
        note: String,
        before: Edit,
        after: Edit,
    }

    #[derive(Deserialize)]
    struct CropCase {
        note: String,
        width: u32,
        height: u32,
        rotation: u16,
        crop: Crop,
        even: bool,
        expected: PixelRect,
    }

    #[derive(Deserialize)]
    struct EditedCase {
        note: String,
        edit: Edit,
        edited: bool,
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9
    }

    fn same(a: &Edit, b: &Edit) -> bool {
        let crops = |a: Crop, b: Crop| {
            close(a.x, b.x)
                && close(a.y, b.y)
                && close(a.width, b.width)
                && close(a.height, b.height)
        };
        a.rotation == b.rotation
            && crops(a.crop, b.crop)
            && a.marks.len() == b.marks.len()
            && a.marks.iter().zip(&b.marks).all(|pair| match pair {
                (
                    Mark::Stroke {
                        color,
                        width,
                        points,
                    },
                    Mark::Stroke {
                        color: other_color,
                        width: other_width,
                        points: other_points,
                    },
                ) => {
                    color == other_color
                        && close(*width, *other_width)
                        && points.len() == other_points.len()
                        && points.iter().zip(other_points).all(|(a, b)| close(*a, *b))
                }
                (
                    Mark::Text {
                        color,
                        size,
                        x,
                        y,
                        text,
                    },
                    Mark::Text {
                        color: other_color,
                        size: other_size,
                        x: other_x,
                        y: other_y,
                        text: other_text,
                    },
                ) => {
                    color == other_color
                        && close(*size, *other_size)
                        && close(*x, *other_x)
                        && close(*y, *other_y)
                        && text == other_text
                }
                _ => false,
            })
    }

    #[test]
    fn shared_cases_produce_identical_geometry() {
        let cases: Cases =
            serde_json::from_str(include_str!("../../../../shared/messages/edit-cases.json"))
                .expect("valid shared edit cases");
        assert!(cases.rotate_left.len() >= 4, "the shared cases loaded");
        for case in cases.rotate_left {
            let turned = case.before.rotate_left();
            assert!(same(&turned, &case.after), "{}: {turned:?}", case.note);
        }
        assert!(cases.crop_pixels.len() >= 7);
        for case in cases.crop_pixels {
            let edit = Edit {
                rotation: case.rotation,
                crop: case.crop,
                marks: Vec::new(),
            };
            assert_eq!(
                edit.crop_pixels(case.width, case.height, case.even),
                case.expected,
                "{}",
                case.note
            );
        }
        assert!(cases.edited.len() >= 4);
        for case in cases.edited {
            assert_eq!(case.edit.is_edited(), case.edited, "{}", case.note);
        }
    }

    #[test]
    fn crops_stay_inside_and_drag_by_their_handles() {
        let crop = clamp_crop(
            Crop {
                x: 0.9,
                y: -0.2,
                width: 0.5,
                height: 0.001,
            },
            0.02,
        );
        assert!(close(crop.x, 0.5) && close(crop.y, 0.0));
        assert!(close(crop.width, 0.5) && close(crop.height, 0.02));
        let start = Crop {
            x: 0.2,
            y: 0.2,
            width: 0.5,
            height: 0.5,
        };
        let moved = drag_crop(start, Handle::Move, 0.5, -0.5, (0.02, 0.02));
        assert!(close(moved.x, 0.5) && close(moved.y, 0.0) && close(moved.width, 0.5));
        let grown = drag_crop(start, Handle::SE, 0.1, 0.9, (0.02, 0.02));
        assert!(close(grown.width, 0.6) && close(grown.height, 0.8));
        // A corner dragged past the opposite one stops at the minimum size.
        let shrunk = drag_crop(start, Handle::NW, 0.9, 0.9, (0.1, 0.1));
        assert!(close(shrunk.x, 0.6) && close(shrunk.width, 0.1));
        assert!(close(shrunk.y, 0.6) && close(shrunk.height, 0.1));
        assert_eq!(Handle::NE.anchor(), (1.0, 0.0));
        assert_eq!(Handle::S.anchor(), (0.5, 1.0));
    }

    #[test]
    fn stills_and_videos_are_editable_but_animations_and_other_files_are_not() {
        let png = |chunk: &[u8; 4]| {
            let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
            bytes.extend_from_slice(&[0, 0, 0, 13]);
            bytes.extend_from_slice(b"IHDR");
            bytes.extend_from_slice(&[0; 17]);
            bytes.extend_from_slice(&[0, 0, 0, 8]);
            bytes.extend_from_slice(chunk);
            bytes
        };
        assert_eq!(
            editable_kind("shot.png", &png(b"IDAT"), false),
            Some(Kind::Image)
        );
        assert_eq!(editable_kind("moving.png", &png(b"acTL"), false), None);
        let webp = |chunk: &[u8; 4], flags: u8| {
            let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
            bytes.extend_from_slice(chunk);
            bytes.extend_from_slice(&[10, 0, 0, 0, flags]);
            bytes
        };
        assert_eq!(
            editable_kind("photo.webp", &webp(b"VP8 ", 0), false),
            Some(Kind::Image)
        );
        assert_eq!(
            editable_kind("alpha.webp", &webp(b"VP8X", 0x10), false),
            Some(Kind::Image)
        );
        assert_eq!(
            editable_kind("moving.webp", &webp(b"VP8X", 0x12), false),
            None
        );
        assert_eq!(
            editable_kind("IMG_1.JPG", b"\xff\xd8\xff\xe0", false),
            Some(Kind::Image)
        );
        assert_eq!(
            editable_kind("scan.tiff", b"II*\0", false),
            Some(Kind::Image)
        );
        assert_eq!(editable_kind("party.gif", b"GIF89a", false), None);
        assert_eq!(editable_kind("logo.svg", b"<svg", false), None);
        assert_eq!(
            editable_kind("IMG_2.HEIC", b"\0\0\0\x18ftypheic", false),
            None
        );
        assert_eq!(editable_kind("notes.pdf", b"%PDF", true), None);
        assert_eq!(
            editable_kind("voice.m4a", b"\0\0\0\x18ftypM4A ", true),
            None
        );
        let mp4 = b"\0\0\0\x18ftypisom";
        assert_eq!(editable_kind("clip.mp4", mp4, true), Some(Kind::Video));
        assert_eq!(editable_kind("clip.mp4", mp4, false), None, "needs FFmpeg");
        assert_eq!(
            editable_kind("clip.mkv", b"\x1a\x45\xdf\xa3", true),
            Some(Kind::Video)
        );
    }

    /// A 4×2 picture: red, green, blue, white along the top; black below.
    fn sample() -> DynamicImage {
        let mut image = image::RgbImage::new(4, 2);
        for (x, color) in [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 255]]
            .into_iter()
            .enumerate()
        {
            image.put_pixel(x as u32, 0, image::Rgb(color));
        }
        DynamicImage::ImageRgb8(image)
    }

    #[test]
    fn rendering_turns_and_crops_at_native_pixels() {
        let rgb = |image: &DynamicImage, x, y| image.to_rgb8().get_pixel(x, y).0;
        let untouched = render(&sample(), &Edit::default()).unwrap();
        assert_eq!(untouched.to_rgb8(), sample().to_rgb8());
        // A quarter turn clockwise puts the top-left (red) at the top right.
        let turned = render(
            &sample(),
            &Edit {
                rotation: 90,
                ..Edit::default()
            },
        )
        .unwrap();
        assert_eq!((turned.width(), turned.height()), (2, 4));
        assert_eq!(rgb(&turned, 1, 0), [255, 0, 0]);
        assert_eq!(rgb(&turned, 1, 3), [255, 255, 255]);
        assert_eq!(rgb(&turned, 0, 0), [0, 0, 0]);
        let left = Edit {
            rotation: 0,
            crop: Crop::FULL,
            marks: Vec::new(),
        }
        .rotate_left();
        let anticlockwise = render(&sample(), &left).unwrap();
        assert_eq!(rgb(&anticlockwise, 0, 0), [255, 255, 255]);
        assert_eq!(rgb(&anticlockwise, 0, 3), [255, 0, 0]);
        // Cropped to the top row's middle pair, then turned upside down.
        let cropped = render(
            &sample(),
            &Edit {
                rotation: 180,
                crop: Crop {
                    x: 0.25,
                    y: 0.5,
                    width: 0.5,
                    height: 0.5,
                },
                marks: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!((cropped.width(), cropped.height()), (2, 1));
        assert_eq!(rgb(&cropped, 0, 0), [0, 0, 255]);
        assert_eq!(rgb(&cropped, 1, 0), [0, 255, 0]);
        for rotation in [90, 270] {
            let region = render(
                &sample(),
                &Edit {
                    rotation,
                    crop: Crop {
                        x: 0.0,
                        y: 0.0,
                        width: 0.5,
                        height: 0.25,
                    },
                    marks: Vec::new(),
                },
            )
            .unwrap();
            assert_eq!((region.width(), region.height()), (1, 1));
            let expected = if rotation == 90 {
                [0, 0, 0]
            } else {
                [255, 255, 255]
            };
            assert_eq!(rgb(&region, 0, 0), expected, "{rotation}");
        }
    }

    #[test]
    fn marks_paint_strokes_dots_and_text_over_the_picture() {
        let grey = DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            200,
            100,
            image::Rgba([128, 128, 128, 255]),
        ));
        let dot = Edit {
            marks: vec![Mark::Stroke {
                color: [229, 72, 77],
                width: 0.05,
                points: vec![0.5, 0.5],
            }],
            ..Edit::default()
        };
        let painted = render(&grey, &dot).unwrap().to_rgba8();
        assert_eq!(painted.get_pixel(100, 50).0, [229, 72, 77, 255]);
        assert_eq!(painted.get_pixel(10, 10).0, [128, 128, 128, 255]);
        // 0.05 of the 200 pixel edge: a 10 pixel dot, nothing 8 pixels out.
        assert_eq!(painted.get_pixel(108, 50).0, [128, 128, 128, 255]);
        let line = Edit {
            marks: vec![Mark::Stroke {
                color: [0, 0, 255],
                width: 0.02,
                points: vec![0.1, 0.5, 0.9, 0.5],
            }],
            ..Edit::default()
        };
        let painted = render(&grey, &line).unwrap().to_rgba8();
        assert_eq!(painted.get_pixel(100, 50).0, [0, 0, 255, 255]);
        // Round caps reach past the ends by half the width.
        assert_eq!(painted.get_pixel(19, 50).0[2], 255);
        assert_eq!(painted.get_pixel(100, 56).0, [128, 128, 128, 255]);
        let text = Edit {
            marks: vec![Mark::Text {
                color: [0, 0, 0],
                size: 0.2,
                x: 0.5,
                y: 0.5,
                text: "Hi".into(),
            }],
            ..Edit::default()
        };
        let painted = render(&grey, &text).unwrap().to_rgba8();
        let dark = |x0: u32, x1: u32, y0: u32, y1: u32| {
            (x0..x1)
                .flat_map(|x| (y0..y1).map(move |y| (x, y)))
                .filter(|&(x, y)| painted.get_pixel(x, y).0[0] < 40)
                .count()
        };
        // 40 pixel Satoshi Bold around the centre, nothing far away.
        assert!(dark(80, 120, 35, 65) > 150, "text darkens its centre");
        assert_eq!(dark(0, 60, 0, 100) + dark(140, 200, 0, 100), 0);
        let metrics = text_metrics("Hi", 40.0).unwrap();
        assert!(metrics.width > 20.0 && metrics.width < 60.0);
        assert!(metrics.ascent > 0.0 && metrics.descent < 0.0);
        // Opaque sources stay RGB; translucent marks blend.
        assert_eq!(
            render(&sample(), &dot).unwrap().color(),
            image::ColorType::Rgb8
        );
        let mut marks = tiny_skia::Pixmap::new(1, 1).unwrap();
        marks.fill(tiny_skia::Color::from_rgba8(255, 255, 255, 128));
        let mut pixels = image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        composite(&mut pixels, &marks);
        assert_eq!(pixels.get_pixel(0, 0).0, [128, 128, 128, 255]);
    }

    #[test]
    fn video_overlays_hold_only_the_marks_at_the_output_size() {
        let edit = Edit {
            rotation: 0,
            crop: Crop {
                x: 0.5,
                y: 0.0,
                width: 0.5,
                height: 1.0,
            },
            marks: vec![Mark::Stroke {
                color: [255, 255, 255],
                width: 0.02,
                points: vec![0.75, 0.5],
            }],
        };
        let region = edit.crop_pixels(400, 200, true);
        assert_eq!(
            region,
            PixelRect {
                left: 200,
                top: 0,
                width: 200,
                height: 200
            }
        );
        let pixmap = overlay(&edit, 400, 200, region, (100, 100)).unwrap();
        assert_eq!((pixmap.width(), pixmap.height()), (100, 100));
        // The dot's centre (300, 100) lands in the middle of the half-size output.
        let at = |x: u32, y: u32| pixmap.pixel(x, y).unwrap().alpha();
        assert_eq!(at(50, 50), 255);
        assert_eq!(at(10, 10), 0);
        assert_eq!(at(50, 56), 0, "8 picture pixels across, 4 here");
        assert_eq!(parse_hex("#3B82F6"), Some([0x3B, 0x82, 0xF6]));
        assert_eq!(parse_hex("3B82F6"), None);
    }
}
