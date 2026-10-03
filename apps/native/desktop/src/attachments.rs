//! Showing message attachments: signed-URL freshness, refresh scheduling and
//! an image cache keyed by attachment id, so refreshed URLs reuse textures.
//! Mirrors `apps/web/src/chat/Attachments.tsx`. Desktop stays open for days,
//! longer than a signed URL lives (24–48 hours), so URLs are refreshed through
//! `POST /api/assets/urls` before they expire and after a 403/404 load.

use crate::model::Attachment;
use eframe::egui;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// Inline media frames, as on web.
pub const MAX_WIDTH: f32 = 360.0;
pub const MAX_HEIGHT: f32 = 300.0;
/// Refresh URLs that expire within this many seconds.
pub const REFRESH_MARGIN_SECONDS: i64 = 60 * 60;
/// Do not ask for the same attachment again sooner than this.
const REFRESH_BACKOFF: Duration = Duration::from_secs(5 * 60);
/// The API accepts at most this many ids per refresh.
pub const MAX_REFRESH_IDS: usize = 100;
const MAX_FETCHES: usize = 4;
const MAX_TEXTURES: usize = 200;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FreshUrl {
    pub url: String,
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
/// the ones a newer history page or gateway frame carried.
pub fn resolve(attachment: &Attachment, fresh: Option<&FreshUrl>) -> Attachment {
    let mut resolved = attachment.clone();
    let Some(fresh) = fresh.filter(|_| !attachment.unavailable) else {
        return resolved;
    };
    let current = attachment
        .url
        .as_deref()
        .map_or(Some(i64::MIN), url_expiry)
        .unwrap_or(i64::MIN);
    if attachment.url.is_none() || url_expiry(&fresh.url).unwrap_or(i64::MIN) > current {
        resolved.url = Some(fresh.url.clone());
        if fresh.preview_url.is_some() {
            resolved.preview_url.clone_from(&fresh.preview_url);
        }
    }
    resolved
}

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
    },
    Ready {
        texture: egui::TextureHandle,
        used: u64,
    },
    Failed {
        url: Option<String>,
        awaiting_refresh: bool,
    },
}

pub struct Fetched {
    generation: u64,
    id: String,
    result: Result<egui::ColorImage, Option<u16>>,
}

/// Per-account attachment display state. Cleared on logout.
pub struct Media {
    fresh: BTreeMap<String, FreshUrl>,
    refresher: Refresher,
    images: HashMap<String, Entry>,
    /// Ids that already used their one refresh-and-retry after a 403/404.
    retried: BTreeSet<String>,
    waiting: VecDeque<(String, String)>,
    in_flight: usize,
    open_after_refresh: Option<(String, String)>,
    sender: Sender<Fetched>,
    receiver: Receiver<Fetched>,
    api: Option<crate::api::Api>,
    generation: u64,
    frame: u64,
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
                .map_or(fallback, |fresh| fresh.url.clone()),
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
            },
        );
    }

    /// The image for an attachment id, starting a load from `url` if needed.
    pub fn image(&mut self, id: &str, url: Option<&str>) -> ImageState {
        match self.images.get_mut(id) {
            Some(Entry::Ready { texture, used }) => {
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
        self.images
            .insert(id.to_owned(), Entry::Loading { url: url.into() });
        self.waiting.push_back((id.to_owned(), url.to_owned()));
        ImageState::Loading
    }

    /// Work waiting for the next [`Media::poll`] or refresh request.
    pub fn busy(&self) -> bool {
        !self.waiting.is_empty() || !self.refresher.queue.is_empty()
    }

    /// Apply finished loads, start queued ones, and bound texture memory.
    pub fn poll(&mut self, context: &egui::Context) {
        self.frame += 1;
        let results: Vec<_> = self.receiver.try_iter().collect();
        for fetched in results {
            if fetched.generation != self.generation {
                continue;
            }
            self.in_flight = self.in_flight.saturating_sub(1);
            let Some(Entry::Loading { url }) = self.images.get(&fetched.id) else {
                continue;
            };
            let url = url.clone();
            match fetched.result {
                Ok(image) => self.insert_local(context, &fetched.id, image),
                Err(status) => {
                    let action = on_fetch_failure(status, self.retried.contains(&fetched.id));
                    let retry = action == FailureAction::RefreshThenRetry;
                    if retry {
                        self.retried.insert(fetched.id.clone());
                        self.refresher.force(&fetched.id);
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
            && let Some((id, url)) = self.waiting.pop_front()
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
                    let image = crate::compress::thumbnail(&bytes).ok_or(None)?;
                    Ok(egui::ColorImage::from_rgba_unmultiplied(
                        [image.width() as usize, image.height() as usize],
                        image.as_raw(),
                    ))
                });
                let _ = sender.send(Fetched {
                    generation,
                    id,
                    result,
                });
                context.request_repaint();
            });
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
    use crate::model::AttachmentKind;

    fn attachment(url: Option<&str>) -> Attachment {
        Attachment {
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
    fn refresh_when_expired_or_within_an_hour() {
        assert!(needs_refresh(&signed(NOW - 1), NOW));
        assert!(needs_refresh(&signed(NOW + 3_600), NOW));
        assert!(!needs_refresh(&signed(NOW + 3_601), NOW));
        assert!(!needs_refresh(&signed(NOW + 86_400), NOW));
        assert!(!needs_refresh("https://cdn.caper.chat/original/a", NOW));
    }

    #[test]
    fn resolve_prefers_the_longer_lived_urls() {
        let fresh = FreshUrl {
            url: signed(NOW + 90_000),
            preview_url: Some("https://cdn.caper.chat/preview/asset1?exp=1&sig=p".into()),
        };
        let old = attachment(Some(&signed(NOW)));
        let resolved = resolve(&old, Some(&fresh));
        assert_eq!(resolved.url, Some(fresh.url.clone()));
        assert_eq!(resolved.preview_url, fresh.preview_url);
        // A newer history page beats an older refresh.
        let newer = attachment(Some(&signed(NOW + 200_000)));
        assert_eq!(resolve(&newer, Some(&fresh)).url, newer.url);
        // A message without URLs gains them; removed files never do.
        assert_eq!(
            resolve(&attachment(None), Some(&fresh)).url,
            Some(fresh.url.clone())
        );
        let mut removed = attachment(None);
        removed.unavailable = true;
        assert_eq!(resolve(&removed, Some(&fresh)).url, None);
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
                result,
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
            url: signed(NOW + 90_000),
            preview_url: None,
        };
        media.refreshed(
            &requested,
            BTreeMap::from([("asset1".into(), fresh.clone())]),
        );
        let resolved = media.resolve(&attachment(Some(&first)));
        assert_eq!(resolved.url.as_deref(), Some(fresh.url.as_str()));
        assert!(matches!(
            media.image("asset1", Some(&fresh.url)),
            ImageState::Loading
        ));
        media.poll(&context);
        deliver(&media, Err(Some(404)));
        media.poll(&context);
        assert!(matches!(
            media.image("asset1", Some(&fresh.url)),
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
            url: signed(NOW + 90_000),
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
