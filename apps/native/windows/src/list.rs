//! Message rows of the virtual list: one DirectWrite layout per message at
//! the current body width, positions measured on demand from the newest row
//! upward (a row is laid out only once it nears the viewport), and drawing
//! of one row.
use crate::emoji;
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::model::{Conversation, SpanKind};
use std::collections::HashMap;
use std::ops::Range;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::*;

pub const AVATAR: f32 = 34.0;
pub const PAD_X: f32 = 18.0;
pub const GAP: f32 = 10.0;
pub const BODY_X: f32 = PAD_X + AVATAR + GAP;
const NAME_H: f32 = 20.0;
const REACTION_H: f32 = 24.0;
const INLINE_EMOJI_SCALE: f32 = 1.2;
/// The Add reaction button after the chips.
const ADD: f32 = 28.0;

pub struct Row {
    pub height: f32,
    /// Body width the text layout is currently wrapped at.
    pub width: f32,
    pub text: IDWriteTextLayout,
    /// What selection copies and screen readers read.
    pub plain: String,
    pub plain16: Vec<u16>,
    pub mentions: Vec<(u32, u32)>,
    /// Links: UTF-16 start, length and address. Those inside a hidden
    /// spoiler are left out, as are its mentions, until it is revealed.
    pub links: Vec<(u32, u32, String)>,
    /// Spoilers (neighbouring spoiler runs joined): UTF-16 start, length
    /// and whether it is still hidden (`spoiler_key` not in `revealed`).
    pub spoilers: Vec<(u32, u32, bool)>,
    /// Quote blocks (consecutive quoted lines joined): UTF-16 start and
    /// length, indented by `gfx::indent_quotes`.
    pub quotes: Vec<(u32, u32)>,
    pub header: Option<Header>,
    pub text_y: f32,
    pub picture_y: f32,
    /// The message's attachments, below its text.
    pub files: crate::files::Files,
    pub files_y: f32,
    /// Its link preview cards, below the files.
    pub embeds: crate::embeds::Embeds,
    pub embeds_y: f32,
    /// Each reaction chip's label, its width and whether it is yours.
    pub reactions: Vec<(IDWriteTextLayout, f32, bool)>,
    pub reactions_y: f32,
    /// "Add reaction" after the chips, on a live message.
    pub add: bool,
    /// A grouped row's "(edited)" marker, its width and its position
    /// from the text's origin.
    pub edited: Option<(IDWriteTextLayout, f32, f32, f32)>,
    /// The quoted message's panel above a reply.
    pub quote: Option<Quote>,
    /// Space above the header for the quote panel.
    pub header_y: f32,
    /// "Replied to a thread · View thread" above a thread broadcast's text.
    pub context: Option<IDWriteTextLayout>,
    pub context_y: f32,
    /// A thread root's "N replies" (and its width) and "View thread".
    pub thread: Option<(IDWriteTextLayout, f32, IDWriteTextLayout)>,
    pub thread_y: f32,
    /// A failed reaction's error (and its width), then Retry and Dismiss.
    pub error: Option<(IDWriteTextLayout, f32, [(IDWriteTextLayout, f32); 2])>,
    pub error_y: f32,
    /// The date divider above the row (its label and width) and the space
    /// it takes; everything else in the row sits below it.
    pub date: Option<(IDWriteTextLayout, f32)>,
    /// The "New messages" divider below the date divider (its label and
    /// width), as in egui.
    pub unread: Option<(IDWriteTextLayout, f32)>,
    pub lead: f32,
    /// "Pinned by …" above the message's header.
    pub pin: Option<IDWriteTextLayout>,
    pub pin_y: f32,
}

impl Row {
    /// The text screen readers read in the row's name: `plain`, with each
    /// hidden spoiler as "Spoiler" (its own button reveals it).
    pub fn spoken(&self) -> String {
        let mut out = Vec::with_capacity(self.plain16.len());
        let mut at = 0;
        for &(s, l, hidden) in &self.spoilers {
            if hidden {
                out.extend_from_slice(&self.plain16[at..s as usize]);
                out.extend("Spoiler".encode_utf16());
                at = (s + l) as usize;
            }
        }
        out.extend_from_slice(&self.plain16[at..]);
        String::from_utf16_lossy(&out)
    }
}

/// What a click on a row's controls does.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Toggle your reaction on this chip.
    Chip(usize),
    /// Open the reaction picker (after the chips).
    Add,
    Retry,
    Dismiss,
}

/// What `Rows::who_at` found: a person to open a profile card for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Who {
    Author,
    Quote,
    /// A mention pill's range in the row's plain text (UTF-16).
    Mention(usize, usize),
}

/// The egui client's reply panel: the quoted author's avatar and name,
/// "View original", and at most three lines of the quoted text.
pub struct Quote {
    pub initials: IDWriteTextLayout,
    pub color: u32,
    pub avatar: crate::avatars::Avatar,
    pub name: IDWriteTextLayout,
    pub view: IDWriteTextLayout,
    pub view_w: f32,
    pub text: IDWriteTextLayout,
    pub height: f32,
}

const QUOTE_PAD_X: f32 = 12.0;
const QUOTE_PAD_Y: f32 = 10.0;
const QUOTE_HEADING: f32 = 20.0;
const QUOTE_AVATAR: f32 = 16.0;
const QUOTE_LINES: usize = 3;
const THREAD_H: f32 = 22.0;
const ERROR_H: f32 = 24.0;
const DATE_H: f32 = 30.0;
const NEW_H: f32 = 20.0;
const PIN_H: f32 = 20.0;

pub struct Header {
    pub name: IDWriteTextLayout,
    pub time: IDWriteTextLayout,
    pub name_w: f32,
    pub initials: IDWriteTextLayout,
}

/// Row positions are DIPs relative to an anchor: the bottom of the newest
/// row present at startup is 0, older rows are negative, appended rows
/// positive.
pub struct Rows {
    pub width: f32,
    pub rows: Vec<Option<Row>>,
    pub top: Vec<f64>,
    /// Heights of measured rows, kept when a row far from the viewport
    /// drops its layouts (`trim`) so positions stay known.
    heights: Vec<f32>,
    /// Lowest index with a known position; [lo, len) is contiguous.
    pub lo: usize,
    pub anchor_bottom: f64,
    pictures: HashMap<usize, ID2D1Bitmap1>,
}

impl Rows {
    pub fn new(n: usize) -> Self {
        Self {
            width: 0.0,
            rows: (0..n).map(|_| None).collect(),
            top: vec![0.0; n],
            heights: vec![0.0; n],
            lo: n,
            anchor_bottom: 0.0,
            pictures: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn row(&self, i: usize) -> Option<&Row> {
        self.rows.get(i).and_then(Option::as_ref)
    }

    pub fn height(&self, i: usize) -> f32 {
        self.heights.get(i).copied().unwrap_or(0.0)
    }

    pub fn body_width(list_width: f32) -> f32 {
        (list_width - BODY_X - PAD_X).max(40.0)
    }

    /// Bottom of the newest row.
    pub fn bottom(&self) -> f64 {
        let n = self.rows.len();
        if self.lo < n {
            self.top[n - 1] + self.height(n - 1) as f64
        } else {
            self.anchor_bottom
        }
    }

    /// A new body width: positions restart from the bottom. Existing layouts
    /// are kept and only re-wrapped (SetMaxWidth keeps DirectWrite's shaping)
    /// when a row is next measured.
    pub fn set_width(&mut self, width: f32) {
        if (width - self.width).abs() < 0.01 {
            return;
        }
        self.anchor_bottom = self.bottom();
        self.width = width;
        self.lo = self.rows.len();
    }

    pub fn ensure(&mut self, g: &Gfx, c: &Conversation, i: usize) -> Result<()> {
        match &mut self.rows[i] {
            None => self.rows[i] = Some(build(g, c, i, self.width)?),
            Some(r) if (r.width - self.width).abs() > 0.01 => {
                unsafe { r.text.SetMaxWidth(self.width)? };
                if !r.quotes.is_empty() {
                    gfx::indent_quotes(&r.text, &r.plain16, &r.quotes)?;
                }
                r.width = self.width;
                place(r, &c.messages[i]);
            }
            _ => {}
        }
        self.heights[i] = self.rows[i].as_ref().map_or(0.0, |r| r.height);
        Ok(())
    }

    /// Drops the layouts of rows outside `keep`; their heights and positions
    /// stay, and `ensure` builds them again when they are next drawn.
    pub fn trim(&mut self, keep: Range<usize>) {
        for (i, row) in self.rows.iter_mut().enumerate() {
            if !keep.contains(&i) {
                *row = None;
            }
        }
    }

    /// Measures rows upward until the content above `y` is known or the
    /// first message is reached.
    pub fn measure_up_to(&mut self, g: &Gfx, c: &Conversation, y: f64) -> Result<()> {
        let n = self.rows.len();
        if self.lo == n && n > 0 {
            self.ensure(g, c, n - 1)?;
            self.top[n - 1] = self.anchor_bottom - self.height(n - 1) as f64;
            self.lo = n - 1;
        }
        while self.lo > 0 && self.top[self.lo] > y {
            let i = self.lo - 1;
            self.ensure(g, c, i)?;
            self.top[i] = self.top[i + 1] - self.height(i) as f64;
            self.lo = i;
        }
        Ok(())
    }

    /// Message `i` changed: an already built row is rebuilt at once and
    /// the measured rows below it move by its change in height. Returns
    /// whether the change has to be redrawn beyond the row itself.
    pub fn replace(&mut self, g: &Gfx, c: &Conversation, i: usize) -> Result<bool> {
        self.pictures.remove(&i);
        // Unmeasured rows wait; a trimmed one still has a position, so it is
        // rebuilt and the rows below move with it.
        if self.rows[i].is_none() && i < self.lo {
            return Ok(false);
        }
        let old = self.heights[i];
        let row = build(g, c, i, self.width)?;
        let delta = (row.height - old) as f64;
        self.heights[i] = row.height;
        self.rows[i] = Some(row);
        if delta == 0.0 {
            return Ok(false);
        }
        if i >= self.lo {
            for top in &mut self.top[i + 1..] {
                *top += delta;
            }
        }
        Ok(true)
    }

    /// `k` older messages were put before the shown ones: they start
    /// unmeasured above them, and the old first row (whose grouping and date
    /// divider can change) is measured again. Positions below it stay.
    pub fn prepend(&mut self, k: usize) {
        let mut rows: Vec<Option<Row>> = (0..k).map(|_| None).collect();
        rows.append(&mut self.rows);
        self.rows = rows;
        let mut top = vec![0.0; k];
        top.append(&mut self.top);
        self.top = top;
        let mut heights = vec![0.0; k];
        heights.append(&mut self.heights);
        self.heights = heights;
        self.pictures = std::mem::take(&mut self.pictures)
            .into_iter()
            .map(|(i, b)| (i + k, b))
            .collect();
        let n = self.rows.len();
        self.lo += k;
        if self.lo == k && k + 1 < n {
            self.rows[k] = None;
            self.lo = k + 1;
        } else if self.lo == k {
            self.rows[k] = None;
            self.lo = n;
        }
    }

    /// A message was appended to the conversation.
    pub fn append(&mut self, g: &Gfx, c: &Conversation) -> Result<()> {
        let bottom = self.bottom();
        let i = self.rows.len();
        self.rows.push(None);
        self.top.push(bottom);
        self.heights.push(0.0);
        if self.lo == i {
            self.anchor_bottom = bottom;
            self.lo = i + 1;
            return Ok(());
        }
        self.ensure(g, c, i)
    }

    /// Measured rows intersecting [y0, y1).
    pub fn range(&self, y0: f64, y1: f64) -> Range<usize> {
        let n = self.rows.len();
        if self.lo >= n {
            return n..n;
        }
        let (mut a, mut b) = (self.lo, n);
        while a < b {
            let m = (a + b) / 2;
            if self.top[m] + self.height(m) as f64 <= y0 {
                a = m + 1
            } else {
                b = m
            }
        }
        let mut end = a;
        while end < n && self.top[end] < y1 {
            end += 1;
        }
        a..end
    }

    pub fn row_at(&self, y: f64) -> Option<usize> {
        let r = self.range(y, y + 0.001);
        (r.start < r.end).then_some(r.start)
    }

    fn picture(&mut self, g: &Gfx, c: &Conversation, i: usize) -> Option<ID2D1Bitmap1> {
        let p = c.messages[i].picture.as_ref()?;
        if let Some(b) = self.pictures.get(&i) {
            return Some(b.clone());
        }
        let bmp = g
            .bitmap(
                p.width,
                p.height,
                &crate::model::demo_pixels(p.width, p.height, p.seed),
            )
            .ok()?;
        // Decoded pictures stay on the GPU; keep only those near the viewport.
        if self.pictures.len() > 96 {
            self.pictures.clear();
        }
        self.pictures.insert(i, bmp.clone());
        Some(bmp)
    }
}

fn text_range(start: usize, len: usize) -> DWRITE_TEXT_RANGE {
    DWRITE_TEXT_RANGE {
        startPosition: start as u32,
        length: len as u32,
    }
}

/// The key in a message's `revealed` set of its text spoiler starting at
/// UTF-16 offset `start`.
pub fn spoiler_key(start: u32) -> String {
    format!("spoiler:{start}")
}

fn build(g: &Gfx, c: &Conversation, i: usize, width: f32) -> Result<Row> {
    let m = &c.messages[i];
    let (plain, spans, hrefs) = c.plain(i);
    let plain16 = gfx::wide(&plain);
    let text = g.layout(&plain16, &g.formats.message, width)?;
    let mut mentions = Vec::new();
    let mut spoiled = Vec::new();
    let mut quotes = Vec::new();
    unsafe {
        for &(s, l, k) in &spans {
            let r = text_range(s, l);
            match k {
                SpanKind::Bold => text.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, r)?,
                SpanKind::Italic => text.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, r)?,
                SpanKind::Underline => text.SetUnderline(true, r)?,
                SpanKind::Strikethrough => text.SetStrikethrough(true, r)?,
                SpanKind::Spoiler => gfx::join_range(&mut spoiled, s, l),
                SpanKind::Quote => gfx::join_range(&mut quotes, s, l),
                SpanKind::Link => text.SetDrawingEffect(&g.brushes.link, r)?,
                SpanKind::Mention => {
                    text.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, r)?;
                    mentions.push((s as u32, l as u32));
                }
            }
        }
    }
    let spoilers: Vec<(u32, u32, bool)> = spoiled
        .into_iter()
        .map(|(s, l)| (s, l, !m.revealed.contains(&spoiler_key(s))))
        .collect();
    // A hidden spoiler's text, links and mentions can't be seen or used.
    let hidden = |p: u32, n: u32| {
        spoilers
            .iter()
            .any(|&(s, l, hidden)| hidden && p < s + l && s < p + n)
    };
    mentions.retain(|&(s, l)| !hidden(s, l));
    let links = hrefs
        .into_iter()
        .map(|(s, l, href)| (s as u32, l as u32, href))
        .filter(|&(s, l, _)| !hidden(s, l))
        .collect();
    for &(s, l, hidden) in &spoilers {
        if hidden {
            unsafe {
                text.SetDrawingEffect(&g.brushes.hidden, text_range(s as usize, l as usize))?
            };
        }
    }
    if m.deleted {
        unsafe { text.SetDrawingEffect(&g.brushes.muted, text_range(0, plain16.len()))? };
    }
    emoji::apply(&text, &plain, 15.0, INLINE_EMOJI_SCALE)?;
    if !quotes.is_empty() {
        gfx::indent_quotes(&text, &plain16, &quotes)?;
    }
    let edited = if m.edited_inline {
        let l = g.layout(&gfx::wide("(edited)"), &g.formats.edited, 10_000.0)?;
        let w = gfx::metrics(&l).widthIncludingTrailingWhitespace;
        Some((l, w, 0.0, 0.0))
    } else {
        None
    };
    let header = if m.grouped {
        None
    } else {
        let author = &c.members[m.author];
        let name = g.layout(&gfx::wide(&author.name), &g.formats.name, 10_000.0)?;
        emoji::apply(&name, &author.name, 15.0, 1.0)?;
        let time = g.layout(&gfx::wide(&m.time), &g.formats.time, 10_000.0)?;
        let initials = g.layout(&gfx::wide(&author.initials), &g.formats.initials, AVATAR)?;
        emoji::apply(&initials, &author.initials, 13.0, 1.0)?;
        unsafe { initials.SetMaxHeight(AVATAR)? };
        Some(Header {
            name_w: gfx::metrics(&name).widthIncludingTrailingWhitespace,
            name,
            time,
            initials,
        })
    };
    let mut reactions = Vec::new();
    for r in &m.reactions {
        let label = format!("{} {}", r.emoji, r.count);
        let l = g.layout(&gfx::wide(&label), &g.formats.reaction, 10_000.0)?;
        emoji::apply(&l, &label, 12.0, 1.35)?;
        reactions.push((
            l.clone(),
            gfx::metrics(&l).widthIncludingTrailingWhitespace,
            r.mine,
        ));
    }
    let quote = match &m.reply {
        Some(r) => {
            let (body, format) = if r.deleted {
                ("Original message was deleted.", &g.formats.quote_deleted)
            } else {
                (r.text.as_str(), &g.formats.quote)
            };
            let text = g.layout(&gfx::wide(body), format, width)?;
            emoji::apply(&text, body, 12.0, 1.0)?;
            let name = g.layout(&gfx::wide(&r.author.name), &g.formats.quote_name, 10_000.0)?;
            emoji::apply(&name, &r.author.name, 12.0, 1.0)?;
            let initials = g.layout(
                &gfx::wide(&r.author.initials),
                &g.formats.quote_initials,
                QUOTE_AVATAR,
            )?;
            emoji::apply(&initials, &r.author.initials, 7.0, 1.0)?;
            unsafe { initials.SetMaxHeight(QUOTE_AVATAR)? };
            // A deleted original cannot be opened.
            let view = g.layout(
                &gfx::wide(if r.deleted { "" } else { "View original" }),
                &g.formats.quote_meta,
                10_000.0,
            )?;
            Some(Quote {
                initials,
                color: r.author.color,
                avatar: r.author.avatar.clone(),
                name,
                view_w: gfx::metrics(&view).widthIncludingTrailingWhitespace,
                view,
                text,
                height: 0.0,
            })
        }
        None => None,
    };
    let context = if m.thread_context {
        Some(g.layout(
            &gfx::wide("Replied to a thread \u{b7} View thread"),
            &g.formats.quote_meta,
            10_000.0,
        )?)
    } else {
        None
    };
    let thread = match m.thread_replies {
        Some(n) => {
            let count = g.layout(
                &gfx::wide(&format!("{n} {}", if n == 1 { "reply" } else { "replies" })),
                &g.formats.thread_count,
                10_000.0,
            )?;
            let view = g.layout(&gfx::wide("View thread"), &g.formats.thread_view, 10_000.0)?;
            Some((
                count.clone(),
                gfx::metrics(&count).widthIncludingTrailingWhitespace,
                view,
            ))
        }
        None => None,
    };
    let error = match &m.reaction_error {
        Some(e) => {
            let l = g.layout(&gfx::wide(e), &g.formats.error, 10_000.0)?;
            let button = |s: &str| -> Result<(IDWriteTextLayout, f32)> {
                let l = g.layout(&gfx::wide(s), &g.formats.reaction, 10_000.0)?;
                let w = gfx::metrics(&l).widthIncludingTrailingWhitespace;
                Ok((l, w))
            };
            Some((
                l.clone(),
                gfx::metrics(&l).widthIncludingTrailingWhitespace,
                [button("Retry")?, button("Dismiss")?],
            ))
        }
        None => None,
    };
    let mut row = Row {
        height: 0.0,
        width,
        text,
        plain,
        plain16,
        mentions,
        links,
        spoilers,
        quotes,
        header,
        text_y: 0.0,
        picture_y: 0.0,
        files: crate::files::build(g, &m.attachments)?,
        files_y: 0.0,
        embeds: crate::embeds::build(g, &m.embeds)?,
        embeds_y: 0.0,
        add: c.is_live() && !m.deleted && !reactions.is_empty(),
        reactions,
        reactions_y: 0.0,
        edited,
        quote,
        header_y: 0.0,
        context,
        context_y: 0.0,
        thread,
        thread_y: 0.0,
        error,
        error_y: 0.0,
        date: match &m.date {
            Some(label) => {
                let l = g.layout(&gfx::wide(label), &g.formats.quote_meta, 10_000.0)?;
                let w = gfx::metrics(&l).widthIncludingTrailingWhitespace;
                Some((l, w))
            }
            None => None,
        },
        unread: if m.unread {
            let l = g.layout(&gfx::wide(NEW), &g.formats.quote_meta, 10_000.0)?;
            let w = gfx::metrics(&l).widthIncludingTrailingWhitespace;
            Some((l, w))
        } else {
            None
        },
        lead: 0.0,
        pin: match &m.pinned_by {
            Some(who) => Some(g.layout(
                &gfx::wide(&format!("Pinned by {who}")),
                &g.formats.quote_meta,
                10_000.0,
            )?),
            None => None,
        },
        pin_y: 0.0,
    };
    place(&mut row, m);
    Ok(row)
}

/// Vertical layout of a row from its wrapped text height.
fn place(row: &mut Row, m: &crate::model::Message) {
    let mut text_h = gfx::metrics(&row.text).height;
    if let Some((_, w, x, y)) = &mut row.edited {
        // After the text's last character, as egui places it; on its own
        // line when the last line is full.
        let (mut px, mut py) = (0.0, 0.0);
        let mut hit = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            let _ = row.text.HitTestTextPosition(
                row.plain16.len() as u32,
                false,
                &mut px,
                &mut py,
                &mut hit,
            );
        }
        let line = hit.height.max(15.0 * 1.35);
        (*x, *y) = (px + 4.0, hit.top + (line - 13.0) / 2.0);
        if *x + *w > row.width {
            (*x, *y) = (0.0, text_h + 2.0);
            text_h += 15.0;
        }
    }
    // The quote panel sits 4 DIPs below the row's top and 8 above the
    // message's own header, as in the egui client; positions below are from
    // the content top (`content_top`), so the panel starts above it.
    let top = content_top(m.grouped);
    row.lead = if row.date.is_some() { DATE_H } else { 0.0 }
        + if row.unread.is_some() { NEW_H } else { 0.0 };
    let mut y = row.lead;
    if let Some(q) = &mut row.quote {
        let w = (row.width - 2.0 * QUOTE_PAD_X).max(20.0);
        let mut lines = [DWRITE_LINE_METRICS::default(); QUOTE_LINES + 1];
        let mut count = 0;
        let shown = unsafe {
            let _ = q.text.SetMaxWidth(w);
            let _ = q.text.SetMaxHeight(f32::MAX);
            let _ = q.text.GetLineMetrics(Some(&mut lines), &mut count);
            let shown: f32 = lines[..(count as usize).min(QUOTE_LINES)]
                .iter()
                .map(|l| l.height)
                .sum();
            // Longer quotes stop after three lines with an ellipsis.
            if count as usize > QUOTE_LINES {
                let _ = q.text.SetMaxHeight(shown);
            }
            shown
        };
        q.height = QUOTE_PAD_Y + QUOTE_HEADING + 2.0 + shown + QUOTE_PAD_Y;
        y = row.lead + 4.0 - top + q.height + 8.0;
    }
    if row.pin.is_some() {
        row.pin_y = y;
        y += PIN_H;
    }
    row.header_y = y;
    if row.header.is_some() {
        y += NAME_H + 2.0;
    }
    if let Some(l) = &row.context {
        row.context_y = y;
        y += gfx::metrics(l).height + 2.0;
    }
    row.text_y = y;
    y += text_h;
    row.picture_y = y + 6.0;
    if let Some(p) = &m.picture {
        y += 6.0 + p.height as f32;
    }
    row.files_y = y + 6.0;
    if !row.files.list.is_empty() {
        let width = row.width;
        y += 6.0 + crate::files::place(&mut row.files, &m.attachments, &m.revealed, width);
    }
    row.embeds_y = y + 6.0;
    if !row.embeds.cards.is_empty() {
        let width = row.width;
        y += 6.0 + crate::embeds::place(&mut row.embeds, &m.embeds, width);
    }
    row.reactions_y = y + 6.0;
    if !row.reactions.is_empty() {
        y += 6.0 + REACTION_H;
    }
    row.thread_y = y + 4.0;
    if row.thread.is_some() {
        y += 4.0 + THREAD_H;
    }
    row.error_y = y + 4.0;
    if row.error.is_some() {
        y += 4.0 + ERROR_H;
    }
    let body = y.max(row.header_y + if m.grouped { 18.0 } else { AVATAR });
    row.height = (top + body + 4.0).ceil();
}

/// The row's reaction chips, then its Retry and Dismiss buttons, with the
/// content top at `top` (list coordinates).
fn controls(row: &Row, top: f32) -> Vec<(Action, Common::D2D_RECT_F)> {
    let mut out = Vec::new();
    let mut x = BODY_X;
    for (k, (_, w, _)) in row.reactions.iter().enumerate() {
        let y = top + row.reactions_y;
        out.push((Action::Chip(k), rect(x, y, x + w + 16.0, y + REACTION_H)));
        x += w + 16.0 + 6.0;
    }
    if row.add {
        let y = top + row.reactions_y - 2.0;
        out.push((Action::Add, rect(x, y, x + ADD, y + ADD)));
    }
    if let Some((_, w, buttons)) = &row.error {
        let y = top + row.error_y;
        let mut x = BODY_X + w + 10.0;
        for (action, (_, bw)) in [Action::Retry, Action::Dismiss].into_iter().zip(buttons) {
            out.push((action, rect(x, y, x + bw + 12.0, y + ERROR_H - 2.0)));
            x += bw + 12.0 + 6.0;
        }
    }
    out
}

impl Rows {
    /// Row `i`'s controls and their rectangles in list coordinates from the
    /// row's top.
    pub fn controls(&self, i: usize, grouped: bool) -> Vec<(Action, Common::D2D_RECT_F)> {
        self.row(i)
            .map(|row| controls(row, content_top(grouped)))
            .unwrap_or_default()
    }

    /// Row `i`'s link into a thread, from the row's top: a root's "N replies
    /// · View thread" or a broadcast reply's "Replied to a thread · View
    /// thread".
    pub fn thread_link(&self, i: usize, grouped: bool) -> Option<Common::D2D_RECT_F> {
        let row = self.row(i)?;
        let top = content_top(grouped);
        if let Some((_, w, view)) = &row.thread {
            let vw = gfx::metrics(view).widthIncludingTrailingWhitespace;
            let y = top + row.thread_y;
            return Some(rect(BODY_X, y, BODY_X + 6.0 + w + 8.0 + vw, y + THREAD_H));
        }
        let m = gfx::metrics(row.context.as_ref()?);
        let y = top + row.context_y;
        Some(rect(
            BODY_X,
            y,
            BODY_X + m.widthIncludingTrailingWhitespace,
            y + m.height,
        ))
    }
}

impl Rows {
    /// Row `i`'s attachment under (x, y) relative to the row's top-left:
    /// its index, and whether the point is on a revealed spoiler's "Hide
    /// spoiler" button.
    pub fn file_at(&self, i: usize, grouped: bool, x: f32, y: f32) -> Option<(usize, bool)> {
        let row = self.row(i)?;
        let (x, y) = (x - BODY_X, y - content_top(grouped) - row.files_y);
        let inside =
            |r: &Common::D2D_RECT_F| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
        let f = row.files.list.iter().find(|f| inside(&f.rect))?;
        Some((f.index, inside(&crate::files::hide_rect(f.rect).0)))
    }

    /// Row `i`'s link preview parts (rectangle relative to the row's
    /// top-left, what each opens, its name), in reading order.
    pub fn embed_targets(
        &self,
        i: usize,
        grouped: bool,
    ) -> Vec<(Common::D2D_RECT_F, String, String)> {
        let Some(row) = self.row(i) else {
            return Vec::new();
        };
        let (dx, dy) = (BODY_X, content_top(grouped) + row.embeds_y);
        crate::embeds::targets(&row.embeds)
            .into_iter()
            .map(|(r, t, n)| {
                (
                    rect(r.left + dx, r.top + dy, r.right + dx, r.bottom + dy),
                    t,
                    n,
                )
            })
            .collect()
    }

    /// Row `i`'s link preview picture `p` of card `c`, relative to the
    /// row's top-left.
    pub fn embed_picture(
        &self,
        i: usize,
        grouped: bool,
        c: usize,
        p: usize,
    ) -> Option<Common::D2D_RECT_F> {
        let row = self.row(i)?;
        let r = crate::embeds::picture_rect(&row.embeds, c, p)?;
        let (dx, dy) = (BODY_X, content_top(grouped) + row.embeds_y);
        Some(rect(r.left + dx, r.top + dy, r.right + dx, r.bottom + dy))
    }

    /// Row `i`'s attachment `k` relative to the row's top-left.
    pub fn file_rect(&self, i: usize, grouped: bool, k: usize) -> Option<Common::D2D_RECT_F> {
        let row = self.row(i)?;
        let f = row.files.list.iter().find(|f| f.index == k)?;
        let (dx, dy) = (BODY_X, content_top(grouped) + row.files_y);
        Some(rect(
            f.rect.left + dx,
            f.rect.top + dy,
            f.rect.right + dx,
            f.rect.bottom + dy,
        ))
    }

    /// Row `i`'s "(edited)" marker after its text, or with `in_time` its
    /// header's time, which carries the marker on an ungrouped row,
    /// relative to the row's top-left.
    pub fn edited_marker(&self, i: usize, in_time: bool) -> Option<Common::D2D_RECT_F> {
        let row = self.row(i)?;
        if let Some((l, w, x, y)) = &row.edited {
            let (x, y) = (BODY_X + x, row.text_y + y);
            return Some(rect(x, y, x + w, y + gfx::metrics(l).height));
        }
        let h = row.header.as_ref().filter(|_| in_time)?;
        let m = gfx::metrics(&h.time);
        let (x, y) = (BODY_X + h.name_w + 8.0, row.header_y + 4.0);
        Some(rect(
            x,
            y,
            x + m.widthIncludingTrailingWhitespace,
            y + m.height,
        ))
    }

    /// Row `i`'s "View original" in its quote, relative to the row's
    /// top-left.
    /// Who a point on row `i` names, in row-local DIPs (from the row's left
    /// edge and top): its author's avatar or name, its quoted author's, or
    /// a mention pill (its text range, joined across style runs).
    pub fn who_at(&self, i: usize, grouped: bool, x: f32, y: f32) -> Option<Who> {
        let row = self.row(i)?;
        let inside = |l: f32, t: f32, w: f32, h: f32| x >= l && x < l + w && y >= t && y < t + h;
        let top = content_top(grouped);
        if let Some(h) = &row.header {
            let hy = top + row.header_y;
            let name_h = gfx::metrics(&h.name).height;
            if inside(PAD_X, hy, AVATAR, AVATAR) || inside(BODY_X, hy, h.name_w, name_h) {
                return Some(Who::Author);
            }
        }
        if let Some(q) = &row.quote {
            let hy = row.lead + 4.0 + QUOTE_PAD_Y;
            let ax = BODY_X + QUOTE_PAD_X;
            let ay = hy + (QUOTE_HEADING - QUOTE_AVATAR) / 2.0;
            let name = gfx::metrics(&q.name);
            if inside(ax, ay, QUOTE_AVATAR, QUOTE_AVATAR)
                || inside(
                    ax + QUOTE_AVATAR + 6.0,
                    hy + 2.0,
                    name.widthIncludingTrailingWhitespace,
                    name.height,
                )
            {
                return Some(Who::Quote);
            }
        }
        let (tx, ty) = (x - BODY_X, y - (top + row.text_y));
        if row.mentions.is_empty() || !gfx::on_text(&row.text, tx, ty) {
            return None;
        }
        let p = gfx::hit_point(&row.text, tx, ty);
        let k = row
            .mentions
            .iter()
            .position(|&(s, l)| p >= s && p < s + l)?;
        let (mut start, mut end) = (row.mentions[k].0, row.mentions[k].0 + row.mentions[k].1);
        for &(s, l) in row.mentions[..k].iter().rev() {
            if s + l != start {
                break;
            }
            start = s;
        }
        for &(s, l) in &row.mentions[k + 1..] {
            if s != end {
                break;
            }
            end = s + l;
        }
        Some(Who::Mention(start as usize, end as usize))
    }

    /// Row `i`'s hidden spoiler under (x, y), in row-local DIPs as
    /// `who_at`: its index in `Row::spoilers`.
    pub fn spoiler_at(&self, i: usize, grouped: bool, x: f32, y: f32) -> Option<usize> {
        let row = self.row(i)?;
        let (tx, ty) = (x - BODY_X, y - (content_top(grouped) + row.text_y));
        row.spoilers.iter().position(|&(s, l, hidden)| {
            hidden
                && gfx::range_rects(&row.text, s, l).iter().any(|m| {
                    tx >= m.left && tx < m.left + m.width && ty >= m.top && ty < m.top + m.height
                })
        })
    }

    /// The bounds of row `i`'s spoiler `k` relative to the row's top-left.
    pub fn spoiler_rect(&self, i: usize, grouped: bool, k: usize) -> Option<Common::D2D_RECT_F> {
        let row = self.row(i)?;
        let &(s, l, _) = row.spoilers.get(k)?;
        let (dx, dy) = (BODY_X, content_top(grouped) + row.text_y);
        gfx::range_rects(&row.text, s, l)
            .iter()
            .map(|m| {
                rect(
                    dx + m.left,
                    dy + m.top,
                    dx + m.left + m.width,
                    dy + m.top + m.height,
                )
            })
            .reduce(|a, b| {
                rect(
                    a.left.min(b.left),
                    a.top.min(b.top),
                    a.right.max(b.right),
                    a.bottom.max(b.bottom),
                )
            })
    }

    /// Row `i`'s hidden spoilers, by index in `Row::spoilers`.
    pub fn hidden_spoilers(&self, i: usize) -> Vec<usize> {
        self.row(i).map_or_else(Vec::new, |row| {
            (0..row.spoilers.len())
                .filter(|&k| row.spoilers[k].2)
                .collect()
        })
    }

    pub fn quote_link(&self, i: usize) -> Option<Common::D2D_RECT_F> {
        let row = self.row(i)?;
        let q = row.quote.as_ref()?;
        let x = BODY_X + row.width - QUOTE_PAD_X - q.view_w;
        let y = row.lead + 4.0 + QUOTE_PAD_Y + 3.0;
        let h = gfx::metrics(&q.view).height;
        Some(rect(x, y, x + q.view_w, y + h))
    }
}

/// The row above the first message: "Load older messages", "Loading…",
/// the error and Retry, or "Beginning of conversation", as in egui.
pub const HEAD_H: f32 = 44.0;

/// The head's label and whether it is a button.
pub fn head_label(state: crate::live::Older) -> (&'static str, bool) {
    use crate::live::Older;
    match state {
        Older::More => ("Load older messages", true),
        Older::Loading => ("Loading\u{2026}", true),
        Older::Failed => ("Retry", true),
        Older::Start => ("Beginning of conversation", false),
    }
}

/// What a history head shows: a centred label or quiet bordered button,
/// optionally after a muted note (the error before Retry).
pub struct Head<'a> {
    pub label: &'a str,
    pub button: bool,
    pub enabled: bool,
    pub note: Option<&'a str>,
}

impl crate::live::Older {
    pub fn head(self) -> Head<'static> {
        let (label, button) = head_label(self);
        Head {
            label,
            button,
            enabled: self != crate::live::Older::Loading,
            note: (self == crate::live::Older::Failed).then_some(HEAD_ERROR),
        }
    }
}

/// The head's button (or label) rectangle relative to the head's top-left,
/// for a list `full` DIPs wide.
pub fn head_rect(g: &Gfx, state: crate::live::Older, full: f32) -> Result<Common::D2D_RECT_F> {
    head_button_rect(g, &state.head(), full)
}

pub fn head_button_rect(g: &Gfx, head: &Head, full: f32) -> Result<Common::D2D_RECT_F> {
    let format = if head.button {
        &g.formats.head_button
    } else {
        &g.formats.head
    };
    let w = gfx::metrics(&g.layout(&gfx::wide(head.label), format, 1e6)?)
        .widthIncludingTrailingWhitespace;
    let bw = if head.button { w + 20.0 } else { w };
    let mut x = (full - bw) / 2.0;
    if let Some(note) = head.note {
        // The note, then the button, centred together.
        let note = g.layout(&gfx::wide(note), &g.formats.head, 1e6)?;
        let nw = gfx::metrics(&note).widthIncludingTrailingWhitespace;
        x = (full - (nw + 8.0 + bw)) / 2.0 + nw + 8.0;
    }
    let h = if head.button { 27.0 } else { 16.0 };
    Ok(rect(x, (HEAD_H - h) / 2.0, x + bw, (HEAD_H + h) / 2.0))
}

const NEW: &str = "New messages";

const HEAD_ERROR: &str = "Couldn\u{2019}t load older messages.";

/// Draws the head with its top-left at (0, y).
pub fn draw_head(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    state: crate::live::Older,
    full: f32,
    y: f32,
    hover: bool,
) -> Result<()> {
    draw_head_row(g, dc, &state.head(), full, y, hover)
}

pub fn draw_head_row(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    head: &Head,
    full: f32,
    y: f32,
    hover: bool,
) -> Result<()> {
    let r = head_button_rect(g, head, full)?;
    let r = rect(r.left, r.top + y, r.right, r.bottom + y);
    if let Some(note) = head.note {
        let note = g.layout(&gfx::wide(note), &g.formats.head, 1e6)?;
        let nw = gfx::metrics(&note).widthIncludingTrailingWhitespace;
        let nh = gfx::metrics(&note).height;
        crate::text::draw(
            dc,
            &note,
            r.left - 8.0 - nw,
            y + (HEAD_H - nh) / 2.0,
            &g.brushes.muted,
        );
    }
    if !head.button {
        let l = g.layout(&gfx::wide(head.label), &g.formats.head, 1e6)?;
        crate::text::draw(dc, &l, r.left, r.top, &g.brushes.muted);
        return Ok(());
    }
    // Web's quiet bordered history button.
    let lit = head.enabled && hover;
    if lit {
        fill_round(dc, r, 7.0, &g.brushes.border);
    }
    stroke_round(
        dc,
        r,
        7.0,
        if lit {
            &g.brushes.muted
        } else {
            &g.brushes.border
        },
    );
    let l = g.layout(&gfx::wide(head.label), &g.formats.head_button, 1e6)?;
    let lh = gfx::metrics(&l).height;
    let brush = if lit {
        &g.brushes.text
    } else {
        &g.brushes.muted
    };
    crate::text::draw(dc, &l, r.left + 10.0, r.top + (27.0 - lh) / 2.0, brush);
    Ok(())
}

/// Top padding of a row's content.
pub fn content_top(grouped: bool) -> f32 {
    if grouped { 2.0 } else { 14.0 }
}

/// Draws row `i` with its top at `y` (DIPs, list coordinates); `sel` is the
/// selected UTF-16 range of its text.
pub fn draw_row(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    c: &Conversation,
    rows: &mut Rows,
    i: usize,
    y: f32,
    sel: Option<(u32, u32)>,
    mut files: Option<&mut crate::files::Ctx>,
) -> Result<()> {
    let m = &c.messages[i];
    let bitmap = if m.picture.is_some() {
        rows.picture(g, c, i)
    } else {
        None
    };
    let Some(row) = rows.row(i) else {
        return Ok(());
    };
    let top = y + content_top(m.grouped);
    unsafe {
        // egui's date divider: the muted label between two border rules.
        if let Some((l, w)) = &row.date {
            let full = BODY_X + row.width + PAD_X;
            let cy = y + row.lead / 2.0 + 4.0;
            let lx = (full - w) / 2.0;
            let rule = |x0: f32, x1: f32| {
                if x1 > x0 {
                    dc.FillRectangle(&rect(x0, cy, x1, cy + 1.0), &g.brushes.border);
                }
            };
            rule(PAD_X, lx - 10.0);
            rule(lx + w + 10.0, full - PAD_X);
            crate::text::draw(dc, l, lx, cy - 7.0, &g.brushes.muted);
        }
        // egui's "New messages": a terracotta rule, then its label.
        if let Some((l, w)) = &row.unread {
            let full = BODY_X + row.width + PAD_X;
            let ny = y + if row.date.is_some() { DATE_H } else { 0.0 };
            let lx = full - PAD_X - w;
            let cy = ny + NEW_H / 2.0;
            if lx - 8.0 > PAD_X {
                dc.FillRectangle(&rect(PAD_X, cy, lx - 8.0, cy + 1.0), &g.brushes.terracotta);
            }
            let h = gfx::metrics(l).height;
            crate::text::draw(dc, l, lx, cy - h / 2.0, &g.brushes.terracotta);
        }
        if let Some(q) = &row.quote {
            let qy = y + row.lead + 4.0;
            let p = rect(BODY_X, qy, BODY_X + row.width, qy + q.height);
            fill_round(dc, p, 8.0, &g.brushes.surface);
            stroke_round(dc, p, 8.0, &g.brushes.border_soft);
            let (x, hy) = (p.left + QUOTE_PAD_X, p.top + QUOTE_PAD_Y);
            let a = rect(
                x,
                hy + (QUOTE_HEADING - QUOTE_AVATAR) / 2.0,
                x + QUOTE_AVATAR,
                hy + (QUOTE_HEADING + QUOTE_AVATAR) / 2.0,
            );
            if !crate::avatars::draw(dc, &q.avatar, a, QUOTE_AVATAR * 0.3) {
                fill_round(dc, a, QUOTE_AVATAR * 0.3, &g.solid(q.color));
                crate::text::draw(dc, &q.initials, a.left, a.top, &g.brushes.text);
            }
            crate::text::draw(dc, &q.name, a.right + 6.0, hy + 2.0, &g.brushes.muted);
            crate::text::draw(
                dc,
                &q.view,
                p.right - QUOTE_PAD_X - q.view_w,
                hy + 3.0,
                &g.brushes.muted,
            );
            crate::text::draw(dc, &q.text, x, hy + QUOTE_HEADING + 2.0, &g.brushes.muted);
        }
        if let Some(l) = &row.pin {
            // egui's muted pin icon, drawn: a head and its needle.
            let (px, py) = (BODY_X, top + row.pin_y + 2.0);
            fill_round(
                dc,
                rect(px + 3.0, py, px + 9.0, py + 6.0),
                2.0,
                &g.brushes.muted,
            );
            dc.FillRectangle(
                &rect(px + 1.0, py + 6.0, px + 11.0, py + 7.5),
                &g.brushes.muted,
            );
            dc.FillRectangle(
                &rect(px + 5.4, py + 7.5, px + 6.6, py + 12.0),
                &g.brushes.muted,
            );
            crate::text::draw(dc, l, px + 16.0, py - 1.0, &g.brushes.muted);
        }
        let hy = top + row.header_y;
        if let Some(h) = &row.header {
            let a = rect(PAD_X, hy, PAD_X + AVATAR, hy + AVATAR);
            let author = &c.members[m.author];
            if !crate::avatars::draw(dc, &author.avatar, a, AVATAR * 0.3) {
                fill_round(dc, a, AVATAR * 0.3, &g.solid(author.color));
                crate::text::draw(dc, &h.initials, a.left, a.top, &g.brushes.text);
            }
            crate::text::draw(dc, &h.name, BODY_X, hy, &g.brushes.text);
            crate::text::draw(
                dc,
                &h.time,
                BODY_X + h.name_w + 8.0,
                hy + 4.0,
                &g.brushes.muted,
            );
        }
        if let Some(l) = &row.context {
            crate::text::draw(dc, l, BODY_X, top + row.context_y, &g.brushes.muted);
        }
        if let Some((count, w, view)) = &row.thread {
            let ty = top + row.thread_y + 3.0;
            crate::text::draw(dc, count, BODY_X + 6.0, ty, &g.brushes.terracotta_bright);
            crate::text::draw(dc, view, BODY_X + 6.0 + w + 8.0, ty, &g.brushes.muted);
        }
        let ty = top + row.text_y;
        let fill = |s: u32, l: u32, brush: &ID2D1SolidColorBrush, pad: f32, radius: f32| {
            for h in gfx::range_rects(&row.text, s, l) {
                fill_round(
                    dc,
                    rect(
                        BODY_X + h.left - pad,
                        ty + h.top,
                        BODY_X + h.left + h.width + pad,
                        ty + h.top + h.height,
                    ),
                    radius,
                    brush,
                );
            }
        };
        // Quote blocks: web's 3px border bar beside the indented lines.
        for &(s, l) in &row.quotes {
            if let Some((t, b)) = gfx::range_extent(&row.text, s, l) {
                dc.FillRectangle(
                    &rect(BODY_X, ty + t, BODY_X + gfx::QUOTE_BAR, ty + b),
                    &g.brushes.border,
                );
            }
        }
        // A hidden spoiler is a border-coloured block (its text is drawn
        // invisible); a revealed one keeps a faint background.
        for &(s, l, hidden) in &row.spoilers {
            let brush = if hidden {
                &g.brushes.border
            } else {
                &g.brushes.hover
            };
            fill(s, l, brush, 1.0, 4.0);
        }
        for &(s, l) in &row.mentions {
            fill(s, l, &g.brushes.mention, 2.0, 4.0);
        }
        for &(s, l, _) in &row.links {
            fill(s, l, &g.brushes.link_bg, 2.0, 4.0);
        }
        if let Some((s, e)) = sel
            && e > s
        {
            fill(s, e - s, &g.brushes.selection, 0.0, 0.0);
        }
        crate::text::draw(dc, &row.text, BODY_X, ty, &g.brushes.text);
        if let (Some(p), Some(bmp)) = (&m.picture, bitmap) {
            let r = rect(
                BODY_X,
                top + row.picture_y,
                BODY_X + p.width as f32,
                top + row.picture_y + p.height as f32,
            );
            let brush = dc.CreateBitmapBrush(&bmp, None, None)?;
            brush.SetTransform(&windows_numerics::Matrix3x2 {
                M11: 1.0,
                M12: 0.0,
                M21: 0.0,
                M22: 1.0,
                M31: r.left,
                M32: r.top,
            });
            dc.FillRoundedRectangle(
                &D2D1_ROUNDED_RECT {
                    rect: r,
                    radiusX: 8.0,
                    radiusY: 8.0,
                },
                &brush,
            );
        }
        if let Some((l, _, ex, ey)) = &row.edited {
            crate::text::draw(dc, l, BODY_X + ex, ty + ey, &g.brushes.muted);
        }
        if !row.files.list.is_empty() {
            crate::files::draw(
                g,
                dc,
                &row.files,
                &m.attachments,
                &m.revealed,
                files.as_deref_mut(),
                i,
                BODY_X,
                top + row.files_y,
            )?;
        }
        if !row.embeds.cards.is_empty() {
            crate::embeds::draw(g, dc, &row.embeds, files, i, BODY_X, top + row.embeds_y)?;
        }
        if let Some((l, _, _)) = &row.error {
            crate::text::draw(dc, l, BODY_X, top + row.error_y + 3.0, &g.brushes.error);
        }
        for (action, r) in controls(row, top) {
            if action == Action::Add {
                let i = rect(r.left + 5.0, r.top + 5.0, r.right - 5.0, r.bottom - 5.0);
                crate::icons::draw(
                    dc,
                    crate::icons::Icon::SmilePlus,
                    i,
                    crate::theme::TEXT_MUTED,
                    1.0,
                );
                continue;
            }
            let (l, fill, border, label) = match action {
                Action::Chip(k) => {
                    let (l, _, mine) = &row.reactions[k];
                    // Yours, as on web: a pressed terracotta chip.
                    if *mine {
                        (l, &g.brushes.pressed, &g.brushes.terracotta, &g.brushes.own)
                    } else {
                        (l, &g.brushes.raised, &g.brushes.border, &g.brushes.text)
                    }
                }
                Action::Add => continue,
                Action::Retry | Action::Dismiss => {
                    let Some((_, _, buttons)) = &row.error else {
                        continue;
                    };
                    let (l, _) = &buttons[usize::from(action == Action::Dismiss)];
                    (l, &g.brushes.raised, &g.brushes.border, &g.brushes.text)
                }
            };
            // Chips round and pad by 8, the small buttons by 6.
            let pad = if matches!(action, Action::Chip(_)) {
                8.0
            } else {
                6.0
            };
            fill_round(dc, r, pad, fill);
            stroke_round(dc, r, pad, border);
            crate::text::draw(dc, l, r.left + pad, r.top + 3.0, label);
        }
    }
    Ok(())
}
