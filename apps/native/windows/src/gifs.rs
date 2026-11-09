//! The KLIPY GIF and sticker picker, as in the egui client (`klipy.rs`):
//! GIFs and Stickers tabs, a "Search KLIPY" field that searches once typing
//! pauses (trending while it is empty), and a three-column grid that loads
//! the next page as it nears its end. Requests, parsing and sharing are
//! `caper_client::klipy`, and tiles load through the shared media cache. A
//! choice is sent at once as its own message, answering the composer's
//! reply. Ads are never requested; an ad item that arrives is skipped.
use crate::composer::{Composer, LINE};
use crate::files::D2d;
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::overlay::Rect;
use crate::theme::*;
use crate::{App, EV_FOCUS, EV_SCREEN, EV_TEXT};
use caper_client::klipy::{self, Config, Gif, Item, Kind, Page, Share, Tile};
use caper_client::media::{ImageState, Media};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

/// Searches wait this long after typing stops.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// egui's 352 × 430 window inside a 6-DIP margin, like the reaction picker.
pub const W: f32 = 364.0;
pub const H: f32 = 442.0;
const M: f32 = 6.0;
const INNER_W: f32 = W - 2.0 * M;
const TAB_Y: f32 = M + 30.0 + 6.0;
const TAB_H: f32 = 36.0;
const SEARCH_Y: f32 = TAB_Y + TAB_H + 10.0;
const SEARCH_H: f32 = 26.0;
const GRID_Y: f32 = SEARCH_Y + SEARCH_H + 8.0;
const CAPTION_H: f32 = 22.0;
const GRID_BOTTOM: f32 = H - M - CAPTION_H;
const COLUMNS: usize = 3;
const GAP: f32 = 4.0;
/// egui's cell: the width of three columns and two gaps, floored.
const CELL: f32 = 114.0;
/// Below the tiles: the next page loading, or why it failed with Retry.
const FOOT_H: f32 = 60.0;
const RETRY_W: f32 = 80.0;
const RETRY_H: f32 = 28.0;
const KINDS: [(Kind, &str); 2] = [(Kind::Gif, "GIFs"), (Kind::Sticker, "Stickers")];
const UNAVAILABLE: &str = "GIFs are unavailable right now.";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum GPart {
    Close,
    Tab(usize),
    Search,
    /// A shown GIF, by its index in the results.
    Tile(usize),
    Retry,
}

struct Loaded {
    request: u64,
    page: u32,
    result: std::result::Result<Page, String>,
}

/// The open picker: its tab, search, results and paging.
pub struct Picker {
    /// The thread whose reply box opened it; None is the channel's composer.
    pub root: Option<String>,
    pub x: f32,
    pub y: f32,
    pub kind: Kind,
    pub search: Composer,
    /// When the search last changed and has not been requested yet.
    edited: Option<Instant>,
    /// The tab and trimmed query the shown results answer.
    shown: (Kind, String),
    items: Vec<(Tile, Gif)>,
    page: u32,
    has_next: bool,
    loading: bool,
    error: Option<String>,
    request: u64,
    pub scroll: f32,
    pub hover: Option<GPart>,
    sender: Sender<Loaded>,
    receiver: Receiver<Loaded>,
}

/// One image to try: its URL and whether to decode animation frames.
type Source<'a> = (&'a str, bool);

/// Animated first, else the still; reduced motion prefers the still, then
/// the GIF's first frame (egui's `tile_sources`).
fn tile_sources(tile: &Tile, playing: bool) -> Vec<Source<'_>> {
    let gif = tile.gif.as_deref().map(|url| (url, true));
    let webp = (tile.gif.as_deref() != Some(&tile.url)).then_some((tile.url.as_str(), true));
    let still = tile.still.as_deref().map(|url| (url, false));
    if playing {
        [gif, webp, still]
    } else {
        [still, gif, webp]
    }
    .into_iter()
    .flatten()
    .collect()
}

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

impl Picker {
    /// Opens ending at its GIF button (centre `anchor`), like egui's, kept
    /// inside the window.
    pub fn new(anchor: (f32, f32), window: (f32, f32)) -> Self {
        let mut search = Composer {
            focused: true,
            caret_on: true,
            ..Default::default()
        };
        search.width = INNER_W - 12.0;
        let (sender, receiver) = channel();
        Self {
            root: None,
            x: (anchor.0 + 14.0 - W).min(window.0 - W).max(0.0),
            y: (anchor.1 - 18.0 - H).min(window.1 - H).max(0.0),
            kind: Kind::Gif,
            search,
            edited: None,
            shown: (Kind::Gif, String::new()),
            items: Vec::new(),
            page: 0,
            has_next: false,
            loading: false,
            error: None,
            request: 0,
            scroll: 0.0,
            hover: None,
            sender,
            receiver,
        }
    }

    fn query(&self) -> String {
        String::from_utf16_lossy(&self.search.text)
            .trim()
            .to_owned()
    }

    /// Requests a page of the current tab and query (page 1 replaces the
    /// results). The answer wakes `hwnd`.
    fn load(&mut self, config: Option<&Config>, page: u32, hwnd: HWND) {
        let query = self.query();
        if page <= 1 {
            self.items.clear();
            self.has_next = false;
            self.scroll = 0.0;
        }
        self.request += 1;
        self.shown = (self.kind, query.clone());
        self.edited = None;
        self.loading = true;
        self.error = None;
        let Some(config) = config.cloned() else {
            self.loading = false;
            self.error = Some(UNAVAILABLE.into());
            return;
        };
        let (request, kind, sender) = (self.request, self.kind, self.sender.clone());
        let raw = hwnd.0 as isize;
        std::thread::spawn(move || {
            let result = klipy::fetch(&config, kind, &query, page);
            let _ = sender.send(Loaded {
                request,
                page,
                result,
            });
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(raw as _)),
                    crate::live::WM_LIVE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        });
    }

    fn apply(&mut self, loaded: Loaded) {
        if loaded.request != self.request {
            return;
        }
        self.loading = false;
        match loaded.result {
            Ok(page) => {
                self.page = loaded.page;
                self.has_next = page.has_next;
                self.items
                    .extend(page.items.into_iter().filter_map(|item| match item {
                        Item::Media { tile, gif, .. } => Some((tile, *gif)),
                        Item::Ad { .. } => None,
                    }));
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Applies finished loads (dropping superseded ones), searches once
    /// typing has paused and loads the next page near the grid's end.
    /// Returns whether anything changed and how long until the search is due.
    fn poll(
        &mut self,
        config: Option<&Config>,
        now: Instant,
        hwnd: HWND,
    ) -> (bool, Option<Duration>) {
        let mut changed = false;
        if self.request == 0 {
            self.load(config, 1, hwnd);
            changed = true;
        }
        for loaded in self.receiver.try_iter().collect::<Vec<_>>() {
            self.apply(loaded);
            changed = true;
        }
        let mut wait = None;
        if let Some(edited) = self.edited {
            let waited = now.saturating_duration_since(edited);
            if waited >= DEBOUNCE {
                self.load(config, 1, hwnd);
                changed = true;
            } else {
                wait = Some(DEBOUNCE - waited);
            }
        }
        changed |= self.more(config, hwnd);
        (changed, wait)
    }

    /// Loads the next page when the grid nears its end.
    fn more(&mut self, config: Option<&Config>, hwnd: HWND) -> bool {
        let remaining = self.content_h() - (self.scroll + self.grid_h());
        if remaining < CELL * 2.0 && self.has_next && !self.loading && self.error.is_none() {
            self.load(config, self.page + 1, hwnd);
            return true;
        }
        false
    }

    /// The search text changed: a search waits for a pause, unless it
    /// already shows these results.
    fn search_edited(&mut self, now: Instant) -> bool {
        self.edited = (self.query() != self.shown.1 || self.kind != self.shown.0).then_some(now);
        self.edited.is_some()
    }

    pub fn rect(&self) -> Rect {
        (self.x, self.y, W, H)
    }

    fn grid_h(&self) -> f32 {
        GRID_BOTTOM - GRID_Y
    }

    fn rows_h(&self) -> f32 {
        let rows = self.items.len().div_ceil(COLUMNS);
        (rows as f32 * (CELL + GAP) - GAP).max(0.0)
    }

    /// Whether the line under the tiles shows (more loading or an error).
    fn foot(&self) -> bool {
        !self.items.is_empty() && (self.loading || self.error.is_some())
    }

    fn content_h(&self) -> f32 {
        self.rows_h() + if self.foot() { FOOT_H } else { 0.0 }
    }

    fn max_scroll(&self) -> f32 {
        (self.content_h() - self.grid_h()).max(0.0)
    }

    /// The tiles at least partly visible.
    fn visible(&self) -> std::ops::Range<usize> {
        let pitch = CELL + GAP;
        let first = (self.scroll / pitch).floor() as usize * COLUMNS;
        let last = (((self.scroll + self.grid_h()) / pitch).ceil() as usize * COLUMNS)
            .min(self.items.len());
        first.min(last)..last
    }

    /// The status line's top (picker-local): under the tiles, or in the
    /// empty grid.
    fn status_y(&self) -> f32 {
        if self.items.is_empty() {
            GRID_Y + 24.0
        } else {
            GRID_Y + self.rows_h() + 6.0 - self.scroll
        }
    }

    /// A part's rectangle relative to the picker's top-left.
    fn local(&self, part: GPart) -> Rect {
        match part {
            GPart::Close => (M + INNER_W - 28.0, M + 1.0, 28.0, 28.0),
            GPart::Tab(k) => (M + INNER_W / 2.0 * k as f32, TAB_Y, INNER_W / 2.0, TAB_H),
            GPart::Search => (M, SEARCH_Y, INNER_W, SEARCH_H),
            GPart::Tile(i) => (
                M + (CELL + GAP) * (i % COLUMNS) as f32,
                GRID_Y + (CELL + GAP) * (i / COLUMNS) as f32 - self.scroll,
                CELL,
                CELL,
            ),
            GPart::Retry => (
                (W - RETRY_W) / 2.0,
                self.status_y() + 22.0,
                RETRY_W,
                RETRY_H,
            ),
        }
    }

    pub fn part_rect(&self, part: GPart) -> Rect {
        let (x, y, w, h) = self.local(part);
        (self.x + x, self.y + y, w, h)
    }

    /// Whether a part is on screen (a tile or Retry inside the grid).
    fn shown_part(&self, part: GPart) -> bool {
        let (_, y, _, h) = self.local(part);
        match part {
            GPart::Tile(_) | GPart::Retry => y + h > GRID_Y && y < GRID_BOTTOM,
            _ => true,
        }
    }

    pub fn parts(&self) -> Vec<GPart> {
        let mut out = vec![GPart::Close, GPart::Tab(0), GPart::Tab(1), GPart::Search];
        out.extend(self.visible().map(GPart::Tile));
        if self.error.is_some() && self.shown_part(GPart::Retry) {
            out.push(GPart::Retry);
        }
        out
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<GPart> {
        let (lx, ly) = (x - self.x, y - self.y);
        self.parts().into_iter().find(|p| {
            let in_grid = matches!(p, GPart::Tile(_) | GPart::Retry);
            contains(self.local(*p), lx, ly) && (!in_grid || (GRID_Y..GRID_BOTTOM).contains(&ly))
        })
    }

    /// Client DIP origin of the search text.
    pub fn text_origin(&self) -> (f32, f32) {
        (
            self.x + M + 6.0,
            self.y + SEARCH_Y + (SEARCH_H - LINE) / 2.0,
        )
    }

    pub fn name(&self, part: GPart) -> String {
        match part {
            GPart::Close => "Close GIF picker".into(),
            GPart::Tab(k) => KINDS[k].1.into(),
            GPart::Search => "Search KLIPY".into(),
            GPart::Tile(i) => self
                .items
                .get(i)
                .map_or_else(String::new, |(_, gif)| gif.alt().to_owned()),
            GPart::Retry => "Retry".into(),
        }
    }

    pub fn selected_tab(&self) -> usize {
        KINDS.iter().position(|(k, _)| *k == self.kind).unwrap_or(0)
    }

    /// What the grid says while it has no tiles: the error, loading, or
    /// nothing found.
    fn empty_text(&self) -> (&str, bool) {
        match &self.error {
            Some(e) => (e, true),
            None if self.loading || self.edited.is_some() => ("Loading\u{2026}", false),
            None => ("No results", false),
        }
    }

    /// GIF `i` to send, and what to tell KLIPY once it is sent.
    fn choose(&self, i: usize, config: &Config) -> Option<(Gif, Share)> {
        let (_, gif) = self.items.get(i)?;
        Some((
            gif.clone(),
            Share {
                config: config.clone(),
                kind: gif.kind,
                slug: gif.slug.clone(),
                query: self.shown.1.clone(),
            },
        ))
    }

    /// Draws the picker. Returns when the next GIF frame is due.
    fn draw(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        mut media: Option<&mut Media<D2d>>,
        tex: &mut D2d,
        playing: bool,
        now: Instant,
    ) -> Result<Option<Duration>> {
        let mut next: Option<Duration> = None;
        unsafe {
            dc.Clear(Some(&D2D1_COLOR_F::default()));
            let body = rect(0.0, 0.0, W, H);
            fill_round(dc, body, RADIUS, &g.brushes.surface);
            stroke_round(dc, body, RADIUS, &g.brushes.border);
            let title = g.layout(&gfx::wide("GIFs and stickers"), &g.formats.section, 1e6)?;
            let th = gfx::metrics(&title).height;
            crate::text::draw(dc, &title, M, M + (30.0 - th) / 2.0, &g.brushes.text);
            let (cx, cy, cw, ch) = self.local(GPart::Close);
            let close_hover = self.hover == Some(GPart::Close);
            if close_hover {
                fill_round(dc, rect(cx, cy, cx + cw, cy + ch), 6.0, &g.brushes.hover);
            }
            icons::draw(
                dc,
                Icon::Close,
                rect(cx + 5.0, cy + 5.0, cx + cw - 5.0, cy + ch - 5.0),
                if close_hover { TEXT } else { TEXT_MUTED },
                1.0,
            );
            for (k, (kind, name)) in KINDS.iter().enumerate() {
                let (x, y, w, h) = self.local(GPart::Tab(k));
                if self.hover == Some(GPart::Tab(k)) {
                    fill_round(
                        dc,
                        rect(x + 2.0, y + 2.0, x + w - 2.0, y + h - 2.0),
                        8.0,
                        &g.brushes.hover,
                    );
                }
                let selected = *kind == self.kind;
                let l = g.layout(&gfx::wide(name), &g.formats.gif_tab, 1e6)?;
                let m = gfx::metrics(&l);
                crate::text::draw(
                    dc,
                    &l,
                    x + (w - m.widthIncludingTrailingWhitespace) / 2.0,
                    y + (h - m.height) / 2.0,
                    if selected {
                        &g.brushes.text
                    } else {
                        &g.brushes.muted
                    },
                );
                if selected {
                    dc.FillRectangle(
                        &rect(x + 2.0, y + h - 1.0, x + w - 2.0, y + h + 1.0),
                        &g.brushes.terracotta,
                    );
                }
            }
            let (sx, sy, sw, sh) = self.local(GPart::Search);
            let field = rect(sx, sy, sx + sw, sy + sh);
            fill_round(dc, field, 7.0, &g.brushes.composer);
            let stroke = if self.search.focused {
                g.brushes.terracotta.clone()
            } else if self.hover == Some(GPart::Search) {
                g.brushes.terracotta_bright.clone()
            } else {
                g.brushes.border.clone()
            };
            stroke_round(dc, field, 7.0, &stroke);
            dc.PushAxisAlignedClip(
                &rect(sx + 2.0, sy + 1.0, sx + sw - 2.0, sy + sh - 1.0),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            // KLIPY requires exactly this placeholder.
            let hint = g.layout(&gfx::wide("Search KLIPY"), &g.formats.composer, 1e6)?;
            let (ox, oy) = (sx + 6.0, sy + (sh - LINE) / 2.0);
            self.search
                .draw_text(g, dc, ox, oy, Some((&hint, &g.brushes.hint)));
            dc.PopAxisAlignedClip();
            dc.PushAxisAlignedClip(
                &rect(0.0, GRID_Y, W, GRID_BOTTOM),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
            for i in self.visible() {
                let (x, y, w, h) = self.local(GPart::Tile(i));
                let r = rect(x, y, x + w, y + h);
                fill_round(dc, r, 8.0, &g.brushes.raised);
                if let Some(media) = media.as_deref_mut() {
                    let due = paint(
                        dc,
                        media,
                        tex,
                        r,
                        &tile_sources(&self.items[i].0, playing),
                        playing,
                        now,
                    )?;
                    next = match (next, due) {
                        (Some(a), Some(b)) => Some(a.min(b)),
                        (a, b) => a.or(b),
                    };
                }
                if self.hover == Some(GPart::Tile(i)) {
                    stroke_round(
                        dc,
                        rect(x + 0.5, y + 0.5, x + w - 0.5, y + h - 0.5),
                        8.0,
                        &g.brushes.terracotta,
                    );
                }
            }
            let status = if self.items.is_empty() {
                Some(self.empty_text())
            } else if let Some(e) = &self.error {
                Some((e.as_str(), true))
            } else if self.loading {
                Some(("Loading\u{2026}", false))
            } else {
                None
            };
            if let Some((text, error)) = status {
                let l = g.layout(&gfx::wide(text), &g.formats.picker, INNER_W)?;
                let m = gfx::metrics(&l);
                crate::text::draw(
                    dc,
                    &l,
                    (W - m.widthIncludingTrailingWhitespace) / 2.0,
                    self.status_y(),
                    if error {
                        &g.brushes.error
                    } else {
                        &g.brushes.muted
                    },
                );
                if error {
                    let (x, y, w, h) = self.local(GPart::Retry);
                    let r = rect(x, y, x + w, y + h);
                    if self.hover == Some(GPart::Retry) {
                        fill_round(dc, r, RADIUS, &g.brushes.hover);
                    }
                    stroke_round(dc, r, RADIUS, &g.brushes.border);
                    let l = g.layout(&gfx::wide("Retry"), &g.formats.picker, 1e6)?;
                    let m = gfx::metrics(&l);
                    crate::text::draw(
                        dc,
                        &l,
                        x + (w - m.widthIncludingTrailingWhitespace) / 2.0,
                        y + (h - m.height) / 2.0,
                        &g.brushes.text,
                    );
                }
            }
            dc.PopAxisAlignedClip();
            let l = g.layout(&gfx::wide("Powered by KLIPY"), &g.formats.file_detail, 1e6)?;
            let m = gfx::metrics(&l);
            crate::text::draw(
                dc,
                &l,
                W - M - m.widthIncludingTrailingWhitespace,
                GRID_BOTTOM + (CAPTION_H - m.height) / 2.0,
                &g.brushes.muted,
            );
        }
        Ok(next)
    }
}

/// Paints the first source that loads, contained in `r` with rounded
/// corners; a loading source waits before the next is tried (egui's
/// `paint`). Returns when its next frame is due.
fn paint(
    dc: &ID2D1DeviceContext,
    media: &mut Media<D2d>,
    tex: &mut D2d,
    r: D2D_RECT_F,
    sources: &[Source<'_>],
    playing: bool,
    now: Instant,
) -> Result<Option<Duration>> {
    for (url, animated) in sources {
        let key = klipy::media_key(url);
        let (state, next) = if *animated {
            media.gif(tex, &key, Some(*url), playing, now)
        } else {
            (media.image(&key, Some(*url)), None)
        };
        match state {
            ImageState::Ready(_) => {
                if let Some(bmp) = media.texture(&key) {
                    contain(dc, bmp, r, 8.0)?;
                }
                return Ok(next);
            }
            ImageState::Loading => return Ok(None),
            ImageState::Failed => {}
        }
    }
    Ok(None)
}

/// Fills the largest rect of `bmp`'s shape centred in `r` with it.
fn contain(dc: &ID2D1DeviceContext, bmp: &ID2D1Bitmap1, r: D2D_RECT_F, radius: f32) -> Result<()> {
    unsafe {
        let size = bmp.GetPixelSize();
        let (bw, bh) = (size.width.max(1) as f32, size.height.max(1) as f32);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let k = (w / bw).min(h / bh);
        let (dw, dh) = (bw * k, bh * k);
        let (x, y) = (r.left + (w - dw) / 2.0, r.top + (h - dh) / 2.0);
        let brush = dc.CreateBitmapBrush(
            bmp,
            Some(&D2D1_BITMAP_BRUSH_PROPERTIES1 {
                extendModeX: D2D1_EXTEND_MODE_CLAMP,
                extendModeY: D2D1_EXTEND_MODE_CLAMP,
                interpolationMode: D2D1_INTERPOLATION_MODE_LINEAR,
            }),
            None,
        )?;
        brush.SetTransform(&windows_numerics::Matrix3x2 {
            M11: k,
            M12: 0.0,
            M21: 0.0,
            M22: k,
            M31: x,
            M32: y,
        });
        dc.FillRoundedRectangle(
            &D2D1_ROUNDED_RECT {
                rect: rect(x, y, x + dw, y + dh),
                radiusX: radius,
                radiusY: radius,
            },
            &brush,
        );
    }
    Ok(())
}

impl App {
    /// A composer's GIF button toggles the picker, as in egui: the
    /// channel's (`root` None) or thread `root`'s reply box. The other
    /// composer's button moves it there.
    pub fn toggle_gifs(&mut self, root: Option<String>) {
        if let Some(p) = &self.gifs {
            let same = p.root == root;
            self.close_gifs(same);
            if same {
                return;
            }
        }
        if !self.gif_shown() {
            return;
        }
        let anchor = if root.is_some() {
            let Some((x, y, w, h)) = self.thread_part_rect(crate::thread::Part::Gif) else {
                return;
            };
            (x + w / 2.0, y + h / 2.0)
        } else {
            let (bx, by, bw, _) = self.box_rect();
            let top = self.box_files(bw).2;
            let b = crate::uploads::gif_rect(top, bw);
            (bx + (b.left + b.right) / 2.0, by + (b.top + b.bottom) / 2.0)
        };
        self.close_picker();
        self.close_menu();
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        let mut picker = Picker::new(anchor, (self.w, self.h));
        picker.root = root;
        self.gifs = Some(picker);
        self.dirty.thread_box = true;
        self.gifs_poll();
        self.gifs_changed(true);
        self.dirty.composer = true;
        self.events |= EV_SCREEN | EV_FOCUS;
    }

    /// Closes the picker; `focus` returns focus to the composer that
    /// opened it.
    pub fn close_gifs(&mut self, focus: bool) {
        let Some(p) = self.gifs.take() else {
            return;
        };
        unsafe {
            let _ = KillTimer(Some(self.hwnd), crate::TIMER_GIFS);
            let _ = KillTimer(Some(self.hwnd), crate::TIMER_GIF_FRAMES);
        }
        self.dirty.gifs = true;
        self.dirty.composer = true;
        self.dirty.thread_box = true;
        self.need = true;
        self.events |= EV_SCREEN | EV_FOCUS;
        if focus && p.root.is_some() {
            self.set_thread_focus(true);
        } else if focus {
            self.set_composer_focus(true);
        }
    }

    /// After the picker's search, results, scroll or hover changed.
    pub fn gifs_changed(&mut self, text: bool) {
        let g = &self.g;
        if let Some(p) = &mut self.gifs
            && text
        {
            p.search.caret_on = true;
            let w = p.search.width;
            if let Err(e) = p.search.relayout_with(g, &g.formats.composer, 15.0, w) {
                eprintln!("GIF search layout failed: {e:?}");
            }
            if p.search_edited(Instant::now()) {
                unsafe {
                    SetTimer(
                        Some(self.hwnd),
                        crate::TIMER_GIFS,
                        DEBOUNCE.as_millis() as u32,
                        None,
                    );
                }
            }
            self.events |= EV_TEXT | EV_SCREEN;
        }
        self.dirty.gifs = true;
        self.need = true;
        self.ime_position();
    }

    /// Applies loaded pages and starts a paused search; closes the picker
    /// once GIFs can't be sent here.
    pub fn gifs_poll(&mut self) {
        if self.gifs.is_none() {
            return;
        }
        let config = self
            .live
            .as_ref()
            .filter(|l| l.can_gif())
            .and_then(|l| l.klipy.clone());
        if config.is_none() {
            return self.close_gifs(false);
        }
        let Some(p) = &mut self.gifs else { return };
        let (changed, wait) = p.poll(config.as_ref(), Instant::now(), self.hwnd);
        if let Some(wait) = wait {
            unsafe {
                SetTimer(
                    Some(self.hwnd),
                    crate::TIMER_GIFS,
                    wait.as_millis().clamp(1, u32::MAX as u128) as u32,
                    None,
                );
            }
        }
        if changed {
            self.dirty.gifs = true;
            self.events |= EV_SCREEN;
            self.need = true;
        }
    }

    /// The search debounce's timer.
    pub fn gifs_tick(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), crate::TIMER_GIFS);
        }
        self.gifs_poll();
    }

    pub fn gifs_activate(&mut self, part: GPart) {
        let config = self.live.as_ref().and_then(|l| l.klipy.clone());
        let hwnd = self.hwnd;
        let Some(p) = &mut self.gifs else { return };
        match part {
            GPart::Close => return self.close_gifs(true),
            GPart::Tab(k) => {
                if p.kind != KINDS[k].0 {
                    p.kind = KINDS[k].0;
                    p.load(config.as_ref(), 1, hwnd);
                    self.events |= EV_SCREEN;
                }
            }
            GPart::Search => {
                p.search.focused = true;
                self.events |= EV_FOCUS;
            }
            GPart::Retry => {
                let page = if p.items.is_empty() { 1 } else { p.page + 1 };
                p.load(config.as_ref(), page, hwnd);
                self.events |= EV_SCREEN;
            }
            GPart::Tile(i) => {
                let root = p.root.clone();
                let Some((gif, share)) = config.as_ref().and_then(|c| p.choose(i, c)) else {
                    return;
                };
                // As in egui: sent at once, into its thread with the box's
                // "Also send to" choice (which stays), or answering the
                // channel composer's reply.
                let (reply, thread) = match root {
                    Some(root) => {
                        let t = self.thread.as_mut().filter(|t| t.root == root);
                        let broadcast = t.as_ref().is_some_and(|t| t.broadcast);
                        if let Some(t) = t {
                            t.at_end = true;
                        }
                        (None, Some((root, broadcast)))
                    }
                    None => {
                        let reply = self.reply.as_ref().map(|r| r.target.id.clone());
                        self.cancel_reply();
                        self.at_end = true;
                        self.need = true;
                        (reply, None)
                    }
                };
                self.close_gifs(true);
                if let Some(live) = &mut self.live {
                    live.send_gif(gif, share, reply, thread);
                }
                return;
            }
        }
        self.gifs_changed(false);
    }

    /// Input while the picker is open. None leaves it to the normal handler.
    pub fn gifs_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let hwnd = self.hwnd;
        let pointer_in = self
            .pointer
            .zip(self.gifs.as_ref())
            .is_some_and(|((px, py), p)| contains(p.rect(), px, py));
        let config = self.live.as_ref().and_then(|l| l.klipy.clone());
        let p = self.gifs.as_mut()?;
        match msg {
            WM_MOUSEMOVE => {
                let hover = p.hit(x, y);
                if hover != p.hover {
                    p.hover = hover;
                    self.dirty.gifs = true;
                    self.need = true;
                }
                None
            }
            WM_LBUTTONDOWN => {
                if !contains(p.rect(), x, y) {
                    // An outside click closes only the picker and still
                    // reaches what it landed on (its own GIF button
                    // toggles; the other composer's moves it).
                    let own = if p.root.is_some() {
                        self.in_thread(x, y)
                            && self.thread_hit(x, y) == Some(crate::thread::Part::Gif)
                    } else {
                        self.box_part_at(x, y) == Some(crate::uploads::Part::Gif)
                    };
                    if own {
                        self.close_gifs(true);
                        return done;
                    }
                    self.close_gifs(false);
                    return None;
                }
                match p.hit(x, y) {
                    Some(GPart::Search) => {
                        let (ox, oy) = p.text_origin();
                        let pos = p.search.hit(x - ox, y - oy);
                        p.search.move_to(pos, shift);
                        p.search.focused = true;
                        self.gifs_changed(false);
                    }
                    Some(part) => self.gifs_activate(part),
                    None => {}
                }
                done
            }
            WM_MOUSEWHEEL if pointer_in => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32;
                p.scroll = (p.scroll - delta / 120.0 * 48.0).clamp(0.0, p.max_scroll());
                p.hover = self.pointer.and_then(|(x, y)| p.hit(x, y));
                p.more(config.as_ref(), hwnd);
                self.events |= EV_SCREEN;
                self.gifs_changed(false);
                done
            }
            WM_LBUTTONDBLCLK if contains(p.rect(), x, y) => done,
            WM_SETCURSOR => {
                let (px, py) = self.pointer.filter(|(x, y)| contains(p.rect(), *x, *y))?;
                let cursor = match p.hit(px, py) {
                    Some(GPart::Search) => IDC_IBEAM,
                    Some(_) => IDC_HAND,
                    None => IDC_ARROW,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                Some(Some(LRESULT(1)))
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                match vk {
                    VK_ESCAPE => self.close_gifs(true),
                    _ if ctrl && vk == VIRTUAL_KEY('C' as u16) => {
                        let (a, b) = p.search.selection();
                        if b > a {
                            crate::set_clipboard(hwnd, &p.search.text[a..b]);
                        }
                    }
                    _ if p.search.focused && p.search.edit_key(hwnd, vk, ctrl, shift, false) => {
                        self.gifs_changed(true);
                    }
                    _ => return Some(None),
                }
                done
            }
            WM_CHAR => {
                let c = wp.0 as u16;
                if p.search.focused && !ctrl && c >= 0x20 && c != 0x7f {
                    p.search.insert(&[c]);
                    self.gifs_changed(true);
                }
                done
            }
            _ => None,
        }
    }

    /// Redraws the picker's surface when it changed, and asks for the next
    /// GIF frame.
    pub fn draw_gifs(&mut self) -> Result<()> {
        if !std::mem::take(&mut self.dirty.gifs) {
            return Ok(());
        }
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let Some(p) = &self.gifs else {
            return self.gifs_p.offset(-10_000.0, 0.0);
        };
        let g = &self.g;
        self.gifs_p.ensure(g, px(W), px(H))?;
        self.gifs_p.offset((p.x * s).round(), (p.y * s).round())?;
        let mut tex = D2d(g.dc.clone());
        let mut media = self.live.as_mut().map(|l| &mut l.media);
        let (playing, now) = (crate::files::motion(), Instant::now());
        let mut next = None;
        self.gifs_p.draw(s, |dc| {
            next = p.draw(g, dc, media.as_deref_mut(), &mut tex, playing, now)?;
            Ok(())
        })?;
        let busy = media.is_some_and(|m| m.busy());
        unsafe {
            if let Some(next) = next {
                SetTimer(
                    Some(self.hwnd),
                    crate::TIMER_GIF_FRAMES,
                    next.as_millis().clamp(10, u32::MAX as u128) as u32,
                    None,
                );
            }
            if busy {
                SetTimer(Some(self.hwnd), crate::TIMER_LIVE, 100, None);
            }
        }
        Ok(())
    }

    /// The next frame of the picker's GIFs.
    pub fn next_picker_frame(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), crate::TIMER_GIF_FRAMES);
        }
        if self.gifs.is_some() {
            self.dirty.gifs = true;
            self.need = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caper_client::klipy::parse;
    use serde_json::Value;

    fn cases() -> Value {
        serde_json::from_str(include_str!("../../../../shared/klipy/cases.json")).unwrap()
    }

    fn picker() -> Picker {
        Picker::new((600.0, 700.0), (1200.0, 800.0))
    }

    #[test]
    fn the_grid_cell_is_eguis_floored_third() {
        assert_eq!(CELL, ((INNER_W - GAP * 2.0) / 3.0).floor());
    }

    #[test]
    fn it_opens_ending_at_its_button_and_stays_in_the_window() {
        let p = picker();
        assert_eq!((p.x + W, p.y + H), (614.0, 682.0));
        // Near the window's left and top it moves inside instead.
        let p = Picker::new((100.0, 200.0), (1200.0, 800.0));
        assert_eq!((p.x, p.y), (0.0, 0.0));
    }

    #[test]
    fn pages_append_skip_ads_and_superseded_loads_are_dropped() {
        let mut p = picker();
        // Without a config nothing is requested, and the picker says so.
        p.load(None, 1, HWND::default());
        assert_eq!(p.error.as_deref(), Some(UNAVAILABLE));
        assert_eq!(p.empty_text(), (UNAVAILABLE, true));
        assert!(p.parts().contains(&GPart::Retry));
        let page = parse(Kind::Gif, &cases()["parse"]["response"]);
        let media = page
            .items
            .iter()
            .filter(|i| matches!(i, Item::Media { .. }))
            .count();
        assert!(media < page.items.len(), "the shared case carries an ad");
        p.request = 5;
        p.error = None;
        p.loading = true;
        p.apply(Loaded {
            request: 4,
            page: 1,
            result: Ok(page.clone()),
        });
        assert!(p.items.is_empty() && p.loading);
        p.apply(Loaded {
            request: 5,
            page: 1,
            result: Ok(page.clone()),
        });
        assert_eq!(p.items.len(), media);
        assert!(p.has_next && !p.loading);
        p.loading = true;
        p.apply(Loaded {
            request: 5,
            page: 2,
            result: Err("x".into()),
        });
        assert_eq!((p.items.len(), p.page), (media, 1));
        assert_eq!(p.error.as_deref(), Some("x"));
    }

    #[test]
    fn typing_waits_for_a_pause_and_the_same_query_does_not_search() {
        let mut p = picker();
        let start = Instant::now();
        p.request = 1;
        p.search.insert(&gfx::wide("cat"));
        assert!(p.search_edited(start));
        let (_, wait) = p.poll(None, start + Duration::from_millis(100), HWND::default());
        assert_eq!(p.request, 1);
        assert_eq!(wait, Some(DEBOUNCE - Duration::from_millis(100)));
        p.poll(None, start + DEBOUNCE, HWND::default());
        assert_eq!(p.request, 2);
        assert_eq!(p.shown, (Kind::Gif, "cat".into()));
        // Adding a space keeps the trimmed query: nothing new to search.
        p.search.insert(&[b' ' as u16]);
        assert!(!p.search_edited(start));
    }

    #[test]
    fn tiles_hit_inside_the_grid_only_and_choices_share_their_query() {
        let mut p = picker();
        p.items = parse(Kind::Gif, &cases()["parse"]["response"])
            .items
            .into_iter()
            .filter_map(|i| match i {
                Item::Media { tile, gif, .. } => Some((tile, *gif)),
                Item::Ad { .. } => None,
            })
            .collect();
        p.shown = (Kind::Gif, "cat".into());
        let (x, y, _, _) = p.part_rect(GPart::Tile(1));
        assert_eq!(p.hit(x + 5.0, y + 5.0), Some(GPart::Tile(1)));
        // Scrolled up past the grid's top, a tile can't be clicked there.
        p.scroll = 20.0;
        let (x, y, _, _) = p.part_rect(GPart::Tile(0));
        assert_eq!(p.hit(x + 5.0, y + 15.0), None);
        assert_eq!(p.hit(x + 5.0, y + 25.0), Some(GPart::Tile(0)));
        let config = Config {
            app_key: "key".into(),
            customer_id: "c".into(),
        };
        let (gif, share) = p.choose(0, &config).unwrap();
        assert_eq!(
            (share.slug, share.query, share.kind),
            (gif.slug, "cat".into(), gif.kind)
        );
    }
}
