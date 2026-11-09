//! The attachment image cache shared by the desktop clients, independent of
//! how each draws textures (egui, Direct2D): a fetch queue, signed-URL
//! refresh scheduling, one refresh-and-retry after a 403/404 load, GIF/WebP
//! animation and memory bounds, keyed by attachment id so refreshed URLs
//! reuse textures. Mirrors `apps/web/src/chat/Attachments.tsx`.
//!
//! Desktop stays open for days, far longer than a message file's signed URL
//! lives (1–2 hours: until the next-but-one full hour), so URLs are refreshed
//! through `POST /api/assets/urls` shortly before they expire and after a
//! 403/404 load. Rendered attachments are rechecked on every frame (see
//! [`Media::visible`]); a signed-in window must repaint well inside the
//! refresh margin even while idle.

use crate::Rgba;
use crate::attachments::{
    EXTERNAL_KEY, FULL_EDGE, FailureAction, FreshUrl, Refresher, decode_animation,
    decode_animation_at, decode_image, decode_thumbnail, inline_source, needs_refresh,
    on_fetch_failure, resolve,
};
use crate::model::{Attachment, AttachmentKind, AttachmentStatus};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

// `media_gateway.rs`, shared with the voice spike where the voice module is
// `crate::media`, names its snapshot by that path.
#[doc(hidden)]
pub use crate::voice_media::Snapshot;

const MAX_FETCHES: usize = 4;
const MAX_TEXTURES: usize = 200;
/// GIF frames not drawn for this long are released (a tile scrolled out of
/// view and back soon after keeps playing without a new download).
const GIF_IDLE: Duration = Duration::from_secs(10);
/// Retained GIF frame pixels across every tile (each GIF decodes to at most
/// 32 MiB); the least recently drawn give way first.
const GIF_BUDGET: usize = 128 * 1024 * 1024;

const BLUR_KEY: &str = "spoiler-cover:";
const FULL_KEY: &str = "full:";
/// A viewer animation's frames together stay within this (a tile's: 32 MiB).
const FULL_ANIMATION_BYTES: usize = 96 * 1024 * 1024;

/// The cache key of an attachment's spoiler cover.
pub fn spoiler_key(id: &str) -> String {
    format!("{BLUR_KEY}{id}")
}

/// The cache key of an attachment's original as the media viewer shows it:
/// up to [`crate::attachments::FULL_EDGE`], apart from its inline tile.
pub fn full_key(id: &str) -> String {
    format!("{FULL_KEY}{id}")
}

/// The attachment id a cache key belongs to.
fn owner(key: &str) -> &str {
    key.strip_prefix(BLUR_KEY)
        .or_else(|| key.strip_prefix(FULL_KEY))
        .unwrap_or(key)
}

/// How a client makes and updates the textures it draws.
pub trait Textures {
    type Texture;
    type Id: Copy + PartialEq + std::fmt::Debug;
    /// Make a texture for cache key `key` (an attachment id, or a spoiler-cover key).
    fn create(&mut self, key: &str, image: &Rgba) -> Self::Texture;
    /// Replace a texture's pixels (the next GIF frame); same size as created.
    fn update(&mut self, texture: &mut Self::Texture, image: &Rgba);
    fn id(texture: &Self::Texture) -> Self::Id;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageState<I> {
    Ready(I),
    Loading,
    Failed,
}

type ImageFrames = Vec<(Rgba, Duration)>;

/// The spoiler cover: blur a bounded still on the CPU; never retain
/// animation frames in the cover.
pub fn blur(image: &Rgba) -> Rgba {
    let rgba = image::RgbaImage::from_raw(
        image.size[0] as u32,
        image.size[1] as u32,
        image.pixels.clone(),
    )
    .expect("thumbnail dimensions");
    let small = image::imageops::thumbnail(&rgba, 32, 32);
    Rgba::from_image(image::imageops::blur(&small, 8.0))
}

enum Entry<T> {
    Loading {
        url: String,
        gif: bool,
    },
    Ready {
        texture: T,
        used: u64,
        animation: Option<Animation>,
    },
    Failed {
        url: Option<String>,
        awaiting_refresh: bool,
    },
}

struct Fetched {
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
pub struct Media<T: Textures> {
    fresh: BTreeMap<String, FreshUrl>,
    refresher: Refresher,
    images: HashMap<String, Entry<T::Texture>>,
    /// Ids that already used their one refresh-and-retry after a 403/404.
    retried: BTreeSet<String>,
    waiting: VecDeque<(String, String, bool)>,
    in_flight: usize,
    open_after_refresh: Option<(String, String)>,
    sender: Sender<Fetched>,
    receiver: Receiver<Fetched>,
    api: Option<crate::api::Api>,
    wake: Arc<dyn Fn() + Send + Sync>,
    generation: u64,
    frame: u64,
}

impl<T: Textures> Media<T> {
    /// `api` is `None` in tests and fixtures, which never fetch. `wake` is
    /// called from fetch threads when a result is ready for [`Media::poll`].
    pub fn new(api: Option<crate::api::Api>, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
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
            wake,
            generation: 0,
            frame: 0,
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
        if !id.starts_with(EXTERNAL_KEY) && !self.retried.contains(id) {
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

    /// The URL to open the full file now in the system browser or player,
    /// or `None` when the signed URL is about to expire and the open waits
    /// for a refresh (then [`Media::refreshed`] returns it).
    pub fn open(&mut self, attachment: &Attachment, unix_now: i64) -> Option<String> {
        let url = attachment.url.clone()?;
        if needs_refresh(&url, unix_now) && self.api.is_some() {
            self.refresher.force(&attachment.id);
            self.open_after_refresh = Some((attachment.id.clone(), url));
            None
        } else {
            Some(url)
        }
    }

    /// Show a locally decoded image for an attachment (own uploads), so the
    /// pending row and the confirmed message need no download.
    pub fn insert_local(&mut self, textures: &mut T, id: &str, image: Rgba) {
        self.local_spoiler(textures, id, &image);
        // A late local thumbnail must not replace an already loading/playing GIF.
        self.insert_original(textures, id, image);
    }

    pub fn local_spoiler(&mut self, textures: &mut T, id: &str, image: &Rgba) -> ImageState<T::Id> {
        let cover_id = spoiler_key(id);
        if !self.images.contains_key(&cover_id) {
            let texture = textures.create(&cover_id, &blur(image));
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

    fn insert_original(&mut self, textures: &mut T, id: &str, image: Rgba) {
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
        let texture = textures.create(id, &image);
        self.images.insert(
            id.to_owned(),
            Entry::Ready {
                texture,
                used: self.frame,
                animation: None,
            },
        );
    }

    /// The ready texture for a cache key (an attachment id, or
    /// [`spoiler_key`]), without marking it used or starting a load.
    pub fn texture(&self, key: &str) -> Option<&T::Texture> {
        match self.images.get(key) {
            Some(Entry::Ready { texture, .. }) => Some(texture),
            _ => None,
        }
    }

    /// The image for an attachment id, starting a load from `url` if needed.
    pub fn image(&mut self, id: &str, url: Option<&str>) -> ImageState<T::Id> {
        self.image_request(id, url, false)
    }

    /// A separate static texture, so revealing always uses the untouched original.
    pub fn spoiler(&mut self, attachment: &Attachment) -> ImageState<T::Id> {
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
            &spoiler_key(&attachment.id),
            attachment
                .preview_url
                .as_deref()
                .or_else(|| inline_source(attachment)),
            false,
        )
    }

    /// The media viewer's copy of `attachment`'s original, loading it from
    /// its URL; a GIF or WebP keeps its animation while `animate`. Returns
    /// its state and, while it plays, when its next frame is due. Draw
    /// [`Media::texture`] of [`full_key`].
    pub fn full(
        &mut self,
        textures: &mut T,
        attachment: &Attachment,
        animate: bool,
        now: Instant,
    ) -> (ImageState<T::Id>, Option<Duration>) {
        let key = full_key(&attachment.id);
        let url = attachment.url.as_deref();
        if animate && matches!(attachment.content_type.as_str(), "image/gif" | "image/webp") {
            self.gif(textures, &key, url, true, now)
        } else {
            (self.image_request(&key, url, false), None)
        }
    }

    /// Forgets a cached image, such as a viewer original no longer shown.
    pub fn release(&mut self, key: &str) {
        self.images.remove(key);
    }

    /// Called only for a visible, ready GIF tile. A local first-frame preview
    /// is replaced once by its original, never on signature refresh alone.
    /// Also returns when the next frame is due, so the caller repaints then
    /// rather than continuously (`None` when paused or a single frame).
    pub fn gif(
        &mut self,
        textures: &mut T,
        id: &str,
        url: Option<&str>,
        playing: bool,
        now: Instant,
    ) -> (ImageState<T::Id>, Option<Duration>) {
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
                textures.update(texture, &animation.frames[index].0);
                animation.shown = index;
            }
            let next = (playing && animation.frames.len() > 1).then_some(next);
            return (ImageState::Ready(T::id(texture)), next);
        }
        (self.image_request(id, url, true), None)
    }

    fn image_request(&mut self, id: &str, url: Option<&str>, gif: bool) -> ImageState<T::Id> {
        match self.images.get_mut(id) {
            Some(Entry::Ready { texture, used, .. }) => {
                *used = self.frame;
                return ImageState::Ready(T::id(texture));
            }
            Some(Entry::Loading { .. }) => return ImageState::Loading,
            // Retry once when the refresh produced a different URL.
            Some(Entry::Failed {
                url: failed,
                awaiting_refresh,
                ..
            }) if !(*awaiting_refresh && url.is_some() && url != failed.as_deref()) => {
                return ImageState::Failed;
            }
            Some(Entry::Failed { .. }) | None => {}
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
        !self.waiting.is_empty() || !self.refresher.is_empty()
    }

    /// Apply finished loads, start queued ones, and bound texture memory.
    pub fn poll(&mut self, textures: &mut T, now: Instant) {
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
                    let texture = textures.create(&fetched.id, &frames[0].0);
                    self.images.insert(
                        fetched.id,
                        Entry::Ready {
                            texture,
                            used: self.frame,
                            animation: gif.then(|| Animation {
                                bytes: frames.iter().map(|(image, _)| image.pixels.len()).sum(),
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
                    let retry = action == FailureAction::RefreshThenRetry
                        && !fetched.id.starts_with(EXTERNAL_KEY);
                    if retry {
                        self.retried.insert(fetched.id.clone());
                        self.refresher.force(owner(&fetched.id));
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
            let wake = self.wake.clone();
            let generation = self.generation;
            std::thread::spawn(move || {
                // GIF and WebP keep every composited frame and its delay,
                // never the uploaded poster; stills decode for display
                // only, within memory limits.
                let result = api.fetch_media(&url).and_then(|bytes| {
                    if id.starts_with(FULL_KEY) {
                        if gif {
                            decode_animation_at(&bytes, FULL_EDGE, FULL_ANIMATION_BYTES)
                        } else {
                            decode_image(&bytes, FULL_EDGE)
                                .map(|image| vec![(image, Duration::ZERO)])
                        }
                    } else if gif {
                        decode_animation(&bytes)
                    } else {
                        decode_thumbnail(&bytes).map(|image| {
                            let image = if id.starts_with(BLUR_KEY) {
                                blur(&image)
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
                wake();
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

    /// Textures that keep their pixels and count creates and updates.
    #[derive(Default)]
    struct Counting {
        next: u64,
        creates: Vec<String>,
        updates: usize,
    }

    struct Texture {
        id: u64,
        image: Rgba,
    }

    impl Textures for Counting {
        type Texture = Texture;
        type Id = u64;
        fn create(&mut self, key: &str, image: &Rgba) -> Texture {
            self.next += 1;
            self.creates.push(key.to_owned());
            Texture {
                id: self.next,
                image: image.clone(),
            }
        }
        fn update(&mut self, texture: &mut Texture, image: &Rgba) {
            assert_eq!(texture.image.size, image.size);
            self.updates += 1;
            texture.image = image.clone();
        }
        fn id(texture: &Texture) -> u64 {
            texture.id
        }
    }

    fn media() -> Media<Counting> {
        Media::new(None, Arc::new(|| {}))
    }

    fn filled(size: [usize; 2], color: [u8; 4]) -> Rgba {
        Rgba {
            size,
            pixels: color.repeat(size[0] * size[1]),
        }
    }

    const RED: [u8; 4] = [255, 0, 0, 255];
    const GREEN: [u8; 4] = [0, 255, 0, 255];
    const BLUE: [u8; 4] = [0, 0, 255, 255];
    const WHITE: [u8; 4] = [255; 4];

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
    fn spoiler_covers_are_blurred_static_cached_and_separate_from_originals() {
        let mut textures = Counting::default();
        let mut media = media();
        let mut image = filled([64, 64], RED);
        for y in 0..64 {
            for x in 32..64 {
                image.pixels[(y * 64 + x) * 4..][..4].copy_from_slice(&BLUE);
            }
        }
        let blurred = blur(&image);
        assert_eq!(blurred.size, [32, 32]);
        let pixel = &blurred.pixels[(16 * 32 + 15) * 4..][..4];
        assert!(pixel[0] > 0 && pixel[2] > 0);
        media.insert_local(&mut textures, "asset1", image);
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
        assert_eq!(textures.creates, [spoiler_key("asset1"), "asset1".into()]);
        assert_eq!(
            media.texture(&spoiler_key("asset1")).unwrap().image,
            blurred
        );
        photo.unavailable = true;
        assert!(matches!(media.spoiler(&photo), ImageState::Failed));

        let mut remote = self::media();
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
    fn gif_and_webp_tiles_replace_local_stills_play_loop_and_honor_reduced_motion() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for (color, delay) in [(RED, 80), (BLUE, 800)] {
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
        // Same red (80ms), blue (800ms) frames, independently encoded as lossless WebP.
        // magick -size 4x2 -delay 8 xc:red -delay 80 xc:blue -loop 0 -define webp:lossless=true animation.webp
        for bytes in [
            bytes.as_slice(),
            include_bytes!("../../../../shared/klipy/animation.webp").as_slice(),
        ] {
            let mut textures = Counting::default();
            let mut media = media();
            let url = signed(NOW);
            media.insert_local(&mut textures, "asset1", decode_thumbnail(bytes).unwrap());
            assert!(matches!(
                media.gif(&mut textures, "asset1", Some(&url), true, Instant::now()),
                (ImageState::Loading, None)
            ));
            media.poll(&mut textures, Instant::now());
            media
                .sender
                .send(Fetched {
                    generation: media.generation,
                    id: "asset1".into(),
                    result: Ok(decode_animation(bytes).unwrap()),
                })
                .unwrap();
            media.poll(&mut textures, Instant::now());
            let now = Instant::now();
            let (ImageState::Ready(texture), Some(next)) =
                media.gif(&mut textures, "asset1", Some(&url), true, now)
            else {
                panic!("GIF must load and play")
            };
            assert!(next <= Duration::from_millis(80));
            let assert_color = |media: &Media<Counting>, expected: [u8; 4]| {
                let shown = media.texture("asset1").unwrap();
                assert_eq!(shown.id, texture);
                assert_eq!(shown.image.size, [4, 2]);
                // WebP's compositing rounds by one channel value in the image decoder.
                for (actual, expected) in shown.image.pixels[..4].iter().zip(expected) {
                    assert!(
                        actual.abs_diff(expected) <= 1,
                        "expected {expected}, got {actual}"
                    );
                }
            };
            assert_color(&media, RED);
            // Re-signing and a late local thumbnail cannot restart or freeze the GIF.
            media.insert_local(&mut textures, "asset1", decode_thumbnail(bytes).unwrap());
            media.gif(
                &mut textures,
                "asset1",
                Some(&signed(NOW + 100)),
                true,
                now + Duration::from_millis(200),
            );
            assert_color(&media, BLUE);
            media.gif(
                &mut textures,
                "asset1",
                Some(&url),
                true,
                now + Duration::from_millis(920),
            );
            assert_color(&media, RED);
            media.gif(
                &mut textures,
                "asset1",
                Some(&url),
                true,
                now + Duration::from_millis(1100),
            );
            assert_color(&media, BLUE);
            let (_, next) = media.gif(
                &mut textures,
                "asset1",
                Some(&url),
                false,
                now + Duration::from_millis(1100),
            );
            assert_eq!(next, None, "a paused GIF schedules no repaint");
            assert_color(&media, RED);
            assert_eq!(textures.updates, 4);
        }
    }

    #[test]
    fn gif_tiles_keep_the_whole_animation_when_full_size_frames_exceed_the_budget() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for index in 0..60 {
                let color = match index {
                    0 => RED,
                    59 => BLUE,
                    _ => GREEN,
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
        let frames = decode_animation(&bytes).unwrap();
        assert_eq!(frames.len(), 60, "a normal GIF must not become a still");
        assert!(
            frames
                .iter()
                .map(|(frame, _)| frame.pixels.len())
                .sum::<usize>()
                <= 32 * 1024 * 1024
        );
        let mut textures = Counting::default();
        let mut media = media();
        let url = signed(NOW);
        media.gif(&mut textures, "asset1", Some(&url), true, Instant::now());
        media.poll(&mut textures, Instant::now());
        media
            .sender
            .send(Fetched {
                generation: media.generation,
                id: "asset1".into(),
                result: Ok(frames),
            })
            .unwrap();
        media.poll(&mut textures, Instant::now());
        let now = Instant::now();
        for (elapsed, playing, expected) in [
            (0, true, RED),
            (5910, true, BLUE),
            (6710, true, RED),
            (6810, true, GREEN),
            (6810, false, RED),
        ] {
            let (ImageState::Ready(texture), _) = media.gif(
                &mut textures,
                "asset1",
                Some(&url),
                playing,
                now + Duration::from_millis(elapsed),
            ) else {
                panic!("GIF must load");
            };
            let shown = media.texture("asset1").unwrap();
            assert_eq!(shown.id, texture);
            assert_eq!(shown.image.pixels[..4], expected, "frame at {elapsed}ms");
        }
    }

    #[test]
    fn gif_frames_outlive_a_short_scroll_away() {
        let mut textures = Counting::default();
        let mut media = media();
        let url = signed(NOW);
        let start = Instant::now();
        media.gif(&mut textures, "asset1", Some(&url), true, start);
        media.poll(&mut textures, start);
        let frames = vec![
            (filled([4, 4], RED), Duration::from_millis(100)),
            (filled([4, 4], BLUE), Duration::from_millis(100)),
        ];
        media
            .sender
            .send(Fetched {
                generation: media.generation,
                id: "asset1".into(),
                result: Ok(frames),
            })
            .unwrap();
        media.poll(&mut textures, start);
        assert!(matches!(
            media.gif(&mut textures, "asset1", Some(&url), true, start),
            (ImageState::Ready(_), Some(_))
        ));
        // Out of view for a few frames and seconds: the frames stay.
        for frame in 1..=5 {
            media.poll(&mut textures, start + Duration::from_secs(frame));
        }
        assert!(matches!(
            media.gif(
                &mut textures,
                "asset1",
                Some(&url),
                true,
                start + Duration::from_secs(5)
            ),
            (ImageState::Ready(_), _)
        ));
        assert!(media.waiting.is_empty(), "scrolling back needs no download");
        // Unseen for longer: released.
        media.poll(&mut textures, start + Duration::from_secs(5) + GIF_IDLE);
        assert!(!media.images.contains_key("asset1"));
        assert!(media.texture("asset1").is_none());
    }

    fn deliver(media: &Media<Counting>, result: Result<Rgba, Option<u16>>) {
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
        let mut textures = Counting::default();
        let mut media = media();
        let first = signed(NOW);
        assert!(matches!(
            media.image("asset1", Some(&first)),
            ImageState::Loading
        ));
        media.poll(&mut textures, Instant::now());
        deliver(&media, Err(Some(403)));
        media.poll(&mut textures, Instant::now());
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
        media.poll(&mut textures, Instant::now());
        deliver(&media, Err(Some(404)));
        media.poll(&mut textures, Instant::now());
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
    fn viewer_originals_refresh_their_attachment_once_and_stay_apart_from_tiles() {
        let mut textures = Counting::default();
        let mut media = media();
        let first = attachment(Some(&signed(NOW)));
        let send = |media: &Media<Counting>, result: Result<Rgba, Option<u16>>| {
            media
                .sender
                .send(Fetched {
                    generation: media.generation,
                    id: full_key("asset1"),
                    result: result.map(|image| vec![(image, Duration::ZERO)]),
                })
                .unwrap();
        };
        let now = Instant::now();
        assert!(matches!(
            media.full(&mut textures, &first, true, now),
            (ImageState::Loading, None)
        ));
        let (key, url, gif) = media.waiting.front().cloned().unwrap();
        assert_eq!(
            (key.as_str(), url.as_str(), gif),
            ("full:asset1", first.url.as_deref().unwrap(), false)
        );
        media.poll(&mut textures, now);
        send(&media, Err(Some(403)));
        media.poll(&mut textures, now);
        // The refresh names the attachment, not the viewer's cache key.
        let requested = media.take_refresh(now);
        assert_eq!(requested, ["asset1"]);
        let fresh = FreshUrl {
            url: Some(signed(NOW + 90_000)),
            preview_url: None,
        };
        media.refreshed(&requested, BTreeMap::from([("asset1".into(), fresh)]));
        let refreshed = media.resolve(&first);
        assert!(matches!(
            media.full(&mut textures, &refreshed, true, now).0,
            ImageState::Loading
        ));
        media.poll(&mut textures, now);
        send(&media, Ok(filled([3000, 2], WHITE)));
        media.poll(&mut textures, now);
        assert!(matches!(
            media.full(&mut textures, &refreshed, true, now).0,
            ImageState::Ready(_)
        ));
        assert_eq!(
            media.texture(&full_key("asset1")).unwrap().image.size,
            [3000, 2]
        );
        assert!(
            media.texture("asset1").is_none(),
            "the tile loads on its own"
        );
        media.release(&full_key("asset1"));
        assert!(media.texture(&full_key("asset1")).is_none());
        // Opened once more after its link went stale for good: one refresh only.
        assert!(matches!(
            media.full(&mut textures, &refreshed, true, now).0,
            ImageState::Loading
        ));
        media.poll(&mut textures, now);
        send(&media, Err(Some(403)));
        media.poll(&mut textures, now);
        assert!(media.take_refresh(now).is_empty());
        assert!(matches!(
            media.full(&mut textures, &refreshed, true, now).0,
            ImageState::Failed
        ));
    }

    #[test]
    fn viewer_animations_keep_their_frames_up_to_the_full_edge() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for color in [RED, BLUE] {
                encoder
                    .encode_frame(image::Frame::from_parts(
                        image::RgbaImage::from_pixel(1600, 10, image::Rgba(color)),
                        0,
                        0,
                        image::Delay::from_numer_denom_ms(100, 1),
                    ))
                    .unwrap();
            }
        }
        let tile = decode_animation(&bytes).unwrap();
        assert_eq!(tile[0].0.size, [720, 4]);
        let full = decode_animation_at(&bytes, FULL_EDGE, FULL_ANIMATION_BYTES).unwrap();
        assert_eq!(full.len(), 2);
        assert_eq!(full[1].0.size, [1600, 10]);
    }

    #[test]
    fn external_media_failures_never_ask_caper_to_re_sign() {
        let mut textures = Counting::default();
        let mut media = media();
        let id = format!("{EXTERNAL_KEY}https://static.klipy.com/a.gif");
        assert!(matches!(
            media.image(&id, Some("https://static.klipy.com/a.gif")),
            ImageState::Loading
        ));
        media.poll(&mut textures, Instant::now());
        media
            .sender
            .send(Fetched {
                generation: media.generation,
                id: id.clone(),
                result: Err(Some(404)),
            })
            .unwrap();
        media.poll(&mut textures, Instant::now());
        assert!(matches!(
            media.image(&id, Some("https://static.klipy.com/a.gif")),
            ImageState::Failed
        ));
        media.refresh_now(&id); // Opening the failed original in the viewer must not sign it either.
        assert!(media.take_refresh(Instant::now()).is_empty());
    }

    #[test]
    fn loaded_images_are_cached_by_id_across_url_changes() {
        let mut textures = Counting::default();
        let mut media = media();
        media.image("asset1", Some(&signed(NOW)));
        media.poll(&mut textures, Instant::now());
        deliver(&media, Ok(filled([2, 2], WHITE)));
        media.poll(&mut textures, Instant::now());
        assert!(matches!(
            media.image("asset1", Some(&signed(NOW + 1))),
            ImageState::Ready(_)
        ));
        assert_eq!(textures.creates, ["asset1"]);
        // Results from before a logout are ignored.
        media.clear();
        assert!(matches!(
            media.image("asset1", Some(&signed(NOW))),
            ImageState::Loading
        ));
    }

    #[test]
    fn expiring_visible_urls_are_queued_and_opens_wait_for_refresh() {
        let mut media = media();
        let now = Instant::now();
        media.visible(&attachment(Some(&signed(NOW + 86_400))), NOW, now);
        assert!(media.take_refresh(now).is_empty());
        media.visible(&attachment(Some(&signed(NOW + 60))), NOW, now);
        assert_eq!(media.take_refresh(now), ["asset1"]);
        // Without an API (tests, fixtures) nothing can refresh: open now.
        assert_eq!(
            media.open(&attachment(Some(&signed(NOW + 60))), NOW),
            Some(signed(NOW + 60))
        );
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
