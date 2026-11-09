//! Link preview cards under a message, as the egui client draws them
//! (docs/media.md "Link embeds"): a bordered card per link with its site
//! line, then its blocks in order (author, title, text clamped to eight
//! lines, images, a direct video or a player's poster, footer), and a small
//! thumbnail beside the text when the card asks for one.
//!
//! Every string is plain text. Images and posters are unsigned and load
//! through the shared media cache under `caper_client::embeds::key`. A
//! card's images open in the media viewer (`viewer`) with the rest of their
//! block; there is no web player or in-app embed video yet, so videos and
//! players open in the browser (a player opens the card's link).
use crate::files::{Ctx, alpha, cover};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::theme::*;
use caper_client::embeds::{fit, fit_within, footer_line, gallery, heading, image_files, key};
use caper_client::media::ImageState;
use caper_client::model::{Embed, EmbedBlock};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::*;

/// Cards keep to web's `min(432px, 100%)`.
const CARD_W: f32 = 432.0;
const PAD_X: f32 = 12.0;
const PAD_TOP: f32 = 10.0;
const PAD_BOTTOM: f32 = 12.0;
const THUMB: f32 = 80.0;
const ICON: f32 = 20.0;
const GAP: f32 = 6.0;
const TEXT_ROWS: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    Meta,
    Author,
    Handle,
    Title,
    Text,
}

/// A line of a card's text: where it goes and what it opens.
pub struct Line {
    pub text: String,
    pub layout: IDWriteTextLayout,
    pub style: Style,
    pub target: Option<String>,
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Badge {
    Gif,
    Play,
    Player,
}

/// An image, poster, author icon or thumbnail and what it opens.
pub struct Picture {
    pub rect: D2D_RECT_F,
    /// The cache key and the URL it loads (none: a poster-less video).
    pub key: String,
    pub source: Option<String>,
    pub badge: Option<Badge>,
    pub round: bool,
    pub target: Option<String>,
    pub label: String,
}

/// One card's parts, relative to the cards' top-left once placed.
pub struct Card {
    pub rect: D2D_RECT_F,
    pub lines: Vec<Line>,
    pub pictures: Vec<Picture>,
    /// Block index of each line and picture, for placing.
    order: Vec<Slot>,
}

#[derive(Clone, Copy)]
enum Slot {
    Site,
    Line(usize),
    /// The author's icon, name and handle share a row.
    Author(Option<usize>, usize, Option<usize>),
    Images(usize, usize),
    Media(usize),
    Thumb(usize),
}

#[derive(Default)]
pub struct Embeds {
    pub cards: Vec<Card>,
    pub height: f32,
}

fn text(g: &Gfx, s: &str, format: &IDWriteTextFormat) -> Result<IDWriteTextLayout> {
    let l = g.layout(&gfx::wide(s), format, CARD_W)?;
    crate::emoji::apply(&l, s, 13.0, 1.0)?;
    Ok(l)
}

/// What a click on media opens: an http(s) link only, as message links.
fn openable(url: &str) -> Option<String> {
    (url.starts_with("https://") || url.starts_with("http://")).then(|| url.to_owned())
}

fn card(g: &Gfx, e: &Embed) -> Result<Card> {
    let mut lines = Vec::new();
    let mut pictures = Vec::new();
    let mut order = Vec::new();
    let line = |lines: &mut Vec<Line>, s: &str, style: Style, target: Option<&str>| {
        let format = match style {
            Style::Meta => &g.formats.embed_meta,
            Style::Author => &g.formats.embed_author,
            Style::Title => &g.formats.embed_title,
            Style::Handle | Style::Text => &g.formats.embed_text,
        };
        lines.push(Line {
            text: s.to_owned(),
            layout: text(g, s, format)?,
            style,
            target: target.and_then(openable),
            x: 0.0,
            y: 0.0,
        });
        Ok::<usize, Error>(lines.len() - 1)
    };
    let site = if e.site.name == e.site.domain {
        e.site.name.clone()
    } else {
        format!("{}  {}", e.site.name, e.site.domain)
    };
    line(&mut lines, &site, Style::Meta, Some(&e.url))?;
    order.push(Slot::Site);
    let name = heading(e).to_owned();
    for block in &e.blocks {
        match block {
            EmbedBlock::Author {
                name,
                handle,
                url,
                icon,
            } => {
                let icon = icon.as_ref().map(|icon| {
                    pictures.push(Picture {
                        rect: rect(0.0, 0.0, ICON, ICON),
                        key: key(icon),
                        source: Some(icon.clone()),
                        badge: None,
                        round: true,
                        target: None,
                        label: String::new(),
                    });
                    pictures.len() - 1
                });
                let n = line(&mut lines, name, Style::Author, url.as_deref())?;
                let h = match handle.as_deref().filter(|h| !h.is_empty()) {
                    Some(h) => Some(line(&mut lines, h, Style::Handle, url.as_deref())?),
                    None => None,
                };
                order.push(Slot::Author(icon, n, h));
            }
            EmbedBlock::Title { text, url } => {
                order.push(Slot::Line(line(
                    &mut lines,
                    text,
                    Style::Title,
                    url.as_deref(),
                )?));
            }
            EmbedBlock::Text { text } => {
                order.push(Slot::Line(line(&mut lines, text, Style::Text, None)?));
            }
            EmbedBlock::Images {
                items,
                thumbnail: true,
            } => {
                if let Some(image) = items.first() {
                    pictures.push(Picture {
                        rect: rect(0.0, 0.0, THUMB, THUMB),
                        key: key(&image.url),
                        source: Some(image.url.clone()),
                        badge: None,
                        round: false,
                        target: openable(&e.url),
                        label: format!("Open {}", image.alt.clone().unwrap_or(name.clone())),
                    });
                    order.push(Slot::Thumb(pictures.len() - 1));
                }
            }
            EmbedBlock::Images { items, .. } => {
                let first = pictures.len();
                for image in items.iter().take(4) {
                    let alt = image.alt.as_deref().filter(|a| !a.is_empty());
                    pictures.push(Picture {
                        rect: rect(0.0, 0.0, 0.0, 0.0),
                        key: key(&image.url),
                        source: Some(image.url.clone()),
                        badge: None,
                        round: false,
                        target: openable(&image.url),
                        label: format!("View {}", alt.unwrap_or("image")),
                    });
                }
                order.push(Slot::Images(first, pictures.len() - first));
            }
            EmbedBlock::Video {
                url, poster, gif, ..
            } => {
                pictures.push(Picture {
                    rect: rect(0.0, 0.0, 0.0, 0.0),
                    key: key(url),
                    source: poster.clone(),
                    badge: Some(if *gif { Badge::Gif } else { Badge::Play }),
                    round: false,
                    target: openable(url),
                    label: if *gif {
                        format!("Play animation {name}")
                    } else {
                        format!("Play {name}")
                    },
                });
                order.push(Slot::Media(pictures.len() - 1));
            }
            EmbedBlock::Player { poster, .. } => {
                pictures.push(Picture {
                    rect: rect(0.0, 0.0, 0.0, 0.0),
                    key: poster.as_deref().map(key).unwrap_or_default(),
                    source: poster.clone(),
                    badge: Some(Badge::Player),
                    round: false,
                    target: openable(&e.url),
                    label: format!("Play {name}"),
                });
                order.push(Slot::Media(pictures.len() - 1));
            }
            EmbedBlock::Footer { text, timestamp } => {
                let s = footer_line(text.as_deref(), timestamp.as_deref());
                if !s.is_empty() {
                    order.push(Slot::Line(line(&mut lines, &s, Style::Meta, None)?));
                }
            }
        }
    }
    Ok(Card {
        rect: rect(0.0, 0.0, 0.0, 0.0),
        lines,
        pictures,
        order,
    })
}

/// The cards' text and pictures, built once per row build.
pub fn build(g: &Gfx, embeds: &[Embed]) -> Result<Embeds> {
    Ok(Embeds {
        cards: embeds.iter().map(|e| card(g, e)).collect::<Result<_>>()?,
        height: 0.0,
    })
}

fn height(l: &IDWriteTextLayout) -> f32 {
    gfx::metrics(l).height
}

/// Lays out the cards in a body `width` wide; returns their height.
pub fn place(embeds: &mut Embeds, sources: &[Embed], width: f32) -> f32 {
    let card_w = width.clamp(60.0, CARD_W);
    let inner = (card_w - 2.0 * PAD_X).max(40.0);
    let mut top = 0.0;
    for (c, e) in embeds.cards.iter_mut().zip(sources) {
        let thumb = c.order.iter().any(|s| matches!(s, Slot::Thumb(_)));
        let body = if thumb {
            (inner - THUMB - 12.0).max(40.0)
        } else {
            inner
        };
        let mut y = PAD_TOP;
        let mut first = true;
        for slot in c.order.clone() {
            if !matches!(slot, Slot::Thumb(_)) && !first {
                y += GAP;
            }
            match slot {
                Slot::Site => {
                    let l = &mut c.lines[0];
                    unsafe { l.layout.SetMaxWidth(body).ok() };
                    (l.x, l.y) = (PAD_X, y);
                    y += height(&l.layout);
                }
                Slot::Line(k) => {
                    let l = &mut c.lines[k];
                    unsafe {
                        l.layout.SetMaxWidth(body).ok();
                        l.layout.SetMaxHeight(f32::MAX).ok();
                    }
                    if l.style == Style::Text {
                        clamp(&l.layout, TEXT_ROWS);
                    }
                    (l.x, l.y) = (PAD_X, y);
                    y += height(&l.layout);
                }
                Slot::Author(icon, n, h) => {
                    let mut x = PAD_X;
                    if let Some(i) = icon {
                        c.pictures[i].rect = rect(x, y, x + ICON, y + ICON);
                        x += ICON + 8.0;
                    }
                    let name_h = height(&c.lines[n].layout);
                    let row_h = name_h.max(if icon.is_some() { ICON } else { 0.0 });
                    for k in [Some(n), h].into_iter().flatten() {
                        let l = &mut c.lines[k];
                        let lw = gfx::metrics(&l.layout).widthIncludingTrailingWhitespace;
                        unsafe { l.layout.SetMaxWidth((PAD_X + body - x).max(1.0)).ok() };
                        (l.x, l.y) = (x, y + (row_h - height(&l.layout)) / 2.0);
                        x += lw + 6.0;
                    }
                    y += row_h;
                }
                Slot::Images(at, n) => {
                    let items = e.blocks.iter().find_map(|b| match b {
                        EmbedBlock::Images {
                            items,
                            thumbnail: false,
                        } if c.pictures[at].source.as_deref()
                            == items.first().map(|i| i.url.as_str()) =>
                        {
                            Some(items)
                        }
                        _ => None,
                    });
                    let slots = items.map(|items| gallery(items, body)).unwrap_or_default();
                    let mut bottom: f32 = 0.0;
                    for (k, [sx, sy, w, h]) in slots.into_iter().take(n).enumerate() {
                        c.pictures[at + k].rect =
                            rect(PAD_X + sx, y + sy, PAD_X + sx + w, y + sy + h);
                        bottom = bottom.max(sy + h);
                    }
                    y += bottom;
                }
                Slot::Media(k) => {
                    let (w, h) = e
                        .blocks
                        .iter()
                        .filter_map(|b| match b {
                            EmbedBlock::Video { width, height, .. }
                            | EmbedBlock::Player { width, height, .. } => Some((*width, *height)),
                            _ => None,
                        })
                        .nth(
                            c.order
                                .iter()
                                .take_while(|s| !matches!(s, Slot::Media(m) if *m == k))
                                .filter(|s| matches!(s, Slot::Media(_)))
                                .count(),
                        )
                        .unwrap_or((None, None));
                    let [w, h] = fit_within(fit(w, h), body);
                    c.pictures[k].rect = rect(PAD_X, y, PAD_X + w, y + h);
                    y += h;
                }
                Slot::Thumb(k) => {
                    let x = PAD_X + body + 12.0;
                    c.pictures[k].rect = rect(x, PAD_TOP, x + THUMB, PAD_TOP + THUMB);
                    continue;
                }
            }
            first = false;
        }
        if thumb {
            y = y.max(PAD_TOP + THUMB);
        }
        let h = y + PAD_BOTTOM;
        c.rect = rect(0.0, top, card_w, top + h);
        // Parts are kept relative to the card's own top-left.
        top += h + GAP;
    }
    embeds.height = (top - GAP).max(0.0);
    embeds.height
}

/// Stops a layout after `rows` lines (the trimming sign adds the ellipsis).
fn clamp(l: &IDWriteTextLayout, rows: usize) {
    let mut lines = vec![DWRITE_LINE_METRICS::default(); rows + 1];
    let mut count = 0;
    unsafe {
        let _ = l.GetLineMetrics(Some(&mut lines), &mut count);
        if count as usize > rows {
            let shown: f32 = lines[..rows].iter().map(|m| m.height).sum();
            let _ = l.SetMaxHeight(shown);
        }
    }
}

fn shift(r: D2D_RECT_F, x: f32, y: f32) -> D2D_RECT_F {
    rect(r.left + x, r.top + y, r.right + x, r.bottom + y)
}

/// Every clickable part, in reading order: its rectangle (relative to the
/// cards' top-left), what it opens and its accessible name.
pub fn targets(embeds: &Embeds) -> Vec<(D2D_RECT_F, String, String)> {
    let mut out = Vec::new();
    for c in &embeds.cards {
        let (cx, cy) = (c.rect.left, c.rect.top);
        for l in &c.lines {
            if let Some(t) = &l.target {
                let m = gfx::metrics(&l.layout);
                let r = rect(
                    l.x,
                    l.y,
                    l.x + m.widthIncludingTrailingWhitespace,
                    l.y + m.height,
                );
                out.push((shift(r, cx, cy), t.clone(), l.text.clone()));
            }
        }
        for p in &c.pictures {
            if let Some(t) = &p.target {
                out.push((shift(p.rect, cx, cy), t.clone(), p.label.clone()));
            }
        }
    }
    out
}

fn badge(g: &Gfx, dc: &ID2D1DeviceContext, r: D2D_RECT_F, b: Badge) -> Result<()> {
    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
    let circle = |radius: f32, brush: &ID2D1Brush| unsafe {
        dc.FillEllipse(
            &D2D1_ELLIPSE {
                point: windows_numerics::Vector2 { X: cx, Y: cy },
                radiusX: radius,
                radiusY: radius,
            },
            brush,
        );
    };
    match b {
        Badge::Gif => {
            let b = rect(r.left + 8.0, r.top + 8.0, r.left + 42.0, r.top + 28.0);
            fill_round(dc, b, 6.0, &alpha(dc, 0, 190.0 / 255.0)?);
            let l = g.layout(&gfx::wide("GIF"), &g.formats.file_name, 100.0)?;
            let m = gfx::metrics(&l);
            crate::text::draw(
                dc,
                &l,
                (b.left + b.right - m.widthIncludingTrailingWhitespace) / 2.0,
                (b.top + b.bottom - m.height) / 2.0,
                &g.brushes.text,
            );
        }
        Badge::Play => {
            circle(22.0, &alpha(dc, 0, 170.0 / 255.0)?.cast()?);
            icons::draw(
                dc,
                Icon::Play,
                rect(cx - 8.5, cy - 10.0, cx + 11.5, cy + 10.0),
                TEXT,
                1.0,
            );
        }
        Badge::Player => {
            // Web's eggshell 52px Play.
            circle(26.0, &alpha(dc, ACTION, 1.0)?.cast()?);
            icons::draw(
                dc,
                Icon::Play,
                rect(cx - 9.5, cy - 11.0, cx + 12.5, cy + 11.0),
                BLACKOUT,
                1.0,
            );
        }
    }
    Ok(())
}

fn picture(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    p: &Picture,
    ctx: Option<&mut Ctx>,
    r: D2D_RECT_F,
) -> Result<()> {
    let radius = if p.round {
        (r.right - r.left) / 2.0
    } else {
        6.0
    };
    fill_round(dc, r, radius, &g.solid(SURFACE_RAISED));
    if let (Some(c), Some(source)) = (ctx, &p.source)
        && matches!(c.media.image(&p.key, Some(source)), ImageState::Ready(_))
        && let Some(bmp) = c.media.texture(&p.key)
    {
        cover(dc, bmp, r, radius)?;
    }
    if let Some(b) = p.badge {
        badge(g, dc, r, b)?;
    }
    if !p.round {
        stroke_round(dc, r, radius, &g.brushes.border);
    }
    Ok(())
}

/// Draws the cards with their top-left at (x, y).
pub fn draw(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    embeds: &Embeds,
    mut ctx: Option<&mut Ctx>,
    x: f32,
    y: f32,
) -> Result<()> {
    for c in &embeds.cards {
        let r = shift(c.rect, x, y);
        fill_round(dc, r, RADIUS, &g.brushes.surface);
        stroke_round(dc, r, RADIUS, &g.brushes.border);
        for l in &c.lines {
            let brush = match l.style {
                Style::Meta | Style::Handle => &g.brushes.muted,
                _ => &g.brushes.text,
            };
            crate::text::draw(dc, &l.layout, r.left + l.x, r.top + l.y, brush);
        }
        for p in &c.pictures {
            picture(g, dc, p, ctx.as_deref_mut(), shift(p.rect, r.left, r.top))?;
        }
    }
    Ok(())
}

impl crate::App {
    /// The channel row and link preview part under client point (x, y).
    pub fn embed_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        if self.live.is_none() || !self.in_history(x, y) {
            return None;
        }
        let ay = self.scroll_top + (y - crate::theme::HEADER_H) as f64;
        let i = self.rows.row_at(ay)?;
        let m = self.c.messages.get(i)?;
        let (rx, ry) = (x - self.list_x(), (ay - self.rows.top[i]) as f32);
        let k = self
            .rows
            .embed_targets(i, m.grouped)
            .iter()
            .position(|(r, _, _)| rx >= r.left && rx < r.right && ry >= r.top && ry < r.bottom)?;
        Some((i, k))
    }

    /// Row `i`'s link preview parts, while it is laid out.
    pub fn embed_parts(&self, i: usize) -> Vec<(D2D_RECT_F, String, String)> {
        if self.live.is_none() || i < self.rows.lo || i >= self.rows.len() {
            return Vec::new();
        }
        self.c
            .messages
            .get(i)
            .map(|m| self.rows.embed_targets(i, m.grouped))
            .unwrap_or_default()
    }

    /// Part `k` of row `i`'s link previews in client DIPs.
    pub fn embed_rect(&self, i: usize, k: usize) -> Option<crate::overlay::Rect> {
        let (r, _, _) = self.embed_parts(i).into_iter().nth(k)?;
        let y = crate::theme::HEADER_H + (self.rows.top[i] - self.scroll_top) as f32;
        Some((
            self.list_x() + r.left,
            y + r.top,
            r.right - r.left,
            r.bottom - r.top,
        ))
    }

    pub fn embed_label(&self, i: usize, k: usize) -> String {
        self.embed_parts(i)
            .into_iter()
            .nth(k)
            .map(|(_, _, name)| name)
            .unwrap_or_default()
    }

    /// A click (or UIA Invoke) on a link preview part: an image opens in
    /// the viewer with the rest of its block, as in egui; anything else
    /// opens its link.
    pub fn embed_activate(&mut self, i: usize, k: usize) {
        let Some((_, target, label)) = self.embed_parts(i).into_iter().nth(k) else {
            return;
        };
        let images = label.starts_with("View ").then(|| {
            self.c.messages.get(i)?.embeds.iter().find_map(|e| {
                e.blocks.iter().find_map(|b| match b {
                    EmbedBlock::Images {
                        items,
                        thumbnail: false,
                    } => {
                        let index = items.iter().position(|image| image.url == target)?;
                        Some((image_files(items, heading(e)), index))
                    }
                    _ => None,
                })
            })
        });
        match images.flatten() {
            Some((items, index)) => self.open_viewer(items, index),
            None => crate::open_link(&target),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_links_open() {
        assert_eq!(
            openable("https://a.test/x").as_deref(),
            Some("https://a.test/x")
        );
        assert_eq!(openable("javascript:alert(1)"), None);
        assert_eq!(openable("file:///C:/x"), None);
    }
}
