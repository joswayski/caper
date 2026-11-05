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

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::OnceLock;
use std::time::Duration;
use url::Url;

const MAX_URL: usize = 2048;
const MAX_TEXT: usize = 200;
const MAX_EDGE: u64 = 4096;
const PER_PAGE: u32 = 24;
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
}
