//! Showing message attachments: signed-URL freshness, refresh scheduling and
//! an image cache keyed by attachment id, so refreshed URLs reuse textures.
//! Mirrors `apps/web/src/chat/Attachments.tsx`. Desktop stays open for days,
//! far longer than a message file's signed URL lives (1–2 hours: until the
//! next-but-one full hour), so URLs are refreshed through
//! `POST /api/assets/urls` shortly before they expire and after a 403/404
//! load. Rendered attachments are rechecked on every frame, and a signed-in
//! window repaints at least every `MEMBERSHIP_REFRESH` (15 s) even while
//! idle, well inside the refresh margin.
//!
//! Inline images come from the `previewUrl` clients upload for large stills
//! (JPEG), else from the PNG/JPEG/WebP/GIF/AVIF original. AVIF thumbnails
//! use the same bundled FFmpeg decoder as the full-size viewer.

use crate::model::{Attachment, AttachmentKind, AttachmentStatus};
use eframe::egui;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// Inline media frames, as on web.
pub const MAX_WIDTH: f32 = 360.0;
pub const MAX_HEIGHT: f32 = 300.0;
/// Refresh URLs that expire within this many seconds. Links live 1–2 hours,
/// so a wider margin would keep every link permanently "expiring".
pub const REFRESH_MARGIN_SECONDS: i64 = 10 * 60;
/// Do not ask for the same attachment again sooner than this (shorter than
/// the margin, so a failed refresh is retried before the link expires).
const REFRESH_BACKOFF: Duration = Duration::from_secs(5 * 60);
/// The API accepts at most this many ids per refresh.
pub const MAX_REFRESH_IDS: usize = 100;
const MAX_FETCHES: usize = 4;
const MAX_TEXTURES: usize = 200;
/// GIF frames not drawn for this long are released (a tile scrolled out of
/// view and back soon after keeps playing without a new download).
const GIF_IDLE: Duration = Duration::from_secs(10);
/// Retained GIF frame pixels across every tile (each GIF decodes to at most
/// 32 MiB); the least recently drawn give way first.
const GIF_BUDGET: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FreshUrl {
    /// Absent while the file is still processing.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub preview_url: Option<String>,
}

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

/// Locally shown images fit the largest inline frame at 2x.
pub const THUMBNAIL_EDGE: u32 = 720;

type ImageFrames = Vec<(egui::ColorImage, Duration)>;

/// GIFs keep every composited frame and its delay, never the uploaded poster.
/// Shrink playback frames to fit the retained-memory budget without freezing
/// normal GIFs. Decode runs on the existing fetch worker.
fn decode_gif(bytes: &[u8]) -> Option<ImageFrames> {
    use image::{AnimationDecoder, ImageDecoder};
    const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
    let mut decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(32 * 1024 * 1024);
    decoder.set_limits(limits).ok()?;
    let (width, height) = decoder.dimensions();
    let denominator = width.max(height).max(THUMBNAIL_EDGE);
    let mut width = (width * THUMBNAIL_EDGE / denominator).max(1);
    let mut height = (height * THUMBNAIL_EDGE / denominator).max(1);
    let mut frames: Vec<(image::RgbaImage, Duration)> = Vec::new();
    let mut total = 0;
    for frame in decoder.into_frames() {
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
            .map(|(image, delay)| {
                (
                    egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        image.as_raw(),
                    ),
                    delay,
                )
            })
            .collect()
    })
}

/// Decode a still for display only, within memory limits.
pub fn decode_thumbnail(bytes: &[u8]) -> Option<egui::ColorImage> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    if reader.format() == Some(image::ImageFormat::Avif) {
        return crate::ffmpeg::decode_image(bytes, "image/avif", THUMBNAIL_EDGE);
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let mut image = reader.decode().ok()?;
    if image.width() > THUMBNAIL_EDGE || image.height() > THUMBNAIL_EDGE {
        image = image.thumbnail(THUMBNAIL_EDGE, THUMBNAIL_EDGE);
    }
    let image = image.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    ))
}

/// Message-order gallery slots, relative to the gallery's top-left corner.
pub fn gallery_rects(count: usize, width: f32) -> Vec<egui::Rect> {
    let width = width.clamp(1.0, 480.0);
    let gap = 4.0;
    let height = width * 2.0 / 3.0;
    let rect = |x, y, w, h| egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
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

/// Blur a bounded still on the CPU; never retain animation frames in the cover.
pub fn blur_thumbnail(image: &egui::ColorImage) -> egui::ColorImage {
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_srgba_unmultiplied())
        .collect();
    let rgba = image::RgbaImage::from_raw(image.size[0] as u32, image.size[1] as u32, bytes)
        .expect("thumbnail dimensions");
    let small = image::imageops::thumbnail(&rgba, 32, 32);
    let blurred = image::imageops::blur(&small, 8.0);
    egui::ColorImage::from_rgba_unmultiplied(
        [blurred.width() as usize, blurred.height() as usize],
        blurred.as_raw(),
    )
}

const BLUR_KEY: &str = "spoiler-cover:";

/// Display size that reserves layout space before an image loads.
pub fn frame_size(attachment: &Attachment) -> egui::Vec2 {
    match (attachment.width, attachment.height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            let (width, height) = (width as f32, height as f32);
            let scale = (MAX_WIDTH / width).min(MAX_HEIGHT / height).min(1.0);
            egui::vec2(
                (width * scale).round().max(1.0),
                (height * scale).round().max(1.0),
            )
        }
        _ => egui::vec2(240.0, 180.0),
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
        if self
            .requested
            .get(id)
            .is_none_or(|at| now.duration_since(*at) >= REFRESH_BACKOFF)
        {
            self.queue.insert(id.to_owned());
        }
    }

    /// Queue regardless of backoff (a failed load or an explicit open).
    pub fn force(&mut self, id: &str) {
        self.queue.insert(id.to_owned());
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

pub enum ImageState {
    Ready(egui::TextureId),
    Loading,
    Failed,
}

enum Entry {
    Loading {
        url: String,
        gif: bool,
    },
    Ready {
        texture: egui::TextureHandle,
        used: u64,
        animation: Option<Animation>,
    },
    Failed {
        url: Option<String>,
        awaiting_refresh: bool,
    },
}

pub struct Fetched {
    generation: u64,
    id: String,
    result: Result<ImageFrames, Option<u16>>,
}

struct Animation {
    frames: ImageFrames,
    started: Instant,
    shown: usize,
    /// When its tile last drew it.
    seen: Instant,
    bytes: usize,
}

impl Animation {
    /// The frame `elapsed` into the loop, and how long until the next one.
    fn frame_at(&self, elapsed: Duration) -> (usize, Duration) {
        let duration: Duration = self.frames.iter().map(|(_, delay)| *delay).sum();
        let mut time = elapsed.as_nanos() % duration.as_nanos().max(1);
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

/// Per-account attachment display state. Cleared on logout.
pub struct Media {
    fresh: BTreeMap<String, FreshUrl>,
    refresher: Refresher,
    images: HashMap<String, Entry>,
    /// Ids that already used their one refresh-and-retry after a 403/404.
    retried: BTreeSet<String>,
    waiting: VecDeque<(String, String, bool)>,
    in_flight: usize,
    open_after_refresh: Option<(String, String)>,
    sender: Sender<Fetched>,
    receiver: Receiver<Fetched>,
    api: Option<crate::api::Api>,
    generation: u64,
    frame: u64,
    copying: Option<Receiver<Option<egui::ColorImage>>>,
    pub copy_status: Option<&'static str>,
}

impl Media {
    /// `api` is `None` in tests and fixtures, which never fetch.
    pub fn new(api: Option<crate::api::Api>) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            fresh: BTreeMap::new(),
            refresher: Refresher::default(),
            images: HashMap::new(),
            retried: BTreeSet::new(),
            waiting: VecDeque::new(),
            in_flight: 0,
            open_after_refresh: None,
            sender,
            receiver,
            api,
            generation: 0,
            frame: 0,
            copying: None,
            copy_status: None,
        }
    }

    pub fn clear(&mut self) {
        self.generation += 1;
        self.fresh.clear();
        self.refresher = Refresher::default();
        self.images.clear();
        self.retried.clear();
        self.waiting.clear();
        self.in_flight = 0;
        self.open_after_refresh = None;
        self.copying = None;
        self.copy_status = None;
    }

    pub fn copying(&self) -> bool {
        self.copying.is_some()
    }

    /// Decode the original, not the cropped tile or JPEG preview. Clipboard
    /// delivery stays on the UI thread; logout drops any in-flight result.
    pub fn copy_image(&mut self, context: &egui::Context, attachment: &Attachment) {
        let (Some(api), Some(url)) = (self.api.clone(), attachment.url.clone()) else {
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
        resolve(attachment, self.fresh.get(&attachment.id))
    }

    /// Note a rendered attachment; expiring URLs are queued for refresh.
    pub fn visible(&mut self, attachment: &Attachment, unix_now: i64, now: Instant) {
        if !attachment.unavailable
            && [&attachment.url, &attachment.preview_url]
                .into_iter()
                .flatten()
                .any(|url| needs_refresh(url, unix_now))
        {
            self.refresher.want(&attachment.id, now);
        }
    }

    /// Refresh one attachment's URLs as soon as possible (a viewer load failed).
    pub fn refresh_now(&mut self, id: &str) {
        if !self.retried.contains(id) {
            self.retried.insert(id.to_owned());
            self.refresher.force(id);
        }
    }

    /// The client media loads use, for loads outside this cache.
    pub fn api(&self) -> Option<&crate::api::Api> {
        self.api.as_ref()
    }

    pub fn take_refresh(&mut self, now: Instant) -> Vec<String> {
        self.refresher.take(now)
    }

    /// Apply a refresh response. Returns a URL to open if an open waited on it.
    pub fn refreshed(
        &mut self,
        requested: &[String],
        urls: BTreeMap<String, FreshUrl>,
    ) -> Option<String> {
        for (id, fresh) in urls {
            if requested.contains(&id) {
                self.fresh.insert(id, fresh);
            }
        }
        let (id, fallback) = self
            .open_after_refresh
            .take_if(|(id, _)| requested.contains(id))?;
        Some(
            self.fresh
                .get(&id)
                .and_then(|fresh| fresh.url.clone())
                .unwrap_or(fallback),
        )
    }

    /// Open the full file in the system browser or player, refreshing first
    /// when the signed URL is about to expire.
    pub fn open(&mut self, context: &egui::Context, attachment: &Attachment, unix_now: i64) {
        let Some(url) = attachment.url.clone() else {
            return;
        };
        if needs_refresh(&url, unix_now) && self.api.is_some() {
            self.refresher.force(&attachment.id);
            self.open_after_refresh = Some((attachment.id.clone(), url));
        } else {
            context.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    /// Show a locally decoded image for an attachment (own uploads), so the
    /// pending row and the confirmed message need no download.
    pub fn insert_local(&mut self, context: &egui::Context, id: &str, image: egui::ColorImage) {
        self.local_spoiler(context, id, &image);
        // A late local thumbnail must not replace an already loading/playing GIF.
        self.insert_original(context, id, image);
    }

    pub fn local_spoiler(
        &mut self,
        context: &egui::Context,
        id: &str,
        image: &egui::ColorImage,
    ) -> ImageState {
        let cover_id = format!("{BLUR_KEY}{id}");
        if !self.images.contains_key(&cover_id) {
            let texture = context.load_texture(
                &cover_id,
                blur_thumbnail(image),
                egui::TextureOptions::LINEAR,
            );
            self.images.insert(
                cover_id.clone(),
                Entry::Ready {
                    texture,
                    used: self.frame,
                    animation: None,
                },
            );
        }
        self.image_request(&cover_id, None, false)
    }

    fn insert_original(&mut self, context: &egui::Context, id: &str, image: egui::ColorImage) {
        // A late local thumbnail must not replace an already loading/playing GIF.
        if matches!(
            self.images.get(id),
            Some(
                Entry::Loading { gif: true, .. }
                    | Entry::Ready {
                        animation: Some(_),
                        ..
                    }
            )
        ) {
            return;
        }
        let texture = context.load_texture(
            format!("attachment-{id}"),
            image,
            egui::TextureOptions::LINEAR,
        );
        self.images.insert(
            id.to_owned(),
            Entry::Ready {
                texture,
                used: self.frame,
                animation: None,
            },
        );
    }

    /// The image for an attachment id, starting a load from `url` if needed.
    pub fn image(&mut self, id: &str, url: Option<&str>) -> ImageState {
        self.image_request(id, url, false)
    }

    /// A separate static texture, so revealing always uses the untouched original.
    pub fn spoiler(&mut self, attachment: &Attachment) -> ImageState {
        if attachment.unavailable
            || attachment.status == AttachmentStatus::Failed
            || !matches!(
                attachment.kind,
                AttachmentKind::Image | AttachmentKind::Video
            )
        {
            return ImageState::Failed;
        }
        self.image_request(
            &format!("{BLUR_KEY}{}", attachment.id),
            attachment
                .preview_url
                .as_deref()
                .or_else(|| inline_source(attachment)),
            false,
        )
    }

    /// Called only for a visible, ready GIF tile. A local first-frame preview
    /// is replaced once by its original, never on signature refresh alone.
    pub fn gif(
        &mut self,
        context: &egui::Context,
        id: &str,
        url: Option<&str>,
        playing: bool,
        now: Instant,
    ) -> ImageState {
        if matches!(
            self.images.get(id),
            Some(Entry::Ready {
                animation: None,
                ..
            })
        ) {
            self.images.remove(id);
        }
        if let Some(Entry::Ready {
            texture,
            used,
            animation: Some(animation),
        }) = self.images.get_mut(id)
        {
            *used = self.frame;
            animation.seen = now;
            let (index, next) = if playing {
                animation.frame_at(now.saturating_duration_since(animation.started))
            } else {
                (0, Duration::MAX)
            };
            if animation.shown != index {
                texture.set(
                    animation.frames[index].0.clone(),
                    egui::TextureOptions::LINEAR,
                );
                animation.shown = index;
            }
            // Repaint when the next frame is due, not continuously.
            if playing && animation.frames.len() > 1 {
                context.request_repaint_after(next);
            }
            return ImageState::Ready(texture.id());
        }
        self.image_request(id, url, true)
    }

    fn image_request(&mut self, id: &str, url: Option<&str>, gif: bool) -> ImageState {
        match self.images.get_mut(id) {
            Some(Entry::Ready { texture, used, .. }) => {
                *used = self.frame;
                return ImageState::Ready(texture.id());
            }
            Some(Entry::Loading { .. }) => return ImageState::Loading,
            Some(Entry::Failed {
                url: failed,
                awaiting_refresh,
                ..
            }) => {
                // Retry once when the refresh produced a different URL.
                if !(*awaiting_refresh && url.is_some() && url != failed.as_deref()) {
                    return ImageState::Failed;
                }
            }
            None => {}
        }
        // Without a URL (pending uploads, unsigned CDN) only a cached or local
        // image can show; nothing is recorded so a later URL still loads.
        let Some(url) = url else {
            return ImageState::Failed;
        };
        self.images.insert(
            id.to_owned(),
            Entry::Loading {
                url: url.into(),
                gif,
            },
        );
        self.waiting.push_back((id.to_owned(), url.to_owned(), gif));
        ImageState::Loading
    }

    /// Work waiting for the next [`Media::poll`] or refresh request.
    pub fn busy(&self) -> bool {
        !self.waiting.is_empty() || !self.refresher.queue.is_empty()
    }

    /// Apply finished loads, start queued ones, and bound texture memory.
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
        self.poll_at(context, Instant::now());
    }

    fn poll_at(&mut self, context: &egui::Context, now: Instant) {
        self.frame += 1;
        let results: Vec<_> = self.receiver.try_iter().collect();
        for fetched in results {
            if fetched.generation != self.generation {
                continue;
            }
            self.in_flight = self.in_flight.saturating_sub(1);
            let Some(Entry::Loading { url, gif }) = self.images.get(&fetched.id) else {
                continue;
            };
            let url = url.clone();
            let gif = *gif;
            match fetched.result {
                Ok(frames) if !frames.is_empty() => {
                    let texture = context.load_texture(
                        format!("attachment-{}", fetched.id),
                        frames[0].0.clone(),
                        egui::TextureOptions::LINEAR,
                    );
                    self.images.insert(
                        fetched.id,
                        Entry::Ready {
                            texture,
                            used: self.frame,
                            animation: gif.then(|| Animation {
                                bytes: frames.iter().map(|(image, _)| image.pixels.len() * 4).sum(),
                                frames,
                                started: now,
                                shown: 0,
                                seen: now,
                            }),
                        },
                    );
                }
                Ok(_) => {
                    self.images.remove(&fetched.id);
                }
                Err(status) => {
                    let action = on_fetch_failure(status, self.retried.contains(&fetched.id));
                    let retry = action == FailureAction::RefreshThenRetry;
                    if retry {
                        self.retried.insert(fetched.id.clone());
                        self.refresher
                            .force(fetched.id.strip_prefix(BLUR_KEY).unwrap_or(&fetched.id));
                    }
                    self.images.insert(
                        fetched.id,
                        Entry::Failed {
                            url: Some(url),
                            awaiting_refresh: retry,
                        },
                    );
                }
            }
        }
        while self.in_flight < MAX_FETCHES
            && let Some((id, url, gif)) = self.waiting.pop_front()
        {
            let Some(api) = self.api.clone() else {
                // Tests and fixtures: leave the entry loading.
                self.in_flight += 1;
                continue;
            };
            self.in_flight += 1;
            let sender = self.sender.clone();
            let context = context.clone();
            let generation = self.generation;
            std::thread::spawn(move || {
                let result = api.fetch_media(&url).and_then(|bytes| {
                    if gif {
                        decode_gif(&bytes)
                    } else {
                        decode_thumbnail(&bytes).map(|image| {
                            let image = if id.starts_with(BLUR_KEY) {
                                blur_thumbnail(&image)
                            } else {
                                image
                            };
                            vec![(image, Duration::ZERO)]
                        })
                    }
                    .ok_or(None)
                });
                let _ = sender.send(Fetched {
                    generation,
                    id,
                    result,
                });
                context.request_repaint();
            });
        }
        // GIF frames are much larger than one texture. Release animations
        // whose tiles have been out of view for a while, not just at 200
        // images, then the least recently drawn past the budget.
        self.images.retain(|_, entry| {
            !matches!(entry,
                Entry::Ready { animation: Some(animation), .. }
                    if now.saturating_duration_since(animation.seen) >= GIF_IDLE
            )
        });
        let mut animated: Vec<(Instant, u64, usize, String)> = self
            .images
            .iter()
            .filter_map(|(id, entry)| match entry {
                Entry::Ready {
                    animation: Some(animation),
                    used,
                    ..
                } => Some((animation.seen, *used, animation.bytes, id.clone())),
                _ => None,
            })
            .collect();
        let mut bytes: usize = animated.iter().map(|(_, _, bytes, _)| bytes).sum();
        animated.sort();
        for (_, used, size, id) in animated {
            // Tiles drawn last frame stay, so on-screen GIFs never reload.
            if bytes <= GIF_BUDGET || used + 1 >= self.frame {
                break;
            }
            self.images.remove(&id);
            bytes -= size;
        }
        let ready = self
            .images
            .values()
            .filter(|entry| matches!(entry, Entry::Ready { .. }))
            .count();
        if ready > MAX_TEXTURES {
            let mut used: Vec<(u64, String)> = self
                .images
                .iter()
                .filter_map(|(id, entry)| match entry {
                    Entry::Ready { used, .. } => Some((*used, id.clone())),
                    _ => None,
                })
                .collect();
            used.sort();
            for (_, id) in used.into_iter().take(ready - MAX_TEXTURES) {
                self.images.remove(&id);
            }
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
        let mut media = Media::new(Some(crate::api::Api::new(&base).unwrap()));
        media.copy_image(&context, &attachment(Some(&format!("{base}/original"))));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let output = context.run(Default::default(), |ctx| media.poll(ctx));
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
    fn spoiler_covers_are_blurred_static_cached_and_separate_from_originals() {
        let context = egui::Context::default();
        let mut media = Media::new(None);
        let mut image = egui::ColorImage::filled([64, 64], egui::Color32::RED);
        for y in 0..64 {
            for x in 32..64 {
                image[(x, y)] = egui::Color32::BLUE;
            }
        }
        let blurred = blur_thumbnail(&image);
        assert_eq!(blurred.size, [32, 32]);
        assert!(blurred[(15, 16)].r() > 0 && blurred[(15, 16)].b() > 0);
        media.insert_local(&context, "asset1", image);
        let mut photo = attachment(Some(&signed(NOW)));
        photo.spoiler = true;
        let ImageState::Ready(cover) = media.spoiler(&photo) else {
            panic!("local cover")
        };
        let ImageState::Ready(original) = media.image("asset1", None) else {
            panic!("original")
        };
        assert_ne!(cover, original);
        assert!(matches!(media.spoiler(&photo), ImageState::Ready(id) if id == cover));
        photo.unavailable = true;
        assert!(matches!(media.spoiler(&photo), ImageState::Failed));

        let mut remote = Media::new(None);
        photo.unavailable = false;
        photo.content_type = "image/gif".into();
        assert!(matches!(remote.spoiler(&photo), ImageState::Loading));
        assert_eq!(
            remote.waiting.front().unwrap().1,
            photo.preview_url.unwrap()
        );
        assert!(
            !remote.waiting.front().unwrap().2,
            "hidden GIFs decode only a still"
        );
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
    #[ignore = "needs CAPER_FFMPEG with AVIF decoding (the bundled release binary)"]
    fn avif_originals_decode_for_inline_and_spoiler_display() {
        let original = image::RgbImage::from_fn(960, 480, |x, _| {
            if x < 480 {
                image::Rgb([220, 20, 20])
            } else {
                image::Rgb([20, 20, 220])
            }
        });
        let bytes = crate::avif::encode(&original, 85, crate::avif::Primaries::Srgb).unwrap();
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

    #[test]
    fn gif_tiles_replace_local_stills_play_loop_and_honor_reduced_motion() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for (color, delay) in [([255, 0, 0, 255], 80), ([0, 0, 255, 255], 800)] {
                encoder
                    .encode_frame(image::Frame::from_parts(
                        image::RgbaImage::from_pixel(4, 2, image::Rgba(color)),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(delay, 1),
                    ))
                    .unwrap();
            }
        }
        let context = egui::Context::default();
        let mut media = Media::new(None);
        let url = signed(NOW);
        media.insert_local(&context, "asset1", decode_thumbnail(&bytes).unwrap());
        assert!(matches!(
            media.gif(&context, "asset1", Some(&url), true, Instant::now()),
            ImageState::Loading
        ));
        media.poll(&context);
        media
            .sender
            .send(Fetched {
                generation: media.generation,
                id: "asset1".into(),
                result: Ok(decode_gif(&bytes).unwrap()),
            })
            .unwrap();
        media.poll(&context);
        let now = Instant::now();
        let ImageState::Ready(texture) = media.gif(&context, "asset1", Some(&url), true, now)
        else {
            panic!("GIF must load")
        };
        let color = || {
            let delta = context.tex_manager().write().take_delta();
            let update = &delta.set.iter().find(|(id, _)| *id == texture).unwrap().1;
            let egui::ImageData::Color(image) = &update.image;
            assert_eq!(image.size, [4, 2]);
            image.pixels[0]
        };
        assert_eq!(color(), egui::Color32::RED);
        // Re-signing and a late local thumbnail cannot restart or freeze the GIF.
        media.insert_local(&context, "asset1", decode_thumbnail(&bytes).unwrap());
        media.gif(
            &context,
            "asset1",
            Some(&signed(NOW + 100)),
            true,
            now + Duration::from_millis(200),
        );
        assert_eq!(color(), egui::Color32::BLUE);
        media.gif(
            &context,
            "asset1",
            Some(&url),
            true,
            now + Duration::from_millis(920),
        );
        assert_eq!(color(), egui::Color32::RED);
        media.gif(
            &context,
            "asset1",
            Some(&url),
            true,
            now + Duration::from_millis(1100),
        );
        assert_eq!(color(), egui::Color32::BLUE);
        media.gif(
            &context,
            "asset1",
            Some(&url),
            false,
            now + Duration::from_millis(1100),
        );
        assert_eq!(color(), egui::Color32::RED);
    }

    #[test]
    fn gif_tiles_keep_the_whole_animation_when_full_size_frames_exceed_the_budget() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for index in 0..60 {
                let color = match index {
                    0 => [255, 0, 0, 255],
                    59 => [0, 0, 255, 255],
                    _ => [0, 255, 0, 255],
                };
                encoder
                    .encode_frame(image::Frame::from_parts(
                        image::RgbaImage::from_pixel(720, 400, image::Rgba(color)),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(if index == 59 { 800 } else { 100 }, 1),
                    ))
                    .unwrap();
            }
        }
        let frames = decode_gif(&bytes).unwrap();
        assert_eq!(frames.len(), 60, "a normal GIF must not become a still");
        assert!(
            frames
                .iter()
                .map(|(frame, _)| frame.pixels.len() * 4)
                .sum::<usize>()
                <= 32 * 1024 * 1024
        );
        let context = egui::Context::default();
        let mut media = Media::new(None);
        let url = signed(NOW);
        media.gif(&context, "asset1", Some(&url), true, Instant::now());
        media.poll(&context);
        media
            .sender
            .send(Fetched {
                generation: media.generation,
                id: "asset1".into(),
                result: Ok(frames),
            })
            .unwrap();
        media.poll(&context);
        let now = Instant::now();
        for (elapsed, playing, expected) in [
            (0, true, egui::Color32::RED),
            (5910, true, egui::Color32::BLUE),
            (6710, true, egui::Color32::RED),
            (6810, true, egui::Color32::GREEN),
            (6810, false, egui::Color32::RED),
        ] {
            let ImageState::Ready(texture) = media.gif(
                &context,
                "asset1",
                Some(&url),
                playing,
                now + Duration::from_millis(elapsed),
            ) else {
                panic!("GIF must load");
            };
            let delta = context.tex_manager().write().take_delta();
            let egui::ImageData::Color(image) = &delta
                .set
                .iter()
                .find(|(id, _)| *id == texture)
                .unwrap()
                .1
                .image;
            assert_eq!(image.pixels[0], expected, "frame at {elapsed}ms");
        }
    }

    #[test]
    fn gif_frames_outlive_a_short_scroll_away() {
        let context = egui::Context::default();
        let mut media = Media::new(None);
        let url = signed(NOW);
        let start = Instant::now();
        media.gif(&context, "asset1", Some(&url), true, start);
        media.poll_at(&context, start);
        let frames = vec![
            (
                egui::ColorImage::filled([4, 4], egui::Color32::RED),
                Duration::from_millis(100),
            ),
            (
                egui::ColorImage::filled([4, 4], egui::Color32::BLUE),
                Duration::from_millis(100),
            ),
        ];
        media
            .sender
            .send(Fetched {
                generation: media.generation,
                id: "asset1".into(),
                result: Ok(frames),
            })
            .unwrap();
        media.poll_at(&context, start);
        assert!(matches!(
            media.gif(&context, "asset1", Some(&url), true, start),
            ImageState::Ready(_)
        ));
        // Out of view for a few frames and seconds: the frames stay.
        for frame in 1..=5 {
            media.poll_at(&context, start + Duration::from_secs(frame));
        }
        assert!(matches!(
            media.gif(
                &context,
                "asset1",
                Some(&url),
                true,
                start + Duration::from_secs(5)
            ),
            ImageState::Ready(_)
        ));
        assert!(media.waiting.is_empty(), "scrolling back needs no download");
        // Unseen for longer: released.
        media.poll_at(&context, start + Duration::from_secs(5) + GIF_IDLE);
        assert!(!media.images.contains_key("asset1"));
    }

    #[test]
    fn frames_reserve_aspect_ratio_within_web_bounds() {
        assert_eq!(frame_size(&attachment(None)), egui::vec2(360.0, 180.0));
        let mut tall = attachment(None);
        (tall.width, tall.height) = (Some(1000), Some(3000));
        assert_eq!(frame_size(&tall), egui::vec2(100.0, 300.0));
        (tall.width, tall.height) = (Some(20), Some(10));
        assert_eq!(frame_size(&tall), egui::vec2(20.0, 10.0));
        (tall.width, tall.height) = (None, Some(10));
        assert_eq!(frame_size(&tall), egui::vec2(240.0, 180.0));
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
    }

    fn deliver(media: &Media, result: Result<egui::ColorImage, Option<u16>>) {
        media
            .sender
            .send(Fetched {
                generation: media.generation,
                id: "asset1".into(),
                result: result.map(|image| vec![(image, Duration::ZERO)]),
            })
            .unwrap();
    }

    #[test]
    fn forbidden_load_refreshes_retries_once_then_gives_up() {
        let context = egui::Context::default();
        let mut media = Media::new(None);
        let first = signed(NOW);
        assert!(matches!(
            media.image("asset1", Some(&first)),
            ImageState::Loading
        ));
        media.poll(&context);
        deliver(&media, Err(Some(403)));
        media.poll(&context);
        assert!(matches!(
            media.image("asset1", Some(&first)),
            ImageState::Failed
        ));
        let requested = media.take_refresh(Instant::now());
        assert_eq!(requested, ["asset1"]);
        let fresh = FreshUrl {
            url: Some(signed(NOW + 90_000)),
            preview_url: None,
        };
        media.refreshed(
            &requested,
            BTreeMap::from([("asset1".into(), fresh.clone())]),
        );
        let resolved = media.resolve(&attachment(Some(&first)));
        assert_eq!(resolved.url, fresh.url);
        let fresh_url = fresh.url.unwrap();
        assert!(matches!(
            media.image("asset1", Some(&fresh_url)),
            ImageState::Loading
        ));
        media.poll(&context);
        deliver(&media, Err(Some(404)));
        media.poll(&context);
        assert!(matches!(
            media.image("asset1", Some(&fresh_url)),
            ImageState::Failed
        ));
        assert!(
            media.take_refresh(Instant::now()).is_empty(),
            "only one retry"
        );
    }

    #[test]
    fn loaded_images_are_cached_by_id_across_url_changes() {
        let context = egui::Context::default();
        let mut media = Media::new(None);
        media.image("asset1", Some(&signed(NOW)));
        media.poll(&context);
        deliver(
            &media,
            Ok(egui::ColorImage::filled([2, 2], egui::Color32::WHITE)),
        );
        media.poll(&context);
        assert!(matches!(
            media.image("asset1", Some(&signed(NOW + 1))),
            ImageState::Ready(_)
        ));
        // Results from before a logout are ignored.
        media.clear();
        assert!(matches!(
            media.image("asset1", Some(&signed(NOW))),
            ImageState::Loading
        ));
    }

    #[test]
    fn expiring_visible_urls_are_queued_and_opens_wait_for_refresh() {
        let mut media = Media::new(None);
        let now = Instant::now();
        media.visible(&attachment(Some(&signed(NOW + 86_400))), NOW, now);
        assert!(media.take_refresh(now).is_empty());
        media.visible(&attachment(Some(&signed(NOW + 60))), NOW, now);
        assert_eq!(media.take_refresh(now), ["asset1"]);
        // A pending open receives the refreshed URL, or the old one when the
        // server omitted the file.
        media.open_after_refresh = Some(("asset1".into(), signed(NOW + 60)));
        assert_eq!(
            media.refreshed(&["asset1".into()], BTreeMap::new()),
            Some(signed(NOW + 60))
        );
        media.open_after_refresh = Some(("asset1".into(), signed(NOW + 60)));
        let fresh = FreshUrl {
            url: Some(signed(NOW + 90_000)),
            preview_url: None,
        };
        assert_eq!(
            media.refreshed(
                &["asset1".into()],
                BTreeMap::from([("asset1".into(), fresh)])
            ),
            Some(signed(NOW + 90_000))
        );
    }
}
