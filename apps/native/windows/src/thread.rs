//! The thread beside the conversation, as the egui client's thread panel:
//! a heading ("Thread", "in #channel" and Close), the root message, its
//! reply count, older replies or the load error, the replies, and a reply
//! composer with "Also send to #channel", Send and the main composer's card
//! for its first link (`preview`). Its rows are the channel
//! list's rows (`list`), all measured at once (threads arrive in pages of
//! 50) and drawn into the pane's surface; the reply box has its own surface,
//! so typing redraws only the box.
use crate::actions::Rect;
use crate::composer::{self, Composer};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::list::{self, HEAD_H, Head, Rows};
use crate::live;
use crate::model::Conversation;
use crate::theme::*;
use crate::{App, EV_FOCUS, EV_ROWS, EV_SCREEN, EV_TEXT, WHEEL_STEP};
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

/// egui's thread panel width (its default; it is not resizable here).
pub const THREAD_W: f32 = 340.0;
const PAD: f32 = 18.0;
const CLOSE: f32 = 36.0;
const SEND: f32 = 36.0;
const CHECK_H: f32 = 44.0;
const CHECK: f32 = 16.0;
const BOX_MIN: f32 = 48.0;
const COUNT_H: f32 = 16.0;
const EMPTY_H: f32 = 40.0;
const OLDER: &str = "Load older replies";
const FAILED: &str = "Couldn\u{2019}t load replies.";

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

/// The pane's parts that take clicks or have UIA nodes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Close,
    Row(usize),
    /// A thread row's reaction chip, and its Add reaction after them.
    Chip(usize, usize),
    Add(usize),
    /// A thread row's hidden text spoiler (its index in `list::Row::spoilers`),
    /// which reveals it.
    Spoiler(usize, usize),
    /// "Load older replies", or Retry after a failed load.
    Older,
    Broadcast,
    Field,
    Format(composer::Style),
    /// Send a GIF, inside the box before Insert emoji.
    Gif,
    /// Insert emoji, at the end of the box's first line.
    Emoji,
    /// The link card's Remove: reply without previews.
    RemovePreview,
    Send,
}

/// Where the history's pieces sit, from the top of its content.
struct Sections {
    root: Option<(usize, f32)>,
    count: Option<f32>,
    head: Option<f32>,
    empty: Option<f32>,
    /// The first reply's index and top.
    replies: Option<(usize, f32)>,
    height: f32,
}

pub struct Pane {
    pub root: String,
    pub c: Conversation,
    pub ids: live::Ids,
    pub rows: Rows,
    /// The root's reply count, for "N replies".
    count: Option<u64>,
    pub loading: bool,
    pub has_more: bool,
    pub error: bool,
    /// Distance scrolled from the top of the history's content.
    pub scroll: f32,
    pub at_end: bool,
    pub composer: Composer,
    selecting: bool,
    /// "Also send to #channel", for the next reply only.
    pub broadcast: bool,
    pub hover: Option<Part>,
    pub box_h: f32,
    /// Whether the box holds Send a GIF (KLIPY answered for this account).
    pub gif: bool,
    destination: String,
    title: IDWriteTextLayout,
    sub: IDWriteTextLayout,
    check: IDWriteTextLayout,
    check_w: f32,
}

impl Pane {
    fn new(g: &Gfx, root: String, channel: &str) -> Result<Self> {
        let destination = channel.to_owned();
        let title = g.layout(&gfx::wide("Thread"), &g.formats.name, 1e6)?;
        let sub = g.layout(
            &gfx::wide(&format!("in {destination}")),
            &g.formats.time,
            THREAD_W - 2.0 * PAD - CLOSE - 10.0,
        )?;
        let label = format!("Also send to {destination}");
        let check = g.layout(
            &gfx::wide(&label),
            &g.formats.head,
            THREAD_W - 2.0 * PAD - CHECK - 7.0,
        )?;
        unsafe {
            check.SetFontWeight(
                DWRITE_FONT_WEIGHT_BOLD,
                DWRITE_TEXT_RANGE {
                    startPosition: 13,
                    length: (label.encode_utf16().count() - 13) as u32,
                },
            )?;
        }
        let check_w = gfx::metrics(&check).widthIncludingTrailingWhitespace;
        Ok(Self {
            root,
            c: Conversation::live(Vec::new(), 0, Vec::new(), 0, Vec::new(), Vec::new()),
            ids: live::Ids::default(),
            rows: Rows::new(0),
            count: None,
            loading: true,
            has_more: false,
            error: false,
            scroll: 0.0,
            at_end: true,
            composer: Composer::formatted(),
            selecting: false,
            broadcast: false,
            hover: None,
            box_h: BOX_MIN,
            gif: false,
            destination,
            title,
            sub,
            check,
            check_w,
        })
    }

    /// New rows from the live thread: all are measured, and the reader's
    /// place is kept from the bottom (older replies arrive above).
    fn set(
        &mut self,
        g: &Gfx,
        view: (Conversation, live::Ids, Option<u64>),
        state: (bool, bool, bool),
        vh: f32,
    ) -> Result<()> {
        let from_bottom = self.content_h() - self.scroll;
        let (mut c, ids, count) = view;
        // What the reader revealed stays revealed as the thread updates.
        for m in &mut c.messages {
            if let Some(old) = self
                .ids
                .rows
                .get(&m.id)
                .and_then(|&k| self.c.messages.get(k))
            {
                m.revealed.clone_from(&old.revealed);
            }
        }
        self.rows = Rows::new(c.messages.len());
        self.rows.set_width(Rows::body_width(THREAD_W));
        self.c = c;
        self.ids = ids;
        self.count = count;
        (self.loading, self.has_more, self.error) = state;
        self.rows.measure_up_to(g, &self.c, f64::NEG_INFINITY)?;
        let max = (self.content_h() - vh).max(0.0);
        self.scroll = if self.at_end {
            max
        } else {
            (self.content_h() - from_bottom).clamp(0.0, max)
        };
        Ok(())
    }

    fn root_index(&self) -> Option<usize> {
        self.ids.rows.get(&self.root).copied()
    }

    fn sections(&self) -> Sections {
        let n = self.c.messages.len();
        let root = self.root_index();
        let mut y = 8.0;
        let root = root.map(|i| {
            let at = y;
            y += self.rows.height(i) + 8.0;
            (i, at)
        });
        let count = self.count.map(|_| {
            let at = y;
            y += COUNT_H + 12.0;
            at
        });
        let first = (0..n).find(|i| Some(*i) != root.map(|(r, _)| r));
        let head = (self.error || self.has_more || (self.loading && first.is_none())).then(|| {
            let at = y;
            y += HEAD_H;
            at
        });
        let empty = (first.is_none() && !self.loading && !self.error).then(|| {
            let at = y;
            y += EMPTY_H;
            at
        });
        let replies = first.map(|i| {
            let at = y;
            let top = self.rows.top[i];
            let bottom = self.rows.bottom();
            y += (bottom - top) as f32;
            (i, at)
        });
        Sections {
            root,
            count,
            head,
            empty,
            replies,
            height: y + 8.0,
        }
    }

    fn content_h(&self) -> f32 {
        self.sections().height
    }

    /// Row `i`'s top in content coordinates.
    fn row_y(&self, i: usize) -> Option<f32> {
        let s = self.sections();
        match (s.root, s.replies) {
            (Some((r, y)), _) if r == i => Some(y),
            (_, Some((first, y))) if i >= first => {
                Some(y + (self.rows.top[i] - self.rows.top[first]) as f32)
            }
            _ => None,
        }
    }

    /// The head's contents: the failed load and Retry, "Load older
    /// replies", or "Loading…" while the first page loads.
    fn head(&self) -> Head<'static> {
        if self.error {
            Head {
                label: "Retry",
                button: true,
                enabled: !self.loading,
                note: Some(FAILED),
            }
        } else if self.has_more {
            Head {
                label: OLDER,
                button: true,
                enabled: !self.loading,
                note: None,
            }
        } else {
            Head {
                label: "Loading\u{2026}",
                button: false,
                enabled: false,
                note: None,
            }
        }
    }

    fn can_send(&self) -> bool {
        !String::from_utf16_lossy(&self.composer.text)
            .trim()
            .is_empty()
    }

    /// Draws the heading, the history at `scroll` within `vh`, the
    /// broadcast checkbox and Send; `h` is the pane's height.
    fn draw(&mut self, g: &Gfx, dc: &ID2D1DeviceContext, h: f32, vh: f32) -> Result<()> {
        let w = THREAD_W;
        unsafe {
            dc.Clear(Some(&gfx::color(SURFACE_CONVERSATION, 1.0)));
            // egui's panel separator on the leading edge.
            dc.FillRectangle(&rect(0.0, 0.0, 1.0, h), &g.brushes.border);
            dc.FillRectangle(&rect(0.0, HEADER_H - 1.0, w, HEADER_H), &g.brushes.border);
        }
        crate::text::draw(dc, &self.title, PAD, 8.0, &g.brushes.text);
        crate::text::draw(dc, &self.sub, PAD, 30.0, &g.brushes.muted);
        let (cx, cy, cw, ch) = close_local();
        let lit = self.hover == Some(Part::Close);
        if lit {
            fill_round(dc, rect(cx, cy, cx + cw, cy + ch), 6.0, &g.brushes.hover);
        }
        icons::draw(
            dc,
            Icon::Close,
            rect(cx + 9.0, cy + 9.0, cx + cw - 9.0, cy + ch - 9.0),
            if lit { TEXT } else { TEXT_MUTED },
            1.0,
        );
        // The history, clipped under the heading.
        unsafe {
            dc.PushAxisAlignedClip(
                &rect(0.0, HEADER_H, w, HEADER_H + vh),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
        }
        let result = self.draw_history(g, dc, vh);
        unsafe { dc.PopAxisAlignedClip() };
        // The composer's lip over the history's bottom, inside the leading separator.
        if vh > LIP_H {
            crate::draw_lip(dc, 1.0, HEADER_H + vh - LIP_H, w - 1.0);
        }
        result?;
        // "Also send to #channel" in its own row above the box.
        let check_y = h - 16.0 - self.box_h - 8.0 - CHECK_H;
        let by = check_y + (CHECK_H - CHECK) / 2.0;
        let b = rect(PAD, by, PAD + CHECK, by + CHECK);
        let lit = self.hover == Some(Part::Broadcast);
        fill_round(
            dc,
            b,
            2.0,
            if lit {
                &g.brushes.border
            } else {
                &g.brushes.composer
            },
        );
        stroke_round(
            dc,
            b,
            2.0,
            if lit {
                &g.brushes.muted
            } else {
                &g.brushes.border
            },
        );
        if self.broadcast {
            icons::draw(
                dc,
                Icon::Check,
                rect(b.left + 2.0, b.top + 2.0, b.right - 2.0, b.bottom - 2.0),
                TEXT,
                1.0,
            );
        }
        let lh = gfx::metrics(&self.check).height;
        crate::text::draw(
            dc,
            &self.check,
            PAD + CHECK + 7.0,
            check_y + (CHECK_H - lh) / 2.0,
            &g.brushes.muted,
        );
        // Send beside the box, eggshell while there is something to send.
        let (sx, sy, sw, sh) = send_local(h, self.box_h);
        let r = rect(sx, sy, sx + sw, sy + sh);
        let enabled = self.can_send();
        if enabled {
            let fill = if self.hover == Some(Part::Send) {
                ACTION_HOVER
            } else {
                ACTION
            };
            fill_round(dc, r, 8.0, &g.solid(fill));
        }
        let (color, alpha) = if enabled {
            (BLACKOUT, 1.0)
        } else {
            (TEXT_MUTED, 0.5)
        };
        icons::draw(
            dc,
            Icon::Send,
            rect(sx + 9.0, sy + 9.0, sx + sw - 9.0, sy + sh - 9.0),
            color,
            alpha,
        );
        Ok(())
    }

    /// The history's rows, reply count, head and empty note (the caller clips).
    fn draw_history(&mut self, g: &Gfx, dc: &ID2D1DeviceContext, vh: f32) -> Result<()> {
        let w = THREAD_W;
        let s = self.sections();
        let off = HEADER_H - self.scroll;
        let mut rows: Vec<(usize, f32)> = s.root.into_iter().collect();
        if let Some((first, _)) = s.replies {
            for i in first..self.c.messages.len() {
                if let Some(y) = self.row_y(i) {
                    rows.push((i, y));
                }
            }
        }
        for (i, y) in rows {
            let h = self.rows.height(i);
            if y + off < HEADER_H + vh && y + h + off > HEADER_H {
                list::draw_row(g, dc, &self.c, &mut self.rows, i, y + off, None, None)?;
            }
        }
        if let (Some(y), Some(n)) = (s.count, self.count) {
            let label = format!("{n} {}", if n == 1 { "reply" } else { "replies" });
            let l = g.layout(&gfx::wide(&label), &g.formats.head, 1e6)?;
            let m = gfx::metrics(&l);
            let y = y + off;
            crate::text::draw(dc, &l, PAD, y, &g.brushes.muted);
            let x = PAD + m.widthIncludingTrailingWhitespace + 12.0;
            let cy = (y + m.height / 2.0).round();
            unsafe {
                dc.FillRectangle(&rect(x, cy, w - PAD, cy + 1.0), &g.brushes.border);
            }
        }
        if let Some(y) = s.head {
            let hover = self.hover == Some(Part::Older);
            list::draw_head_row(g, dc, &self.head(), w, y + off, hover)?;
        }
        if let Some(y) = s.empty {
            let l = g.layout(
                &gfx::wide("No replies yet. Start the thread."),
                &g.formats.quote,
                w - 2.0 * PAD,
            )?;
            crate::text::draw(dc, &l, PAD, y + off + 12.0, &g.brushes.muted);
        }
        Ok(())
    }
}

/// Close in the heading, relative to the pane.
fn close_local() -> Rect {
    (
        THREAD_W - PAD - CLOSE,
        (HEADER_H - CLOSE) / 2.0,
        CLOSE,
        CLOSE,
    )
}

/// The reply box's width.
fn box_w() -> f32 {
    THREAD_W - 2.0 * PAD - SEND - 8.0
}

/// Send, relative to the pane, at the bottom of a box `box_h` tall.
fn send_local(h: f32, box_h: f32) -> Rect {
    let by = h - 16.0 - box_h;
    (
        PAD + box_w() + 8.0,
        by + (box_h - 42.0).max(0.0),
        SEND,
        SEND,
    )
}

/// The reply box's text width, beside Insert emoji and Send a GIF (when
/// it is shown).
fn text_w(gif: bool) -> f32 {
    let gif_w = if gif { crate::uploads::GIF_W } else { 0.0 };
    box_w() - 2.0 * composer::PAD - crate::uploads::EMOJI_W - gif_w
}

/// Insert emoji, relative to the box, at the end of its first line.
fn emoji_local() -> Rect {
    let cy = composer::PAD + composer::LINE / 2.0;
    (box_w() - 36.0, cy - 14.0, 28.0, 28.0)
}

/// Send a GIF, before Insert emoji (where egui's thread field keeps it).
fn gif_local() -> Rect {
    let (x, y, w, h) = emoji_local();
    (x - crate::uploads::EMOJI_W, y, w, h)
}

impl App {
    /// The open thread's width beside the conversation.
    pub fn thread_w(&self) -> f32 {
        if self.thread.is_some() { THREAD_W } else { 0.0 }
    }

    pub fn thread_x(&self) -> f32 {
        self.w - self.members_w() - THREAD_W
    }

    /// The history's height in the pane.
    pub fn thread_vh(&self) -> f32 {
        let box_h = self.thread.as_ref().map_or(BOX_MIN, |t| t.box_h);
        (self.h - 16.0 - box_h - 8.0 - CHECK_H - 12.0 - HEADER_H).max(0.0)
    }

    pub fn thread_box_rect(&self) -> Option<Rect> {
        let t = self.thread.as_ref()?;
        Some((
            self.thread_x() + PAD,
            self.h - 16.0 - t.box_h,
            box_w(),
            t.box_h,
        ))
    }

    pub fn thread_text_origin(&self) -> (f32, f32) {
        let (x, y, _, _) = self.thread_box_rect().unwrap_or_default();
        let scroll = self.thread.as_ref().map_or(0.0, |t| t.composer.scroll());
        let top = self.thread.as_ref().map_or(0.0, |t| t.composer.inset.1);
        (x + composer::PAD, y + composer::PAD + top - scroll)
    }

    /// A part's rectangle in client DIPs, while it is on screen.
    pub fn thread_part_rect(&self, part: Part) -> Option<Rect> {
        let t = self.thread.as_ref()?;
        let x0 = self.thread_x();
        let vh = self.thread_vh();
        let off = HEADER_H - t.scroll;
        let clip = |r: Rect| (r.1 + r.3 > HEADER_H && r.1 < HEADER_H + vh).then_some(r);
        match part {
            Part::Close => {
                let (x, y, w, h) = close_local();
                Some((x0 + x, y, w, h))
            }
            Part::Row(i) => {
                let y = t.row_y(i)?;
                clip((x0, y + off, THREAD_W, t.rows.height(i)))
            }
            Part::Chip(i, _) | Part::Add(i) => {
                let action = match part {
                    Part::Chip(_, k) => list::Action::Chip(k),
                    _ => list::Action::Add,
                };
                let y = t.row_y(i)? + off;
                let grouped = t.c.messages.get(i)?.grouped;
                let (_, r) = t
                    .rows
                    .controls(i, grouped)
                    .into_iter()
                    .find(|(a, _)| *a == action)?;
                clip((x0 + r.left, y + r.top, r.right - r.left, r.bottom - r.top))
            }
            Part::Spoiler(i, k) => {
                let y = t.row_y(i)? + off;
                let grouped = t.c.messages.get(i)?.grouped;
                let r = t.rows.spoiler_rect(i, grouped, k)?;
                clip((x0 + r.left, y + r.top, r.right - r.left, r.bottom - r.top))
            }
            Part::Older => {
                let y = t.sections().head? + off;
                let head = t.head();
                if !head.button {
                    return None;
                }
                let r = list::head_button_rect(&self.g, &head, THREAD_W).ok()?;
                clip((x0 + r.left, y + r.top, r.right - r.left, r.bottom - r.top))
            }
            Part::Broadcast => {
                let y = self.h - 16.0 - t.box_h - 8.0 - CHECK_H;
                Some((x0 + PAD, y, CHECK + 7.0 + t.check_w, CHECK_H))
            }
            Part::Field => self.thread_box_rect(),
            Part::Format(style) => {
                let (bx, by, _, _) = self.thread_box_rect()?;
                let (x, y, w, h) = t.composer.format_rect(style)?;
                Some((bx + x, by + y, w, h))
            }
            Part::Gif => {
                if !t.gif {
                    return None;
                }
                let (bx, by, _, _) = self.thread_box_rect()?;
                let (x, y, w, h) = gif_local();
                Some((bx + x, by + y + t.composer.inset.1, w, h))
            }
            Part::Emoji => {
                let (bx, by, _, _) = self.thread_box_rect()?;
                let (x, y, w, h) = emoji_local();
                Some((bx + x, by + y + t.composer.inset.1, w, h))
            }
            Part::RemovePreview => {
                self.thread_preview_card()?;
                let (bx, by, bw, bh) = self.thread_box_rect()?;
                let r = crate::preview::remove_rect(crate::preview::card_rect(bw, bh));
                Some((bx + r.left, by + r.top, r.right - r.left, r.bottom - r.top))
            }
            Part::Send => {
                let (x, y, w, h) = send_local(self.h, t.box_h);
                Some((x0 + x, y, w, h))
            }
        }
    }

    /// The pane's parts in reading order, for hit tests and UIA.
    pub fn thread_parts(&self) -> Vec<Part> {
        let Some(t) = &self.thread else {
            return Vec::new();
        };
        let mut out = vec![Part::Close];
        let s = t.sections();
        let mut rows: Vec<usize> = s.root.map(|(i, _)| i).into_iter().collect();
        if let Some((first, _)) = s.replies {
            rows.extend(first..t.c.messages.len());
        }
        let head_first = s.head.is_some();
        let push_rows = |out: &mut Vec<Part>, rows: &[usize]| {
            for &i in rows {
                if self.thread_part_rect(Part::Row(i)).is_some() {
                    out.push(Part::Row(i));
                    for k in t.rows.hidden_spoilers(i) {
                        if self.thread_part_rect(Part::Spoiler(i, k)).is_some() {
                            out.push(Part::Spoiler(i, k));
                        }
                    }
                    for k in 0..t.c.messages[i].reactions.len() {
                        if self.thread_part_rect(Part::Chip(i, k)).is_some() {
                            out.push(Part::Chip(i, k));
                        }
                    }
                    if self.thread_part_rect(Part::Add(i)).is_some() {
                        out.push(Part::Add(i));
                    }
                }
            }
        };
        let (before, after) = rows.split_at(usize::from(s.root.is_some()).min(rows.len()));
        push_rows(&mut out, before);
        if head_first && self.thread_part_rect(Part::Older).is_some() {
            out.push(Part::Older);
        }
        push_rows(&mut out, after);
        out.extend([Part::Broadcast, Part::Field]);
        if t.composer.format_height() > 0.0 {
            out.extend(composer::Style::ALL.map(Part::Format));
        }
        if t.gif {
            out.push(Part::Gif);
        }
        out.push(Part::Emoji);
        if self.thread_preview_card().is_some() {
            out.push(Part::RemovePreview);
        }
        out.push(Part::Send);
        out
    }

    pub fn thread_hit(&self, x: f32, y: f32) -> Option<Part> {
        let parts = self.thread_parts();
        // Controls before the rows and the field that hold them.
        let holder = |p: &&Part| matches!(p, Part::Row(_) | Part::Field);
        parts
            .iter()
            .filter(|p| !holder(p))
            .chain(parts.iter().filter(holder))
            .copied()
            .find(|p| match *p {
                // A spoiler takes only its own lines, not its bounds.
                Part::Spoiler(i, k) => self.thread_spoiler_at(x, y) == Some((i, k)),
                p => self.thread_part_rect(p).is_some_and(|r| contains(r, x, y)),
            })
    }

    /// The thread row's hidden spoiler under client point (x, y): its row
    /// and its index in the row's spoilers.
    fn thread_spoiler_at(&self, x: f32, y: f32) -> Option<(usize, usize)> {
        let t = self.thread.as_ref()?;
        if !self.in_thread(x, y) || y < HEADER_H || y >= HEADER_H + self.thread_vh() {
            return None;
        }
        let ay = y - HEADER_H + t.scroll;
        t.c.messages.iter().enumerate().find_map(|(i, m)| {
            let top = t.row_y(i)?;
            let ry = ay - top;
            if ry < 0.0 || ry >= t.rows.height(i) {
                return None;
            }
            let k = t.rows.spoiler_at(i, m.grouped, x - self.thread_x(), ry)?;
            Some((i, k))
        })
    }

    /// Reveals thread row `i`'s spoiler `k` (with its neighbouring runs).
    fn reveal_thread_spoiler(&mut self, i: usize, k: usize) {
        let g = &self.g;
        let Some(t) = &mut self.thread else { return };
        let Some(&(start, _, true)) = t.rows.row(i).and_then(|r| r.spoilers.get(k)) else {
            return;
        };
        let Some(m) = t.c.messages.get_mut(i) else {
            return;
        };
        m.revealed.insert(list::spoiler_key(start));
        if let Err(e) = t.rows.replace(g, &t.c, i) {
            eprintln!("spoiler layout failed: {e:?}");
        }
        self.dirty.thread = true;
        self.events |= EV_ROWS;
        self.need = true;
    }

    /// The person a point on a thread row names (its author, quoted
    /// author or a mention pill), as `person_at` in the history.
    pub fn thread_person_at(
        &self,
        x: f32,
        y: f32,
    ) -> Option<(
        String,
        caper_client::model::Mention,
        Option<caper_client::model::Author>,
    )> {
        let t = self.thread.as_ref()?;
        let Some(Part::Row(i)) = self.thread_hit(x, y) else {
            return None;
        };
        let top = t.row_y(i)? + HEADER_H - t.scroll;
        let grouped = t.c.messages.get(i)?.grouped;
        let who = t.rows.who_at(i, grouped, x - self.thread_x(), y - top)?;
        self.person_of(&t.c.messages[i].id, t.rows.row(i)?, who)
    }

    pub fn thread_name(&self) -> String {
        self.thread
            .as_ref()
            .map_or_else(String::new, |t| format!("Thread in {}", t.destination))
    }

    pub fn thread_part_name(&self, part: Part) -> String {
        let Some(t) = &self.thread else {
            return String::new();
        };
        match part {
            Part::Close => "Close thread".into(),
            Part::Older => {
                let head = t.head();
                match head.note {
                    Some(note) => format!("{note} {}", head.label),
                    None => head.label.into(),
                }
            }
            Part::Broadcast => format!("Also send to {}", t.destination),
            Part::Field => "Reply to thread".into(),
            Part::Format(style) => style.name().into(),
            Part::Gif => "Send a GIF".into(),
            Part::Emoji => "Insert emoji".into(),
            Part::RemovePreview => "Remove link preview".into(),
            Part::Send => "Send reply".into(),
            Part::Add(_) => "Add reaction".into(),
            Part::Spoiler(..) => "Spoiler, reveal".into(),
            Part::Row(_) | Part::Chip(..) => String::new(),
        }
    }

    pub fn thread_part_enabled(&self, part: Part) -> bool {
        let Some(t) = &self.thread else {
            return false;
        };
        match part {
            Part::Older => !t.loading,
            Part::Send => t.can_send(),
            _ => true,
        }
    }

    /// Client position of thread row `i`'s message text origin.
    pub fn thread_row_text_origin(&self, i: usize) -> (f32, f32) {
        let Some(t) = &self.thread else {
            return (0.0, 0.0);
        };
        let y = t.row_y(i).unwrap_or(0.0) + HEADER_H - t.scroll;
        let grouped = t.c.messages.get(i).is_some_and(|m| m.grouped);
        let ty = t.rows.row(i).map_or(0.0, |r| r.text_y);
        (
            self.thread_x() + list::BODY_X,
            y + list::content_top(grouped) + ty,
        )
    }

    pub fn in_thread(&self, x: f32, y: f32) -> bool {
        self.thread.is_some()
            && x >= self.thread_x()
            && x < self.thread_x() + THREAD_W
            && y < self.h
    }

    /// The live thread changed (opened, loaded, a reply arrived, closed):
    /// the pane follows it.
    pub fn thread_refresh(&mut self) -> Result<()> {
        // A thread's GIF picker goes with its thread, as in egui.
        let open = self
            .live
            .as_ref()
            .and_then(|l| l.thread.as_ref())
            .map(|t| t.root.clone());
        if self
            .gifs
            .as_ref()
            .and_then(|p| p.root.as_ref())
            .is_some_and(|r| open.as_ref() != Some(r))
        {
            self.close_gifs(false);
        }
        let Some(live) = &self.live else {
            return Ok(());
        };
        let (Some(t), Some(view)) = (live.thread.as_ref(), live.thread_view()) else {
            if self.thread.take().is_some() {
                self.thread_layout_changed();
                // Its reply went with it, and so does that reply's link card.
                self.preview_tick();
            }
            return Ok(());
        };
        let root = t.root.clone();
        let state = (t.loading, t.has_more, t.error.is_some());
        let opened = self.thread.as_ref().is_none_or(|p| p.root != root);
        if opened {
            let mut pane = Pane::new(&self.g, root, &self.c.place())?;
            pane.gif = self.gif_shown();
            pane.box_h = pane
                .composer
                .relayout(&self.g, text_w(pane.gif))?
                .max(BOX_MIN);
            self.thread = Some(pane);
            self.thread_layout_changed();
            // The new thread's reply box is empty: its link card starts over.
            self.preview_tick();
        }
        let vh = self.thread_vh();
        if let Some(p) = &mut self.thread {
            p.set(&self.g, view, state, vh)?;
        }
        self.dirty.thread = true;
        self.events |= EV_SCREEN;
        self.need = true;
        Ok(())
    }

    fn thread_layout_changed(&mut self) {
        self.dirty.layout = true;
        self.dirty.thread = true;
        self.dirty.thread_box = true;
        self.drawn = None;
        self.events |= EV_SCREEN | EV_ROWS;
        self.need = true;
    }

    /// Opens the thread of row `i` of the channel: a root's own thread, or
    /// the thread a broadcast reply belongs to.
    pub fn open_thread_of(&mut self, i: usize) {
        let Some(id) = self.c.messages.get(i).map(|m| m.id.clone()) else {
            return;
        };
        let Some(live) = &mut self.live else {
            return;
        };
        let root = live
            .message(&id)
            .and_then(|m| m.thread_root_id)
            .unwrap_or(id);
        live.open_thread(&root);
        if let Err(e) = self.drain_live() {
            eprintln!("thread failed: {e:?}");
        }
    }

    pub fn close_thread(&mut self) {
        self.set_thread_focus(false);
        if let Some(live) = &mut self.live {
            live.close_thread();
        }
        if let Err(e) = self.drain_live() {
            eprintln!("thread failed: {e:?}");
        }
    }

    /// The channel row `i`'s thread link under client point (x, y).
    pub fn thread_link_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.in_history(x, y) || self.live.is_none() {
            return None;
        }
        let ay = self.scroll_top + (y - HEADER_H) as f64;
        let i = self.rows.row_at(ay)?;
        let ry = (ay - self.rows.top[i]) as f32;
        let r = self.rows.thread_link(i, self.c.messages[i].grouped)?;
        let lx = x - self.list_x();
        (lx >= r.left && lx < r.right && ry >= r.top && ry < r.bottom).then_some(i)
    }

    pub fn set_thread_focus(&mut self, focused: bool) {
        let Some(t) = &mut self.thread else {
            return;
        };
        if t.composer.focused == focused {
            return;
        }
        if focused {
            self.set_composer_focus(false);
            self.set_kb(None);
        }
        let Some(t) = &mut self.thread else {
            return;
        };
        t.composer.focused = focused;
        if !focused {
            t.composer.end();
        }
        t.composer.caret_on = true;
        unsafe {
            if focused {
                SetTimer(
                    Some(self.hwnd),
                    crate::TIMER_CARET,
                    GetCaretBlinkTime(),
                    None,
                );
            } else {
                let _ = KillTimer(Some(self.hwnd), crate::TIMER_CARET);
            }
        }
        self.thread_composer_changed();
        self.events |= EV_FOCUS;
        self.need = true;
    }

    /// The reply box follows whether GIFs are offered (KLIPY answers
    /// after the thread may have opened).
    pub fn thread_tools_changed(&mut self) {
        let gif = self.gif_shown();
        if self.thread.as_ref().is_some_and(|t| t.gif != gif) {
            self.thread_composer_changed();
        }
    }

    /// After an edit or caret move in the reply box.
    pub fn thread_composer_changed(&mut self) {
        // The reply's link card follows its text (and lays the box out
        // itself when the card changed).
        self.preview_tick();
        self.thread_box_changed();
    }

    /// Lays the reply box out again: its text, format buttons and link card.
    pub fn thread_box_changed(&mut self) {
        let gif = self.gif_shown();
        let card = if self.thread_preview_card().is_some() {
            crate::preview::PREVIEW_H
        } else {
            0.0
        };
        let g = &self.g;
        let Some(t) = &mut self.thread else {
            return;
        };
        t.composer.caret_on = true;
        t.gif = gif;
        t.composer.inset.1 = t.composer.format_height();
        t.composer.inset_bottom = card;
        if let Ok(h) = t.composer.relayout(g, text_w(gif)) {
            let h = h.max(BOX_MIN) + t.composer.inset.1 + card;
            if (h - t.box_h).abs() > 0.01 {
                t.box_h = h;
                self.dirty.layout = true;
                self.dirty.thread = true;
                self.events |= EV_SCREEN;
            }
        }
        // Send's look follows whether there is anything to send.
        self.dirty.thread = true;
        self.dirty.thread_box = true;
        self.events |= EV_TEXT;
        self.need = true;
        self.ime_position();
    }

    /// Sends the reply box's text into the thread (and to the channel when
    /// "Also send to" is checked, which then resets, as in egui).
    pub fn send_thread_reply(&mut self) {
        let Some(t) = &mut self.thread else {
            return;
        };
        let text = String::from_utf16_lossy(&t.composer.text)
            .trim()
            .to_string();
        if text.is_empty() {
            return;
        }
        let (root, broadcast) = (t.root.clone(), t.broadcast);
        t.composer.set_text(Vec::new());
        t.broadcast = false;
        t.at_end = true;
        if let Some(live) = &self.live {
            live.send(text, None, Some((root, broadcast)));
        }
        self.thread_composer_changed();
    }

    pub fn thread_activate(&mut self, part: Part) {
        match part {
            Part::Format(style) => {
                if let Some(t) = &mut self.thread
                    && t.composer.format_rect(style).is_some()
                    && t.composer.toggle(style)
                {
                    self.thread_composer_changed();
                }
            }
            Part::Close => self.close_thread(),
            Part::Older => {
                let Some(t) = &self.thread else { return };
                if t.loading {
                    return;
                }
                let error = t.error;
                if let Some(live) = &mut self.live {
                    if error {
                        live.retry_thread();
                    } else {
                        live.load_thread(true);
                    }
                }
                if let Err(e) = self.drain_live() {
                    eprintln!("thread failed: {e:?}");
                }
            }
            Part::Chip(i, k) => {
                let Some(t) = &self.thread else { return };
                let Some(m) = t.c.messages.get(i) else { return };
                let Some(r) = m.reactions.get(k) else { return };
                let (id, emoji, on) = (m.id.clone(), r.emoji.clone(), !r.mine);
                if let Some(live) = &mut self.live {
                    live.react(&id, &emoji, on);
                }
                if let Err(e) = self.drain_live() {
                    eprintln!("reaction update failed: {e:?}");
                }
            }
            Part::Add(i) => {
                let Some(id) = self
                    .thread
                    .as_ref()
                    .and_then(|t| t.c.messages.get(i))
                    .map(|m| m.id.clone())
                else {
                    return;
                };
                let Some((x, y, _, h)) = self.thread_part_rect(part) else {
                    return;
                };
                // The picker keeps itself inside the window.
                self.open_picker_for(usize::MAX, id, (x, y + h + 4.0));
            }
            Part::Broadcast => {
                if let Some(t) = &mut self.thread {
                    t.broadcast = !t.broadcast;
                }
                self.dirty.thread = true;
                self.need = true;
            }
            Part::Field => self.set_thread_focus(true),
            Part::Gif => {
                let root = self.thread.as_ref().map(|t| t.root.clone());
                self.toggle_gifs(root);
            }
            Part::Emoji => {
                let root = self.thread.as_ref().map(|t| t.root.clone());
                self.toggle_composer_emoji(root);
            }
            Part::RemovePreview => self.remove_thread_preview(),
            Part::Send => self.send_thread_reply(),
            Part::Spoiler(i, k) => self.reveal_thread_spoiler(i, k),
            Part::Row(_) => {}
        }
    }

    fn set_thread_hover(&mut self, hover: Option<Part>) {
        if let Some(t) = &mut self.thread
            && t.hover != hover
        {
            // Send a GIF, Insert emoji and the link card's Remove are drawn
            // in the box's own surface.
            let gif = [t.hover, hover]
                .iter()
                .any(|h| matches!(h, Some(Part::Gif | Part::Emoji | Part::RemovePreview)));
            t.hover = hover;
            self.dirty.thread = true;
            self.dirty.thread_box |= gif;
            self.need = true;
        }
    }

    /// Input for the open thread. None leaves it to the normal handler.
    pub fn thread_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let focused = self.thread.as_ref()?.composer.focused;
        match msg {
            WM_MOUSEMOVE => {
                if self.thread.as_ref()?.selecting {
                    let (tx, ty) = self.thread_text_origin();
                    let t = self.thread.as_mut()?;
                    let p = t.composer.hit(x - tx, y - ty - t.composer.scroll());
                    t.composer.move_to(p, true);
                    self.thread_composer_changed();
                    return done;
                }
                let hover = self
                    .in_thread(x, y)
                    .then(|| self.thread_hit(x, y))
                    .flatten()
                    .filter(|p| !matches!(p, Part::Row(_) | Part::Field));
                self.set_thread_hover(hover);
                None
            }
            WM_LBUTTONDOWN if self.in_thread(x, y) => {
                match self.thread_hit(x, y) {
                    Some(Part::Row(_))
                        if let Some((id, entry, author)) = self.thread_person_at(x, y) =>
                    {
                        self.set_thread_focus(false);
                        self.open_profile(id, entry, author, x, y);
                    }
                    Some(Part::Field) => {
                        self.set_thread_focus(true);
                        let (tx, ty) = self.thread_text_origin();
                        if let Some(t) = &mut self.thread {
                            let scroll = t.composer.scroll();
                            let p = t.composer.hit(x - tx, y - ty - scroll);
                            t.composer.move_to(p, shift);
                            t.selecting = true;
                        }
                        unsafe {
                            SetCapture(self.hwnd);
                        }
                        self.thread_composer_changed();
                    }
                    // Neither moves the keyboard out of the box.
                    Some(part @ (Part::Format(_) | Part::RemovePreview)) => {
                        self.thread_activate(part)
                    }
                    Some(part) => {
                        self.set_thread_focus(false);
                        self.set_composer_focus(false);
                        self.thread_activate(part);
                    }
                    None => {
                        self.set_thread_focus(false);
                        self.set_composer_focus(false);
                    }
                }
                done
            }
            WM_LBUTTONUP if self.thread.as_ref()?.selecting => {
                self.thread.as_mut()?.selecting = false;
                unsafe {
                    let _ = ReleaseCapture();
                }
                done
            }
            WM_CAPTURECHANGED => {
                self.thread.as_mut()?.selecting = false;
                None
            }
            WM_MOUSEWHEEL if self.pointer.is_some_and(|(x, y)| self.in_thread(x, y)) => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32;
                let vh = self.thread_vh();
                let t = self.thread.as_mut()?;
                let max = (t.content_h() - vh).max(0.0);
                t.scroll = (t.scroll - delta / 120.0 * WHEEL_STEP as f32).clamp(0.0, max);
                t.at_end = t.scroll >= max - 0.5;
                self.dirty.thread = true;
                self.events |= EV_SCREEN;
                self.need = true;
                done
            }
            WM_SETCURSOR if self.pointer.is_some_and(|(x, y)| self.in_thread(x, y)) => {
                let (px, py) = self.pointer?;
                let cursor = match self.thread_hit(px, py) {
                    Some(Part::Field) => IDC_IBEAM,
                    Some(Part::Row(_)) if self.thread_person_at(px, py).is_some() => IDC_HAND,
                    Some(Part::Row(_)) | None => IDC_ARROW,
                    Some(_) => IDC_HAND,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                Some(Some(LRESULT(1)))
            }
            WM_KEYDOWN if focused => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                let hwnd = self.hwnd;
                let t = self.thread.as_mut()?;
                if t.composer.composing || !t.composer.comp.is_empty() {
                    return Some(None);
                }
                if ctrl && vk == VIRTUAL_KEY('C' as u16) {
                    let copied = t.composer.copied();
                    if !copied.is_empty() {
                        crate::set_clipboard(hwnd, &copied);
                    }
                    return done;
                }
                match vk {
                    VK_RETURN if shift => t.composer.insert(&['\n' as u16]),
                    VK_RETURN => {
                        self.send_thread_reply();
                        return done;
                    }
                    VK_ESCAPE => {
                        self.set_thread_focus(false);
                        return done;
                    }
                    _ if t.composer.edit_key(hwnd, vk, ctrl, shift, true) => {}
                    _ => return Some(None),
                }
                self.thread_composer_changed();
                done
            }
            WM_CHAR if focused => {
                let c = wp.0 as u16;
                if !ctrl && c >= 0x20 && c != 0x7f {
                    self.thread.as_mut()?.composer.insert(&[c]);
                    self.thread_composer_changed();
                }
                done
            }
            _ => None,
        }
    }

    /// Positions and redraws the pane's surfaces when they changed.
    pub fn draw_thread(&mut self) -> Result<()> {
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        if self.thread.is_none() {
            self.thread_p.offset(-10_000.0, 0.0)?;
            self.thread_box_p.offset(-10_000.0, 0.0)?;
            return Ok(());
        }
        let card = self.thread_preview_card().cloned();
        let thumb = self.preview_thumb(true);
        let (x, h, vh) = (self.thread_x(), self.h, self.thread_vh());
        let (bx, by, bw, bh) = self.thread_box_rect().unwrap_or_default();
        let g = &self.g;
        let fresh = self.thread_p.ensure(g, px(THREAD_W), px(h))?;
        self.thread_p.offset((x * s).round(), 0.0)?;
        if (fresh || std::mem::take(&mut self.dirty.thread))
            && let Some(t) = &mut self.thread
        {
            self.thread_p.draw(s, |dc| t.draw(g, dc, h, vh))?;
        }
        let Some(t) = &self.thread else {
            return Ok(());
        };
        let fresh = self.thread_box_p.ensure(g, px(bw), px(bh))?;
        self.thread_box_p
            .offset((bx * s).round(), (by * s).round())?;
        if fresh || std::mem::take(&mut self.dirty.thread_box) {
            let placeholder = "Reply to thread\u{2026}";
            // Lit while its picker is open, as the channel's.
            let open = self
                .gifs
                .as_ref()
                .is_some_and(|p| p.root.as_deref() == Some(t.root.as_str()));
            let lit = open || t.hover == Some(Part::Gif);
            let emoji = self.emoji_open_for(Some(t.root.as_str())) || t.hover == Some(Part::Emoji);
            self.thread_box_p.draw(s, |dc| {
                t.composer.draw(g, dc, bw, bh, placeholder)?;
                if t.gif {
                    let (x, y, w, h) = gif_local();
                    let y = y + t.composer.inset.1;
                    let color = if lit { TEXT } else { TEXT_MUTED };
                    crate::uploads::gif_button(g, dc, rect(x, y, x + w, y + h), color)?;
                }
                let (x, y, w, h) = emoji_local();
                let y = y + t.composer.inset.1;
                icons::draw(
                    dc,
                    Icon::Smile,
                    rect(x + 6.0, y + 6.0, x + w - 6.0, y + h - 6.0),
                    if emoji { TEXT } else { TEXT_MUTED },
                    1.0,
                );
                if let Some(embed) = &card {
                    crate::preview::draw(
                        g,
                        dc,
                        embed,
                        thumb.as_ref(),
                        crate::preview::card_rect(bw, bh),
                        t.hover == Some(Part::RemovePreview),
                    )?;
                }
                let r = rect(0.5, 0.5, bw - 0.5, bh - 0.5);
                if t.composer.focused {
                    stroke_round(dc, r, RADIUS, &g.brushes.terracotta);
                } else {
                    stroke_round(dc, r, RADIUS, &g.brushes.border);
                }
                Ok(())
            })?;
        }
        Ok(())
    }
}
