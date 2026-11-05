//! The bundled FFmpeg (`caper_client::ffmpeg`), plus decoding stills into
//! egui images for display.

pub use caper_client::ffmpeg::*;

/// Decode an AVIF/HEIC still at the caller's display bound. The container
/// needs seeking, so use the same disposable scratch files as video uploads.
pub fn decode_image(
    bytes: &[u8],
    content_type: &str,
    max_edge: u32,
) -> Option<eframe::egui::ColorImage> {
    let image = decode_still(bytes, content_type, Some(max_edge))?;
    let size = [image.width() as usize, image.height() as usize];
    Some(eframe::egui::ColorImage::from_rgba_unmultiplied(
        size,
        image.as_raw(),
    ))
}
