//! The KLIPY GIF and sticker picker and sent GIFs' display. Parsing, the
//! shared message and URL rules and KLIPY requests are in
//! `caper_client::klipy`. Desktop cannot render ad HTML in an isolated web
//! view, so an ad item that arrives anyway is skipped in the grid.

use crate::attachments::{ImageState, Media};
pub use caper_client::klipy::{
    Config, Gif, Item, Kind, Page, Share, Tile, fetch, media_key, viewer_file,
};
use eframe::egui::{self, Color32, CornerRadius, RichText};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

/// Searches wait this long after typing stops.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// The longest edge a sent GIF or sticker shows at, as on web.
const GIF_EDGE: f32 = 300.0;
const STICKER_EDGE: f32 = 160.0;

/// How a sent GIF or sticker is sized in a message.
pub trait GifSize {
    fn display_size(&self, available: f32) -> egui::Vec2;
}

impl GifSize for Gif {
    /// Display size: the aspect ratio, at most 300 px on the longer edge
    /// (stickers 160 px), never wider than `available`.
    fn display_size(&self, available: f32) -> egui::Vec2 {
        let limit = match self.kind {
            Kind::Gif => GIF_EDGE,
            Kind::Sticker => STICKER_EDGE,
        };
        let (width, height) = (self.width as f32, self.height as f32);
        let scale = (limit / width.max(height))
            .min(available.max(1.0) / width)
            .min(1.0);
        egui::vec2(
            (width * scale).round().max(1.0),
            (height * scale).round().max(1.0),
        )
    }
}

/// A chosen GIF: what to send, and what to tell KLIPY once it is sent.
pub struct Choice {
    pub gif: Gif,
    pub share: Share,
}

struct Loaded {
    request: u64,
    page: u32,
    result: Result<Page, String>,
}

/// The open picker for one composer: its tab, search, results and paging.
pub struct Picker {
    /// The thread the picker sends into; `None` is the channel composer.
    pub root: Option<String>,
    /// The conversation it opened in; navigating away closes it.
    pub channel: Option<String>,
    kind: Kind,
    search: String,
    /// When `search` last changed and has not been requested yet.
    edited: Option<Instant>,
    /// The tab and trimmed query the shown results answer.
    shown: (Kind, String),
    items: Vec<Item>,
    page: u32,
    has_next: bool,
    loading: bool,
    error: Option<String>,
    request: u64,
    /// Bumped by each new first page, so the grid scrolls back to the top.
    results: u64,
    sender: Sender<Loaded>,
    receiver: Receiver<Loaded>,
    anchor: Option<egui::Pos2>,
    opened: u64,
    rect: Option<egui::Rect>,
    focus_search: bool,
}

impl Picker {
    pub fn new(root: Option<String>, anchor: Option<egui::Pos2>, frame: u64) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            root,
            channel: None,
            kind: Kind::Gif,
            search: String::new(),
            edited: None,
            shown: (Kind::Gif, String::new()),
            items: Vec::new(),
            page: 0,
            has_next: false,
            loading: false,
            error: None,
            request: 0,
            results: 0,
            sender,
            receiver,
            anchor,
            opened: frame,
            rect: None,
            focus_search: true,
        }
    }

    /// Requests a page of the current tab and query (page 1 replaces).
    fn load(&mut self, context: &egui::Context, config: Option<&Config>, page: u32) {
        let query = self.search.trim().to_owned();
        if page <= 1 {
            self.results += 1;
            self.items.clear();
            self.has_next = false;
        }
        self.request += 1;
        self.shown = (self.kind, query.clone());
        self.edited = None;
        self.loading = true;
        self.error = None;
        let Some(config) = config.cloned() else {
            self.loading = false;
            self.error = Some("GIFs are unavailable right now.".into());
            return;
        };
        let (request, kind, sender, context) = (
            self.request,
            self.kind,
            self.sender.clone(),
            context.clone(),
        );
        std::thread::spawn(move || {
            let result = fetch(&config, kind, &query, page);
            let _ = sender.send(Loaded {
                request,
                page,
                result,
            });
            context.request_repaint();
        });
    }

    /// Applies finished loads (dropping superseded ones) and starts a search
    /// once typing has paused.
    fn poll(&mut self, context: &egui::Context, config: Option<&Config>, now: Instant) {
        if self.request == 0 {
            self.load(context, config, 1);
        }
        for loaded in self.receiver.try_iter().collect::<Vec<_>>() {
            self.apply(loaded);
        }
        if let Some(edited) = self.edited {
            let waited = now.saturating_duration_since(edited);
            if waited >= DEBOUNCE {
                self.load(context, config, 1);
            } else {
                context.request_repaint_after(DEBOUNCE - waited);
            }
        }
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
                self.items.extend(page.items);
            }
            Err(error) => self.error = Some(error),
        }
    }

    fn select_kind(&mut self, context: &egui::Context, config: Option<&Config>, kind: Kind) {
        if self.kind != kind {
            self.kind = kind;
            self.load(context, config, 1);
        }
    }

    /// The query a send from the shown results was found with.
    fn shown_query(&self) -> String {
        self.shown.1.clone()
    }

    /// The picker's screen rect last frame, and the frame it opened in.
    pub fn bounds(&self) -> Option<(egui::Rect, u64)> {
        self.rect.map(|rect| (rect, self.opened))
    }

    /// Draws the picker. Returns the chosen GIF, or `Err(())` to close.
    pub fn show(
        &mut self,
        context: &egui::Context,
        config: Option<&Config>,
        media: &mut Media,
    ) -> Result<Option<Choice>, ()> {
        // Static previews and tests never fetch, as their media never loads.
        let config = config.filter(|_| media.api().is_some());
        let now = Instant::now();
        self.poll(context, config, now);
        let mut close =
            context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        let mut chosen = None;
        let mut window = egui::Window::new("GIFs")
            .id(egui::Id::unique("gif-picker"))
            .title_bar(false)
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .fixed_size(egui::vec2(352.0, 430.0));
        // Opens above its GIF button, ending at it, like the emoji picker.
        if let Some(anchor) = self.anchor {
            window = window
                .pivot(egui::Align2::RIGHT_BOTTOM)
                .fixed_pos(anchor + egui::vec2(14.0, -18.0));
        }
        let shown = window.show(context, |ui| {
            let playing = crate::reply_motion_enabled() && crate::pictures::playing(ui);
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            if crate::floating_heading(ui, "GIFs and stickers", "Close GIF picker", true) {
                close = true;
            }
            ui.add_space(6.0);
            let tab_width = ui.available_width() / 2.0;
            ui.horizontal(|ui| {
                for (kind, name) in [(Kind::Gif, "GIFs"), (Kind::Sticker, "Stickers")] {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(tab_width, 36.0), egui::Sense::click());
                    let selected = self.kind == kind;
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(egui::Role::Button, true, selected, name)
                    });
                    if response.hovered() || response.has_focus() {
                        ui.painter()
                            .rect_filled(rect.shrink(2.0), 8.0, crate::SURFACE_HOVER);
                    }
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        name,
                        egui::FontId::proportional(13.0),
                        if selected { crate::TEXT } else { crate::MUTED },
                    );
                    if selected {
                        ui.painter().line_segment(
                            [
                                rect.left_bottom() + egui::vec2(2.0, 0.0),
                                rect.right_bottom() - egui::vec2(2.0, 0.0),
                            ],
                            egui::Stroke::new(2.0_f32, crate::TERRACOTTA),
                        );
                    }
                    if response.clicked() {
                        self.select_kind(ui.ctx(), config, kind);
                    }
                }
            });
            ui.add_space(10.0);
            let search_id = egui::Id::unique("gif-picker-search");
            // KLIPY requires exactly this placeholder.
            let search = ui.add(
                egui::TextEdit::singleline(&mut self.search)
                    .id(search_id)
                    .hint_text("Search KLIPY")
                    .desired_width(f32::INFINITY),
            );
            crate::focus_frame(ui, &search);
            if self.focus_search {
                ui.memory_mut(|memory| memory.request_focus(search_id));
                self.focus_search = false;
            }
            if search.changed() {
                self.edited = (self.search.trim() != self.shown.1 || self.kind != self.shown.0)
                    .then_some(now);
                if self.edited.is_some() {
                    ui.ctx().request_repaint_after(DEBOUNCE);
                }
            }
            ui.add_space(8.0);
            let caption_height = 22.0;
            let grid_height = (ui.available_height() - caption_height).max(40.0);
            let columns = 3;
            let gap = 4.0;
            let cell = ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32)
                .floor()
                .max(1.0);
            // Ads are skipped: desktop never asks for them (see the module docs).
            let media_items: Vec<(&Tile, &Gif)> = self
                .items
                .iter()
                .filter_map(|item| match item {
                    Item::Media { tile, gif, .. } => Some((tile, gif.as_ref())),
                    Item::Ad { .. } => None,
                })
                .collect();
            let mut retry = false;
            let mut more = false;
            ui.allocate_ui(egui::vec2(ui.available_width(), grid_height), |ui| {
                ui.set_min_height(grid_height);
                if media_items.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.spacing_mut().item_spacing.y = 8.0;
                        if let Some(error) = &self.error {
                            ui.label(RichText::new(error).color(crate::ERROR));
                            retry = ui.button("Retry").clicked();
                        } else if self.loading || self.edited.is_some() {
                            ui.spinner();
                        } else {
                            ui.label(RichText::new("No results").color(crate::MUTED));
                        }
                    });
                    return;
                }
                let rows = media_items.len().div_ceil(columns);
                let output = egui::ScrollArea::vertical()
                    .id_salt(("gif-grid", self.results))
                    .auto_shrink([false, false])
                    .max_height(grid_height)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = egui::vec2(gap, gap);
                        for row in 0..rows {
                            ui.horizontal(|ui| {
                                for (tile, gif) in
                                    media_items.iter().skip(row * columns).take(columns)
                                {
                                    let (rect, response) = ui.allocate_exact_size(
                                        egui::vec2(cell, cell),
                                        egui::Sense::click(),
                                    );
                                    let response = response.on_hover_text(gif.alt());
                                    response.widget_info(|| {
                                        egui::WidgetInfo::labeled(
                                            egui::Role::Button,
                                            true,
                                            gif.alt(),
                                        )
                                    });
                                    ui.painter().rect_filled(rect, 8.0, crate::SURFACE);
                                    if ui.is_rect_visible(rect) {
                                        let sources = tile_sources(tile, playing);
                                        paint(ui, media, rect, &sources, playing, 8);
                                    }
                                    if response.hovered() || response.has_focus() {
                                        ui.painter().rect_stroke(
                                            rect,
                                            8.0,
                                            egui::Stroke::new(1.0_f32, crate::TERRACOTTA),
                                            egui::StrokeKind::Inside,
                                        );
                                    }
                                    if response.clicked() {
                                        chosen = Some((*gif).clone());
                                    }
                                }
                            });
                        }
                        if let Some(error) = &self.error {
                            ui.add_space(6.0);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new(error).color(crate::ERROR));
                                retry = ui.button("Retry").clicked();
                            });
                        } else if self.loading {
                            ui.add_space(6.0);
                            ui.vertical_centered(|ui| ui.spinner());
                        }
                    });
                // Load more as the grid nears its end.
                let remaining = output.content_size.y
                    - (output.state.clamped_offset().y + output.inner_rect.height());
                more = remaining < cell * 2.0;
            });
            if retry {
                let page = if self.items.is_empty() {
                    1
                } else {
                    self.page + 1
                };
                self.load(ui.ctx(), config, page);
            } else if more && self.has_next && !self.loading && self.error.is_none() {
                self.load(ui.ctx(), config, self.page + 1);
            }
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), caption_height),
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    ui.label(
                        RichText::new("Powered by KLIPY")
                            .size(11.0)
                            .color(crate::MUTED),
                    )
                },
            );
        });
        if let Some(shown) = &shown {
            crate::motion::overlay(
                context,
                shown.response.layer_id,
                crate::motion::BASE,
                8.0,
                false,
            );
        }
        self.rect = shown.map(|shown| shown.response.rect);
        if media.busy() {
            context.request_repaint();
        }
        if let (Some(gif), Some(config)) = (chosen, config) {
            return Ok(Some(Choice {
                share: Share {
                    config: config.clone(),
                    kind: gif.kind,
                    slug: gif.slug.clone(),
                    query: self.shown_query(),
                },
                gif,
            }));
        }
        if close { Err(()) } else { Ok(None) }
    }
}

/// One image to try: its URL and whether to decode animation frames.
type Source<'a> = (&'a str, bool);

/// Animated first, else the still; reduced motion prefers the still, then
/// the GIF's first frame.
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

/// A sent GIF's sources, in the order [`tile_sources`] uses.
fn gif_sources(gif: &Gif, playing: bool) -> Vec<Source<'_>> {
    let animated = gif.gif.as_deref().map(|url| (url, true));
    let webp = gif.webp.as_deref().map(|url| (url, true));
    let still = gif.still.as_deref().map(|url| (url, false));
    if playing {
        [animated, webp, still]
    } else {
        [still, animated, webp]
    }
    .into_iter()
    .flatten()
    .collect()
}

/// Paints the first source that loads, contained in `rect`; a loading
/// source waits before the next is tried. True when every source failed.
fn paint(
    ui: &egui::Ui,
    media: &mut Media,
    rect: egui::Rect,
    sources: &[Source<'_>],
    playing: bool,
    radius: u8,
) -> bool {
    for (url, animated) in sources {
        let key = media_key(url);
        let state = if *animated {
            media.gif(ui.ctx(), &key, Some(url), playing, Instant::now())
        } else {
            media.image(&key, Some(url))
        };
        match state {
            ImageState::Ready(texture) => {
                let size = ui
                    .ctx()
                    .tex_manager()
                    .read()
                    .meta(texture)
                    .map(|meta| meta.size)
                    .unwrap_or([1, 1]);
                let scale = (rect.width() / size[0].max(1) as f32)
                    .min(rect.height() / size[1].max(1) as f32);
                let fitted = egui::Rect::from_center_size(
                    rect.center(),
                    egui::vec2(size[0] as f32 * scale, size[1] as f32 * scale),
                );
                egui::Image::new((texture, fitted.size()))
                    .corner_radius(CornerRadius::same(radius))
                    .paint_at(ui, fitted);
                return false;
            }
            ImageState::Loading => return false,
            ImageState::Failed => {}
        }
    }
    true
}

/// A message's GIF or sticker, below its text. Hover shows the title.
pub fn message(ui: &mut egui::Ui, media: &mut Media, gif: &Gif) -> egui::Response {
    let size = gif.display_size(ui.available_width());
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let label = gif.alt().to_owned();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Open {label} in viewer"))
    });
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let sticker = gif.kind == Kind::Sticker;
    let playing = crate::reply_motion_enabled() && crate::pictures::playing(ui);
    // Stickers keep their transparency: no background behind them.
    if !sticker {
        ui.painter().rect_filled(rect, 8.0, crate::SURFACE);
    }
    let sources = gif_sources(gif, playing);
    if paint(
        ui,
        media,
        rect,
        &sources,
        playing,
        if sticker { 0 } else { 8 },
    ) {
        if sticker {
            ui.painter().rect_filled(rect, 8.0, crate::SURFACE);
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "GIF unavailable",
            egui::FontId::proportional(11.5),
            crate::MUTED,
        );
    }
    if media.busy() {
        ui.ctx().request_repaint();
    }
    response
        .on_hover_cursor(egui::CursorIcon::ZoomIn)
        .on_hover_text(label)
}

/// A muted "GIF" label for the composer button, in a quiet outline.
pub fn button(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    if response.hovered() || response.has_focus() {
        ui.painter().rect_filled(
            rect,
            if size > 28.0 { 8.0 } else { 6.0 },
            crate::SURFACE_HOVER,
        );
    }
    let color = if !ui.is_enabled() {
        Color32::from_gray(110)
    } else if response.hovered() {
        crate::TEXT
    } else {
        crate::MUTED
    };
    let badge = egui::Rect::from_center_size(rect.center(), egui::vec2(20.0, 14.0));
    ui.painter().rect_stroke(
        badge,
        3.0,
        egui::Stroke::new(1.4_f32, color),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        "GIF",
        egui::FontId::proportional(8.5),
        color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), "Send a GIF")
    });
    response.on_hover_text("Send a GIF")
}

#[cfg(test)]
mod tests {
    use super::*;
    use caper_client::klipy::parse;
    use serde_json::Value;

    fn cases() -> Value {
        serde_json::from_str(include_str!("../../../../shared/klipy/cases.json")).unwrap()
    }

    #[test]
    fn sizes_keep_the_aspect_ratio_within_the_edge() {
        let mut gif = Gif::from_value(&cases()["messages"]["valid"][0]).unwrap();
        gif.width = 600;
        gif.height = 300;
        assert_eq!(gif.display_size(1000.0), egui::vec2(300.0, 150.0));
        assert_eq!(gif.display_size(150.0), egui::vec2(150.0, 75.0));
        gif.kind = Kind::Sticker;
        assert_eq!(gif.display_size(1000.0), egui::vec2(160.0, 80.0));
        gif.width = 50;
        gif.height = 100;
        assert_eq!(gif.display_size(1000.0), egui::vec2(50.0, 100.0));
        assert_eq!(gif.alt(), "Hello");
        gif.title.clear();
        assert_eq!(gif.alt(), "Sticker");
    }

    #[test]
    fn reduced_motion_prefers_stills_then_first_frames() {
        let gif = Gif::from_value(&cases()["messages"]["valid"][0]).unwrap();
        let urls = |playing| -> Vec<(String, bool)> {
            gif_sources(&gif, playing)
                .into_iter()
                .map(|(url, animated)| (url.rsplit('/').next().unwrap().to_owned(), animated))
                .collect()
        };
        let owned = |items: &[(&str, bool)]| -> Vec<(String, bool)> {
            items
                .iter()
                .map(|(url, animated)| ((*url).to_owned(), *animated))
                .collect()
        };
        assert_eq!(
            urls(true),
            owned(&[
                ("mdgif.gif", true),
                ("mdwebp.webp", true),
                ("mdjpg.jpg", false)
            ])
        );
        assert_eq!(
            urls(false),
            owned(&[
                ("mdjpg.jpg", false),
                ("mdgif.gif", true),
                ("mdwebp.webp", true)
            ])
        );
    }

    #[test]
    fn pages_append_and_superseded_loads_are_dropped() {
        let context = egui::Context::default();
        let mut picker = Picker::new(None, None, 0);
        // Without a config nothing is requested, and the picker says so.
        picker.poll(&context, None, Instant::now());
        assert!(picker.error.is_some());
        let page = parse(Kind::Gif, &cases()["parse"]["response"]);
        picker.request = 5;
        picker.error = None;
        picker.loading = true;
        picker.apply(Loaded {
            request: 4,
            page: 1,
            result: Ok(page.clone()),
        });
        assert!(picker.items.is_empty() && picker.loading);
        picker.apply(Loaded {
            request: 5,
            page: 1,
            result: Ok(page.clone()),
        });
        assert_eq!(picker.items.len(), 5);
        assert!(picker.has_next && !picker.loading);
        picker.loading = true;
        picker.apply(Loaded {
            request: 5,
            page: 2,
            result: Err("x".into()),
        });
        assert_eq!((picker.items.len(), picker.page), (5, 1));
        assert_eq!(picker.error.as_deref(), Some("x"));
    }

    #[test]
    fn typing_waits_for_a_pause_before_searching() {
        let context = egui::Context::default();
        let mut picker = Picker::new(None, None, 0);
        let start = Instant::now();
        picker.request = 1;
        picker.search = "cat".into();
        picker.edited = Some(start);
        picker.poll(&context, None, start + Duration::from_millis(100));
        assert_eq!(picker.request, 1);
        picker.poll(&context, None, start + DEBOUNCE);
        assert_eq!(picker.request, 2);
        assert_eq!(picker.shown, (Kind::Gif, "cat".into()));
        assert!(picker.edited.is_none());
    }
}
