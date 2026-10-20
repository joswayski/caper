//! KLIPY GIFs and stickers (`docs/media.md`, "GIFs and stickers (KLIPY)").
//!
//! KLIPY's terms require search, trending, share and media requests to come
//! straight from the device, so the API only hands signed-in clients the app
//! key and an unlinkable customer id (`GET /api/klipy`) and validates the
//! `content.gif` a message carries. Every client runs the shared cases in
//! `shared/klipy/cases.json` (parsing, message and URL rules); change them
//! together with `apps/api/src/klipy.rs`.
//!
//! Desktop never asks for ads (KLIPY serves them only to mobile user agents
//! that send the `ad-*` parameters), and it cannot render their HTML in an
//! isolated web view, so an ad item that arrives anyway is skipped in the grid.

use crate::attachments::{ImageState, Media};
use eframe::egui::{self, Color32, CornerRadius, RichText};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use url::Url;

const MAX_URL: usize = 2048;
const MAX_TEXT: usize = 200;
const MAX_EDGE: u64 = 4096;
const PER_PAGE: u32 = 24;
/// Searches wait this long after typing stops.
const DEBOUNCE: Duration = Duration::from_millis(300);
/// The longest edge a sent GIF or sticker shows at, as on web.
const GIF_EDGE: f32 = 300.0;
const STICKER_EDGE: f32 = 160.0;
/// Bounds a KLIPY list response; a page of 24 items is a few tens of KB.
const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;

/// The picker's tab, and a sent GIF's `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Gif,
    Sticker,
}

impl Kind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "gif" => Some(Self::Gif),
            "sticker" => Some(Self::Sticker),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Sticker => "sticker",
        }
    }

    /// KLIPY's path segment.
    fn path(self) -> &'static str {
        match self {
            Self::Gif => "gifs",
            Self::Sticker => "stickers",
        }
    }

    /// What reply quotes and labels say for one.
    pub fn label(self) -> &'static str {
        match self {
            Self::Gif => "GIF",
            Self::Sticker => "Sticker",
        }
    }
}

/// `GET /api/klipy`: KLIPY's client key (it ships to devices by design) and
/// this account's stable, meaningless-to-KLIPY customer id.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    pub app_key: String,
    pub customer_id: String,
}

impl Config {
    /// The key lands in a URL path: 1-200 letters, digits, `-` or `_`, as the
    /// API checks.
    pub fn valid(&self) -> bool {
        (1..=200).contains(&self.app_key.len())
            && self
                .app_key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            && !self.customer_id.is_empty()
    }
}

/// A message's `content.gif`, holding KLIPY's media URLs unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gif {
    pub kind: Kind,
    pub slug: String,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub gif: Option<String>,
    pub webp: Option<String>,
    pub mp4: Option<String>,
    pub still: Option<String>,
}

/// KLIPY media only: HTTPS on klipy.com or a subdomain, with no credentials,
/// port or fragment. A query string is fine.
pub fn media_url(value: &str) -> bool {
    value.len() <= MAX_URL
        && Url::parse(value).is_ok_and(|url| {
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.port().is_none()
                && url.fragment().is_none()
                && url
                    .host_str()
                    .is_some_and(|host| host == "klipy.com" || host.ends_with(".klipy.com"))
        })
}

fn slug(value: &str) -> bool {
    (1..=MAX_TEXT).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn edge(value: &Value) -> Option<u32> {
    value
        .as_u64()
        .filter(|value| (1..=MAX_EDGE).contains(value))
        .map(|value| value as u32)
}

impl Gif {
    /// The shared `messages` rules; anything else renders the text alone.
    pub fn from_value(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        if object.get("provider")?.as_str()? != "klipy" {
            return None;
        }
        let kind = Kind::parse(object.get("kind")?.as_str()?)?;
        let slug_value = object.get("slug")?.as_str()?;
        let title = match object.get("title") {
            None | Some(Value::Null) => "",
            Some(title) => title.as_str()?,
        };
        if !slug(slug_value)
            || title.chars().count() > MAX_TEXT
            || title.chars().any(char::is_control)
        {
            return None;
        }
        let url = |key: &str| -> Result<Option<String>, ()> {
            match object.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(url)) if media_url(url) => Ok(Some(url.clone())),
                Some(_) => Err(()),
            }
        };
        let gif = Self {
            kind,
            slug: slug_value.to_owned(),
            title: title.to_owned(),
            width: edge(object.get("width")?)?,
            height: edge(object.get("height")?)?,
            gif: url("gif").ok()?,
            webp: url("webp").ok()?,
            mp4: url("mp4").ok()?,
            still: url("still").ok()?,
        };
        (gif.gif.is_some() || gif.webp.is_some()).then_some(gif)
    }

    /// The `gif` a send carries, in the shape the API stores.
    pub fn to_value(&self) -> Value {
        let mut value = json!({
            "provider": "klipy",
            "kind": self.kind.name(),
            "slug": self.slug,
            "title": self.title,
            "width": self.width,
            "height": self.height,
        });
        for (key, url) in [
            ("gif", &self.gif),
            ("webp", &self.webp),
            ("mp4", &self.mp4),
            ("still", &self.still),
        ] {
            if let Some(url) = url {
                value[key] = json!(url);
            }
        }
        value
    }

    pub fn label(&self) -> &'static str {
        self.kind.label()
    }

    /// The accessible label and hover text: the title, or `GIF` / `Sticker`.
    pub fn alt(&self) -> &str {
        if self.title.trim().is_empty() {
            self.label()
        } else {
            &self.title
        }
    }

    /// Display size: the aspect ratio, at most 300 px on the longer edge
    /// (stickers 160 px), never wider than `available`.
    pub fn display_size(&self, available: f32) -> egui::Vec2 {
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

impl Serialize for Gif {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(serializer)
    }
}

/// Never fails: a missing, null or invalid `content.gif` is no GIF, so one
/// bad value never rejects a message or a history page.
pub fn tolerant<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Gif>, D::Error> {
    Ok(Gif::from_value(&Value::deserialize(deserializer)?))
}

/// A picker grid tile. `gif` is the animated GIF for this decoder (desktop
/// decodes GIF but not animated WebP): the tile size's own, else the first
/// size that has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    pub url: String,
    pub still: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub gif: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Media {
        slug: String,
        title: String,
        tile: Tile,
        gif: Box<Gif>,
    },
    /// Kept for parity with the shared parser; desktop never shows ads.
    Ad {
        html: String,
        width: Option<u32>,
        height: Option<u32>,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page {
    pub items: Vec<Item>,
    pub has_next: bool,
}

/// A size's format URL when it follows the media rules.
fn format_url<'a>(size: &'a Value, format: &str) -> Option<&'a str> {
    size.get(format)?
        .get("url")?
        .as_str()
        .filter(|url| media_url(url))
}

fn format_size(size: &Value, format: &str) -> Option<(u32, u32)> {
    format_url(size, format)?;
    let entry = size.get(format)?;
    Some((edge(entry.get("width")?)?, edge(entry.get("height")?)?))
}

fn parse_item(kind: Kind, value: &Value) -> Option<Item> {
    if value.get("type").and_then(Value::as_str) == Some("ad") {
        let html = value
            .get("content")?
            .as_str()
            .filter(|html| !html.is_empty())?;
        let positive = |key| {
            value
                .get(key)
                .and_then(Value::as_u64)
                .filter(|value| *value > 0)
                .and_then(|value| u32::try_from(value).ok())
        };
        return Some(Item::Ad {
            html: html.to_owned(),
            width: positive("width"),
            height: positive("height"),
        });
    }
    let slug_value = value.get("slug")?.as_str().filter(|value| slug(value))?;
    let file = value.get("file")?;
    let sized = |names: &[&str]| -> Vec<&Value> {
        names.iter().filter_map(|name| file.get(*name)).collect()
    };
    // The sent GIF: the first of md, hd, sm with a usable GIF or WebP.
    let source = sized(&["md", "hd", "sm"])
        .into_iter()
        .find(|size| format_url(size, "gif").is_some() || format_url(size, "webp").is_some())?;
    let (width, height) = format_size(source, "gif").or_else(|| format_size(source, "webp"))?;
    let title: String = value
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_TEXT)
        .collect();
    let owned = |size: &Value, format: &str| format_url(size, format).map(str::to_owned);
    let gif = Gif {
        kind,
        slug: slug_value.to_owned(),
        title: title.clone(),
        width,
        height,
        gif: owned(source, "gif"),
        webp: owned(source, "webp"),
        mp4: owned(source, "mp4"),
        still: owned(source, "jpg").or_else(|| owned(source, "png")),
    };
    // The tile: the first of sm, xs, md, hd with a WebP or GIF.
    let tiles = sized(&["sm", "xs", "md", "hd"]);
    let (size, format) = tiles.iter().find_map(|size| {
        ["webp", "gif"]
            .into_iter()
            .find(|format| format_url(size, format).is_some())
            .map(|format| (*size, format))
    })?;
    let dimension = |key: &str| {
        size.get(format)
            .and_then(|entry| entry.get(key))
            .and_then(edge)
    };
    let tile = Tile {
        url: owned(size, format)?,
        still: owned(size, "jpg").or_else(|| owned(size, "png")),
        width: dimension("width"),
        height: dimension("height"),
        gif: owned(size, "gif").or_else(|| tiles.iter().find_map(|size| owned(size, "gif"))),
    };
    Some(Item::Media {
        slug: slug_value.to_owned(),
        title,
        tile,
        gif: Box::new(gif),
    })
}

/// A KLIPY list response requested for `kind`, in KLIPY's order.
pub fn parse(kind: Kind, response: &Value) -> Page {
    let data = &response["data"];
    Page {
        items: data["data"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| parse_item(kind, item))
                    .collect()
            })
            .unwrap_or_default(),
        has_next: data["has_next"].as_bool().unwrap_or(false),
    }
}

fn base(config: &Config, kind: Kind) -> Option<Url> {
    if !config.valid() {
        return None;
    }
    let mut url = Url::parse("https://api.klipy.com/api/v1/").ok()?;
    url.path_segments_mut()
        .ok()?
        .pop_if_empty()
        .push(&config.app_key)
        .push(kind.path());
    Some(url)
}

/// Trending while `query` is empty, else a search. Desktop sends no ad
/// parameters.
pub fn list_url(
    config: &Config,
    kind: Kind,
    query: &str,
    page: u32,
    locale: Option<&str>,
) -> Option<Url> {
    let mut url = base(config, kind)?;
    let query = query.trim();
    url.path_segments_mut().ok()?.push(if query.is_empty() {
        "trending"
    } else {
        "search"
    });
    {
        let mut pairs = url.query_pairs_mut();
        if !query.is_empty() {
            pairs.append_pair("q", query);
        }
        pairs
            .append_pair("page", &page.to_string())
            .append_pair("per_page", &PER_PAGE.to_string())
            .append_pair("customer_id", &config.customer_id)
            .append_pair("content_filter", "off");
        if let Some(locale) = locale {
            pairs.append_pair("locale", locale);
        }
    }
    Some(url)
}

fn share_url(config: &Config, kind: Kind, slug_value: &str) -> Option<Url> {
    if !slug(slug_value) {
        return None;
    }
    let mut url = base(config, kind)?;
    url.path_segments_mut().ok()?.push("share").push(slug_value);
    Some(url)
}

fn client() -> Option<&'static Client> {
    static CLIENT: OnceLock<Option<Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .timeout(Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .user_agent(concat!("Caper-Desktop/", env!("CARGO_PKG_VERSION")))
                .build()
                .ok()
        })
        .as_ref()
}

/// ISO 3166 alpha-2 from a POSIX locale such as `en_US.UTF-8` or a
/// BCP 47 tag such as `en-US`, lowercased.
fn region_of(locale: &str) -> Option<String> {
    let locale = locale.split(['.', '@']).next()?;
    let region = locale.split(['_', '-']).nth(1)?;
    (region.len() == 2 && region.bytes().all(|b| b.is_ascii_alphabetic()))
        .then(|| region.to_ascii_lowercase())
}

/// The device region, or `None` (the parameter is then left out).
fn region() -> Option<String> {
    static REGION: OnceLock<Option<String>> = OnceLock::new();
    REGION
        .get_or_init(|| {
            #[cfg(target_os = "windows")]
            {
                let international = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
                    .open_subkey("Control Panel\\International");
                if let Some(region) = international
                    .as_ref()
                    .ok()
                    .and_then(|key| key.open_subkey("Geo").ok())
                    .and_then(|key| key.get_value::<String, _>("Name").ok())
                    .and_then(|name| region_of(&format!("x-{name}")))
                    .or_else(|| {
                        international
                            .ok()
                            .and_then(|key| key.get_value::<String, _>("LocaleName").ok())
                            .and_then(|name| region_of(&name))
                    })
                {
                    return Some(region);
                }
            }
            ["LC_ALL", "LC_MESSAGES", "LANG"]
                .into_iter()
                .filter_map(|name| std::env::var(name).ok())
                .find(|value| !value.is_empty())
                .and_then(|value| region_of(&value))
        })
        .clone()
}

/// One page of trending or search results, from the device.
pub fn fetch(config: &Config, kind: Kind, query: &str, page: u32) -> Result<Page, String> {
    const FAILED: &str = "Couldn’t load GIFs. Try again.";
    let url = list_url(config, kind, query, page, region().as_deref()).ok_or(FAILED)?;
    let response = client()
        .ok_or(FAILED)?
        .get(url)
        .send()
        .map_err(|_| FAILED)?;
    if !response.status().is_success() {
        return Err(FAILED.into());
    }
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(response, MAX_RESPONSE_BYTES + 1),
        &mut bytes,
    )
    .map_err(|_| FAILED)?;
    if bytes.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(FAILED.into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| FAILED)?;
    if value["result"] == Value::Bool(false) {
        return Err(FAILED.into());
    }
    Ok(parse(kind, &value))
}

/// Tells KLIPY what was sent (after a successful send), off the UI thread.
/// Best effort: failures are ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Share {
    pub config: Config,
    pub kind: Kind,
    pub slug: String,
    /// The search text, empty for trending.
    pub query: String,
}

impl Share {
    pub fn send(self) {
        let (Some(url), Some(client)) = (share_url(&self.config, self.kind, &self.slug), client())
        else {
            return;
        };
        std::thread::spawn(move || {
            let _ = client
                .post(url)
                .json(&json!({"customer_id": self.config.customer_id, "q": self.query}))
                .send();
        });
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
            .id(egui::Id::new("gif-picker"))
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
                        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, name)
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
                            egui::Stroke::new(2.0, crate::TERRACOTTA),
                        );
                    }
                    if response.clicked() {
                        self.select_kind(ui.ctx(), config, kind);
                    }
                }
            });
            ui.add_space(10.0);
            let search_id = egui::Id::new("gif-picker-search");
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
                                            egui::WidgetType::Button,
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
                                            egui::Stroke::new(1.0, crate::TERRACOTTA),
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
                let remaining =
                    output.content_size.y - (output.state.offset.y + output.inner_rect.height());
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

/// One image to try: its URL and whether it loads as an animated GIF.
type Source<'a> = (&'a str, bool);

/// Animated first, else the still; reduced motion prefers the still, then
/// the GIF's first frame.
fn tile_sources(tile: &Tile, playing: bool) -> Vec<Source<'_>> {
    let gif = tile.gif.as_deref().map(|url| (url, true));
    let webp = Some((tile.url.as_str(), false)).filter(|_| tile.gif.as_deref() != Some(&tile.url));
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
    let webp = gif.webp.as_deref().map(|url| (url, false));
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

/// The attachment cache key for a KLIPY URL; such keys never ask Caper's API
/// to re-sign them.
pub fn media_key(url: &str) -> String {
    format!("{}{url}", crate::attachments::EXTERNAL_KEY)
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
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::hover());
    let label = gif.alt().to_owned();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, &label));
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
    response.on_hover_text(label)
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
        egui::Stroke::new(1.4, color),
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
        egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), "Send a GIF")
    });
    response.on_hover_text("Send a GIF")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cases() -> Value {
        serde_json::from_str(include_str!("../../../../shared/klipy/cases.json")).unwrap()
    }

    /// The shared case shape of a parsed item (desktop's extra tile GIF aside).
    fn case_json(item: &Item) -> Value {
        match item {
            Item::Ad {
                html,
                width,
                height,
            } => json!({"type": "ad", "html": html, "width": width, "height": height}),
            Item::Media {
                slug,
                title,
                tile,
                gif,
            } => {
                let mut tile_value = json!({"url": tile.url});
                if let Some(still) = &tile.still {
                    tile_value["still"] = json!(still);
                }
                if let Some(width) = tile.width {
                    tile_value["width"] = json!(width);
                }
                if let Some(height) = tile.height {
                    tile_value["height"] = json!(height);
                }
                json!({"type": "item", "slug": slug, "title": title, "tile": tile_value, "gif": gif.to_value()})
            }
        }
    }

    fn check_parse(case: &Value) {
        let kind = Kind::parse(case["kind"].as_str().unwrap()).unwrap();
        let page = parse(kind, &case["response"]);
        let items: Vec<Value> = page.items.iter().map(case_json).collect();
        assert_eq!(Value::Array(items), case["items"]);
        assert_eq!(page.has_next, case["hasNext"].as_bool().unwrap());
    }

    #[test]
    fn parses_shared_responses_in_klipy_order() {
        let cases = cases();
        check_parse(&cases["parse"]);
        check_parse(&cases["parseSticker"]);
    }

    #[test]
    fn tiles_keep_an_animated_gif_for_this_decoder() {
        let page = parse(Kind::Gif, &cases()["parse"]["response"]);
        let gifs: Vec<_> = page
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Media { tile, .. } => Some(tile.gif.as_deref()),
                Item::Ad { .. } => None,
            })
            .collect();
        assert_eq!(
            gifs,
            [
                Some("https://static.klipy.com/ii/935d/14/af/smgif.gif"),
                // xs has only WebP: the first size with a GIF.
                Some("https://static.klipy.com/ii/935d/14/af/stgif.gif"),
                None,
            ]
        );
    }

    #[test]
    fn validates_shared_messages() {
        let cases = cases();
        for valid in cases["messages"]["valid"].as_array().unwrap() {
            let gif = Gif::from_value(valid).unwrap_or_else(|| panic!("valid: {valid}"));
            assert_eq!(&gif.to_value(), valid);
        }
        for invalid in cases["messages"]["invalid"].as_array().unwrap() {
            assert!(Gif::from_value(invalid).is_none(), "invalid: {invalid}");
        }
    }

    #[test]
    fn applies_shared_url_rules() {
        for case in cases()["urls"].as_array().unwrap() {
            let url = case["url"].as_str().unwrap();
            assert_eq!(media_url(url), case["valid"].as_bool().unwrap(), "{url}");
        }
    }

    #[test]
    fn content_gif_never_rejects_a_message() {
        let valid = &cases()["messages"]["valid"][0];
        let content: crate::model::Content =
            serde_json::from_value(json!({"version":1,"type":"text","text":"","gif":valid}))
                .unwrap();
        assert_eq!(
            content.gif.as_ref().map(Gif::to_value).as_ref(),
            Some(valid)
        );
        // Serialized content keeps it, so cached messages round-trip.
        assert_eq!(&serde_json::to_value(&content).unwrap()["gif"], valid);
        for bad in [
            json!("x"),
            json!(null),
            json!({"provider":"giphy"}),
            json!(7),
        ] {
            let content: crate::model::Content =
                serde_json::from_value(json!({"version":1,"type":"text","text":"hi","gif":bad}))
                    .unwrap();
            assert!(content.gif.is_none());
            assert!(serde_json::to_value(&content).unwrap().get("gif").is_none());
        }
    }

    #[test]
    fn requests_follow_the_klipy_contract() {
        let config = Config {
            app_key: "fixtureKlipyKey".into(),
            customer_id: "0123456789abcdef0123456789abcdef".into(),
        };
        assert_eq!(
            list_url(&config, Kind::Gif, "  ", 1, Some("us"))
                .unwrap()
                .as_str(),
            "https://api.klipy.com/api/v1/fixtureKlipyKey/gifs/trending?page=1&per_page=24\
             &customer_id=0123456789abcdef0123456789abcdef&content_filter=off&locale=us"
        );
        assert_eq!(
            list_url(&config, Kind::Sticker, " hi & bye ", 3, None)
                .unwrap()
                .as_str(),
            "https://api.klipy.com/api/v1/fixtureKlipyKey/stickers/search?q=hi+%26+bye&page=3\
             &per_page=24&customer_id=0123456789abcdef0123456789abcdef&content_filter=off"
        );
        assert_eq!(
            share_url(&config, Kind::Gif, "hello-hi-662")
                .unwrap()
                .as_str(),
            "https://api.klipy.com/api/v1/fixtureKlipyKey/gifs/share/hello-hi-662"
        );
        assert!(share_url(&config, Kind::Gif, "../x").is_none());
        let dashed = Config {
            app_key: "key-with_dash".into(),
            ..config.clone()
        };
        assert!(
            list_url(&dashed, Kind::Gif, "", 1, None)
                .unwrap()
                .as_str()
                .starts_with("https://api.klipy.com/api/v1/key-with_dash/gifs/trending?")
        );
        for app_key in ["a/b", "", "a.b", "a b"] {
            let bad = Config {
                app_key: app_key.into(),
                ..config.clone()
            };
            assert!(
                list_url(&bad, Kind::Gif, "", 1, None).is_none(),
                "{app_key}"
            );
        }
    }

    #[test]
    fn regions_come_from_the_device_locale() {
        assert_eq!(region_of("en_US.UTF-8").as_deref(), Some("us"));
        assert_eq!(region_of("pt-BR").as_deref(), Some("br"));
        assert_eq!(region_of("de_DE@euro").as_deref(), Some("de"));
        assert_eq!(region_of("x-GB").as_deref(), Some("gb"));
        assert_eq!(region_of("C"), None);
        assert_eq!(region_of("C.UTF-8"), None);
        assert_eq!(region_of("zh_Hans_CN"), None);
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
                ("mdwebp.webp", false),
                ("mdjpg.jpg", false)
            ])
        );
        assert_eq!(
            urls(false),
            owned(&[
                ("mdjpg.jpg", false),
                ("mdgif.gif", true),
                ("mdwebp.webp", false)
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
