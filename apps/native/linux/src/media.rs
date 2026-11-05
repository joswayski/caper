//! Message media: attachment tiles, link-preview images and KLIPY GIFs,
//! fetched and decoded off the main loop and cached by key, never by URL.
//!
//! Attachment keys are their ids, so a re-signed URL reuses the texture.
//! Signed URLs refresh through `POST /api/assets/urls` shortly before they
//! expire and once after a 403/404 load (`caper_client::attachments`).
//! Link-preview images (`EMBED_KEY`) and KLIPY media (`EXTERNAL_KEY`) are
//! never signed, so a failed load is final.
//!
//! Pictures wait for their key: once it loads they are filled in place, so
//! a picture arriving never rebuilds its message row.

use crate::host::Signal;
use caper_client::Rgba;
use caper_client::api::Api;
use caper_client::attachments::{
    EMBED_KEY, EXTERNAL_KEY, FailureAction, FreshUrl, Refresher, needs_refresh, on_fetch_failure,
};
use caper_client::model::Attachment;
use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant};

const MAX_FETCHES: usize = 4;
/// The viewer's full-size stills.
const FULL_EDGE: u32 = 4_096;
/// Viewer keys: the attachment id after this prefix.
const FULL_KEY: &str = "full:";

/// A decoded picture: one still, or every frame of an animation with its
/// delay.
pub type Frames = Vec<(gdk::Texture, Duration)>;

enum Entry {
    Loading,
    Ready(Rc<Frames>),
    Failed { url: String, awaiting_refresh: bool },
}

pub struct Media {
    api: Api,
    signals: async_channel::Sender<Signal>,
    entries: RefCell<HashMap<String, Entry>>,
    /// Pictures waiting for their key.
    waiters: RefCell<HashMap<String, Vec<glib::WeakRef<gtk::Picture>>>>,
    fresh: RefCell<BTreeMap<String, FreshUrl>>,
    refresher: RefCell<Refresher>,
    retried: RefCell<BTreeSet<String>>,
    waiting: RefCell<VecDeque<(String, String)>>,
    in_flight: Cell<usize>,
    /// Fences fetches started before a sign-out.
    generation: Cell<u64>,
    /// Opens the full file once its URL was refreshed.
    open_after_refresh: RefCell<Option<(String, String)>>,
    /// Spoiler attachments you revealed.
    pub revealed: RefCell<HashSet<String>>,
}

fn unsigned(key: &str) -> bool {
    let key = refresh_id(key);
    key.starts_with(EMBED_KEY) || key.starts_with(EXTERNAL_KEY)
}

/// The attachment id a key's URL is refreshed by.
fn refresh_id(key: &str) -> &str {
    key.strip_prefix(FULL_KEY).unwrap_or(key)
}

impl Media {
    pub fn new(api: Api, signals: async_channel::Sender<Signal>) -> Self {
        Self {
            api,
            signals,
            entries: RefCell::default(),
            waiters: RefCell::default(),
            fresh: RefCell::default(),
            refresher: RefCell::default(),
            retried: RefCell::default(),
            waiting: RefCell::default(),
            in_flight: Cell::new(0),
            generation: Cell::new(0),
            open_after_refresh: RefCell::default(),
            revealed: RefCell::default(),
        }
    }

    pub fn clear(&self) {
        self.generation.set(self.generation.get() + 1);
        self.entries.borrow_mut().clear();
        self.waiters.borrow_mut().clear();
        self.fresh.borrow_mut().clear();
        *self.refresher.borrow_mut() = Refresher::default();
        self.retried.borrow_mut().clear();
        self.waiting.borrow_mut().clear();
        self.in_flight.set(0);
        self.open_after_refresh.replace(None);
        self.revealed.borrow_mut().clear();
    }

    /// The attachment with the longest-lived URLs known.
    pub fn resolve(&self, attachment: &Attachment) -> Attachment {
        caper_client::attachments::resolve(attachment, self.fresh.borrow().get(&attachment.id))
    }

    /// Fills `picture` with the picture for `key` from `url` now or once it
    /// loads. A full-size viewer picture uses [`Self::full_key`].
    pub fn bind(&self, picture: &gtk::Picture, key: &str, url: &str) {
        if let Some(frames) = self.frames(key, url) {
            show(picture, &frames);
            return;
        }
        let mut waiters = self.waiters.borrow_mut();
        let list = waiters.entry(key.to_owned()).or_default();
        list.retain(|weak| {
            weak.upgrade()
                .is_some_and(|picture| picture.parent().is_some())
        });
        list.push(picture.downgrade());
    }

    pub fn full_key(id: &str) -> String {
        format!("{FULL_KEY}{id}")
    }

    /// The loaded frames for `key`, starting its load when needed.
    fn frames(&self, key: &str, url: &str) -> Option<Rc<Frames>> {
        match self.entries.borrow().get(key) {
            Some(Entry::Ready(frames)) => return Some(frames.clone()),
            Some(Entry::Loading) => return None,
            Some(Entry::Failed {
                url: failed,
                awaiting_refresh,
            }) if *awaiting_refresh || failed == url || unsigned(key) => return None,
            _ => {}
        }
        if !unsigned(key) && needs_refresh(url, chrono::Utc::now().timestamp()) {
            self.want_refresh(refresh_id(key), false);
            return None;
        }
        self.entries
            .borrow_mut()
            .insert(key.to_owned(), Entry::Loading);
        self.waiting
            .borrow_mut()
            .push_back((key.to_owned(), url.to_owned()));
        self.pump();
        None
    }

    fn want_refresh(&self, id: &str, force: bool) {
        let mut refresher = self.refresher.borrow_mut();
        if force {
            refresher.force(id);
        } else {
            refresher.want(id, Instant::now());
        }
        if !refresher.is_empty() {
            let _ = self.signals.try_send(Signal::RefreshMedia);
        }
    }

    fn wake(&self, key: &str, frames: &Rc<Frames>) {
        let waiters = self.waiters.borrow_mut().remove(key).unwrap_or_default();
        for picture in waiters.iter().filter_map(glib::WeakRef::upgrade) {
            if picture.parent().is_some() {
                show(&picture, frames);
            }
        }
    }

    fn pump(&self) {
        while self.in_flight.get() < MAX_FETCHES {
            let Some((key, url)) = self.waiting.borrow_mut().pop_front() else {
                return;
            };
            self.in_flight.set(self.in_flight.get() + 1);
            let (api, signals, generation) = (
                self.api.clone(),
                self.signals.clone(),
                self.generation.get(),
            );
            std::thread::spawn(move || {
                let full = key.starts_with(FULL_KEY);
                let result = api.fetch_media(&url).and_then(|bytes| {
                    if full {
                        caper_client::attachments::decode_image(&bytes, FULL_EDGE)
                            .map(|image| vec![(image, Duration::ZERO)])
                    } else {
                        caper_client::attachments::decode_animation(&bytes).or_else(|| {
                            caper_client::attachments::decode_thumbnail(&bytes)
                                .map(|image| vec![(image, Duration::ZERO)])
                        })
                    }
                    .ok_or(None)
                });
                let _ = signals.try_send(Signal::Media {
                    generation,
                    key,
                    url,
                    result,
                });
            });
        }
    }

    /// A fetch finished: fill the pictures waiting for it, or remember the
    /// failure (asking for a fresh URL once after a 403/404).
    pub fn loaded(
        &self,
        generation: u64,
        key: String,
        url: String,
        result: Result<Vec<(Rgba, Duration)>, Option<u16>>,
    ) {
        if generation != self.generation.get() {
            return;
        }
        self.in_flight.set(self.in_flight.get().saturating_sub(1));
        let frames: Option<Frames> = result.as_ref().ok().and_then(|frames| {
            frames
                .iter()
                .map(|(image, delay)| texture(image).map(|texture| (texture, *delay)))
                .collect()
        });
        match frames {
            Some(frames) if !frames.is_empty() => {
                let frames = Rc::new(frames);
                self.entries
                    .borrow_mut()
                    .insert(key.clone(), Entry::Ready(frames.clone()));
                self.wake(&key, &frames);
            }
            _ => {
                let status = result.err().flatten();
                let retried = self.retried.borrow().contains(&key);
                let refresh = !unsigned(&key)
                    && on_fetch_failure(status, retried) == FailureAction::RefreshThenRetry;
                if refresh {
                    self.retried.borrow_mut().insert(key.clone());
                }
                self.entries.borrow_mut().insert(
                    key.clone(),
                    Entry::Failed {
                        url,
                        awaiting_refresh: refresh,
                    },
                );
                if refresh {
                    self.want_refresh(refresh_id(&key), true);
                }
            }
        }
        self.pump();
    }

    /// Ids whose URLs should be refreshed now (at most the API's batch).
    pub fn take_refresh(&self) -> Vec<String> {
        self.refresher.borrow_mut().take(Instant::now())
    }

    /// Refreshed URLs arrived: keep them, let failed loads try again, and
    /// return a file waiting to open.
    pub fn refreshed(
        &self,
        requested: &[String],
        urls: BTreeMap<String, FreshUrl>,
    ) -> Option<String> {
        {
            let mut fresh = self.fresh.borrow_mut();
            for (id, url) in urls {
                if requested.contains(&id) {
                    fresh.insert(id, url);
                }
            }
        }
        {
            let mut entries = self.entries.borrow_mut();
            for id in requested {
                for key in [id.clone(), Self::full_key(id)] {
                    if matches!(
                        entries.get(&key),
                        Some(Entry::Failed {
                            awaiting_refresh: true,
                            ..
                        })
                    ) {
                        entries.remove(&key);
                    }
                }
            }
        }
        let (id, fallback) = self
            .open_after_refresh
            .borrow_mut()
            .take_if(|(id, _)| requested.contains(id))?;
        Some(
            self.fresh
                .borrow()
                .get(&id)
                .and_then(|fresh| fresh.url.clone())
                .unwrap_or(fallback),
        )
    }

    /// The full file in the browser or the system's player, refreshing first
    /// when its signed URL is about to expire.
    pub fn open(&self, attachment: &Attachment) {
        let attachment = self.resolve(attachment);
        let Some(url) = attachment.url.clone() else {
            return;
        };
        if needs_refresh(&url, chrono::Utc::now().timestamp()) {
            self.open_after_refresh
                .replace(Some((attachment.id.clone(), url)));
            self.want_refresh(&attachment.id, true);
        } else {
            open_uri(&url);
        }
    }
}

/// Opens an http(s) link in the default browser.
pub fn open_uri(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    if let Err(error) =
        gtk::gio::AppInfo::launch_default_for_uri(url, None::<&gtk::gio::AppLaunchContext>)
    {
        eprintln!("Couldn’t open a link: {error}");
    }
}

pub fn texture(image: &Rgba) -> Option<gdk::Texture> {
    let [width, height] = image.size;
    if width == 0 || height == 0 || image.pixels.len() != width * height * 4 {
        return None;
    }
    let bytes = glib::Bytes::from(&image.pixels[..]);
    Some(
        gdk::MemoryTexture::new(
            width as i32,
            height as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            width * 4,
        )
        .into(),
    )
}

/// Shows `frames` in `picture`: a still, or an animation that plays while
/// the picture is on screen (tick callbacks stop while it is unmapped) and
/// stays on its first frame when animations are off.
pub fn show(picture: &gtk::Picture, frames: &Rc<Frames>) {
    let Some((first, _)) = frames.first() else {
        return;
    };
    picture.set_paintable(Some(first));
    picture.add_css_class("loaded");
    let animate = frames.len() > 1
        && gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations());
    if !animate {
        return;
    }
    let total: Duration = frames.iter().map(|(_, delay)| *delay).sum();
    let total = total.as_micros().max(1) as i64;
    let frames = frames.clone();
    let shown = Cell::new(0usize);
    let start = Cell::new(None::<i64>);
    // Tiles are built per row, so each picture runs one callback.
    picture.add_tick_callback(move |picture, clock| {
        let now = clock.frame_time();
        let first = *start.get().get_or_insert(now);
        start.set(Some(first));
        let mut time = (now - first) % total;
        let mut index = 0;
        for (position, (_, delay)) in frames.iter().enumerate() {
            let length = delay.as_micros() as i64;
            if time < length {
                index = position;
                break;
            }
            time -= length;
        }
        if shown.replace(index) != index {
            picture.set_paintable(Some(&frames[index].0));
        }
        glib::ControlFlow::Continue
    });
}
