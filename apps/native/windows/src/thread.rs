//! The thread beside the conversation, as the egui client's thread panel:
//! a heading ("Thread", "in #channel" and Close), the root message, its
//! reply count, older replies or the load error, the replies, and a reply
//! composer with a selected quote, "Also send to #channel", Send and the
//! main composer's card for its first link (`preview`). Its rows are the channel
//! list's rows (`list`), all measured at once (threads arrive in pages of
//! 50) and drawn into the pane's surface; the reply box has its own surface,
//! so typing redraws only the box.
use crate::actions::{self, BarKey, REPLY_GAP, Rect, ReplyBar, Tool, ToolState};
use crate::composer::{self, Composer};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::list::{self, HEAD_H, Head, Rows};
use crate::live;
use crate::model::Conversation;
use crate::theme::*;
use crate::{App, EV_FOCUS, EV_ROWS, EV_SCREEN, EV_TEXT, WHEEL_STEP, WM_MOUSELEAVE};
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

/// egui's thread panel width (its default; it is not resizable here).
pub const THREAD_W: f32 = 340.0;
/// Separate from channel GIFs: either surface may redraw without the other.
pub const TIMER_MEDIA: usize = 20;
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

/// Clipboard text uses the same UTF-16 ranges as the row highlights. UIA
/// may supply a boundary inside a surrogate pair; copy the whole scalar.
fn copy_selection<'a>(
    selection: Option<crate::Sel>,
    row: impl Fn(usize) -> Option<&'a [u16]>,
) -> Vec<u16> {
    let mut out = Vec::new();
    let Some((a, b)) = selection else { return out };
    for i in a.0.min(b.0)..=a.0.max(b.0) {
        let Some(text) = row(i) else { continue };
        let Some((s, e)) = App::sel_for(selection, i, text.len() as u32) else {
            continue;
        };
        let (mut s, mut e) = ((s as usize).min(text.len()), (e as usize).min(text.len()));
        if s >= e {
            continue;
        }
        let inside_pair = |p: usize| {
            p > 0
                && p < text.len()
                && (0xd800..=0xdbff).contains(&text[p - 1])
                && (0xdc00..=0xdfff).contains(&text[p])
        };
        if inside_pair(s) {
            s -= 1;
        }
        if inside_pair(e) {
            e += 1;
        }
        if !out.is_empty() {
            out.extend([13, 10]);
        }
        out.extend_from_slice(&text[s..e]);
    }
    out
}

/// The pane's parts that take clicks or have UIA nodes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Close,
    Row(usize),
    Tool(usize, Tool),
    /// A thread row's reaction chip, and its Add reaction after them.
    Chip(usize, usize),
    Add(usize),
    /// A thread row's hidden text spoiler (its index in `list::Row::spoilers`),
    /// which reveals it.
    Spoiler(usize, usize),
    Forward(usize, list::ForwardPart),
    /// "Load older replies", or Retry after a failed load.
    Older,
    Reply,
    CancelReply,
    Broadcast,
    Field,
    Format(composer::Style),
    /// Send a GIF, inside the box before Insert emoji.
    Gif,
    /// Insert emoji, at the end of the box's first line.
    Emoji,
    /// The link card's Remove: reply without previews.
    RemovePreview,
    Upload(crate::uploads::Part),
    Send,
    /// Jump to latest over the history's bottom right (`jump`).
    Jump,
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
    /// Jump to latest (`jump`), and its glide (from where, since when).
    pub jump: caper_client::jump::Jump,
    pub jump_glide: Option<(f32, std::time::Instant)>,
    width: f32,
    pub composer: Composer,
    has_files: bool,
    pub attach_error: Option<String>,
    send_waiting: Option<(Vec<u16>, Vec<u64>)>,
    /// The quote belongs to this thread, never the channel composer.
    pub reply: Option<ReplyBar>,
    selecting: bool,
    /// Endpoints follow message IDs when older replies are inserted.
    selection: Option<((String, u32), (String, u32))>,
    history_drag: bool,
    pub(crate) history_focused: bool,
    hover_row: Option<usize>,
    tool_focus: Option<(String, Tool)>,
    bar: Option<(usize, BarKey)>,
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
    fn new(g: &Gfx, root: String, channel: &str, width: f32) -> Result<Self> {
        let destination = channel.to_owned();
        let title = g.layout(&gfx::wide("Thread"), &g.formats.name, 1e6)?;
        let sub = g.layout(
            &gfx::wide(&format!("in {destination}")),
            &g.formats.time,
            width - 2.0 * PAD - CLOSE - 10.0,
        )?;
        let label = format!("Also send to {destination}");
        let check = g.layout(
            &gfx::wide(&label),
            &g.formats.head,
            width - 2.0 * PAD - CHECK - 7.0,
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
            jump: Default::default(),
            jump_glide: None,
            width,
            composer: Composer::formatted(),
            has_files: false,
            attach_error: None,
            send_waiting: None,
            reply: None,
            selecting: false,
            selection: None,
            history_drag: false,
            history_focused: false,
            hover_row: None,
            tool_focus: None,
            bar: None,
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
        watching: bool,
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
                m.forward_revealed.clone_from(&old.forward_revealed);
            }
        }
        self.hover_row = self
            .hover_row
            .and_then(|i| self.c.messages.get(i))
            .and_then(|m| ids.rows.get(&m.id).copied());
        let before: Vec<_> = self.c.messages.iter().map(|m| m.id.clone()).collect();
        let after: Vec<_> = c.messages.iter().map(|m| m.id.clone()).collect();
        let mut arrivals = std::mem::take(&mut self.rows.arrivals);
        arrivals.observe(
            &before,
            &after,
            watching && self.at_end && !self.loading && !state.0,
            std::time::Instant::now(),
        );
        self.rows = Rows::new(c.messages.len());
        self.rows.arrivals = arrivals;
        self.rows.set_width(Rows::body_width(self.width));
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

    pub(crate) fn content_h(&self) -> f32 {
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

    fn scroll_to(&mut self, id: &str, vh: f32) {
        if let Some(y) = self.ids.rows.get(id).and_then(|&i| self.row_y(i)) {
            self.scroll = (y - 16.0).clamp(0.0, (self.content_h() - vh).max(0.0));
            self.at_end = false;
        }
    }

    pub fn selection(&self) -> Option<crate::Sel> {
        let (a, b) = self.selection.as_ref()?;
        let end = |(id, at): &(String, u32)| {
            let i = *self.ids.rows.get(id)?;
            Some((i, (*at).min(self.rows.row(i)?.plain16.len() as u32)))
        };
        Some((end(a)?, end(b)?))
    }

    fn bar_rect(&self, i: usize, n: usize) -> Option<Rect> {
        let w = actions::bar_width(n);
        let y =
            (HEADER_H + self.row_y(i)? - self.scroll + self.rows.row(i)?.lead - 12.0).max(HEADER_H);
        Some((self.width - w - 16.0, y, w, actions::BAR_H))
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
        !self.loading
            && (self.has_files
                || !String::from_utf16_lossy(&self.composer.text)
                    .trim()
                    .is_empty())
    }

    fn reply_h(&self) -> f32 {
        self.reply.as_ref().map_or(0.0, |r| r.height + REPLY_GAP)
    }

    fn reply_rect(&self, h: f32) -> Option<Rect> {
        let r = self.reply.as_ref()?;
        Some((
            PAD,
            h - 16.0 - self.box_h - 8.0 - CHECK_H - self.reply_h(),
            self.width - 2.0 * PAD,
            r.height,
        ))
    }

    /// Draws the heading, the history at `scroll` within `vh`, the
    /// broadcast checkbox and Send; `h` is the pane's height.
    fn draw(
        &mut self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        h: f32,
        vh: f32,
        narrow: bool,
        media: Option<&mut crate::files::Ctx>,
    ) -> Result<()> {
        let w = self.width;
        unsafe {
            dc.Clear(Some(&gfx::color(SURFACE_CONVERSATION, 1.0)));
            // egui's panel separator on the leading edge.
            dc.FillRectangle(&rect(0.0, 0.0, 1.0, h), &g.brushes.border);
            dc.FillRectangle(&rect(0.0, HEADER_H - 1.0, w, HEADER_H), &g.brushes.border);
        }
        let text_x = if narrow { 60.0 } else { PAD };
        crate::text::draw(dc, &self.title, text_x, 8.0, &g.brushes.text);
        crate::text::draw(dc, &self.sub, text_x, 30.0, &g.brushes.muted);
        let (cx, cy, cw, ch) = close_local(w, narrow);
        let lit = self.hover == Some(Part::Close);
        if lit {
            fill_round(dc, rect(cx, cy, cx + cw, cy + ch), 6.0, &g.brushes.hover);
        }
        icons::draw(
            dc,
            if narrow {
                Icon::ChevronLeft
            } else {
                Icon::Close
            },
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
        let result = self.draw_history(g, dc, vh, media);
        if let Some((i, key)) = &self.bar
            && let Some((x, y, _, _)) = self.bar_rect(*i, key.tools.len())
        {
            actions::draw_bar(g, dc, key, x, y);
        }
        unsafe { dc.PopAxisAlignedClip() };
        // The composer's lip over the history's bottom, inside the leading separator.
        if vh > LIP_H {
            crate::draw_lip(dc, 1.0, HEADER_H + vh - LIP_H, w - 1.0);
        }
        result?;
        let reply_rect = self.reply_rect(h);
        if let (Some(r), Some((x, y, w, _))) = (&mut self.reply, reply_rect) {
            r.cancel_hover = self.hover == Some(Part::CancelReply);
            r.draw(g, dc, x, y, w);
        }
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
        // Send beside the box: as web, a chalk wash and ring rather than a
        // solid fill while there is something to send.
        let (sx, sy, sw, sh) = send_local(w, h, self.box_h);
        let r = rect(sx, sy, sx + sw, sy + sh);
        let enabled = self.can_send();
        if enabled {
            let wash = if self.hover == Some(Part::Send) {
                0.15
            } else {
                0.09
            };
            let (fill, ring) = unsafe {
                (
                    dc.CreateSolidColorBrush(&gfx::color(TEXT, wash), None)?,
                    dc.CreateSolidColorBrush(&gfx::color(TEXT, 0.24), None)?,
                )
            };
            fill_round(dc, r, 8.0, &fill);
            stroke_round(dc, r, 8.0, &ring);
        }
        let (color, alpha) = if enabled {
            (TEXT, 1.0)
        } else {
            (TEXT_MUTED, 0.5)
        };
        icons::draw(
            dc,
            Icon::Send,
            // The paper plane's weight sits up and right of its box: nudge it
            // to look centred.
            rect(sx + 8.0, sy + 10.0, sx + sw - 10.0, sy + sh - 8.0),
            color,
            alpha,
        );
        Ok(())
    }

    /// The history's rows, reply count, head and empty note (the caller clips).
    fn draw_history(
        &mut self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        vh: f32,
        mut media: Option<&mut crate::files::Ctx>,
    ) -> Result<()> {
        let w = self.width;
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
        let active_row = self.bar.as_ref().map(|(row, _)| *row).or(self.hover_row);
        for (i, y) in rows {
            let h = self.rows.height(i);
            if y + off < HEADER_H + vh && y + h + off > HEADER_H {
                let lead = self.rows.row(i).map_or(0.0, |r| r.lead);
                if active_row == Some(i) {
                    unsafe {
                        dc.FillRectangle(
                            &rect(0.0, y + off + lead, w, y + off + h),
                            &g.brushes.hover,
                        );
                    }
                }
                let sel = self
                    .rows
                    .row(i)
                    .and_then(|r| App::sel_for(self.selection(), i, r.plain16.len() as u32));
                list::draw_row(
                    g,
                    dc,
                    &self.c,
                    &mut self.rows,
                    i,
                    y + off,
                    sel,
                    media.as_deref_mut(),
                )?;
                if active_row == Some(i) {
                    list::draw_grouped_time(g, dc, &self.c, &self.rows, i, y + off)?;
                }
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
fn close_local(width: f32, narrow: bool) -> Rect {
    if narrow {
        (8.0, 5.0, 44.0, 44.0)
    } else {
        (width - PAD - CLOSE, (HEADER_H - CLOSE) / 2.0, CLOSE, CLOSE)
    }
}

/// The reply box's width.
fn box_w(width: f32) -> f32 {
    width - 2.0 * PAD - SEND - 8.0
}

/// Send, relative to the pane, at the bottom of a box `box_h` tall.
fn send_local(width: f32, h: f32, box_h: f32) -> Rect {
    let by = h - 16.0 - box_h;
    (
        PAD + box_w(width) + 8.0,
        by + (box_h - 42.0).max(0.0),
        SEND,
        SEND,
    )
}

/// The reply box's text width, beside Insert emoji and Send a GIF (when
/// it is shown).
fn text_w(width: f32, gif: bool) -> f32 {
    let gif_w = if gif { crate::uploads::GIF_W } else { 0.0 };
    box_w(width) - 2.0 * composer::PAD - crate::uploads::EMOJI_W - gif_w
}

/// Insert emoji, relative to the box, at the end of its first line.
fn emoji_local(width: f32) -> Rect {
    let cy = composer::PAD + composer::LINE / 2.0;
    (box_w(width) - 36.0, cy - 14.0, 28.0, 28.0)
}

/// Send a GIF, before Insert emoji (where egui's thread field keeps it).
fn gif_local(width: f32) -> Rect {
    let (x, y, w, h) = emoji_local(width);
    (x - crate::uploads::EMOJI_W, y, w, h)
}

impl App {
    /// The open thread's width beside the conversation.
    pub fn thread_w(&self) -> f32 {
        if self.thread.is_some() {
            if self.is_narrow() {
                self.w
            } else {
                self.panes
                    .width(crate::panes::Pane::Thread)
                    .min((self.w - self.list_x() - self.members_reserved()).max(0.0))
            }
        } else {
            0.0
        }
    }

    pub fn thread_x(&self) -> f32 {
        self.w - self.members_reserved() - self.thread_w()
    }

    /// Rewrap only on width changes, keeping the visible reply anchored.
    pub fn relayout_thread_width(&mut self) -> Result<()> {
        let width = self.thread_w();
        let Some(t) = &mut self.thread else {
            return Ok(());
        };
        if t.width == width {
            return Ok(());
        }
        let keep = (!t.at_end)
            .then(|| {
                (0..t.c.messages.len()).find_map(|i| {
                    let y = t.row_y(i)?;
                    (y + t.rows.height(i) > t.scroll).then_some((i, t.scroll - y))
                })
            })
            .flatten();
        t.width = width;
        t.rows.set_width(Rows::body_width(width));
        t.rows.measure_up_to(&self.g, &t.c, f64::NEG_INFINITY)?;
        if let Some((i, offset)) = keep {
            t.scroll = t.row_y(i).unwrap_or(0.0) + offset;
        }
        unsafe {
            t.sub.SetMaxWidth(width - 2.0 * PAD - CLOSE - 10.0)?;
            t.check.SetMaxWidth(width - 2.0 * PAD - CHECK - 7.0)?;
        }
        t.check_w = gfx::metrics(&t.check).widthIncludingTrailingWhitespace;
        if let Some(reply) = &t.reply {
            t.reply = Some(ReplyBar::new(
                &self.g,
                reply.target.clone(),
                width - 2.0 * PAD,
            )?);
        }
        self.thread_box_changed();
        self.thread_layout_changed();
        Ok(())
    }

    pub fn thread_tools(&self, i: usize) -> Vec<ToolState> {
        let Some(m) = self.thread.as_ref().and_then(|t| t.c.messages.get(i)) else {
            return Vec::new();
        };
        let enabled = !m.deleted && self.live.as_ref().is_some_and(|l| l.can_react());
        let mut tools = Vec::new();
        if !m.deleted {
            tools.extend(actions::QUICK.iter().enumerate().map(|(k, emoji)| {
                ToolState {
                    tool: Tool::Quick(k),
                    enabled,
                    on: m
                        .reactions
                        .iter()
                        .any(|r| r.mine && actions::same_emoji(&r.emoji, emoji)),
                }
            }));
            if enabled {
                tools.push(ToolState {
                    tool: Tool::Reply,
                    enabled,
                    on: false,
                });
            }
            tools.push(ToolState {
                tool: Tool::Emoji,
                enabled,
                on: false,
            });
        }
        tools.push(ToolState {
            tool: Tool::More,
            enabled: self.live.is_some(),
            on: false,
        });
        tools
    }

    fn thread_bar_row(&self) -> Option<usize> {
        let t = self.thread.as_ref()?;
        self.menu
            .as_ref()
            .filter(|m| m.in_thread)
            .and_then(|m| t.ids.rows.get(&m.id).copied())
            .or_else(|| {
                self.picker
                    .as_ref()
                    .filter(|p| p.row == usize::MAX && p.insert.is_none())
                    .and_then(|p| t.ids.rows.get(&p.id).copied())
            })
            .or_else(|| {
                t.tool_focus
                    .as_ref()
                    .and_then(|(id, _)| t.ids.rows.get(id).copied())
            })
            .or(t.hover_row)
            .filter(|&i| self.thread_part_rect(Part::Row(i)).is_some())
    }

    fn thread_bar_rect(&self, i: usize) -> Option<Rect> {
        let (x, y, w, h) = self
            .thread
            .as_ref()?
            .bar_rect(i, self.thread_tools(i).len())?;
        Some((self.thread_x() + x, y, w, h))
    }

    fn thread_bar(&self) -> Option<(usize, BarKey)> {
        let i = self.thread_bar_row()?;
        let t = self.thread.as_ref()?;
        let tools = self.thread_tools(i);
        let hover = tools.iter().position(|s| {
            t.hover == Some(Part::Tool(i, s.tool))
                || t.tool_focus
                    .as_ref()
                    .is_some_and(|(id, tool)| id == &t.c.messages[i].id && *tool == s.tool)
        });
        let open = if self
            .menu
            .as_ref()
            .is_some_and(|m| m.in_thread && m.id == t.c.messages[i].id)
        {
            Some(Tool::More)
        } else if self
            .picker
            .as_ref()
            .is_some_and(|p| p.row == usize::MAX && p.id == t.c.messages[i].id)
        {
            Some(Tool::Emoji)
        } else {
            None
        };
        Some((i, BarKey { tools, hover, open }))
    }

    fn track_thread_row(&mut self, x: f32, y: f32) {
        let row = self
            .thread_bar_row()
            .filter(|&i| self.thread_bar_rect(i).is_some_and(|r| contains(r, x, y)))
            .or_else(|| self.thread_row_at(x, y));
        if let Some(t) = &mut self.thread
            && t.hover_row != row
        {
            t.hover_row = row;
            self.dirty.thread = true;
            self.need = true;
        }
    }

    fn thread_row_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.in_thread(x, y) || y < HEADER_H || y >= HEADER_H + self.thread_vh() {
            return None;
        }
        let t = self.thread.as_ref()?;
        (0..t.c.messages.len()).find(|&i| {
            self.thread_part_rect(Part::Row(i))
                .is_some_and(|r| contains(r, x, y))
        })
    }

    fn thread_msg_hit(&self, x: f32, y: f32) -> Option<(usize, u32)> {
        let t = self.thread.as_ref()?;
        let y = y.clamp(HEADER_H, HEADER_H + self.thread_vh());
        let i = (0..t.c.messages.len())
            .filter_map(|i| self.thread_part_rect(Part::Row(i)).map(|r| (i, r)))
            .min_by(|(_, a), (_, b)| {
                let distance = |r: &Rect| (r.1 - y).max(0.0) + (y - r.1 - r.3).max(0.0);
                distance(a).total_cmp(&distance(b))
            })?
            .0;
        let (ox, oy) = self.thread_row_text_origin(i);
        Some((i, gfx::hit_point(&t.rows.row(i)?.text, x - ox, y - oy)))
    }

    fn over_thread_text(&self, x: f32, y: f32) -> bool {
        let Some(i) = self.thread_row_at(x, y) else {
            return false;
        };
        let Some(row) = self.thread.as_ref().and_then(|t| t.rows.row(i)) else {
            return false;
        };
        let (ox, oy) = self.thread_row_text_origin(i);
        gfx::on_text(&row.text, x - ox, y - oy)
    }

    pub fn set_thread_selection(&mut self, selection: Option<crate::Sel>) {
        self.set_thread_focus(false);
        self.set_composer_focus(false);
        self.set_kb(None);
        let Some(t) = &mut self.thread else { return };
        t.selection = selection.and_then(|(a, b)| {
            Some((
                (t.c.messages.get(a.0)?.id.clone(), a.1),
                (t.c.messages.get(b.0)?.id.clone(), b.1),
            ))
        });
        t.history_focused = true;
        t.tool_focus = None;
        self.dirty.thread = true;
        self.events |= EV_FOCUS | EV_TEXT;
        self.need = true;
    }

    pub fn thread_selected_text(&self) -> Vec<u16> {
        let Some(t) = &self.thread else {
            return Vec::new();
        };
        copy_selection(t.selection(), |i| {
            t.rows.row(i).map(|r| r.plain16.as_slice())
        })
    }

    pub fn focused_thread_part(&self) -> Option<Part> {
        let t = self
            .thread
            .as_ref()
            .filter(|t| t.history_focused && !t.composer.focused && !self.composer.focused)?;
        if let Some((id, tool)) = &t.tool_focus {
            return Some(Part::Tool(*t.ids.rows.get(id)?, *tool));
        }
        Some(Part::Row(t.selection()?.1.0))
    }

    pub fn focus_thread_tool(&mut self, i: usize, tool: Tool) {
        self.set_thread_focus(false);
        self.set_composer_focus(false);
        self.set_kb(None);
        let Some(t) = &mut self.thread else { return };
        let Some(m) = t.c.messages.get(i) else { return };
        t.tool_focus = Some((m.id.clone(), tool));
        t.history_focused = true;
        self.dirty.thread = true;
        self.events |= EV_FOCUS;
        self.need = true;
    }

    /// The history's height in the pane.
    pub fn thread_vh(&self) -> f32 {
        let box_h = self.thread.as_ref().map_or(BOX_MIN, |t| t.box_h);
        let reply_h = self.thread.as_ref().map_or(0.0, Pane::reply_h);
        (self.h - 16.0 - box_h - 8.0 - CHECK_H - reply_h - 12.0 - HEADER_H).max(0.0)
    }

    /// Wheel and middle-click scrolling share the same bounds/live-end state.
    pub fn scroll_thread_by(&mut self, delta: f32) {
        let vh = self.thread_vh();
        let Some(t) = &mut self.thread else { return };
        t.jump_glide = None;
        let max = (t.content_h() - vh).max(0.0);
        t.scroll = (t.scroll + delta).clamp(0.0, max);
        t.at_end = t.scroll >= max - 0.5;
        self.dirty.thread = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    pub fn thread_box_rect(&self) -> Option<Rect> {
        let t = self.thread.as_ref()?;
        Some((
            self.thread_x() + PAD,
            self.h - 16.0 - t.box_h,
            box_w(t.width),
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
                let (x, y, w, h) = close_local(t.width, self.is_narrow());
                Some((x0 + x, y, w, h))
            }
            Part::Jump => self.thread_jump_rect(),
            Part::Row(i) => {
                let y = t.row_y(i)?;
                clip((x0, y + off, t.width, t.rows.height(i)))
            }
            Part::Tool(i, tool) => {
                if self.thread_bar_row() != Some(i) {
                    return None;
                }
                let k = self.thread_tools(i).iter().position(|s| s.tool == tool)?;
                let (x, y, _, _) = self.thread_bar_rect(i)?;
                let (sx, sy, w, h) = actions::slot_rect(k);
                clip((x + sx, y + sy, w, h))
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
            Part::Forward(i, p) => self
                .forward_parts(true, i)
                .into_iter()
                .find(|(q, _, _)| *q == p)
                .map(|(_, r, _)| r),
            Part::Older => {
                let y = t.sections().head? + off;
                let head = t.head();
                if !head.button {
                    return None;
                }
                let r = list::head_button_rect(&self.g, &head, t.width).ok()?;
                clip((x0 + r.left, y + r.top, r.right - r.left, r.bottom - r.top))
            }
            Part::Reply | Part::CancelReply => {
                let (x, y, w, h) = t.reply_rect(self.h)?;
                Some(if part == Part::CancelReply {
                    (x0 + x + w - 52.0, y + 8.0, 44.0, 44.0)
                } else {
                    (x0 + x, y, w, h)
                })
            }
            Part::Broadcast => {
                let y = self.h - 16.0 - t.box_h - 8.0 - CHECK_H;
                Some((x0 + PAD, y, CHECK + 7.0 + t.check_w, CHECK_H))
            }
            Part::Field => self.thread_box_rect(),
            Part::Upload(p) => {
                let (bx, by, _, _) = self.thread_box_rect()?;
                let (_, tiles, top, _) = self.thread_uploads();
                let r = crate::uploads::part_rect(&tiles, top, p)?;
                Some((bx + r.left, by + r.top, r.right - r.left, r.bottom - r.top))
            }
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
                let (x, y, w, h) = gif_local(t.width);
                Some((bx + x, by + y + t.composer.inset.1, w, h))
            }
            Part::Emoji => {
                let (bx, by, _, _) = self.thread_box_rect()?;
                let (x, y, w, h) = emoji_local(t.width);
                Some((bx + x, by + y + t.composer.inset.1, w, h))
            }
            Part::RemovePreview => {
                self.thread_preview_card()?;
                let (bx, by, bw, bh) = self.thread_box_rect()?;
                let r = crate::preview::remove_rect(crate::preview::card_rect(bw, bh));
                Some((bx + r.left, by + r.top, r.right - r.left, r.bottom - r.top))
            }
            Part::Send => {
                let (x, y, w, h) = send_local(t.width, self.h, t.box_h);
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
                    out.extend(
                        self.thread_tools(i)
                            .into_iter()
                            .map(|s| Part::Tool(i, s.tool)),
                    );
                    for (p, _, _) in self.forward_parts(true, i) {
                        let part = Part::Forward(i, p);
                        if !out.contains(&part) {
                            out.push(part);
                        }
                    }
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
        if self.thread_jump_rect().is_some() {
            out.push(Part::Jump);
        }
        if t.reply.is_some() {
            out.extend([Part::Reply, Part::CancelReply]);
        }
        out.extend([Part::Broadcast, Part::Field]);
        if t.composer.format_height() > 0.0 {
            out.extend(composer::Style::ALL.map(Part::Format));
        }
        for (i, d) in self.thread_uploads().0.iter().enumerate() {
            use crate::uploads::Part as U;
            // Hit-test the small controls before the preview covering the tile.
            out.extend([Part::Upload(U::Remove(i)), Part::Upload(U::Spoiler(i))]);
            if d.upload.error.is_some() {
                out.push(Part::Upload(U::Retry(i)));
            }
            if crate::uploads::previewable(d) {
                out.push(Part::Upload(U::Open(i)));
            }
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
        // It floats over the rows and their toolbars.
        if self.thread_jump_rect().is_some_and(|r| contains(r, x, y)) {
            return Some(Part::Jump);
        }
        let parts = self.thread_parts();
        if y >= HEADER_H
            && y < HEADER_H + self.thread_vh()
            && let Some(p) = parts.iter().copied().find(|p| {
                matches!(p, Part::Tool(..))
                    && self.thread_part_rect(*p).is_some_and(|r| contains(r, x, y))
            })
        {
            return Some(p);
        }
        // Controls before the rows and the field that hold them.
        let holder = |p: &&Part| matches!(p, Part::Row(_) | Part::Field | Part::Reply);
        parts
            .iter()
            .filter(|p| !holder(p))
            .chain(parts.iter().filter(holder))
            .copied()
            .find(|p| match *p {
                Part::Tool(..) => false,
                Part::Row(_) | Part::Chip(..) | Part::Add(_) | Part::Older => {
                    y >= HEADER_H
                        && y < HEADER_H + self.thread_vh()
                        && self.thread_part_rect(*p).is_some_and(|r| contains(r, x, y))
                }
                // A spoiler takes only its own lines, not its bounds.
                Part::Spoiler(i, k) => self.thread_spoiler_at(x, y) == Some((i, k)),
                Part::Forward(i, part) => {
                    part != list::ForwardPart::Body
                        && y >= HEADER_H
                        && y < HEADER_H + self.thread_vh()
                        && self
                            .forward_parts(true, i)
                            .into_iter()
                            .any(|(p, r, _)| p == part && contains(r, x, y))
                }
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
            Part::Close => {
                if self.is_narrow() {
                    format!("Back to {}", t.destination)
                } else {
                    "Close thread".into()
                }
            }
            Part::Tool(_, tool) => tool.label(),
            Part::Older => {
                let head = t.head();
                match head.note {
                    Some(note) => format!("{note} {}", head.label),
                    None => head.label.into(),
                }
            }
            Part::Broadcast => format!("Also send to {}", t.destination),
            Part::Reply => t.reply.as_ref().map_or_else(String::new, |r| {
                format!("Replying to {}: {}", r.target.author.name, r.target.text)
            }),
            Part::CancelReply => "Cancel reply".into(),
            Part::Jump => t.jump.name(),
            Part::Field => "Reply to thread".into(),
            Part::Upload(p) => {
                use crate::uploads::Part as U;
                let i = match p {
                    U::Open(i) | U::Remove(i) | U::Retry(i) | U::Spoiler(i) => i,
                    _ => return String::new(),
                };
                let drafts = self.thread_uploads().0;
                let Some(d) = drafts.get(i) else {
                    return String::new();
                };
                let action = match p {
                    U::Open(_) => "Preview",
                    U::Remove(_) => "Remove",
                    U::Retry(_) => "Retry uploading",
                    _ if d.upload.spoiler => "Remove spoiler from",
                    _ => "Mark as spoiler:",
                };
                format!("{action} {}", d.upload.name)
            }
            Part::Format(style) => style.name().into(),
            Part::Gif => "Send a GIF".into(),
            Part::Emoji => "Insert emoji".into(),
            Part::RemovePreview => "Remove link preview".into(),
            Part::Send => "Send reply".into(),
            Part::Add(_) => "Add reaction".into(),
            Part::Spoiler(..) => "Spoiler, reveal".into(),
            Part::Forward(i, p) => self
                .forward_parts(true, i)
                .into_iter()
                .find(|(q, _, _)| *q == p)
                .map_or_else(String::new, |(_, _, name)| name),
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
            Part::Tool(i, tool) => self
                .thread_tools(i)
                .iter()
                .any(|s| s.tool == tool && s.enabled),
            _ => true,
        }
    }

    /// The thread row and the address of the message link under client point
    /// (x, y).
    pub fn thread_text_link_at(&self, x: f32, y: f32) -> Option<(usize, String)> {
        let Some(Part::Row(i)) = self.thread_hit(x, y) else {
            return None;
        };
        let row = self.thread.as_ref()?.rows.row(i)?;
        let (ox, oy) = self.thread_row_text_origin(i);
        if !gfx::on_text(&row.text, x - ox, y - oy) {
            return None;
        }
        let p = gfx::hit_point(&row.text, x - ox, y - oy);
        let href = row
            .links
            .iter()
            .find_map(|(s, l, href)| (p >= *s && p < s + l).then(|| href.clone()))?;
        Some((i, href))
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
        self.thread_shown()
            && !self.on_member_overlay(x, y)
            && x >= self.thread_x()
            && x < self.thread_x() + self.thread_w()
            && y < self.h
    }

    pub fn thread_file_rect(&self, i: usize, k: usize) -> Option<crate::overlay::Rect> {
        let t = self.thread.as_ref()?;
        let m = t.c.messages.get(i)?;
        let r = t.rows.file_rect(i, m.grouped, k)?;
        let y = HEADER_H + t.row_y(i)? - t.scroll;
        Some((
            self.thread_x() + r.left,
            y + r.top,
            r.right - r.left,
            r.bottom - r.top,
        ))
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
            let mut pane = Pane::new(
                &self.g,
                root,
                &self.c.place(),
                self.panes.width(crate::panes::Pane::Thread),
            )?;
            pane.gif = self.gif_shown();
            pane.box_h = pane
                .composer
                .relayout(&self.g, text_w(pane.width, pane.gif))?
                .max(BOX_MIN);
            self.thread = Some(pane);
            self.thread_layout_changed();
            // The new thread's reply box is empty: its link card starts over.
            self.preview_tick();
            // Its files may have kept uploading while another thread was open.
            self.thread_box_changed();
        }
        let vh = self.thread_vh();
        let watching = self.watching_arrivals() && self.thread_shown();
        let known: Option<Vec<String>> = self
            .thread
            .as_ref()
            .filter(|p| !opened && !p.loading)
            .map(|p| p.c.messages.iter().map(|m| m.id.clone()).collect());
        if let Some(p) = &mut self.thread {
            p.set(&self.g, view, state, vh, watching)?;
        }
        // Others' replies landing after the newest one shown before are
        // counted while Jump to latest shows.
        if let (Some(known), Some(p), Some(live)) = (known, &self.thread, &self.live) {
            let tail =
                p.c.messages
                    .iter()
                    .rposition(|m| known.contains(&m.id))
                    .map_or(0, |k| k + 1);
            let others = p.c.messages[tail..]
                .iter()
                .filter(|m| {
                    p.ids.members.get(m.author).is_some_and(|id| {
                        live.me.as_deref() != Some(id.as_str()) && !live.is_blocked(id)
                    })
                })
                .count();
            for _ in 0..others {
                self.thread_jump_arrived();
            }
        }
        self.arm_arrivals();
        if let Some(id) = self.live.as_mut().and_then(|l| l.take_thread_focus())
            && let Some(p) = &mut self.thread
        {
            p.scroll_to(&id, vh);
            // Pins, inbox and forwarded sources arrive here from a control
            // that has closed or scrolled away; keep the keyboard somewhere.
            self.set_thread_focus(true);
        }
        if self
            .thread
            .as_ref()
            .is_some_and(|t| !t.loading && t.send_waiting.is_some())
        {
            self.thread_files_changed();
        }
        self.dirty.thread = true;
        self.events |= EV_SCREEN;
        self.need = true;
        Ok(())
    }

    fn thread_layout_changed(&mut self) {
        let vh = self.thread_vh();
        if let Some(t) = &mut self.thread {
            let max = (t.content_h() - vh).max(0.0);
            t.scroll = if t.at_end { max } else { t.scroll.min(max) };
        }
        self.dirty.layout = true;
        // The narrow header leaves out its buttons under a thread.
        self.dirty.header = true;
        self.dirty.thread = true;
        self.dirty.thread_box = true;
        self.drawn = None;
        self.events |= EV_SCREEN | EV_ROWS;
        self.need = true;
    }

    /// Start or continue a channel message's thread and accept typing even
    /// while its history loads. A broadcast reply opens its existing root.
    pub fn reply_in_thread(&mut self, id: &str) {
        let Some(m) = self.live.as_ref().and_then(|l| l.message(id)) else {
            return;
        };
        let root = m.thread_root_id.unwrap_or(m.id);
        self.close_menu();
        self.close_picker();
        self.close_pins();
        if let Some(live) = &mut self.live {
            live.open_thread(&root);
        }
        if let Err(e) = self.drain_live() {
            eprintln!("thread failed: {e:?}");
        }
        self.set_thread_focus(true);
    }

    /// Quote a root or reply without changing the channel draft or creating
    /// a nested thread. Menu actions keep the message ID, not its row index.
    pub fn select_thread_reply(&mut self, id: &str) {
        let Some(live) = &self.live else { return };
        let Some(m) = live.message(id).filter(|m| !m.is_deleted()) else {
            return;
        };
        let Some(t) = &mut self.thread else { return };
        if !live.can_react() || (m.id != t.root && m.thread_root_id.as_deref() != Some(&t.root)) {
            return;
        }
        match ReplyBar::new(
            &self.g,
            caper_client::model::MessageReply::from_message(&m),
            t.width - 2.0 * PAD,
        ) {
            Ok(reply) => t.reply = Some(reply),
            Err(e) => {
                eprintln!("thread reply layout failed: {e:?}");
                return;
            }
        }
        self.close_menu();
        self.close_pins();
        self.set_thread_focus(true);
        self.thread_layout_changed();
    }

    pub fn cancel_thread_reply(&mut self) {
        if self
            .thread
            .as_mut()
            .is_some_and(|t| t.reply.take().is_some())
        {
            self.thread_layout_changed();
        }
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

    /// Close (Back when narrow) or Escape: the keys return to the composer.
    pub fn dismiss_thread(&mut self) {
        self.close_thread();
        self.set_composer_focus(true);
    }

    /// The channel row `i`'s thread link under client point (x, y).
    pub fn thread_link_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.in_history(x, y) || self.live.is_none() {
            return None;
        }
        let ay = self.scroll_top + (y - self.history_y()) as f64;
        let i = self.rows.row_at(ay)?;
        let ry = (ay - self.rows.top[i]) as f32;
        let r = self.rows.thread_link(i, self.c.messages[i].grouped)?;
        let lx = x - self.list_x();
        (lx >= r.left && lx < r.right && ry >= r.top && ry < r.bottom).then_some(i)
    }

    pub fn set_thread_focus(&mut self, focused: bool) {
        if focused {
            self.narrow.focus = None;
            self.clear_pane_focus();
            self.clear_side_focus();
        }
        let Some(t) = &mut self.thread else {
            return;
        };
        if focused {
            t.history_focused = false;
            t.tool_focus = None;
        }
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

    fn thread_uploads(
        &self,
    ) -> (
        Vec<&crate::uploads::Draft>,
        Vec<windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F>,
        f32,
        Option<String>,
    ) {
        let Some(t) = &self.thread else {
            return (Vec::new(), Vec::new(), 0.0, None);
        };
        let drafts = self
            .live
            .as_ref()
            .map_or_else(Vec::new, |l| l.composer_drafts(Some(&t.root)));
        let failed = drafts.iter().filter(|d| d.upload.error.is_some()).count();
        let notice = caper_client::drafts::failed_notice(failed)
            .or_else(|| t.attach_error.clone())
            .or_else(|| {
                t.send_waiting
                    .as_ref()
                    .map(|_| crate::uploads::UPLOAD_WAIT.into())
            });
        let (mut tiles, mut top) =
            crate::uploads::strip(&vec![false; drafts.len()], box_w(t.width), false);
        let bar = t.composer.format_height();
        for tile in &mut tiles {
            tile.top += bar;
            tile.bottom += bar;
        }
        top += bar;
        if let Some(notice) = &notice {
            top += 6.0
                + self
                    .g
                    .layout(
                        &gfx::wide(notice),
                        &self.g.formats.file_detail,
                        box_w(t.width) - 2.0 * composer::PAD,
                    )
                    .map_or(18.0, |l| gfx::metrics(&l).height);
        }
        (drafts, tiles, top, notice)
    }

    /// Upload events only complete the pending send for this exact draft.
    pub fn thread_files_changed(&mut self) {
        let mut send = false;
        if let (Some(t), Some(live)) = (&mut self.thread, &self.live)
            && let Some((text, keys)) = &t.send_waiting
        {
            let drafts = live.composer_drafts(Some(&t.root));
            let now: Vec<_> = drafts.iter().map(|d| d.upload.key).collect();
            if *text != t.composer.text
                || *keys != now
                || drafts.iter().any(|d| d.upload.error.is_some())
            {
                t.send_waiting = None;
            } else {
                send = drafts
                    .iter()
                    .all(|d| !d.upload.uploading && !d.upload.needs_upload());
            }
        }
        self.thread_box_changed();
        if send {
            self.send_thread_reply();
        }
    }

    /// After an edit or caret move in the reply box.
    pub fn thread_composer_changed(&mut self) {
        if let Some(t) = &mut self.thread
            && t.send_waiting
                .as_ref()
                .is_some_and(|(text, _)| *text != t.composer.text)
        {
            t.send_waiting = None;
        }
        // The reply's link card follows its text (and lays the box out
        // itself when the card changed).
        self.preview_tick();
        self.thread_box_changed();
    }

    /// Lays the reply box out again: its text, format buttons and link card.
    pub fn thread_box_changed(&mut self) {
        let gif = self.gif_shown();
        let (files, _, top, _) = self.thread_uploads();
        let has_files = !files.is_empty();
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
        t.has_files = has_files;
        t.composer.inset.1 = top;
        t.composer.inset_bottom = card;
        if let Ok(h) = t.composer.relayout(g, text_w(t.width, gif)) {
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
        if !self.live.as_ref().is_some_and(|l| l.can_react()) {
            return;
        }
        let Some(t) = &mut self.thread else {
            return;
        };
        if !t.can_send() {
            return;
        }
        let text = String::from_utf16_lossy(&t.composer.text)
            .trim()
            .to_string();
        let (root, broadcast) = (t.root.clone(), t.broadcast);
        let reply = t.reply.as_ref().map(|r| r.target.id.clone());
        let Some(live) = &mut self.live else { return };
        let keys = live
            .composer_drafts(Some(&root))
            .iter()
            .map(|d| d.upload.key)
            .collect();
        if live.send_with_files(text, reply, Some((root, broadcast))) == live::Sent::Waiting {
            t.send_waiting = Some((t.composer.text.clone(), keys));
            self.thread_box_changed();
            return;
        }
        t.send_waiting = None;
        t.attach_error = None;
        t.composer.set_text(Vec::new());
        t.broadcast = false;
        t.at_end = true;
        self.cancel_thread_reply();
        self.thread_composer_changed();
    }

    pub fn thread_activate(&mut self, part: Part) {
        match part {
            Part::Upload(p) => {
                use crate::uploads::Part as U;
                let i = match p {
                    U::Open(i) | U::Remove(i) | U::Retry(i) | U::Spoiler(i) => i,
                    _ => return,
                };
                let drafts = self.thread_uploads().0;
                let Some(d) = drafts.get(i) else { return };
                let key = d.upload.key;
                let open = d
                    .upload
                    .local_path
                    .as_ref()
                    .map(|p| p.to_string_lossy().into_owned())
                    .or_else(|| d.upload.attachment.as_ref().and_then(|a| a.url.clone()));
                if matches!(p, U::Open(_)) {
                    if !self.open_attachment_editor(key)
                        && let Some(target) = open
                    {
                        crate::open_link(&target);
                    }
                    return;
                }
                if let Some(live) = &mut self.live {
                    match p {
                        U::Remove(_) => {
                            live.remove_draft(key);
                        }
                        U::Retry(_) => live.retry_draft(key),
                        U::Spoiler(_) => live.toggle_spoiler(key),
                        _ => {}
                    }
                }
                if let Some(t) = &mut self.thread {
                    t.attach_error = None;
                }
                self.drafts_changed();
            }
            Part::Tool(i, tool) => {
                let Some(state) = self
                    .thread_tools(i)
                    .into_iter()
                    .find(|s| s.tool == tool && s.enabled)
                else {
                    return;
                };
                let Some(id) = self
                    .thread
                    .as_ref()
                    .and_then(|t| t.c.messages.get(i))
                    .map(|m| m.id.clone())
                else {
                    return;
                };
                match tool {
                    Tool::Quick(k) => {
                        if let Some(live) = &mut self.live {
                            live.react(&id, actions::QUICK[k], !state.on);
                        }
                        if let Err(e) = self.drain_live() {
                            eprintln!("reaction update failed: {e:?}");
                        }
                    }
                    Tool::Reply => self.select_thread_reply(&id),
                    Tool::Emoji => {
                        if let Some((x, y, _, h)) = self.thread_bar_rect(i) {
                            self.open_picker_for(usize::MAX, id, (x, y + h + 4.0));
                        }
                    }
                    Tool::More => {
                        if let Some((x, y, _, h)) = self.thread_bar_rect(i) {
                            self.open_thread_menu(i, x, y + h + 4.0, false);
                        }
                    }
                    Tool::Thread => {}
                }
            }
            Part::Format(style) => {
                if let Some(t) = &mut self.thread
                    && t.composer.format_rect(style).is_some()
                    && t.composer.toggle(style)
                {
                    self.thread_composer_changed();
                }
            }
            Part::Close => self.dismiss_thread(),
            Part::CancelReply => self.cancel_thread_reply(),
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
            Part::Jump => self.thread_jump_activate(),
            Part::Spoiler(i, k) => self.reveal_thread_spoiler(i, k),
            Part::Forward(i, p) => self.forward_activate(true, i, p),
            Part::Row(_) | Part::Reply => {}
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
                self.track_thread_row(x, y);
                if self.thread.as_ref()?.history_drag {
                    if let (Some(end), Some((start, _))) =
                        (self.thread_msg_hit(x, y), self.thread.as_ref()?.selection())
                    {
                        self.set_thread_selection(Some((start, end)));
                    }
                    return done;
                }
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
                    .filter(|p| !matches!(p, Part::Row(_) | Part::Field | Part::Reply));
                self.set_thread_hover(hover);
                None
            }
            WM_MOUSELEAVE => {
                self.thread.as_mut()?.hover_row = None;
                self.set_thread_hover(None);
                self.dirty.thread = true;
                self.need = true;
                None
            }
            WM_LBUTTONDOWN if self.in_thread(x, y) => {
                let part = self.thread_hit(x, y);
                self.thread.as_mut()?.tool_focus = None;
                self.thread.as_mut()?.history_focused = false;
                match part {
                    Some(Part::Row(_))
                        if let Some((id, entry, author)) = self.thread_person_at(x, y) =>
                    {
                        self.set_thread_focus(false);
                        self.open_profile(id, entry, author, x, y);
                    }
                    Some(Part::Row(_)) => {
                        if let Some(hit) = self.thread_msg_hit(x, y) {
                            let anchor = if shift {
                                self.thread.as_ref()?.selection().map_or(hit, |s| s.0)
                            } else {
                                hit
                            };
                            self.set_thread_selection(Some((anchor, hit)));
                            self.thread.as_mut()?.history_drag = true;
                            unsafe {
                                SetCapture(self.hwnd);
                            }
                        }
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
                    Some(part @ (Part::Format(_) | Part::RemovePreview | Part::CancelReply)) => {
                        self.thread_activate(part)
                    }
                    Some(Part::Forward(i, part)) => {
                        self.set_thread_focus(false);
                        self.set_composer_focus(false);
                        self.forward_click(true, i, part, x, y);
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
            WM_LBUTTONDOWN => {
                self.thread.as_mut()?.history_focused = false;
                self.thread.as_mut()?.tool_focus = None;
                None
            }
            WM_LBUTTONDBLCLK if matches!(self.thread_hit(x, y), Some(Part::Row(_))) => {
                if let Some((i, p)) = self.thread_msg_hit(x, y) {
                    let text = &self.thread.as_ref()?.rows.row(i)?.plain16;
                    let word = |c: u16| char::from_u32(c as u32).is_some_and(char::is_alphanumeric);
                    let (mut s, mut e) = (p as usize, p as usize);
                    while s > 0 && word(text[s - 1]) {
                        s -= 1;
                    }
                    while e < text.len() && word(text[e]) {
                        e += 1;
                    }
                    self.set_thread_selection(Some(((i, s as u32), (i, e as u32))));
                }
                done
            }
            WM_LBUTTONUP if self.thread.as_ref()?.history_drag => {
                let clicked = self
                    .thread
                    .as_ref()?
                    .selection()
                    .is_some_and(|(a, b)| a == b);
                self.thread.as_mut()?.history_drag = false;
                unsafe {
                    let _ = ReleaseCapture();
                }
                if clicked && let Some((_, href)) = self.thread_text_link_at(x, y) {
                    self.set_thread_selection(None);
                    crate::open_link(&href);
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
                self.thread.as_mut()?.history_drag = false;
                None
            }
            WM_CONTEXTMENU if x < 0.0 && y < 0.0 && self.thread.as_ref()?.history_focused => {
                let part = self.focused_thread_part()?;
                let i = match part {
                    Part::Row(i) | Part::Tool(i, _) => i,
                    _ => return None,
                };
                if let Some((x, y, _, h)) = self.thread_bar_rect(i) {
                    self.open_thread_menu(i, x, y + h, false);
                }
                done
            }
            WM_MOUSEWHEEL if self.pointer.is_some_and(|(x, y)| self.in_thread(x, y)) => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32;
                self.scroll_thread_by(-delta / 120.0 * WHEEL_STEP as f32);
                done
            }
            WM_SETCURSOR
                if self.thread.as_ref()?.history_drag
                    || self.pointer.is_some_and(|(x, y)| self.in_thread(x, y)) =>
            {
                let (px, py) = self.pointer?;
                self.track_thread_row(px, py);
                let cursor = match self.thread_hit(px, py) {
                    Some(Part::Field) => IDC_IBEAM,
                    _ if self.thread.as_ref()?.history_drag => IDC_IBEAM,
                    Some(Part::Row(_))
                        if self.thread_person_at(px, py).is_some()
                            || self.thread_text_link_at(px, py).is_some() =>
                    {
                        IDC_HAND
                    }
                    Some(Part::Row(_)) if self.over_thread_text(px, py) => IDC_IBEAM,
                    Some(Part::Row(_) | Part::Reply) | None => IDC_ARROW,
                    Some(_) => IDC_HAND,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                Some(Some(LRESULT(1)))
            }
            WM_KEYDOWN
                if !focused && !self.composer.focused && self.thread.as_ref()?.history_focused =>
            {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                if ctrl && vk == VIRTUAL_KEY('C' as u16) {
                    let text = self.thread_selected_text();
                    if !text.is_empty() {
                        crate::set_clipboard(self.hwnd, &text);
                    }
                    return done;
                }
                if vk == VK_ESCAPE {
                    self.set_thread_selection(None);
                    return done;
                }
                if (vk == VK_RETURN || vk == VK_SPACE)
                    && let Some(part @ Part::Tool(..)) = self.focused_thread_part()
                {
                    self.thread_activate(part);
                    return done;
                }
                if vk == VK_TAB
                    && let Some(part) = self.focused_thread_part()
                {
                    let (i, tool) = match part {
                        Part::Row(i) => (i, None),
                        Part::Tool(i, tool) => (i, Some(tool)),
                        _ => return None,
                    };
                    let tools = self.thread_tools(i);
                    let current = tool.and_then(|t| tools.iter().position(|s| s.tool == t));
                    let next = if shift {
                        current.map_or(Some(tools.len() - 1), |k| k.checked_sub(1))
                    } else {
                        Some(current.map_or(0, |k| k + 1)).filter(|k| *k < tools.len())
                    };
                    if let Some(k) = next {
                        self.focus_thread_tool(i, tools[k].tool);
                    } else {
                        self.set_thread_focus(true);
                    }
                    return done;
                }
                if vk == VK_END {
                    self.stop_autoscroll();
                }
                let vh = self.thread_vh();
                let t = self.thread.as_mut()?;
                let max = (t.content_h() - vh).max(0.0);
                t.scroll = match vk {
                    VK_PRIOR => (t.scroll - vh * 0.9).max(0.0),
                    VK_NEXT => (t.scroll + vh * 0.9).min(max),
                    VK_HOME if ctrl => 0.0,
                    VK_END => max,
                    _ => return None,
                };
                t.at_end = t.scroll >= max - 0.5;
                self.dirty.thread = true;
                self.events |= EV_SCREEN;
                self.need = true;
                done
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
                    // As web's textarea: Tab moves on (the sidebar, past the
                    // page's end) and Shift+Tab back to the main composer.
                    VK_TAB if !ctrl && !shift => {
                        self.tab_to_sidebar();
                        return done;
                    }
                    VK_TAB if !ctrl => {
                        self.set_composer_focus(true);
                        return done;
                    }
                    VK_RETURN if shift => t.composer.insert(&['\n' as u16]),
                    VK_RETURN => {
                        self.send_thread_reply();
                        return done;
                    }
                    // As on web and in egui, Escape closes the thread even
                    // from its reply box (suggestions and IME take it first).
                    VK_ESCAPE => {
                        self.dismiss_thread();
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

    /// Advance visible thread GIFs without replacing the channel's timer.
    pub fn thread_media_tick(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_MEDIA);
        }
        if self.thread.is_some() {
            self.dirty.thread = true;
            self.need = true;
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
        let bar = self.thread_bar();
        if let Some(t) = &mut self.thread
            && t.bar != bar
        {
            t.bar = bar;
            self.dirty.thread = true;
        }
        let card = self.thread_preview_card().cloned();
        let thumb = self.preview_thumb(true);
        let (x, h, vh) = (self.thread_x(), self.h, self.thread_vh());
        let (bx, by, bw, bh) = self.thread_box_rect().unwrap_or_default();
        let video = self.inline_video_view();
        let video_focus = self.video_focus_view();
        let width = self.thread_w();
        let narrow = self.is_narrow();
        let g = &self.g;
        let fresh = self.thread_p.ensure(g, px(width), px(h))?;
        self.thread_p.offset((x * s).round(), 0.0)?;
        if (fresh || std::mem::take(&mut self.dirty.thread))
            && let Some(t) = &mut self.thread
        {
            let mut tex = crate::files::D2d(g.dc.clone());
            let mut media = self.live.as_mut().map(|l| crate::files::Ctx {
                voice: self.playback.view(),
                video,
                video_focus,
                ..crate::files::Ctx::new(&mut l.media, &mut tex)
            });
            self.thread_p.draw(s, |dc| {
                t.draw(g, dc, h, vh, narrow, media.as_mut())?;
                if self.panes.focus == Some(crate::panes::Pane::Thread) {
                    unsafe {
                        dc.FillRectangle(&rect(0.0, 0.0, 2.0, h), &g.brushes.terracotta);
                    }
                }
                Ok(())
            })?;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_MEDIA);
                if let Some(ctx) = media {
                    if let Some(next) = ctx.next {
                        SetTimer(
                            Some(self.hwnd),
                            TIMER_MEDIA,
                            next.as_millis().clamp(10, u32::MAX as u128) as u32,
                            None,
                        );
                    }
                    if ctx.media.busy() {
                        SetTimer(Some(self.hwnd), crate::TIMER_LIVE, 100, None);
                    }
                }
            }
        }
        let Some(t) = &self.thread else {
            return Ok(());
        };
        let fresh = self.thread_box_p.ensure(g, px(bw), px(bh))?;
        self.thread_box_p
            .offset((bx * s).round(), (by * s).round())?;
        if fresh || std::mem::take(&mut self.dirty.thread_box) {
            let (drafts, tiles, top, notice) = self.thread_uploads();
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
                for (draft, tile) in drafts.iter().zip(&tiles) {
                    crate::uploads::tile(g, dc, draft, *tile)?;
                }
                if let Some(notice) = &notice {
                    let l = g.layout(
                        &gfx::wide(notice),
                        &g.formats.file_detail,
                        bw - 2.0 * composer::PAD,
                    )?;
                    crate::text::draw(
                        dc,
                        &l,
                        composer::PAD,
                        top - gfx::metrics(&l).height,
                        if notice == crate::uploads::UPLOAD_WAIT {
                            &g.brushes.muted
                        } else {
                            &g.brushes.error
                        },
                    );
                }
                if t.gif {
                    let (x, y, w, h) = gif_local(t.width);
                    let y = y + t.composer.inset.1;
                    let color = if lit { TEXT } else { TEXT_MUTED };
                    crate::uploads::gif_button(g, dc, rect(x, y, x + w, y + h), color)?;
                }
                let (x, y, w, h) = emoji_local(t.width);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copying_a_thread_selection_preserves_order_unicode_and_partial_endpoints() {
        let rows = ["alpha 👋", "middle reply", "omega"].map(gfx::wide);
        let row = |i: usize| rows.get(i).map(Vec::as_slice);
        let expected = gfx::wide("👋\r\nmiddle reply\r\nome");
        assert_eq!(copy_selection(Some(((0, 6), (2, 3))), row), expected);
        assert_eq!(copy_selection(Some(((2, 3), (0, 6))), row), expected);
        // UIA endpoints inside an emoji must not put broken UTF-16 on the clipboard.
        assert_eq!(copy_selection(Some(((0, 7), (0, 8))), row), gfx::wide("👋"));
        assert_eq!(copy_selection(Some(((0, 6), (0, 7))), row), gfx::wide("👋"));
        assert!(copy_selection(Some(((0, 7), (0, 7))), row).is_empty());
        // An edit shortened the selected message; clamp rather than panic or copy a neighbour.
        assert_eq!(
            copy_selection(Some(((2, 2), (2, 99))), row),
            gfx::wide("ega")
        );
        assert!(copy_selection(None, row).is_empty());
    }
}
