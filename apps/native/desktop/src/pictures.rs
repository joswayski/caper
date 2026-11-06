//! Custom profile pictures and space icons (`avatarAsset`/`avatarUrl` on
//! people, `iconAsset`/`iconUrl` on spaces; the shared pictures contract in
//! docs/media.md): a bounded cache that shows the pictures wherever a mascot
//! or space initial is drawn. Preparing and uploading are `caper_client`'s.
//!
//! Pictures are cached by asset id, never by URL: signed URLs re-sign daily.
//! Fetching and decoding run off the UI thread at display size (at most
//! [`DISPLAY_EDGE`](caper_client::pictures::DISPLAY_EDGE) pixels, cropped square), and an animated GIF keeps its
//! composited frames and advances them on one texture, as inline GIF
//! attachments do (`attachments`). Animations over the frame or memory
//! budget show one still instead of a truncated loop, reduced motion shows
//! the first frame, and animations that are no longer drawn release their
//! frames. Until a picture is ready, or when it fails, callers draw the
//! saved mascot or the space initial.

use crate::api::Api;
use caper_client::pictures::FRAME_BUDGET;
pub use caper_client::pictures::{
    Applied, Changed, Failure, MAX_SOURCE_BYTES, PICKER_EXTENSIONS, PreviousPicture, Source,
    Target, UNAVAILABLE, is_gif, prepare, square_frame,
};
#[cfg(test)]
pub use caper_client::pictures::{INVALID, failure, select};
use eframe::egui;
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// Display frames and how long each shows; a still is one frame.
pub type Frames = Vec<(egui::ColorImage, Duration)>;

/// A core picture's frames as egui images.
pub fn frames(frames: caper_client::pictures::Frames) -> Frames {
    frames
        .into_iter()
        .map(|(image, delay)| (crate::host::color_image(image), delay))
        .collect()
}

/// Decode a picture for display: every composited frame of an animated GIF
/// when `animate` and the budget allow, otherwise one still
/// ([`caper_client::pictures::decode`]).
pub fn decode(bytes: &[u8], animate: bool) -> Option<Frames> {
    caper_client::pictures::decode(bytes, animate).map(frames)
}

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
        let id = egui::Id::unique("caper-pictures");
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
        let mut delta = context.tex_manager().write().take_delta();
        let egui::ImageData::Color(image) = &delta.set[&texture].last().unwrap().image;
        let color = image.pixels[0];
        delta.clear();
        color
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
