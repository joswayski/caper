//! Showing message attachments in egui: decoding, gallery layout, and the
//! egui textures for the shared attachment image cache
//! ([`caper_client::media`]: signed-URL freshness, refresh scheduling and
//! an image cache keyed by attachment id, so refreshed URLs reuse textures).
//! Mirrors `apps/web/src/chat/Attachments.tsx`. A signed-in window repaints
//! at least every `MEMBERSHIP_REFRESH` (15 s) even while idle, well inside
//! the refresh margin.
//!
//! Inline images come from the `previewUrl` clients upload for large stills
//! (JPEG), else from the PNG/JPEG/WebP/GIF/AVIF original. AVIF thumbnails
//! use the same bundled FFmpeg decoder as the full-size viewer.

use crate::model::Attachment;
pub use caper_client::attachments::{
    EMBED_KEY, FreshUrl, THUMBNAIL_EDGE, decodable, inline_source,
};
use caper_client::media::{self, spoiler_key};
use eframe::egui;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

fn color_image(image: &caper_client::Rgba) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(image.size, &image.pixels)
}

fn rgba(image: &egui::ColorImage) -> caper_client::Rgba {
    caper_client::Rgba {
        size: image.size,
        pixels: image
            .pixels
            .iter()
            .flat_map(|pixel| pixel.to_srgba_unmultiplied())
            .collect(),
    }
}

/// Decode a still for display only, within memory limits.
pub fn decode_thumbnail(bytes: &[u8]) -> Option<egui::ColorImage> {
    caper_client::attachments::decode_thumbnail(bytes).map(|image| color_image(&image))
}

/// Message-order gallery slots, relative to the gallery's top-left corner.
pub fn gallery_rects(count: usize, width: f32) -> Vec<egui::Rect> {
    caper_client::attachments::gallery(count, width)
        .into_iter()
        .map(|[x, y, w, h]| egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h)))
        .collect()
}

/// Blur a bounded still on the CPU; never retain animation frames in the cover.
pub fn blur_thumbnail(image: &egui::ColorImage) -> egui::ColorImage {
    color_image(&media::blur(&rgba(image)))
}

/// Display size that reserves layout space before an image loads.
pub fn frame_size(attachment: &Attachment) -> egui::Vec2 {
    caper_client::attachments::frame_size(attachment).into()
}

pub type ImageState = media::ImageState<egui::TextureId>;

/// Attachment textures in egui.
pub struct EguiTextures(pub egui::Context);

impl media::Textures for EguiTextures {
    type Texture = egui::TextureHandle;
    type Id = egui::TextureId;

    fn create(&mut self, key: &str, image: &caper_client::Rgba) -> egui::TextureHandle {
        self.0.load_texture(
            format!("attachment-{key}"),
            color_image(image),
            egui::TextureOptions::LINEAR,
        )
    }

    fn update(&mut self, texture: &mut egui::TextureHandle, image: &caper_client::Rgba) {
        texture.set(color_image(image), egui::TextureOptions::LINEAR);
    }

    fn id(texture: &egui::TextureHandle) -> egui::TextureId {
        texture.id()
    }
}

/// Per-account attachment display state. Cleared on logout.
pub struct Media {
    cache: media::Media<EguiTextures>,
    copying: Option<Receiver<Option<egui::ColorImage>>>,
    pub copy_status: Option<&'static str>,
}

impl Media {
    /// `api` is `None` in tests and fixtures, which never fetch.
    pub fn new(context: &egui::Context, api: Option<crate::api::Api>) -> Self {
        let context = context.clone();
        Self {
            cache: media::Media::new(api, Arc::new(move || context.request_repaint())),
            copying: None,
            copy_status: None,
        }
    }

    pub fn clear(&mut self) {
        self.cache.clear();
        self.copying = None;
        self.copy_status = None;
    }

    pub fn copying(&self) -> bool {
        self.copying.is_some()
    }

    /// Decode the original, not the cropped tile or JPEG preview. Clipboard
    /// delivery stays on the UI thread; logout drops any in-flight result.
    pub fn copy_image(&mut self, context: &egui::Context, attachment: &Attachment) {
        let (Some(api), Some(url)) = (self.cache.api().cloned(), attachment.url.clone()) else {
            self.copy_status = Some("Image copying is unavailable in this preview.");
            return;
        };
        let content_type = attachment.content_type.clone();
        let context = context.clone();
        let (sender, receiver) = mpsc::channel();
        self.copying = Some(receiver);
        self.copy_status = Some("Copying image…");
        std::thread::spawn(move || {
            let image = api.fetch_media(&url).ok().and_then(|bytes| {
                crate::viewer::decode_original(&bytes, &content_type).map(|image| {
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        image.as_raw(),
                    )
                })
            });
            let _ = sender.send(image);
            context.request_repaint();
        });
    }

    pub fn resolve(&self, attachment: &Attachment) -> Attachment {
        self.cache.resolve(attachment)
    }

    /// Note a rendered attachment; expiring URLs are queued for refresh.
    pub fn visible(&mut self, attachment: &Attachment, unix_now: i64, now: Instant) {
        self.cache.visible(attachment, unix_now, now);
    }

    /// Refresh one attachment's URLs as soon as possible (a viewer load failed).
    pub fn refresh_now(&mut self, id: &str) {
        self.cache.refresh_now(id);
    }

    /// The client media loads use, for loads outside this cache.
    pub fn api(&self) -> Option<&crate::api::Api> {
        self.cache.api()
    }

    pub fn take_refresh(&mut self, now: Instant) -> Vec<String> {
        self.cache.take_refresh(now)
    }

    /// Apply a refresh response. Returns a URL to open if an open waited on it.
    pub fn refreshed(
        &mut self,
        requested: &[String],
        urls: BTreeMap<String, FreshUrl>,
    ) -> Option<String> {
        self.cache.refreshed(requested, urls)
    }

    /// Open the full file in the system browser or player, refreshing first
    /// when the signed URL is about to expire.
    pub fn open(&mut self, context: &egui::Context, attachment: &Attachment, unix_now: i64) {
        if let Some(url) = self.cache.open(attachment, unix_now) {
            context.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    /// Show a locally decoded image for an attachment (own uploads), so the
    /// pending row and the confirmed message need no download.
    pub fn insert_local(&mut self, context: &egui::Context, id: &str, image: egui::ColorImage) {
        self.cache
            .insert_local(&mut EguiTextures(context.clone()), id, rgba(&image));
    }

    pub fn local_spoiler(
        &mut self,
        context: &egui::Context,
        id: &str,
        image: &egui::ColorImage,
    ) -> ImageState {
        // Drawn every frame: convert the pixels only for a cover not made yet.
        let key = spoiler_key(id);
        if self.cache.texture(&key).is_some() {
            return self.cache.image(&key, None);
        }
        self.cache
            .local_spoiler(&mut EguiTextures(context.clone()), id, &rgba(image))
    }

    /// The image for an attachment id, starting a load from `url` if needed.
    pub fn image(&mut self, id: &str, url: Option<&str>) -> ImageState {
        self.cache.image(id, url)
    }

    /// A separate static texture, so revealing always uses the untouched original.
    pub fn spoiler(&mut self, attachment: &Attachment) -> ImageState {
        self.cache.spoiler(attachment)
    }

    /// Called only for a visible, ready GIF tile.
    pub fn gif(
        &mut self,
        context: &egui::Context,
        id: &str,
        url: Option<&str>,
        playing: bool,
        now: Instant,
    ) -> ImageState {
        let (state, next) =
            self.cache
                .gif(&mut EguiTextures(context.clone()), id, url, playing, now);
        // Repaint when the next frame is due, not continuously.
        if let Some(next) = next {
            context.request_repaint_after(next);
        }
        state
    }

    /// Work waiting for the next [`Media::poll`] or refresh request.
    pub fn busy(&self) -> bool {
        self.cache.busy()
    }

    /// Apply finished loads and copies, start queued loads, and bound texture memory.
    pub fn poll(&mut self, context: &egui::Context) {
        if let Some(result) = self
            .copying
            .as_ref()
            .and_then(|receiver| receiver.try_recv().ok())
        {
            self.copying = None;
            self.copy_status = Some(if let Some(image) = result {
                context.copy_image(image);
                "Image copied."
            } else {
                "Couldn’t copy image. Try again."
            });
        }
        self.cache
            .poll(&mut EguiTextures(context.clone()), Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TestPass as _;
    use crate::model::{AttachmentKind, AttachmentStatus};
    use std::time::Duration;

    fn attachment(url: Option<&str>) -> Attachment {
        Attachment {
            spoiler: false,
            id: "asset1".into(),
            kind: AttachmentKind::Image,
            content_type: "image/png".into(),
            name: "shot.png".into(),
            size: 10,
            width: Some(1200),
            height: Some(600),
            duration_ms: None,
            preview: true,
            url: url.map(str::to_owned),
            preview_url: url.map(|url| url.replace("original", "preview")),
            unavailable: false,
            status: AttachmentStatus::Ready,
            animated: false,
            voice: false,
            waveform: None,
            transcript: None,
        }
    }

    #[test]
    fn copy_image_delivers_original_pixels_not_preview_or_link() {
        use std::io::{Read, Write};
        let pixels = vec![255, 0, 0, 255, 0, 70, 230, 128];
        let image = image::RgbaImage::from_raw(2, 1, pixels.clone()).unwrap();
        let mut encoded = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let bytes = encoded.into_inner();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let length = stream.read(&mut request).unwrap();
            assert!(
                std::str::from_utf8(&request[..length])
                    .unwrap()
                    .starts_with("GET /original ")
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            stream.write_all(&bytes).unwrap();
        });
        let context = egui::Context::default();
        let mut media = Media::new(&context, Some(crate::api::Api::new(&base).unwrap()));
        media.copy_image(&context, &attachment(Some(&format!("{base}/original"))));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let output = context.test_pass(Default::default(), |ctx| media.poll(ctx));
            if let Some(egui::OutputCommand::CopyImage(image)) =
                output.platform_output.commands.first()
            {
                assert_eq!(image.size, [2, 1]);
                let expected = egui::ColorImage::from_rgba_unmultiplied([2, 1], &pixels);
                assert_eq!(image.pixels, expected.pixels);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "image clipboard output never arrived"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        server.join().unwrap();
    }
    #[test]
    #[ignore = "needs CAPER_FFMPEG with AVIF decoding (the bundled release binary)"]
    fn avif_originals_decode_for_inline_and_spoiler_display() {
        let original = image::RgbImage::from_fn(960, 480, |x, _| {
            if x < 480 {
                image::Rgb([220, 20, 20])
            } else {
                image::Rgb([20, 20, 220])
            }
        });
        let bytes =
            caper_client::avif::encode(&original, 85, caper_client::avif::Primaries::Srgb).unwrap();
        let decoded = decode_thumbnail(&bytes).expect("AVIF original must render inline");
        assert_eq!(decoded.size, [720, 360]);
        let left = decoded.pixels[180 * 720 + 90];
        let right = decoded.pixels[180 * 720 + 630];
        assert!(left.r() > 180 && left.b() < 60);
        assert!(right.b() > 180 && right.r() < 60);
        assert!(decode_thumbnail(&bytes[..bytes.len() / 2]).is_none());
    }

    #[test]
    fn webp_previews_decode_and_shrink_to_the_thumbnail_edge() {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            1440,
            360,
            image::Rgba([182, 77, 50, 255]),
        ))
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::WebP,
        )
        .unwrap();
        assert_eq!(&bytes[8..12], b"WEBP");
        let image = decode_thumbnail(&bytes).unwrap();
        assert_eq!(image.size, [720, 180]);
        assert!(decode_thumbnail(b"not an image").is_none());
    }
}
