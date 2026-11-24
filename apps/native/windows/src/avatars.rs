//! Avatars as every client draws them: a person's uploaded picture, else
//! their Caper mascot (`caper_client::avatar_images`), else the caller's
//! initials; a space's uploaded icon, else its initial.
//!
//! One cache on the UI thread serves every panel. Mascots are rasterized
//! once with resvg. Pictures are cached by asset id, never by URL (signed
//! URLs re-sign daily); they are fetched and decoded off the UI thread
//! (`caper_client::pictures::decode`, a square of at most 128 px) and arrive
//! through `WM_LIVE`, after which every panel showing avatars redraws. An
//! animated picture keeps its frames and shows the one due now on a shared
//! clock; message rows redraw on the GIF timer (`next_frame`), so it plays
//! while Caper is watched and holds still with reduced motion. Until a picture is ready, or when
//! it fails, callers see the mascot or initials; a failed URL is not tried
//! again until the server signs a new one.
use caper_client::api::Api;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;
use windows_numerics::Matrix3x2;

/// Mascots are rasterized at this edge, crisp at 64 DIPs on a 2x display.
const MASCOT_EDGE: u32 = 128;
const MAX_FETCHES: usize = 4;

/// What an avatar shows besides its initials.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct Avatar {
    pub mascot: Option<u16>,
    /// The uploaded picture's asset id and its signed URL (absent when the
    /// server signs none: only a cached picture shows).
    pub picture: Option<(String, Option<String>)>,
}

impl Avatar {
    pub fn new(avatar_id: Option<i32>, asset: &Option<String>, url: &Option<String>) -> Self {
        Self {
            mascot: caper_client::caper_avatar_index(avatar_id),
            picture: caper_client::pictures::Source::new(asset, url)
                .map(|s| (s.asset.to_owned(), s.url.map(str::to_owned))),
        }
    }

    /// A space's icon: a picture, never a mascot.
    pub fn icon(space: &caper_client::model::Space) -> Self {
        Self::new(None, &space.icon_asset, &space.icon_url)
    }
}

type Loaded = (String, String, Option<caper_client::pictures::Frames>);

/// An uploaded picture on the GPU: its frames and how long each shows.
struct Picture {
    frames: Vec<(ID2D1Bitmap1, Duration)>,
    total: Duration,
}

impl Picture {
    /// The frame due now and how long until the next one; a still has none.
    fn now(&self, motion: bool) -> (&ID2D1Bitmap1, Option<Duration>) {
        if !motion || self.frames.len() < 2 || self.total.is_zero() {
            return (&self.frames[0].0, None);
        }
        let total = self.total.as_millis().max(1);
        let mut at = (EPOCH.with(|e| e.elapsed()).as_millis() % total) as u64;
        for (bitmap, delay) in &self.frames {
            let d = delay.as_millis() as u64;
            if at < d {
                return (bitmap, Some(Duration::from_millis(d - at)));
            }
            at -= d;
        }
        (&self.frames[0].0, Some(Duration::from_millis(10)))
    }
}

struct Cache {
    api: Option<Api>,
    hwnd: isize,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
    mascots: HashMap<u16, Option<ID2D1Bitmap1>>,
    branding: HashMap<u16, Option<ID2D1Bitmap1>>,
    /// Decoded pictures awaiting their bitmaps, then the bitmaps.
    decoded: HashMap<String, caper_client::pictures::Frames>,
    pictures: HashMap<String, std::rc::Rc<Picture>>,
    /// Windows' animation setting, read once a second at most.
    motion: (bool, Option<Instant>),
    loading: HashSet<String>,
    failed: HashMap<String, String>,
}

impl Default for Cache {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            api: None,
            hwnd: 0,
            tx,
            rx,
            mascots: HashMap::new(),
            branding: HashMap::new(),
            decoded: HashMap::new(),
            pictures: HashMap::new(),
            motion: (true, None),
            loading: HashSet::new(),
            failed: HashMap::new(),
        }
    }
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::default();
    /// Every animated avatar shares one clock, so the same picture plays in step everywhere.
    static EPOCH: Instant = Instant::now();
}

/// Pictures load through `api` and wake `hwnd` when they arrive.
pub fn connect(api: Api, hwnd: HWND) {
    CACHE.with_borrow_mut(|c| {
        c.api = Some(api);
        c.hwnd = hwnd.0 as isize;
    });
}

/// Signed out: pictures belong to the account that saw them.
pub fn clear() {
    CACHE.with_borrow_mut(|c| {
        c.decoded.clear();
        c.pictures.clear();
        c.failed.clear();
    });
}

/// A private profile preview, removed when its editor closes or replaces it.
pub fn local(key: String, image: caper_client::Rgba) {
    CACHE.with_borrow_mut(|c| {
        c.decoded.insert(key, vec![(image, Duration::ZERO)]);
    });
}

pub fn forget_local(key: &str) {
    CACHE.with_borrow_mut(|c| {
        c.decoded.remove(key);
        c.pictures.remove(key);
    });
}

/// Keep the saved draft visible while its server URL is still loading.
pub fn save_local(key: &str, asset: &str) {
    CACHE.with_borrow_mut(|c| {
        if let Some(image) = c.decoded.get(key).cloned() {
            c.decoded.insert(asset.to_owned(), image);
        }
        if let Some(picture) = c.pictures.get(key).cloned() {
            c.pictures.insert(asset.to_owned(), picture);
        }
    });
}

/// Takes finished loads; true when a picture became ready, so panels that
/// show avatars redraw.
pub fn poll() -> bool {
    CACHE.with_borrow_mut(|c| {
        let mut ready = false;
        while let Ok((asset, url, image)) = c.rx.try_recv() {
            c.loading.remove(&asset);
            match image {
                Some(image) => {
                    c.failed.remove(&asset);
                    c.pictures.remove(&asset);
                    c.decoded.insert(asset, image);
                    ready = true;
                }
                None => {
                    c.failed.insert(asset, url);
                }
            }
        }
        ready
    })
}

/// Unmultiplied RGBA to premultiplied BGRA, Direct2D's bitmap format.
fn premultiplied(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len());
    for p in rgba.as_chunks::<4>().0 {
        let a = p[3] as u32;
        let m = |c: u8| ((c as u32 * a + 127) / 255) as u8;
        out.extend_from_slice(&[m(p[2]), m(p[1]), m(p[0]), p[3]]);
    }
    out
}

fn svg_bitmap(dc: &ID2D1DeviceContext, svg: &[u8], edge: u32) -> Option<ID2D1Bitmap1> {
    let tree = resvg::usvg::Tree::from_data(svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(edge, edge)?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(
            edge as f32 / tree.size().width(),
            edge as f32 / tree.size().height(),
        ),
        &mut pixmap.as_mut(),
    );
    // tiny-skia is premultiplied RGBA already: swap to BGRA.
    let mut pixels = pixmap.take();
    for p in pixels.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    crate::gfx::bitmap_on(dc, edge, edge, &pixels).ok()
}

fn mascot(dc: &ID2D1DeviceContext, index: u16) -> Option<ID2D1Bitmap1> {
    let svg = caper_client::avatar_images::SVG.get(usize::from(index))?;
    svg_bitmap(dc, svg, MASCOT_EDGE)
}

/// Draws transparent branding Caper `index` in `r` (DIPs) for empty conversations.
pub fn draw_branding(dc: &ID2D1DeviceContext, index: usize, r: D2D_RECT_F) -> bool {
    let Ok(key) = u16::try_from(index) else {
        return false;
    };
    let bitmap = CACHE.with_borrow_mut(|c| {
        c.branding
            .entry(key)
            .or_insert_with(|| {
                let svg = caper_client::avatar_images::BRANDING.get(index)?;
                svg_bitmap(dc, svg, 176)
            })
            .clone()
    });
    let Some(bitmap) = bitmap else {
        return false;
    };
    unsafe {
        dc.DrawBitmap(
            &bitmap,
            Some(&r),
            1.0,
            D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
            None,
            None,
        );
    }
    true
}

/// The bitmap `avatar` shows now and whether it is an uploaded picture
/// (drawn as a rounded square) rather than a mascot (drawn whole); a
/// picture not loaded yet starts loading. `None`: draw the initials.
pub fn bitmap(dc: &ID2D1DeviceContext, avatar: &Avatar) -> Option<(ID2D1Bitmap1, bool)> {
    frame(dc, avatar).map(|(bitmap, picture, _)| (bitmap, picture))
}

/// When `avatar`'s next frame is due, for an animated picture that is
/// playing; `None` for a still, a mascot or with reduced motion.
pub fn next_frame(dc: &ID2D1DeviceContext, avatar: &Avatar) -> Option<Duration> {
    frame(dc, avatar).and_then(|(_, _, next)| next)
}

fn frame(
    dc: &ID2D1DeviceContext,
    avatar: &Avatar,
) -> Option<(ID2D1Bitmap1, bool, Option<Duration>)> {
    CACHE.with_borrow_mut(|c| {
        if let Some((asset, url)) = &avatar.picture {
            if c.motion
                .1
                .is_none_or(|at| at.elapsed() > Duration::from_secs(1))
            {
                c.motion = (crate::files::motion(), Some(Instant::now()));
            }
            let motion = c.motion.0;
            if let Some(p) = c.pictures.get(asset) {
                let (b, next) = p.now(motion);
                return Some((b.clone(), true, next));
            }
            if let Some(frames) = c.decoded.remove(asset) {
                let bitmaps: Option<Vec<_>> = frames
                    .iter()
                    .map(|(image, delay)| {
                        let (w, h) = (image.size[0] as u32, image.size[1] as u32);
                        crate::gfx::bitmap_on(dc, w, h, &premultiplied(&image.pixels))
                            .ok()
                            .map(|b| (b, *delay))
                    })
                    .collect();
                if let Some(frames) = bitmaps.filter(|f| !f.is_empty()) {
                    let total = frames.iter().map(|(_, d)| *d).sum();
                    let p = std::rc::Rc::new(Picture { frames, total });
                    c.pictures.insert(asset.clone(), p.clone());
                    let (b, next) = p.now(motion);
                    return Some((b.clone(), true, next));
                }
            }
            if let (Some(url), Some(api)) = (url, &c.api)
                && !c.loading.contains(asset)
                && c.failed.get(asset) != Some(url)
                && c.loading.len() < MAX_FETCHES
            {
                c.loading.insert(asset.clone());
                let (api, tx, hwnd) = (api.clone(), c.tx.clone(), c.hwnd);
                let (asset, url) = (asset.clone(), url.clone());
                std::thread::spawn(move || {
                    let frames = api
                        .fetch_media(&url)
                        .ok()
                        .and_then(|bytes| caper_client::pictures::decode(&bytes, true))
                        .filter(|f| f.iter().all(|(i, _)| i.size[0] > 0 && i.size[1] > 0))
                        .filter(|f| !f.is_empty());
                    let _ = tx.send((asset, url, frames));
                    unsafe {
                        let _ = PostMessageW(
                            Some(HWND(hwnd as _)),
                            crate::live::WM_LIVE,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                });
            }
        }
        let index = avatar.mascot?;
        c.mascots
            .entry(index)
            .or_insert_with(|| mascot(dc, index))
            .clone()
            .map(|b| (b, false, None))
    })
}

/// Draws `avatar` in `r` (DIPs): a picture clipped to a rounded square of
/// `radius`, or the mascot whole. False when neither is ready, so the
/// caller draws its initials.
pub fn draw(dc: &ID2D1DeviceContext, avatar: &Avatar, r: D2D_RECT_F, radius: f32) -> bool {
    let Some((bitmap, picture)) = bitmap(dc, avatar) else {
        return false;
    };
    unsafe {
        let size = bitmap.GetPixelSize();
        if size.width == 0 || size.height == 0 {
            return false;
        }
        if !picture {
            dc.DrawBitmap(
                &bitmap,
                Some(&r),
                1.0,
                D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                None,
                None,
            );
            return true;
        }
        let (sx, sy) = (
            (r.right - r.left) / size.width as f32,
            (r.bottom - r.top) / size.height as f32,
        );
        let Ok(brush) = dc.CreateBitmapBrush(
            &bitmap,
            Some(&D2D1_BITMAP_BRUSH_PROPERTIES1 {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
            }),
            Some(&D2D1_BRUSH_PROPERTIES {
                opacity: 1.0,
                transform: Matrix3x2 {
                    M11: sx,
                    M12: 0.0,
                    M21: 0.0,
                    M22: sy,
                    M31: r.left,
                    M32: r.top,
                },
            }),
        ) else {
            return false;
        };
        dc.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: r,
                radiusX: radius,
                radiusY: radius,
            },
            &brush,
        );
    }
    true
}

/// Every person shape the server sends carries the same picture fields.
pub trait Face {
    fn avatar(&self) -> Avatar;
}

macro_rules! face {
    ($($kind:ty),* $(,)?) => {$(
        impl Face for $kind {
            fn avatar(&self) -> Avatar {
                Avatar::new(self.avatar_id, &self.avatar_asset, &self.avatar_url)
            }
        }
    )*};
}

face!(
    caper_client::model::Account,
    caper_client::model::Inviter,
    caper_client::model::DirectPeer,
    caper_client::model::BlockedAccount,
    caper_client::model::VoiceOccupant,
    caper_client::model::Member,
    caper_client::model::Person,
    caper_client::model::Author,
    caper_client::model::Reactor,
);

/// A round avatar centred on (`cx`, `cy`) with radius `r`: the picture or
/// mascot, else `initials` on a circle of `color`.
pub fn circle(
    g: &crate::gfx::Gfx,
    dc: &ID2D1DeviceContext,
    avatar: &Avatar,
    (cx, cy, r): (f32, f32, f32),
    color: u32,
    initials: &str,
) -> windows::core::Result<()> {
    let bounds = crate::gfx::rect(cx - r, cy - r, cx + r, cy + r);
    if draw(dc, avatar, bounds, r * 2.0 * 0.3) {
        return Ok(());
    }
    unsafe {
        dc.FillEllipse(
            &D2D1_ELLIPSE {
                point: windows_numerics::Vector2 { X: cx, Y: cy },
                radiusX: r,
                radiusY: r,
            },
            &g.solid(color)?,
        );
    }
    if initials.is_empty() {
        return Ok(());
    }
    let i = g.layout(&crate::gfx::wide(initials), &g.formats.direct_initials, 1e6)?;
    crate::emoji::apply(&i, initials, 8.5, 1.0)?;
    let m = crate::gfx::metrics(&i);
    crate::text::draw(
        dc,
        &i,
        cx - m.widthIncludingTrailingWhitespace / 2.0,
        cy - m.height / 2.0,
        &g.brushes.text,
    );
    Ok(())
}

impl crate::App {
    /// A picture arrived: every panel that may show it redraws.
    pub fn avatars_changed(&mut self) {
        self.dirty = crate::Dirty::all();
        self.drawn = None;
        self.events |= crate::EV_ROWS;
        self.need = true;
    }
}
