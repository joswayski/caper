//! Message rows: DirectWrite layouts per message at the current body width, bottom-anchored
//! positions measured on demand (only rows that have been near the viewport are ever laid out),
//! and drawing of one row in DIPs.
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use fixture::{Fixture, Seg};
use std::collections::HashMap;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::*;

pub const AVATAR: f32 = 34.0;
pub const PAD_X: f32 = 18.0;
pub const GAP: f32 = 10.0;

pub struct Row {
    pub height: f32,
    /// Body width the text layout is currently wrapped at.
    pub width: f32,
    pub initials: Option<IDWriteTextLayout>,
    pub text: IDWriteTextLayout,
    pub plain: Vec<u16>,
    pub mentions: Vec<(u32, u32)>,
    pub header: Option<(IDWriteTextLayout, IDWriteTextLayout, f32)>,
    pub text_y: f32,
    pub image_y: f32,
    pub reactions: Vec<(IDWriteTextLayout, f32)>,
    pub reactions_y: f32,
}

/// Row positions are DIPs relative to an anchor: the bottom of the last message present at
/// startup is 0, older rows are negative, appended rows positive.
pub struct Rows {
    pub width: f32,
    pub rows: Vec<Option<Row>>,
    pub top: Vec<f64>,
    /// Lowest index with a known position; [lo, len) is contiguous.
    pub lo: usize,
    pub anchor_bottom: f64,
    pub images: HashMap<usize, ID2D1Bitmap1>,
}

impl Rows {
    pub fn new(n: usize) -> Self {
        Self {
            width: 0.0,
            rows: (0..n).map(|_| None).collect(),
            top: vec![0.0; n],
            lo: n,
            anchor_bottom: 0.0,
            images: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn body_width(center_width: f32) -> f32 {
        (center_width - 2.0 * PAD_X - AVATAR - GAP).max(40.0)
    }

    /// Bottom of the newest row.
    pub fn bottom(&self) -> f64 {
        match self.rows.len() {
            0 => 0.0,
            n if self.lo < n => {
                self.top[n - 1] + self.rows[n - 1].as_ref().map_or(0.0, |r| r.height as f64)
            }
            _ => 0.0,
        }
    }

    /// New body width: positions restart from the bottom; existing layouts are kept and only
    /// re-wrapped (SetMaxWidth keeps DirectWrite's shaping) when a row is next measured.
    pub fn set_width(&mut self, width: f32) {
        if (width - self.width).abs() < 0.01 {
            return;
        }
        let bottom = self.bottom();
        self.width = width;
        self.lo = self.rows.len();
        self.anchor_bottom = bottom;
    }

    pub fn ensure(&mut self, g: &Gfx, fx: &Fixture, i: usize) -> Result<()> {
        match &mut self.rows[i] {
            None => self.rows[i] = Some(build(g, fx, i, self.width)?),
            Some(r) if (r.width - self.width).abs() > 0.01 => {
                unsafe { r.text.SetMaxWidth(self.width)? };
                r.width = self.width;
                place(r, &fx.messages[i]);
            }
            _ => {}
        }
        Ok(())
    }

    /// Measures rows upward until the content above `y` (anchor-relative DIPs) is known or the
    /// first message is reached.
    pub fn measure_up_to(&mut self, g: &Gfx, fx: &Fixture, y: f64) -> Result<()> {
        let n = self.rows.len();
        if self.lo == n && n > 0 {
            self.ensure(g, fx, n - 1)?;
            let h = self.rows[n - 1].as_ref().unwrap().height as f64;
            self.top[n - 1] = self.anchor_bottom - h;
            self.lo = n - 1;
        }
        while self.lo > 0 && self.top[self.lo] > y {
            let i = self.lo - 1;
            self.ensure(g, fx, i)?;
            self.top[i] = self.top[i + 1] - self.rows[i].as_ref().unwrap().height as f64;
            self.lo = i;
        }
        Ok(())
    }

    pub fn append(&mut self, g: &Gfx, fx: &Fixture) -> Result<()> {
        let bottom = self.bottom();
        let i = self.rows.len();
        self.rows.push(None);
        self.top.push(bottom);
        if self.lo == i {
            // nothing measured yet: keep the empty state consistent
            self.anchor_bottom = bottom;
        }
        self.ensure(g, fx, i)?;
        if self.lo > i {
            self.lo = i;
        }
        Ok(())
    }

    /// Index range of measured rows intersecting [y0, y1).
    pub fn range(&self, y0: f64, y1: f64) -> std::ops::Range<usize> {
        let n = self.rows.len();
        if self.lo >= n {
            return n..n;
        }
        let h = |i: usize| self.rows[i].as_ref().map_or(0.0, |r| r.height as f64);
        // first row whose bottom > y0
        let mut a = self.lo;
        let mut b = n;
        while a < b {
            let m = (a + b) / 2;
            if self.top[m] + h(m) <= y0 {
                a = m + 1
            } else {
                b = m
            }
        }
        let start = a;
        let mut end = start;
        while end < n && self.top[end] < y1 {
            end += 1;
        }
        start..end
    }

    pub fn row_at(&self, y: f64) -> Option<usize> {
        let r = self.range(y, y + 0.001);
        (r.start < r.end).then_some(r.start)
    }

    pub fn image(
        &mut self,
        dc: &ID2D1DeviceContext,
        fx: &Fixture,
        i: usize,
    ) -> Option<ID2D1Bitmap1> {
        let (w, h, seed) = fx.messages[i].image?;
        if let Some(b) = self.images.get(&i) {
            return Some(b.clone());
        }
        let rgba = fixture::image_pixels(w, h, seed);
        let mut bgra = Vec::with_capacity(rgba.len());
        for p in rgba.chunks_exact(4) {
            bgra.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let bmp = unsafe {
            dc.CreateBitmap(
                D2D_SIZE_U {
                    width: w,
                    height: h,
                },
                Some(bgra.as_ptr() as _),
                w * 4,
                &props,
            )
            .ok()?
        };
        if self.images.len() > 96 {
            self.images.clear();
        }
        self.images.insert(i, bmp.clone());
        Some(bmp)
    }
}

fn range(start: usize, len: usize) -> DWRITE_TEXT_RANGE {
    DWRITE_TEXT_RANGE {
        startPosition: start as u32,
        length: len as u32,
    }
}

pub fn message_plain(fx: &Fixture, i: usize) -> (Vec<u16>, Vec<(usize, usize, u8)>) {
    let mut out: Vec<u16> = Vec::new();
    let mut spans = Vec::new();
    for seg in &fx.messages[i].segs {
        let start = out.len();
        let kind = match seg {
            Seg::Text(t) => {
                out.extend(t.encode_utf16());
                0
            }
            Seg::Bold(t) => {
                out.extend(t.encode_utf16());
                1
            }
            Seg::Italic(t) => {
                out.extend(t.encode_utf16());
                2
            }
            Seg::Link(u) => {
                out.extend(u.encode_utf16());
                3
            }
            Seg::Mention(a) => {
                out.push('@' as u16);
                out.extend(fx.authors[*a].name.encode_utf16());
                4
            }
        };
        if kind != 0 {
            spans.push((start, out.len() - start, kind));
        }
    }
    (out, spans)
}

fn build(g: &Gfx, fx: &Fixture, i: usize, width: f32) -> Result<Row> {
    let m = &fx.messages[i];
    let (plain, spans) = message_plain(fx, i);
    let text = g.layout(&plain, &g.formats.message, width)?;
    let mut mentions = Vec::new();
    unsafe {
        for (s, l, k) in &spans {
            let r = range(*s, *l);
            match k {
                1 => text.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, r)?,
                2 => text.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, r)?,
                3 => text.SetDrawingEffect(&g.brushes.link, r)?,
                _ => {
                    text.SetFontWeight(DWRITE_FONT_WEIGHT_MEDIUM, r)?;
                    mentions.push((*s as u32, *l as u32));
                }
            }
        }
    }
    let header = if m.grouped {
        None
    } else {
        let name = g.layout(
            &gfx::wide(&fx.authors[m.author].name),
            &g.formats.name,
            10_000.0,
        )?;
        let time = g.layout(&gfx::wide(&m.time), &g.formats.time, 10_000.0)?;
        let nw = gfx::metrics(&name).widthIncludingTrailingWhitespace;
        Some((name, time, nw))
    };
    let initials = if m.grouped {
        None
    } else {
        let l = g.layout(
            &gfx::wide(&fx.authors[m.author].initials),
            &g.formats.initials,
            AVATAR,
        )?;
        unsafe { l.SetMaxHeight(AVATAR)? };
        Some(l)
    };
    let mut reactions = Vec::new();
    for (emoji, count) in &m.reactions {
        let l = g.layout(
            &gfx::wide(&format!("{emoji} {count}")),
            &g.formats.reaction,
            10_000.0,
        )?;
        let w = gfx::metrics(&l).widthIncludingTrailingWhitespace;
        reactions.push((l, w));
    }
    let mut row = Row {
        height: 0.0,
        width,
        initials,
        text,
        plain,
        mentions,
        header,
        text_y: 0.0,
        image_y: 0.0,
        reactions,
        reactions_y: 0.0,
    };
    place(&mut row, m);
    Ok(row)
}

/// Vertical layout of a row from its wrapped text height.
fn place(row: &mut Row, m: &fixture::Message) {
    let text_h = gfx::metrics(&row.text).height;
    let mut y = if row.header.is_some() {
        18.0 + 4.0
    } else {
        0.0
    };
    row.text_y = y;
    y += text_h;
    row.image_y = y + 4.0;
    if let Some((_, h, _)) = m.image {
        y += 4.0 + h as f32;
    }
    row.reactions_y = y + 4.0;
    if !row.reactions.is_empty() {
        y += 4.0 + 20.0;
    }
    let top = if m.grouped { 2.0 } else { 10.0 };
    let body = y.max(if m.grouped { 18.0 } else { AVATAR });
    row.height = (top + body + 6.0).ceil();
}

/// Selection inside one row, as a UTF-16 range of its text.
pub fn draw_row(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    fx: &Fixture,
    rows: &mut Rows,
    i: usize,
    y: f32,
    width: f32,
    sel: Option<(u32, u32)>,
) -> Result<()> {
    let m = &fx.messages[i];
    let bitmap = if m.image.is_some() {
        rows.image(dc, fx, i)
    } else {
        None
    };
    let row = rows.rows[i].as_ref().unwrap();
    let top = y + if m.grouped { 2.0 } else { 10.0 };
    let x_body = PAD_X + AVATAR + GAP;
    unsafe {
        if !m.grouped {
            let a = rect(PAD_X, top, PAD_X + AVATAR, top + AVATAR);
            fill_round(dc, a, 8.0, &g.brushes.authors[m.author % 6]);
            if let Some(initials) = &row.initials {
                gfx::text(dc, initials, a.left, a.top, &g.brushes.text);
            }
        }
        if let Some((name, time, nw)) = &row.header {
            gfx::text(dc, name, x_body, top, &g.brushes.text);
            gfx::text(dc, time, x_body + nw + 10.0, top + 3.0, &g.brushes.muted);
        }
        let ty = top + row.text_y;
        let mut buf = [DWRITE_HIT_TEST_METRICS::default(); 16];
        let mut hit = |s: u32, l: u32, brush: &ID2D1SolidColorBrush, pad: f32, radius: f32| {
            let mut n = 0u32;
            if row
                .text
                .HitTestTextRange(s, l, x_body, ty, Some(&mut buf), &mut n)
                .is_ok()
            {
                for h in &buf[..(n as usize).min(16)] {
                    fill_round(
                        dc,
                        rect(
                            h.left - pad,
                            h.top,
                            h.left + h.width + pad,
                            h.top + h.height,
                        ),
                        radius,
                        brush,
                    );
                }
            }
        };
        for (s, l) in &row.mentions {
            hit(*s, *l, &g.brushes.mention_bg, 2.0, 4.0);
        }
        if let Some((s, e)) = sel {
            if e > s {
                hit(s, e - s, &g.brushes.selection, 0.0, 0.0);
            }
        }
        gfx::text(dc, &row.text, x_body, ty, &g.brushes.text);
        if let (Some((w, h, _)), Some(bmp)) = (m.image, bitmap) {
            let r = rect(
                x_body,
                top + row.image_y,
                x_body + w as f32,
                top + row.image_y + h as f32,
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
        let mut x = x_body;
        for (l, w) in &row.reactions {
            let r = rect(
                x,
                top + row.reactions_y,
                x + w + 12.0,
                top + row.reactions_y + 20.0,
            );
            fill_round(dc, r, 8.0, &g.brushes.surface);
            stroke_round(dc, r, 8.0, &g.brushes.border);
            gfx::text(dc, l, x + 6.0, r.top + 2.0, &g.brushes.text);
            x = r.right + 6.0;
        }
    }
    let _ = width;
    Ok(())
}
