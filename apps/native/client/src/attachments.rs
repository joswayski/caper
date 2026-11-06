//! Message attachments: signed-URL freshness and refresh scheduling, what
//! an inline tile loads and how large it is, and the edge locally shown
//! images are decoded at. Mirrors `apps/web/src/chat/Attachments.tsx`.
//! Clients stay open for days, far longer than a message file's signed URL
//! lives (1–2 hours: until the next-but-one full hour), so URLs are
//! refreshed through `POST /api/assets/urls` shortly before they expire and
//! after a 403/404 load.

use crate::Rgba;
use crate::model::{Attachment, AttachmentKind, AttachmentStatus};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// A profile picture as a viewer image, keyed by its stable asset.
pub fn profile_picture(source: crate::pictures::Source<'_>, name: &str) -> Option<Attachment> {
    Some(Attachment {
        id: format!("profile-picture:{}", source.asset),
        name: format!("{name} profile picture"),
        kind: AttachmentKind::Image,
        content_type: "image/webp".into(),
        url: Some(source.url?.to_owned()),
        preview_url: None,
        size: 0,
        width: None,
        height: None,
        duration_ms: None,
        preview: false,
        unavailable: false,
        status: AttachmentStatus::Ready,
        animated: false,
        voice: false,
        waveform: None,
        transcript: None,
        spoiler: false,
    })
}
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FreshUrl {
    /// Absent while the file is still processing.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub preview_url: Option<String>,
}

/// Locally shown images fit the largest inline frame at 2x.
pub const THUMBNAIL_EDGE: u32 = 720;

/// Inline media frames, as on web.
pub const MAX_WIDTH: f32 = 360.0;
pub const MAX_HEIGHT: f32 = 300.0;
/// Refresh URLs that expire within this many seconds. Links live 1–2 hours,
/// so a wider margin would keep every link permanently "expiring".
pub const REFRESH_MARGIN_SECONDS: i64 = 10 * 60;
/// Do not ask for the same attachment again sooner than this (shorter than
/// the margin, so a failed refresh is retried before the link expires).
pub const REFRESH_BACKOFF: Duration = Duration::from_secs(5 * 60);
/// The API accepts at most this many ids per refresh.
pub const MAX_REFRESH_IDS: usize = 100;

/// Unix seconds at which a signed URL stops working, if it says.
pub fn url_expiry(url: &str) -> Option<i64> {
    url::Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(name, _)| name == "exp")?
        .1
        .parse::<i64>()
        .ok()
        .filter(|expiry| *expiry > 0)
}

/// Whether a URL is expired or expires within [`REFRESH_MARGIN_SECONDS`].
/// URLs without an expiry are never refreshed proactively.
pub fn needs_refresh(url: &str, now: i64) -> bool {
    url_expiry(url).is_some_and(|expiry| expiry - now <= REFRESH_MARGIN_SECONDS)
}

/// The attachment with whichever signed URLs live longer: a refreshed pair or
/// the ones a newer history page or gateway frame carried. A refresh taken
/// while the file was processing (preview only) never hides a later `url`.
pub fn resolve(attachment: &Attachment, fresh: Option<&FreshUrl>) -> Attachment {
    let mut resolved = attachment.clone();
    let Some(fresh) = fresh.filter(|_| !attachment.unavailable) else {
        return resolved;
    };
    let expiry = |url: Option<&str>| url.and_then(url_expiry).unwrap_or(i64::MIN);
    if let Some(url) = &fresh.url {
        if attachment.url.is_none() || expiry(Some(url)) > expiry(attachment.url.as_deref()) {
            resolved.url = Some(url.clone());
            if fresh.preview_url.is_some() {
                resolved.preview_url.clone_from(&fresh.preview_url);
            }
        }
    } else if let Some(preview) = &fresh.preview_url
        && attachment.url.is_none()
        && (attachment.preview_url.is_none()
            || expiry(Some(preview)) > expiry(attachment.preview_url.as_deref()))
    {
        resolved.preview_url = Some(preview.clone());
    }
    resolved
}

/// Still formats the bundled pure-Rust codecs decode. Originals in these
/// formats use `image`; AVIF uses the bundled FFmpeg instead.
pub fn decodable(content_type: &str) -> bool {
    matches!(
        content_type,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    )
}

/// What to download for an inline tile: the preview (images and video
/// posters), else a ready original that decodes here.
pub fn inline_source(attachment: &Attachment) -> Option<&str> {
    if attachment.kind == AttachmentKind::Image
        && attachment.content_type == "image/gif"
        && attachment.status == AttachmentStatus::Ready
    {
        return attachment.url.as_deref();
    }
    attachment.preview_url.as_deref().or_else(|| {
        (attachment.kind == AttachmentKind::Image
            && attachment.status == AttachmentStatus::Ready
            && (decodable(&attachment.content_type) || attachment.content_type == "image/avif"))
            .then_some(attachment.url.as_deref())
            .flatten()
    })
}

/// Link preview media (`crate::embeds`) share this cache under ids with
/// this prefix. Their URLs are not signed, so they are never refreshed.
pub const EMBED_KEY: &str = "embed:";
/// Cache keys for media Caper never signs (KLIPY GIFs): a failed load is
/// final instead of asking `POST /api/assets/urls` to re-sign it.
pub const EXTERNAL_KEY: &str = "external:";

/// Display size that reserves layout space before an image loads.
pub fn frame_size(attachment: &Attachment) -> [f32; 2] {
    match (attachment.width, attachment.height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            let (width, height) = (width as f32, height as f32);
            let scale = (MAX_WIDTH / width).min(MAX_HEIGHT / height).min(1.0);
            [
                (width * scale).round().max(1.0),
                (height * scale).round().max(1.0),
            ]
        }
        _ => [240.0, 180.0],
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum FailureAction {
    /// The URL probably expired: refresh it, then load once more.
    RefreshThenRetry,
    GiveUp,
}

/// A 403/404 from the CDN means an expired or rotated signature: refresh and
/// retry once. Anything else, or a second failure, is final.
pub fn on_fetch_failure(status: Option<u16>, already_retried: bool) -> FailureAction {
    if matches!(status, Some(403 | 404)) && !already_retried {
        FailureAction::RefreshThenRetry
    } else {
        FailureAction::GiveUp
    }
}

/// Batches refresh requests with a per-attachment backoff.
#[derive(Default)]
pub struct Refresher {
    requested: BTreeMap<String, Instant>,
    queue: BTreeSet<String>,
}

impl Refresher {
    /// Queue an id unless it was requested recently.
    pub fn want(&mut self, id: &str, now: Instant) {
        if !id.starts_with(EMBED_KEY)
            && self
                .requested
                .get(id)
                .is_none_or(|at| now.duration_since(*at) >= REFRESH_BACKOFF)
        {
            self.queue.insert(id.to_owned());
        }
    }

    /// Queue regardless of backoff (a failed load or an explicit open).
    pub fn force(&mut self, id: &str) {
        if !id.starts_with(EMBED_KEY) {
            self.queue.insert(id.to_owned());
        }
    }

    /// Whether any id waits to be refreshed.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// At most [`MAX_REFRESH_IDS`] queued ids, marked as requested now.
    pub fn take(&mut self, now: Instant) -> Vec<String> {
        let ids: Vec<String> = self.queue.iter().take(MAX_REFRESH_IDS).cloned().collect();
        for id in &ids {
            self.queue.remove(id);
            self.requested.insert(id.clone(), now);
        }
        ids
    }
}

/// GIF and WebP keep every composited frame and its delay, never the uploaded poster.
/// Shrink playback frames to fit the retained-memory budget without freezing
/// normal GIFs. Decode runs on the existing fetch worker.
pub fn decode_animation(bytes: &[u8]) -> Option<Vec<(Rgba, Duration)>> {
    use image::{AnimationDecoder, ImageDecoder};
    const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(32 * 1024 * 1024);
    let cursor = || std::io::Cursor::new(bytes);
    let ((width, height), decoded) = match image::guess_format(bytes).ok()? {
        image::ImageFormat::Gif => {
            let mut decoder = image::codecs::gif::GifDecoder::new(cursor()).ok()?;
            decoder.set_limits(limits).ok()?;
            (decoder.dimensions(), decoder.into_frames())
        }
        image::ImageFormat::WebP => {
            let mut decoder = image::codecs::webp::WebPDecoder::new(cursor()).ok()?;
            decoder.set_limits(limits).ok()?;
            if !decoder.has_animation() {
                return decode_thumbnail(bytes).map(|image| vec![(image, Duration::ZERO)]);
            }
            (decoder.dimensions(), decoder.into_frames())
        }
        _ => return None,
    };
    let denominator = width.max(height).max(THUMBNAIL_EDGE);
    let mut width = (width * THUMBNAIL_EDGE / denominator).max(1);
    let mut height = (height * THUMBNAIL_EDGE / denominator).max(1);
    let mut frames: Vec<(image::RgbaImage, Duration)> = Vec::new();
    let mut total = 0;
    for frame in decoded {
        let frame = frame.ok()?;
        if frames.len() >= 600 {
            // Keep the decode-work ceiling; never loop a truncated prefix.
            frames.truncate(1);
            break;
        }
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = Duration::from_millis(u64::from(numerator / denominator.max(1)));
        let delay = if delay < Duration::from_millis(20) {
            Duration::from_millis(100)
        } else {
            delay
        };
        let mut image = frame.into_buffer();
        if image.dimensions() != (width, height) {
            image = image::imageops::thumbnail(&image, width, height);
        }
        while total + image.len() > MAX_FRAME_BYTES {
            // GIF exposes no frame count up front. Geometric resizing bounds
            // retained pixels in one decode pass, preserving every delay.
            width = (width / 2).max(1);
            height = (height / 2).max(1);
            total = 0;
            for (previous, _) in &mut frames {
                *previous = image::imageops::thumbnail(previous, width, height);
                total += previous.len();
            }
            image = image::imageops::thumbnail(&image, width, height);
        }
        total += image.len();
        frames.push((image, delay));
    }
    (!frames.is_empty()).then(|| {
        frames
            .into_iter()
            .map(|(image, delay)| (Rgba::from_image(image), delay))
            .collect()
    })
}

/// Decode a still for display only, within memory limits.
pub fn decode_thumbnail(bytes: &[u8]) -> Option<Rgba> {
    decode_image(bytes, THUMBNAIL_EDGE)
}

/// Decode a still at most `edge` on its long side, within memory limits.
pub fn decode_image(bytes: &[u8], edge: u32) -> Option<Rgba> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if reader.format() == Some(image::ImageFormat::Avif) {
        return crate::ffmpeg::decode_still(bytes, "image/avif", Some(edge)).map(Rgba::from_image);
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let mut image = reader.decode().ok()?;
    if image.width() > edge || image.height() > edge {
        image = image.thumbnail(edge, edge);
    }
    Some(Rgba::from_image(image.to_rgba8()))
}

/// Message-order gallery slots as `[x, y, width, height]`, relative to the
/// gallery's top-left corner: one wide row of two, a large tile beside two
/// small ones, then rows of three (the first rows two when they do not divide).
pub fn gallery(count: usize, width: f32) -> Vec<[f32; 4]> {
    let width = width.clamp(1.0, 480.0);
    let gap = 4.0;
    let height = width * 2.0 / 3.0;
    let rect = |x, y, w, h| [x, y, w, h];
    match count {
        0 | 1 => vec![],
        2 => {
            let column = (width - gap) / 2.0;
            vec![
                rect(0.0, 0.0, column, height),
                rect(column + gap, 0.0, column, height),
            ]
        }
        3 => {
            let column = (width - gap) / 3.0;
            let half = (height - gap) / 2.0;
            vec![
                rect(0.0, 0.0, column * 2.0, height),
                rect(column * 2.0 + gap, 0.0, column, half),
                rect(column * 2.0 + gap, half + gap, column, half),
            ]
        }
        _ => {
            let rows = count.div_ceil(3);
            let short_rows = rows * 3 - count;
            let height = if count == 4 {
                (width - gap) / 2.0
            } else {
                (width - gap * 2.0) / 3.0
            };
            (0..rows)
                .flat_map(|row| {
                    let columns = if row < short_rows { 2 } else { 3 };
                    let column = (width - gap * (columns - 1) as f32) / columns as f32;
                    (0..columns).map(move |col| {
                        rect(
                            col as f32 * (column + gap),
                            row as f32 * (height + gap),
                            column,
                            height,
                        )
                    })
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    const NOW: i64 = 1_790_000_000;

    fn signed(expiry: i64) -> String {
        format!("https://cdn.caper.chat/original/asset1?exp={expiry}&sig=abc")
    }

    #[test]
    fn expiry_comes_from_the_exp_parameter() {
        assert_eq!(url_expiry(&signed(NOW)), Some(NOW));
        assert_eq!(url_expiry("https://cdn.caper.chat/original/a?sig=x"), None);
        assert_eq!(
            url_expiry("https://cdn.caper.chat/original/a?exp=soon"),
            None
        );
        assert_eq!(url_expiry("https://cdn.caper.chat/original/a?exp=0"), None);
        assert_eq!(url_expiry("not a url"), None);
    }

    #[test]
    fn refresh_when_expired_or_within_ten_minutes() {
        assert!(needs_refresh(&signed(NOW - 1), NOW));
        assert!(needs_refresh(&signed(NOW + 600), NOW));
        assert!(!needs_refresh(&signed(NOW + 601), NOW));
        // A fresh link (next-but-one full hour) is not expiring yet.
        assert!(!needs_refresh(&signed(NOW + 3_600), NOW));
        assert!(!needs_refresh(&signed(NOW + 7_200), NOW));
        const { assert!(REFRESH_BACKOFF.as_secs() < REFRESH_MARGIN_SECONDS as u64) };
        assert!(!needs_refresh("https://cdn.caper.chat/original/a", NOW));
    }

    #[test]
    fn resolve_prefers_the_longer_lived_urls() {
        let fresh = FreshUrl {
            url: Some(signed(NOW + 90_000)),
            preview_url: Some("https://cdn.caper.chat/preview/asset1?exp=1&sig=p".into()),
        };
        let old = attachment(Some(&signed(NOW)));
        let resolved = resolve(&old, Some(&fresh));
        assert_eq!(resolved.url, fresh.url);
        assert_eq!(resolved.preview_url, fresh.preview_url);
        // A newer history page beats an older refresh.
        let newer = attachment(Some(&signed(NOW + 200_000)));
        assert_eq!(resolve(&newer, Some(&fresh)).url, newer.url);
        // A message without URLs gains them; removed files never do.
        assert_eq!(resolve(&attachment(None), Some(&fresh)).url, fresh.url);
        let mut removed = attachment(None);
        removed.unavailable = true;
        assert_eq!(resolve(&removed, Some(&fresh)).url, None);
    }

    #[test]
    fn processing_refreshes_add_previews_without_hiding_later_urls() {
        let mut processing = attachment(None);
        processing.status = AttachmentStatus::Processing;
        let poster = FreshUrl {
            url: None,
            preview_url: Some("https://cdn.caper.chat/preview/asset1?exp=9&sig=p".into()),
        };
        let resolved = resolve(&processing, Some(&poster));
        assert_eq!(resolved.preview_url, poster.preview_url);
        assert_eq!(resolved.url, None);
        // Once ready, the message's own URLs win over the older refresh.
        let ready = attachment(Some(&signed(NOW)));
        let resolved = resolve(&ready, Some(&poster));
        assert_eq!(resolved.url, ready.url);
        assert_eq!(resolved.preview_url, ready.preview_url);
    }

    #[test]
    fn inline_images_use_the_preview_or_decodable_originals() {
        let mut photo = attachment(Some(&signed(NOW)));
        photo.content_type = "image/avif".into();
        assert_eq!(inline_source(&photo), photo.preview_url.as_deref());
        photo.preview_url = None;
        assert_eq!(inline_source(&photo), photo.url.as_deref());
        photo.status = AttachmentStatus::Processing;
        assert_eq!(inline_source(&photo), None);
        // A legacy PNG without a preview still loads its original.
        let mut legacy = attachment(Some(&signed(NOW)));
        legacy.preview_url = None;
        assert_eq!(inline_source(&legacy), legacy.url.as_deref());
        legacy.status = AttachmentStatus::Processing;
        assert_eq!(inline_source(&legacy), None);
        let mut gif = attachment(Some(&signed(NOW)));
        gif.content_type = "image/gif".into();
        assert_eq!(
            inline_source(&gif),
            gif.url.as_deref(),
            "GIFs bypass still previews"
        );
        gif.status = AttachmentStatus::Processing;
        assert_eq!(inline_source(&gif), gif.preview_url.as_deref());
        gif.status = AttachmentStatus::Ready;
        gif.kind = AttachmentKind::Video;
        gif.animated = true;
        gif.preview_url = None;
        assert_eq!(inline_source(&gif), None, "MP4s are never decoded");
    }

    #[test]
    fn frames_reserve_aspect_ratio_within_web_bounds() {
        assert_eq!(frame_size(&attachment(None)), [360.0, 180.0]);
        let mut tall = attachment(None);
        (tall.width, tall.height) = (Some(1000), Some(3000));
        assert_eq!(frame_size(&tall), [100.0, 300.0]);
        (tall.width, tall.height) = (Some(20), Some(10));
        assert_eq!(frame_size(&tall), [20.0, 10.0]);
        (tall.width, tall.height) = (None, Some(10));
        assert_eq!(frame_size(&tall), [240.0, 180.0]);
    }

    #[test]
    fn only_expired_signatures_refresh_and_only_once() {
        assert_eq!(
            on_fetch_failure(Some(403), false),
            FailureAction::RefreshThenRetry
        );
        assert_eq!(
            on_fetch_failure(Some(404), false),
            FailureAction::RefreshThenRetry
        );
        assert_eq!(on_fetch_failure(Some(403), true), FailureAction::GiveUp);
        assert_eq!(on_fetch_failure(Some(500), false), FailureAction::GiveUp);
        assert_eq!(on_fetch_failure(None, false), FailureAction::GiveUp);
    }

    #[test]
    fn refresh_batches_respect_backoff_and_the_api_limit() {
        let mut refresher = Refresher::default();
        let start = Instant::now();
        for index in 0..150 {
            refresher.want(&format!("id{index:03}"), start);
        }
        assert_eq!(refresher.take(start).len(), 100);
        assert_eq!(refresher.take(start).len(), 50);
        assert!(refresher.take(start).is_empty());
        refresher.want("id000", start + Duration::from_secs(60));
        assert!(refresher.take(start).is_empty(), "requested a minute ago");
        refresher.want("id000", start + REFRESH_BACKOFF);
        assert_eq!(refresher.take(start + REFRESH_BACKOFF), ["id000"]);
        refresher.force("id001");
        assert_eq!(refresher.take(start), ["id001"]);
        // Link preview media are unsigned: nothing to refresh.
        refresher.force(&format!("{EMBED_KEY}https://embed.caper.chat/m/a"));
        refresher.want(&format!("{EMBED_KEY}https://embed.caper.chat/m/b"), start);
        assert!(refresher.take(start).is_empty());
    }
}
