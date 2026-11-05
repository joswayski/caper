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

pub struct Row {
    pub height: f32,
    /// Body width the text layout is currently wrapped at.
    pub width: f32,
    pub text: IDWriteTextLayout,
    /// What selection copies and screen readers read.
    pub plain: String,
    pub plain16: Vec<u16>,
    pub mentions: Vec<(u32, u32)>,
    /// Links: UTF-16 start, length and address.
    pub links: Vec<(u32, u32, String)>,
    pub header: Option<Header>,
    pub text_y: f32,
    pub picture_y: f32,
    /// Each reaction chip's label, its width and whether it is yours.
    pub reactions: Vec<(IDWriteTextLayout, f32, bool)>,
    pub reactions_y: f32,
    /// A grouped row's "(edited)" marker, its width and its position
    /// from the text's origin.
    pub edited: Option<(IDWriteTextLayout, f32, f32, f32)>,
}

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
        self.row(i).map_or(0.0, |r| r.height)
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
                r.width = self.width;
                place(r, &c.messages[i]);
            }
            _ => {}
        }
        Ok(())
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
        let Some(old) = self.rows[i].as_ref().map(|r| r.height) else {
            return Ok(false);
        };
        let row = build(g, c, i, self.width)?;
        let delta = (row.height - old) as f64;
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

    /// A message was appended to the conversation.
    pub fn append(&mut self, g: &Gfx, c: &Conversation) -> Result<()> {
        let bottom = self.bottom();
        let i = self.rows.len();
        self.rows.push(None);
        self.top.push(bottom);
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

fn build(g: &Gfx, c: &Conversation, i: usize, width: f32) -> Result<Row> {
    let m = &c.messages[i];
    let (plain, spans, hrefs) = c.plain(i);
    let plain16 = gfx::wide(&plain);
    let text = g.layout(&plain16, &g.formats.message, width)?;
    let mut mentions = Vec::new();
    let links = hrefs
        .into_iter()
        .map(|(s, l, href)| (s as u32, l as u32, href))
        .collect();
    unsafe {
        for &(s, l, k) in &spans {
            let r = text_range(s, l);
            match k {
                SpanKind::Bold => text.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, r)?,
                SpanKind::Italic => text.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, r)?,
                SpanKind::Link => text.SetDrawingEffect(&g.brushes.link, r)?,
                SpanKind::Mention => {
                    text.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, r)?;
                    mentions.push((s as u32, l as u32));
                }
            }
        }
    }
    if m.deleted {
        unsafe { text.SetDrawingEffect(&g.brushes.muted, text_range(0, plain16.len()))? };
    }
    emoji::apply(&text, &plain, 15.0, INLINE_EMOJI_SCALE)?;
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
    let mut row = Row {
        height: 0.0,
        width,
        text,
        plain,
        plain16,
        mentions,
        links,
        header,
        text_y: 0.0,
        picture_y: 0.0,
        reactions,
        reactions_y: 0.0,
        edited,
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
    let mut y = if row.header.is_some() {
        NAME_H + 2.0
    } else {
        0.0
    };
    row.text_y = y;
    y += text_h;
    row.picture_y = y + 6.0;
    if let Some(p) = &m.picture {
        y += 6.0 + p.height as f32;
    }
    row.reactions_y = y + 6.0;
    if !row.reactions.is_empty() {
        y += 6.0 + REACTION_H;
    }
    let top = if m.grouped { 2.0 } else { 14.0 };
    let body = y.max(if m.grouped { 18.0 } else { AVATAR });
    row.height = (top + body + 4.0).ceil();
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
        if let Some(h) = &row.header {
            let a = rect(PAD_X, top, PAD_X + AVATAR, top + AVATAR);
            fill_round(dc, a, AVATAR * 0.3, &g.solid(c.members[m.author].color));
            crate::text::draw(dc, &h.initials, a.left, a.top, &g.brushes.text);
            crate::text::draw(dc, &h.name, BODY_X, top, &g.brushes.text);
            crate::text::draw(
                dc,
                &h.time,
                BODY_X + h.name_w + 8.0,
                top + 4.0,
                &g.brushes.muted,
            );
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
        let mut x = BODY_X;
        if let Some((l, _, ex, ey)) = &row.edited {
            crate::text::draw(dc, l, BODY_X + ex, ty + ey, &g.brushes.muted);
        }
        for (l, w, mine) in &row.reactions {
            let r = rect(
                x,
                top + row.reactions_y,
                x + w + 16.0,
                top + row.reactions_y + REACTION_H,
            );
            // Yours, as on web: a pressed terracotta chip.
            let (fill, border, label) = if *mine {
                (&g.brushes.pressed, &g.brushes.terracotta, &g.brushes.own)
            } else {
                (&g.brushes.raised, &g.brushes.border, &g.brushes.text)
            };
            fill_round(dc, r, 8.0, fill);
            stroke_round(dc, r, 8.0, border);
            crate::text::draw(dc, l, x + 8.0, r.top + 3.0, label);
            x = r.right + 6.0;
        }
    }
    Ok(())
}
