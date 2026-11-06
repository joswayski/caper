//! Message history, as in the egui client: the "(edited)" marker, or a
//! header time that carries it, and the message menu open a dialog that
//! pages through the retained versions. It compares the previous version
//! with the current one word by word, and any earlier version with the one
//! before it, with removed words in terracotta and added ones in green.
use crate::App;
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::overlay::{CARD_PAD, Dialog, Kind, Part, Rect, Tone};
use crate::theme::*;
use caper_client::edits::{diff, merge_versions, version_label};
use caper_client::model as core;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::Result;

/// What the history knows about the message's versions, newest first.
pub struct Versions {
    pub list: Vec<core::MessageVersion>,
    /// The earlier version picked to compare with the one before it.
    pub selected: Option<u32>,
    pub loading: bool,
    /// Whether the page in flight is an older one, appended to the list.
    pub older: bool,
    pub more: bool,
    /// The message's revision when its versions were last requested; a
    /// newer edit reloads them.
    pub requested_revision: u32,
}

/// One comparison column, or a lone version: its heading, its version's
/// name and time, its text and the UTF-16 ranges that changed.
struct Column {
    heading: String,
    label: Option<String>,
    text: String,
    marks: Vec<(u32, u32)>,
    tone: u32,
}

const WIDTH: f32 = 840.0;
const GAP: f32 = 8.0;
const BOX_PAD: f32 = 8.0;
const ROW_H: f32 = 30.0;
const CLOSE_H: f32 = 30.0;

fn layout(g: &Gfx, text: &str, format: &IDWriteTextFormat, w: f32) -> Result<IDWriteTextLayout> {
    let l = g.layout(&gfx::wide(text), format, w.max(1.0))?;
    crate::emoji::apply(&l, text, 12.0, 1.0)?;
    Ok(l)
}

fn height(l: &IDWriteTextLayout) -> f32 {
    gfx::metrics(l).height
}

fn width(l: &IDWriteTextLayout) -> f32 {
    gfx::metrics(l).widthIncludingTrailingWhitespace
}

fn title_of(revision: u32) -> String {
    if revision == 1 {
        "Original version".into()
    } else {
        format!("Version {revision}")
    }
}

/// `before` and `after` compared; `side` 0 is the before column.
fn compared(
    before: &core::MessageVersion,
    after: &core::MessageVersion,
    side: u8,
    current: bool,
) -> Column {
    let (old, new) = diff(&before.content.text, &after.content.text);
    let tokens = if side == 0 { old } else { new };
    let (mut text, mut marks, mut at) = (String::new(), Vec::new(), 0u32);
    for (token, changed) in tokens {
        let n = token.encode_utf16().count() as u32;
        if changed && n > 0 {
            marks.push((at, n));
        }
        at += n;
        text.push_str(&token);
    }
    let heading = match (current, side) {
        (true, 0) => "Previous version",
        (true, _) => "Current version",
        (false, 0) => "Before",
        (false, _) => "After",
    };
    Column {
        heading: heading.into(),
        label: Some(version_label(if side == 0 { before } else { after })),
        text,
        marks,
        tone: if side == 0 { TERRACOTTA } else { CAPER },
    }
}

/// A column's heading, version label and text laid out at width `w`.
fn column_layouts(
    g: &Gfx,
    c: &Column,
    w: f32,
) -> Result<(
    IDWriteTextLayout,
    Option<IDWriteTextLayout>,
    IDWriteTextLayout,
)> {
    Ok((
        layout(g, &c.heading, &g.formats.label, w)?,
        c.label
            .as_deref()
            .map(|l| layout(g, l, &g.formats.dialog_note, w))
            .transpose()?,
        layout(g, &c.text, &g.formats.mono, w - 2.0 * BOX_PAD)?,
    ))
}

fn column_height(
    (heading, label, text): &(
        IDWriteTextLayout,
        Option<IDWriteTextLayout>,
        IDWriteTextLayout,
    ),
) -> f32 {
    height(heading)
        + label.as_ref().map_or(0.0, |l| 2.0 + height(l))
        + 4.0
        + height(text)
        + 2.0 * BOX_PAD
}

impl Dialog {
    pub fn history(message: core::Message) -> Self {
        let revision = message.revision;
        let mut d = Self::new(Kind::History, message, Default::default());
        d.history = Some(Box::new(Versions {
            list: Vec::new(),
            selected: None,
            loading: false,
            older: false,
            more: false,
            requested_revision: revision,
        }));
        d
    }

    /// The picked earlier version and the one before it, when listed.
    fn picked(&self) -> Option<(&core::MessageVersion, Option<&core::MessageVersion>)> {
        let v = self.history.as_ref()?;
        let after = v.list.iter().find(|x| Some(x.revision) == v.selected)?;
        let before = v.list.iter().find(|x| x.revision + 1 == after.revision);
        Some((after, before))
    }

    /// Column `k`: 0 and 1 compare the previous version with the current
    /// one, 2 and 3 the picked version with the one before it; 4 is the
    /// only version and 5 a picked one whose predecessor isn't loaded.
    fn column(&self, k: u8) -> Option<Column> {
        let list = &self.history.as_ref()?.list;
        let lone = |v: &core::MessageVersion| Column {
            heading: title_of(v.revision),
            label: None,
            text: v.content.text.clone(),
            marks: Vec::new(),
            tone: TEXT,
        };
        match k {
            0 | 1 if list.len() > 1 => Some(compared(&list[1], &list[0], k, true)),
            2 | 3 => match self.picked()? {
                (after, Some(before)) => Some(compared(before, after, k - 2, false)),
                _ => None,
            },
            4 if list.len() == 1 => Some(lone(&list[0])),
            5 => match self.picked()? {
                (after, None) => Some(lone(after)),
                _ => None,
            },
            _ => None,
        }
    }

    /// The text of a history part, for drawing and UI Automation.
    pub fn history_text(&self, part: Part) -> String {
        let Some(v) = &self.history else {
            return String::new();
        };
        match part {
            Part::Choose => "View previous versions".into(),
            Part::Hint => "Load older versions to compare this change.".into(),
            Part::Loading => "Loading versions\u{2026}".into(),
            Part::Retry => "Retry".into(),
            Part::Older => "Load older versions".into(),
            Part::Close => "Close".into(),
            Part::Version(k) => v.list.get(k).map(version_label).unwrap_or_default(),
            Part::Column(k) => self.column(k).map_or_else(String::new, |c| {
                let mut s = c.heading;
                for l in c.label.iter().chain([&c.text]) {
                    s.push_str(", ");
                    s.push_str(l);
                }
                s
            }),
            _ => String::new(),
        }
    }

    /// Whether earlier version `k` is the one picked.
    pub fn version_selected(&self, k: usize) -> bool {
        self.history.as_ref().is_some_and(|v| {
            v.selected.is_some() && v.list.get(k).map(|x| x.revision) == v.selected
        })
    }

    pub(crate) fn relayout_history(&mut self, g: &Gfx, w: f32, h: f32) -> Result<()> {
        let card_w = WIDTH.min(w - 48.0).max(240.0);
        let inner = card_w - 2.0 * CARD_PAD;
        let mut parts = Vec::new();
        let close = layout(g, "Close", &g.formats.button, 1e6)?;
        let cw = width(&close) + 24.0;
        parts.push((Part::Close, (inner - cw, 0.0, cw, CLOSE_H)));
        let title = layout(g, self.title(), &g.formats.dialog_title, inner - cw - 8.0)?;
        parts.push((Part::Title, (0.0, 0.0, inner - cw - 8.0, height(&title))));
        let mut y = height(&title).max(CLOSE_H) + 2.0;
        let note = layout(g, &self.note(), &g.formats.dialog_note, inner)?;
        parts.push((Part::Note, (0.0, y, inner, height(&note))));
        y += height(&note) + 8.0;
        let body_top = y;
        let half = (inner - GAP) / 2.0;
        let pair = |parts: &mut Vec<(Part, Rect)>, y: &mut f32, a: u8| -> Result<()> {
            let (Some(l), Some(r)) = (self.column(a), self.column(a + 1)) else {
                return Ok(());
            };
            let lh = column_height(&column_layouts(g, &l, half)?);
            let rh = column_height(&column_layouts(g, &r, half)?);
            parts.push((Part::Column(a), (0.0, *y, half, lh)));
            parts.push((Part::Column(a + 1), (half + GAP, *y, half, rh)));
            *y += lh.max(rh) + 8.0;
            Ok(())
        };
        let single = |parts: &mut Vec<(Part, Rect)>, y: &mut f32, k: u8| -> Result<()> {
            if let Some(c) = self.column(k) {
                let ch = column_height(&column_layouts(g, &c, inner)?);
                parts.push((Part::Column(k), (0.0, *y, inner, ch)));
                *y += ch + 8.0;
            }
            Ok(())
        };
        let (count, loading, more) = self
            .history
            .as_ref()
            .map_or((0, false, false), |v| (v.list.len(), v.loading, v.more));
        let error = self.error.clone();
        pair(&mut parts, &mut y, 0)?;
        single(&mut parts, &mut y, 4)?;
        if count > 1 {
            y += 9.0;
            let l = layout(g, "View previous versions", &g.formats.note, inner)?;
            parts.push((Part::Choose, (0.0, y, inner, height(&l))));
            y += height(&l) + 6.0;
            for k in 1..count {
                parts.push((Part::Version(k), (0.0, y, inner, ROW_H)));
                y += ROW_H + 4.0;
            }
            y += 4.0;
            pair(&mut parts, &mut y, 2)?;
            single(&mut parts, &mut y, 5)?;
            if self
                .picked()
                .is_some_and(|(after, before)| before.is_none() && after.revision > 1 && more)
            {
                let l = layout(
                    g,
                    &self.history_text(Part::Hint),
                    &g.formats.dialog_note,
                    inner,
                )?;
                parts.push((Part::Hint, (0.0, y, inner, height(&l))));
                y += height(&l) + 8.0;
            }
        }
        if loading {
            let l = layout(g, &self.history_text(Part::Loading), &g.formats.note, inner)?;
            parts.push((Part::Loading, (0.0, y, inner, height(&l))));
            y += height(&l) + 8.0;
        }
        if let Some(e) = &error {
            let l = layout(g, e, &g.formats.error, inner)?;
            parts.push((Part::Error, (0.0, y, inner, height(&l))));
            y += height(&l) + 6.0;
            let l = layout(g, "Retry", &g.formats.button, 1e6)?;
            parts.push((Part::Retry, (0.0, y, width(&l) + 24.0, ROW_H)));
            y += ROW_H + 8.0;
        } else if more {
            let l = layout(g, "Load older versions", &g.formats.button, 1e6)?;
            parts.push((Part::Older, (0.0, y, width(&l) + 24.0, ROW_H)));
            y += ROW_H + 8.0;
        }
        // As egui's scroll area: the body grows to fit, then scrolls.
        let content = (y - 8.0 - body_top).max(0.0);
        let room = (h - 48.0 - 2.0 * CARD_PAD - body_top).clamp(120.0, 560.0);
        let view = content.min(room).max(24.0);
        self.scroll = self.scroll.clamp(0.0, (content - view).max(0.0));
        let card_h = body_top + view + 2.0 * CARD_PAD;
        let (cx, cy) = (
            ((w - card_w) / 2.0).round(),
            ((h - card_h) / 2.0).max(24.0).round(),
        );
        self.card = (cx, cy, card_w, card_h);
        for (p, r) in &mut parts {
            r.0 += cx + CARD_PAD;
            r.1 += cy + CARD_PAD;
            if !matches!(p, Part::Title | Part::Note | Part::Close) {
                r.1 -= self.scroll;
            }
        }
        self.body = (cx + CARD_PAD, cy + CARD_PAD + body_top, inner, view);
        self.parts = parts;
        Ok(())
    }

    pub(crate) fn draw_history(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        let (bx, by, bw, bh) = self.body;
        for (part, r) in &self.parts {
            let (x, y, w, _) = *r;
            match part {
                Part::Title => {
                    let l = layout(g, self.title(), &g.formats.dialog_title, w)?;
                    crate::text::draw(dc, &l, x, y, &g.brushes.text);
                }
                Part::Note => {
                    let l = layout(g, &self.note(), &g.formats.dialog_note, w)?;
                    crate::text::draw(dc, &l, x, y, &g.brushes.muted);
                }
                Part::Close => self.button(g, dc, *part, "Close", *r, Tone::Secondary)?,
                _ => {}
            }
        }
        unsafe {
            dc.PushAxisAlignedClip(&rect(bx, by, bx + bw, by + bh), D2D1_ANTIALIAS_MODE_ALIASED);
        }
        let drawn = self.draw_body(g, dc);
        unsafe {
            dc.PopAxisAlignedClip();
        }
        drawn
    }

    fn draw_body(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        let (_, by, _, bh) = self.body;
        for (part, r) in &self.parts {
            let (x, y, w, h) = *r;
            if !part_shown(*part) || y > by + bh || y + h < by {
                continue;
            }
            match part {
                Part::Column(k) => {
                    let Some(c) = self.column(*k) else { continue };
                    draw_column(g, dc, &c, x, y, w)?;
                }
                Part::Choose => unsafe {
                    dc.FillRectangle(&rect(x, y - 5.0, x + w, y - 4.0), &g.brushes.border);
                    let l = layout(g, &self.history_text(*part), &g.formats.note, w)?;
                    crate::text::draw(dc, &l, x, y, &g.brushes.text);
                },
                Part::Version(k) => {
                    let selected = self.version_selected(*k);
                    let (fill, border) = if selected {
                        (0x39231E, TERRACOTTA)
                    } else if self.pressed == Some(*part) && self.hover == Some(*part) {
                        (0x39231E, TERRACOTTA_BRIGHT)
                    } else if self.hover == Some(*part) {
                        (BORDER, TEXT_MUTED)
                    } else {
                        (SURFACE_RAISED, BORDER)
                    };
                    let body = rect(x, y, x + w, y + h);
                    fill_round(dc, body, 7.0, &g.solid(fill));
                    stroke_round(dc, body, 7.0, &g.solid(border));
                    let l = layout(g, &self.history_text(*part), &g.formats.picker, w - 20.0)?;
                    crate::text::draw(
                        dc,
                        &l,
                        x + 10.0,
                        y + (h - height(&l)) / 2.0,
                        &g.brushes.text,
                    );
                }
                Part::Hint | Part::Loading => {
                    let format = if *part == Part::Hint {
                        &g.formats.dialog_note
                    } else {
                        &g.formats.note
                    };
                    let l = layout(g, &self.history_text(*part), format, w)?;
                    crate::text::draw(dc, &l, x, y, &g.brushes.muted);
                }
                Part::Error => {
                    let l = layout(
                        g,
                        self.error.as_deref().unwrap_or_default(),
                        &g.formats.error,
                        w,
                    )?;
                    crate::text::draw(dc, &l, x, y, &g.brushes.error);
                }
                Part::Retry | Part::Older => {
                    let label = self.history_text(*part);
                    self.button(g, dc, *part, &label, *r, Tone::Secondary)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn part_shown(p: Part) -> bool {
    !matches!(p, Part::Title | Part::Note | Part::Close)
}

/// A column: its bold heading, muted version label, and the text in a
/// raised box with the changed words tinted, as egui's comparison.
fn draw_column(g: &Gfx, dc: &ID2D1DeviceContext, c: &Column, x: f32, y: f32, w: f32) -> Result<()> {
    let ls = column_layouts(g, c, w)?;
    let (heading, label, text) = &ls;
    crate::text::draw(dc, heading, x, y, &g.brushes.text);
    let mut ty = y + height(heading);
    if let Some(l) = label {
        ty += 2.0;
        crate::text::draw(dc, l, x, ty, &g.brushes.muted);
        ty += height(l);
    }
    ty += 4.0;
    let bh = height(text) + 2.0 * BOX_PAD;
    let body = rect(x, ty, x + w, ty + bh);
    fill_round(dc, body, RADIUS, &g.solid(SURFACE_RAISED));
    stroke_round(dc, body, RADIUS, &g.brushes.border);
    let (ox, oy) = (x + BOX_PAD, ty + BOX_PAD);
    if !c.marks.is_empty() {
        let tint = unsafe { dc.CreateSolidColorBrush(&gfx::color(c.tone, 0.3), None)? };
        for &(start, len) in &c.marks {
            for m in gfx::range_rects(text, start, len) {
                unsafe {
                    dc.FillRectangle(
                        &rect(
                            ox + m.left,
                            oy + m.top,
                            ox + m.left + m.width,
                            oy + m.top + m.height,
                        ),
                        &tint,
                    );
                }
            }
        }
    }
    crate::text::draw(dc, text, ox, oy, &g.brushes.text);
    Ok(())
}

impl App {
    /// Opens the history of `m`, an edited message that isn't a forward.
    pub fn open_history(&mut self, m: core::Message) {
        if m.forward.is_some() || m.is_deleted() {
            return;
        }
        self.close_menu();
        self.open_dialog(Dialog::history(m));
        self.history_load(false);
    }

    /// Opens the history of channel row `i`.
    pub fn open_history_of(&mut self, i: usize) {
        let Some(m) = self
            .c
            .messages
            .get(i)
            .and_then(|r| self.live.as_ref()?.message(&r.id))
        else {
            return;
        };
        self.open_history(m);
    }

    /// Requests the newest versions, or with `older` the page before the
    /// oldest listed one.
    pub fn history_load(&mut self, older: bool) {
        let (Some(d), Some(live)) = (
            self.dialog.as_mut().filter(|d| d.kind == Kind::History),
            &mut self.live,
        ) else {
            return;
        };
        let Some(v) = &mut d.history else { return };
        let before = if older {
            v.list.last().map(|x| x.revision)
        } else {
            None
        };
        v.loading = true;
        v.older = older;
        d.error = None;
        d.request = live.load_versions(&d.message, before);
        self.dialog_changed();
    }

    /// A page of versions, or why it failed, for request `request`.
    pub fn versions_loaded(
        &mut self,
        request: u64,
        message: &str,
        result: std::result::Result<core::MessageVersions, String>,
    ) {
        let Some(d) = self
            .dialog
            .as_mut()
            .filter(|d| d.kind == Kind::History && d.request == request && d.message.id == message)
        else {
            return;
        };
        let Some(v) = &mut d.history else { return };
        v.loading = false;
        match result {
            Ok(page) => {
                v.more = page.has_more;
                if !v.older {
                    v.selected = None;
                }
                merge_versions(&mut v.list, page.versions, v.older);
            }
            Err(e) => d.error = Some(e),
        }
        self.dialog_changed();
    }

    /// The latest copy of a message: a deletion closes its open history
    /// and a newer edit reloads its versions, as in egui.
    pub fn history_follow(&mut self, m: &core::Message) {
        let Some(d) = self
            .dialog
            .as_mut()
            .filter(|d| d.kind == Kind::History && d.message.id == m.id)
        else {
            return;
        };
        if m.is_deleted() {
            return self.close_dialog();
        }
        if let Some(v) = &mut d.history
            && m.revision > v.requested_revision
        {
            v.requested_revision = m.revision;
            self.history_load(false);
        }
    }

    /// Row `i`'s "(edited)" marker, or its header time when that carries
    /// the marker, in client DIPs while the row is laid out.
    pub fn edited_rect(&self, i: usize) -> Option<Rect> {
        if self.live.is_none() || i < self.rows.lo || i >= self.rows.len() {
            return None;
        }
        let m = self.c.messages.get(i)?;
        let r = self.rows.edited_marker(i, m.time.ends_with("(edited)"))?;
        let y = HEADER_H + (self.rows.top[i] - self.scroll_top) as f32;
        Some((
            self.list_x() + r.left,
            y + r.top,
            r.right - r.left,
            r.bottom - r.top,
        ))
    }

    /// The channel row whose edited marker is under client point (x, y).
    pub fn edited_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.in_history(x, y) {
            return None;
        }
        let ay = self.scroll_top + (y - HEADER_H) as f64;
        let i = self.rows.row_at(ay)?;
        let (l, t, w, h) = self.edited_rect(i)?;
        (x >= l && x < l + w && y >= t && y < t + h).then_some(i)
    }
}
