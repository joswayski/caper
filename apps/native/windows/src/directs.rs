//! Direct messages, as in the egui client: the sidebar's "Direct messages"
//! section under the channels ("Message requests" with its count, opening
//! to the requests, then your own DM, then each conversation with its
//! avatar, unread count or dot and its Blocked / Request sent note),
//! opening a DM in place of the channel (no member list), what an empty
//! conversation says, the bar that replaces the composer in a request
//! (Accept, Decline or Block) or a DM you blocked (Unblock), and the
//! "Waiting for @x to accept" note above your own request's composer. The
//! row rules are the shared `caper_client::model::DirectConversation` ones.
use crate::gfx::{self, Gfx, fill_round, rect};
use crate::live::Live;
use crate::overlay::{Rect, Tone};
use crate::theme::*;
use crate::{App, text};
use caper_client::model::{DirectConversation, DirectStatus};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::*;

/// A row and its pitch, egui's 28 + 2.
const ROW_H: f32 = 28.0;
const PITCH: f32 = 30.0;
/// The gap between the waiting note and what is under it.
const NOTE_GAP: f32 = 6.0;
/// The gap above the section and its label's height.
const GAP: f32 = 14.0;
const LABEL_H: f32 = 28.0;

/// What the pointer is over in the section: its heading, the heading's
/// "New direct message" button, or a row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spot {
    Heading,
    New,
    Row(usize),
}

/// What a row of the section is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Direct,
    /// "Message requests", open or closed, with the requests' count.
    Requests {
        open: bool,
    },
    /// An incoming request, under "Message requests" while it is open.
    Request,
}

/// One row of the section.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: RowKind,
    /// The conversation; None for your own DM before it exists.
    pub id: Option<String>,
    pub name: String,
    /// "you" on your own DM, else Blocked or Request sent; a request's
    /// @username.
    pub note: Option<String>,
    pub own: bool,
    pub unread: bool,
    pub count: Option<u64>,
    pub color: u32,
    pub initials: String,
    pub active: bool,
}

impl Row {
    /// Its accessible name, as egui's `channel_button` words it.
    pub fn label(&self) -> String {
        match self.kind {
            RowKind::Requests { .. } => return self.name.clone(),
            RowKind::Request => {
                let note = self.note.as_deref().unwrap_or_default();
                return format!("Message request from {} {note}", self.name);
            }
            RowKind::Direct => {}
        }
        let name = match &self.note {
            Some(note) => format!("{} {note}", self.name),
            None => self.name.clone(),
        };
        match (self.count, self.unread) {
            (Some(n), true) => format!("{name}, {n} unread messages"),
            (_, true) => format!("{name}, unread activity"),
            _ => name,
        }
    }
}

impl Live {
    /// The section's rows: "Message requests" (and the requests while it
    /// is open), your own DM, then the others in the server's order.
    pub fn direct_rows(&self) -> Vec<Row> {
        let Some(me) = &self.account_id else {
            return Vec::new();
        };
        let open = self.direct.as_deref();
        let row = |d: Option<&DirectConversation>, name: &str, id: &str| {
            let p = crate::live::person(id, name);
            let unread = d.is_some_and(|d| !d.blocked && d.has_unread());
            Row {
                kind: RowKind::Direct,
                id: d.map(|d| d.id.clone()),
                name: name.to_owned(),
                note: None,
                own: false,
                unread,
                count: d.and_then(|d| d.unread_count).filter(|n| unread && *n > 0),
                color: p.color,
                initials: p.initials,
                active: d.is_some_and(|d| open == Some(d.id.as_str())),
            }
        };
        let mut out = Vec::new();
        let requests: Vec<_> = self
            .directs
            .iter()
            .filter(|d| d.status == DirectStatus::Incoming)
            .collect();
        if !requests.is_empty() {
            // Open while asked, or while one of them is open.
            let shown = self.requests_open || requests.iter().any(|d| open == Some(d.id.as_str()));
            out.push(Row {
                kind: RowKind::Requests { open: shown },
                id: None,
                name: "Message requests".into(),
                note: None,
                own: false,
                unread: false,
                count: Some(requests.len() as u64),
                color: 0,
                initials: String::new(),
                active: false,
            });
            if shown {
                // Requests never show unread, as in egui.
                out.extend(requests.iter().map(|d| Row {
                    kind: RowKind::Request,
                    note: Some(format!("@{}", d.peer.username)),
                    unread: false,
                    count: None,
                    ..row(Some(d), &d.peer.display_name, &d.peer.id)
                }));
            }
        }
        let own = self.directs.iter().find(|d| &d.peer.id == me);
        out.push(Row {
            note: Some("you".into()),
            own: true,
            // egui's own row shows only a dot.
            count: None,
            ..row(own, &self.account_name, me)
        });
        out.extend(
            self.directs
                .iter()
                .filter(|d| &d.peer.id != me && d.status != DirectStatus::Incoming)
                .map(|d| Row {
                    note: d.row_suffix(false).map(str::to_owned),
                    ..row(Some(d), &d.peer.display_name, &d.peer.id)
                }),
        );
        out
    }
}

/// Draws the section in the sidebar (sidebar-local DIPs), its label at `top`.
pub fn draw(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    rows: &[Row],
    top: f32,
    hover: Option<Spot>,
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let label = g.layout(&gfx::wide("Direct messages"), &g.formats.section, 10_000.0)?;
    text::draw(dc, &label, 20.0, top + GAP + 4.0, &g.brushes.muted);
    // The heading's "+" shows while the heading is hovered, as in egui.
    if matches!(hover, Some(Spot::Heading | Spot::New)) {
        let y = top + GAP;
        let b = rect(SIDEBAR_W - 40.0, y, SIDEBAR_W - 12.0, y + 28.0);
        let on = hover == Some(Spot::New);
        if on {
            fill_round(dc, b, 6.0, &g.brushes.hover);
        }
        let i = rect(b.left + 5.0, b.top + 5.0, b.right - 5.0, b.bottom - 5.0);
        crate::icons::draw(
            dc,
            crate::icons::Icon::Plus,
            i,
            if on { TEXT } else { TEXT_MUTED },
            1.0,
        );
    }
    for (k, row) in rows.iter().enumerate() {
        let y = top + GAP + LABEL_H + PITCH * k as f32;
        let r = rect(12.0, y, SIDEBAR_W - 12.0, y + ROW_H);
        let lit = hover == Some(Spot::Row(k));
        if row.active {
            fill_round(dc, r, 6.0, &g.brushes.selected);
        } else if lit {
            fill_round(dc, r, 6.0, &g.brushes.hover);
        }
        let (cx, cy) = (r.left + 17.5, y + ROW_H / 2.0);
        if let RowKind::Requests { open } = row.kind {
            draw_requests(g, dc, row, r, open, lit)?;
            continue;
        }
        let e = D2D1_ELLIPSE {
            point: windows_numerics::Vector2 { X: cx, Y: cy },
            radiusX: 10.0,
            radiusY: 10.0,
        };
        unsafe { dc.FillEllipse(&e, &g.solid(row.color)) };
        let i = g.layout(&gfx::wide(&row.initials), &g.formats.direct_initials, 1e6)?;
        let m = gfx::metrics(&i);
        text::draw(
            dc,
            &i,
            cx - m.widthIncludingTrailingWhitespace / 2.0,
            cy - m.height / 2.0,
            &g.brushes.text,
        );
        // From the right: the unread count or dot, then the name.
        let mut right = r.right - 8.0;
        if let Some(n) = row.count {
            let label = if n > 99 { "99+".into() } else { n.to_string() };
            let l = g.layout(&gfx::wide(&label), &g.formats.badge, 1e6)?;
            let w = gfx::metrics(&l).widthIncludingTrailingWhitespace + 12.0;
            let b = rect(right - w, cy - 9.0, right, cy + 9.0);
            fill_round(dc, b, 8.0, &g.brushes.terracotta);
            // The badge format centres its text in the layout's box.
            unsafe {
                l.SetMaxWidth(w)?;
                l.SetMaxHeight(18.0)?;
            }
            text::draw(dc, &l, b.left, b.top, &g.brushes.text);
            right = b.left - 8.0;
        } else if row.unread {
            let e = D2D1_ELLIPSE {
                point: windows_numerics::Vector2 {
                    X: right - 6.0,
                    Y: cy,
                },
                radiusX: 3.0,
                radiusY: 3.0,
            };
            unsafe { dc.FillEllipse(&e, &g.brushes.terracotta_bright) };
            right -= 16.0;
        }
        let x = r.left + 32.0;
        let format = if row.unread {
            &g.formats.direct_unread
        } else {
            &g.formats.direct_name
        };
        let name = g.layout(&gfx::wide(&row.name), format, (right - x).max(1.0))?;
        unsafe {
            name.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let sign = DWRITE_TRIMMING {
                granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                delimiter: 0,
                delimiterCount: 0,
            };
            let ellipsis = g.dwrite.CreateEllipsisTrimmingSign(format)?;
            name.SetTrimming(&sign, &ellipsis)?;
        }
        crate::emoji::apply(&name, &row.name, 13.0, 1.0)?;
        let m = gfx::metrics(&name);
        let brush = if row.active || lit || row.unread {
            &g.brushes.text
        } else {
            &g.brushes.muted
        };
        text::draw(dc, &name, x, cy - 1.0 - m.height / 2.0, brush);
        if let Some(note) = &row.note {
            let nx = x + m.widthIncludingTrailingWhitespace.min(right - x) + 5.0;
            if nx < right {
                let l = g.layout(&gfx::wide(note), &g.formats.direct_note, right - nx)?;
                let nm = gfx::metrics(&l);
                text::draw(dc, &l, nx, cy - 1.0 - nm.height / 2.0, &g.brushes.muted);
            }
        }
    }
    Ok(())
}

/// "Message requests": egui's chevron (pointing down while open), the
/// label, and the count at the row's right.
fn draw_requests(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    row: &Row,
    r: D2D_RECT_F,
    open: bool,
    lit: bool,
) -> Result<()> {
    let (cx, cy) = (r.left + 17.5, (r.top + r.bottom) / 2.0);
    let brush = if lit {
        &g.brushes.text
    } else {
        &g.brushes.muted
    };
    let p = |x: f32, y: f32| windows_numerics::Vector2 { X: x, Y: y };
    let (a, b, c) = if open {
        (
            p(cx - 4.0, cy - 2.0),
            p(cx, cy + 2.0),
            p(cx + 4.0, cy - 2.0),
        )
    } else {
        (
            p(cx - 2.0, cy - 4.0),
            p(cx + 2.0, cy),
            p(cx - 2.0, cy + 4.0),
        )
    };
    unsafe {
        dc.DrawLine(a, b, brush, 1.6, None);
        dc.DrawLine(b, c, brush, 1.6, None);
    }
    let count = g.layout(
        &gfx::wide(&row.count.unwrap_or_default().to_string()),
        &g.formats.direct_note,
        1e6,
    )?;
    let cm = gfx::metrics(&count);
    let right = r.right - 12.0;
    text::draw(
        dc,
        &count,
        right - cm.widthIncludingTrailingWhitespace,
        cy - cm.height / 2.0,
        &g.brushes.muted,
    );
    let x = r.left + 32.0;
    let name = g.layout(&gfx::wide(&row.name), &g.formats.direct_name, 1e6)?;
    let m = gfx::metrics(&name);
    text::draw(dc, &name, x, cy - 1.0 - m.height / 2.0, brush);
    Ok(())
}

/// Which bar replaces the composer in a DM.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarKind {
    /// An incoming request: Accept, Decline or Block.
    Request,
    /// You blocked the other person: Unblock.
    Blocked,
}

/// What replaces the composer in a DM, as egui's `request_bar` and
/// `blocked_composer`: who it is, the bar's buttons, and why the last
/// answer failed.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    pub kind: BarKind,
    pub name: String,
    pub username: String,
    /// The answer in flight: Some(true) while declining.
    pub busy: Option<bool>,
    /// An unblock in flight.
    pub unblocking: bool,
    /// Caper's own accounts can't be blocked: no Block button.
    pub protected: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarPart {
    Text,
    Accept,
    Decline,
    Error,
    Block,
    Unblock,
}

impl BarPart {
    pub fn index(self) -> usize {
        self as usize
    }

    /// Whether the part is one of the bar's buttons.
    pub fn button(self) -> bool {
        !matches!(self, Self::Text | Self::Error)
    }
}

const BAR_BUTTON_H: f32 = 36.0;

impl Bar {
    pub fn text(&self) -> String {
        match self.kind {
            BarKind::Request => format!(
                "{} (@{}) wants to message you. You don't share a space.",
                self.name, self.username
            ),
            BarKind::Blocked => format!("You blocked @{}.", self.username),
        }
    }

    /// Each part's accessible name (a button's label as drawn).
    pub fn label(&self, part: BarPart) -> String {
        match part {
            BarPart::Text => self.text(),
            // Only the clicked action says it is in progress.
            BarPart::Accept if self.busy == Some(false) => "Accepting\u{2026}".into(),
            BarPart::Accept => "Accept".into(),
            BarPart::Decline if self.busy == Some(true) => "Declining\u{2026}".into(),
            BarPart::Decline => "Decline".into(),
            BarPart::Block => "Block".into(),
            BarPart::Unblock if self.unblocking => "Unblocking\u{2026}".into(),
            BarPart::Unblock => "Unblock".into(),
            BarPart::Error => self.error.clone().unwrap_or_default(),
        }
    }

    fn layout_text(&self, g: &Gfx, w: f32) -> Result<IDWriteTextLayout> {
        let text = self.text();
        let l = g.layout(&gfx::wide(&text), &g.formats.dialog_note, w)?;
        if self.kind == BarKind::Request {
            let range = DWRITE_TEXT_RANGE {
                startPosition: 0,
                length: gfx::wide(&self.name).len() as u32,
            };
            unsafe {
                l.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, range)?;
                l.SetDrawingEffect(&g.brushes.text, range)?;
            }
        }
        crate::emoji::apply(&l, &text, 13.0, 1.0)?;
        Ok(l)
    }

    fn button_w(&self, g: &Gfx, part: BarPart) -> Result<f32> {
        let l = g.layout(&gfx::wide(&self.label(part)), &g.formats.button, 1e6)?;
        Ok((gfx::metrics(&l).widthIncludingTrailingWhitespace + 24.0).max(88.0))
    }

    /// The parts' rectangles from the bar's origin for a bar `w` wide, and
    /// its height.
    pub fn layout(&self, g: &Gfx, w: f32) -> Result<(Vec<(BarPart, Rect)>, f32)> {
        let m = gfx::metrics(&self.layout_text(g, w)?);
        let mut parts = Vec::new();
        let mut y;
        match self.kind {
            BarKind::Request => {
                parts.push((BarPart::Text, (0.0, 0.0, w, m.height)));
                y = m.height + 8.0;
                let mut x = 0.0;
                let mut buttons = vec![BarPart::Accept, BarPart::Decline];
                if !self.protected {
                    buttons.push(BarPart::Block);
                }
                for part in buttons {
                    let bw = self.button_w(g, part)?;
                    parts.push((part, (x, y, bw, BAR_BUTTON_H)));
                    x += bw + 8.0;
                }
                y += BAR_BUTTON_H;
            }
            BarKind::Blocked => {
                // The note and Unblock on one line, as egui's row, unless
                // the bar is too narrow.
                let tw = m.widthIncludingTrailingWhitespace.ceil() + 1.0;
                let bw = self.button_w(g, BarPart::Unblock)?;
                if tw + 8.0 + bw <= w {
                    let ty = (BAR_BUTTON_H - m.height) / 2.0;
                    parts.push((BarPart::Text, (0.0, ty, tw, m.height)));
                    parts.push((BarPart::Unblock, (tw + 8.0, 0.0, bw, BAR_BUTTON_H)));
                    y = BAR_BUTTON_H;
                } else {
                    parts.push((BarPart::Text, (0.0, 0.0, w, m.height)));
                    y = m.height + 8.0;
                    parts.push((BarPart::Unblock, (0.0, y, bw, BAR_BUTTON_H)));
                    y += BAR_BUTTON_H;
                }
            }
        }
        if let Some(e) = &self.error {
            y += 6.0;
            let l = g.layout(&gfx::wide(e), &g.formats.error, w)?;
            let h = gfx::metrics(&l).height;
            parts.push((BarPart::Error, (0.0, y, w, h)));
            y += h;
        }
        Ok((parts, y))
    }

    pub fn enabled(&self, part: BarPart) -> bool {
        match part {
            BarPart::Accept | BarPart::Decline | BarPart::Block => self.busy.is_none(),
            BarPart::Unblock => !self.unblocking,
            BarPart::Text | BarPart::Error => true,
        }
    }

    /// Draws the bar `w` wide (bar-local DIPs) over a clear surface.
    pub fn draw(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        w: f32,
        hover: Option<BarPart>,
        pressed: Option<BarPart>,
    ) -> Result<()> {
        unsafe { dc.Clear(Some(&D2D1_COLOR_F::default())) };
        let (parts, _) = self.layout(g, w)?;
        for (part, (x, y, pw, ph)) in parts {
            match part {
                BarPart::Text => {
                    let l = self.layout_text(g, pw)?;
                    text::draw(dc, &l, x, y, &g.brushes.muted);
                }
                BarPart::Error => {
                    let l = g.layout(&gfx::wide(&self.label(part)), &g.formats.error, pw)?;
                    text::draw(dc, &l, x, y, &g.brushes.error);
                }
                _ => {
                    let state = if !self.enabled(part) {
                        0
                    } else if pressed == Some(part) && hover == Some(part) {
                        3
                    } else if hover == Some(part) {
                        2
                    } else {
                        1
                    };
                    let tone = if part == BarPart::Accept {
                        Tone::Primary
                    } else {
                        Tone::Secondary
                    };
                    crate::overlay::draw_button(
                        g,
                        dc,
                        &self.label(part),
                        (x, y, pw, ph),
                        tone,
                        state,
                    )?;
                }
            }
        }
        Ok(())
    }
}

impl App {
    /// The open DM's bar, which replaces the composer.
    pub fn dm_bar(&self) -> Option<Bar> {
        self.live.as_ref().filter(|_| self.c.is_live())?.dm_bar()
    }

    /// Lays out the waiting note for the composer `bw` wide: its text and
    /// height, or None without one.
    pub fn layout_note(&mut self, bw: f32) -> Result<()> {
        self.note = match self
            .live
            .as_ref()
            .filter(|_| self.c.is_live())
            .and_then(Live::waiting_note)
        {
            Some(t) => {
                let l = self
                    .g
                    .layout(&gfx::wide(&t), &self.g.formats.dialog_note, bw)?;
                Some((t, gfx::metrics(&l).height))
            }
            None => None,
        };
        Ok(())
    }

    /// The note's height and the gap under it; 0 without a note.
    pub fn note_h(&self) -> f32 {
        self.note.as_ref().map_or(0.0, |(_, h)| h + NOTE_GAP)
    }

    /// The waiting note's rectangle, above the reply and the composer.
    pub fn note_rect(&self) -> Option<Rect> {
        let (_, h) = self.note.as_ref()?;
        let (bx, by, bw, _) = self.box_rect();
        Some((bx, by - self.reply_h() - NOTE_GAP - h, bw, *h))
    }

    /// Draws the waiting note `w` wide over a clear surface.
    pub fn draw_note(g: &Gfx, dc: &ID2D1DeviceContext, note: &str, w: f32) -> Result<()> {
        unsafe { dc.Clear(Some(&D2D1_COLOR_F::default())) };
        let l = g.layout(&gfx::wide(note), &g.formats.dialog_note, w)?;
        text::draw(dc, &l, 0.0, 0.0, &g.brushes.muted);
        Ok(())
    }

    /// The bar's parts in client DIPs.
    pub fn bar_parts(&self) -> Vec<(BarPart, Rect)> {
        let Some(bar) = self.dm_bar() else {
            return Vec::new();
        };
        let (bx, by, bw, _) = self.box_rect();
        bar.layout(&self.g, bw).map_or_else(
            |_| Vec::new(),
            |(parts, _)| {
                parts
                    .into_iter()
                    .map(|(p, (x, y, w, h))| (p, (bx + x, by + y, w, h)))
                    .collect()
            },
        )
    }

    /// The bar's enabled button under (x, y).
    pub fn bar_at(&self, x: f32, y: f32) -> Option<BarPart> {
        let bar = self.dm_bar()?;
        self.bar_parts()
            .into_iter()
            .find(|(p, (l, t, w, h))| {
                p.button() && bar.enabled(*p) && x >= *l && x < l + w && y >= *t && y < t + h
            })
            .map(|(p, _)| p)
    }

    /// A click (or UIA Invoke) on one of the bar's buttons. Block asks
    /// first, as egui's `confirm_block`.
    pub fn bar_activate(&mut self, part: BarPart) {
        if !self.dm_bar().is_some_and(|b| b.enabled(part)) {
            return;
        }
        if part == BarPart::Block {
            if let Some((account, request)) = self.live.as_ref().and_then(Live::block_target) {
                self.open_dialog(crate::overlay::Dialog::block(account, request));
            }
            return;
        }
        if let Some(live) = &mut self.live {
            match part {
                BarPart::Accept => live.answer_request(true),
                BarPart::Decline => live.answer_request(false),
                BarPart::Unblock => live.unblock_open(),
                BarPart::Block | BarPart::Text | BarPart::Error => return,
            }
        }
        self.bar_changed();
    }

    /// The bar appeared, went, or its state changed: the composer's slot
    /// is laid out and drawn again.
    pub fn bar_changed(&mut self) {
        if self.dm_bar().is_some() {
            self.set_composer_focus(false);
        }
        self.dirty.layout = true;
        self.dirty.composer = true;
        self.events |= crate::EV_SCREEN;
        self.need = true;
    }

    pub fn track_bar_hover(&mut self) {
        let hover = self.pointer.and_then(|(x, y)| self.bar_at(x, y));
        if hover != self.bar_hover {
            self.bar_hover = hover;
            self.dirty.composer = true;
            self.need = true;
        }
    }

    /// The section's rows (none without a live account).
    pub fn direct_rows(&self) -> Vec<Row> {
        self.live
            .as_ref()
            .filter(|_| self.c.is_live())
            .map_or_else(Vec::new, Live::direct_rows)
    }

    /// The section's top, under the channels.
    pub fn directs_top(&self) -> f32 {
        CHANNEL_TOP + CHANNEL_H * self.c.channels.len() as f32
    }

    /// Row `k`'s rectangle in client DIPs.
    pub fn direct_rect(&self, k: usize) -> Rect {
        (
            RAIL_W + 12.0,
            self.directs_top() + GAP + LABEL_H + PITCH * k as f32,
            SIDEBAR_W - 24.0,
            ROW_H,
        )
    }

    /// The section's label and rows, for UI Automation.
    pub fn directs_rect(&self, n: usize) -> Rect {
        (
            RAIL_W,
            self.directs_top() + GAP,
            SIDEBAR_W,
            LABEL_H + PITCH * n as f32,
        )
    }

    /// The heading's "New direct message" button.
    pub fn new_direct_rect(&self) -> Rect {
        (
            RAIL_W + SIDEBAR_W - 40.0,
            self.directs_top() + GAP,
            28.0,
            28.0,
        )
    }

    pub fn direct_at(&self, x: f32, y: f32) -> Option<Spot> {
        let n = self.direct_rows().len();
        let inside = |(l, t, w, h): Rect| x >= l && x < l + w && y >= t && y < t + h;
        if n == 0 {
            return None;
        }
        if inside(self.new_direct_rect()) {
            return Some(Spot::New);
        }
        let top = self.directs_top() + GAP;
        if inside((RAIL_W, top, SIDEBAR_W, LABEL_H)) {
            return Some(Spot::Heading);
        }
        (0..n).find(|&k| inside(self.direct_rect(k))).map(Spot::Row)
    }

    pub fn direct_click(&mut self, spot: Spot) {
        match spot {
            Spot::New => self.open_start_direct(),
            Spot::Row(k) => self.open_direct_row(k),
            Spot::Heading => {}
        }
    }

    /// Opens row `k`'s DM (your own is created first when it is new), or
    /// opens or closes "Message requests".
    pub fn open_direct_row(&mut self, k: usize) {
        let Some(row) = self.direct_rows().into_iter().nth(k) else {
            return;
        };
        let Some(live) = &mut self.live else { return };
        match (row.kind, row.own, row.id) {
            (RowKind::Requests { open }, ..) => {
                live.requests_open = !open;
                self.dirty.sidebar = true;
                self.events |= crate::EV_ROWS;
                self.need = true;
            }
            (_, true, _) => live.open_self_direct(),
            (_, false, Some(id)) => live.open_direct(id),
            (_, false, None) => {}
        }
    }

    /// The members column's width: none in a DM, as in egui.
    pub fn members_w(&self) -> f32 {
        if self.c.direct.is_some() {
            0.0
        } else {
            MEMBERS_W
        }
    }

    /// The open DM's name or note changed.
    pub fn direct_head_changed(&mut self) {
        let head = self.live.as_ref().and_then(Live::direct_head);
        if self.c.direct != head && self.c.direct.is_some() {
            self.c.direct = head;
            self.dirty.header = true;
            self.dirty.composer = true;
            self.need = true;
        }
    }

    /// "No messages yet." and its note in an empty live conversation,
    /// centred in the history.
    pub fn draw_empty(&mut self) -> Result<()> {
        let shown = self.live.is_some() && self.c.is_live() && self.c.messages.is_empty();
        let key = shown.then(|| (self.c.empty_text(), self.list_w(), self.vh()));
        if key == self.empty_drawn {
            return Ok(());
        }
        self.empty_drawn = key.clone();
        let Some(((title, note), lw, vh)) = key else {
            return self.empty_p.offset(-10_000.0, 0.0);
        };
        let s = self.scale;
        let g = &self.g;
        let w = (lw - 48.0).max(40.0);
        let t = title
            .map(|t| g.layout(&gfx::wide(t), &g.formats.message, w))
            .transpose()?;
        let n = g.layout(&gfx::wide(&note), &g.formats.head, w)?;
        for l in t.iter().chain([&n]) {
            unsafe { l.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)? };
        }
        let th = t.as_ref().map_or(0.0, |l| gfx::metrics(l).height + 4.0);
        let h = th + gfx::metrics(&n).height;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        self.empty_p.ensure(g, px(w), px(h))?;
        self.empty_p.offset(
            ((self.list_x() + 24.0) * s).round(),
            ((HEADER_H + (vh - h) / 2.0) * s).round(),
        )?;
        self.empty_p.draw(s, |dc| {
            unsafe { dc.Clear(Some(&D2D1_COLOR_F::default())) };
            if let Some(t) = &t {
                text::draw(dc, t, 0.0, 0.0, &g.brushes.text);
            }
            text::draw(dc, &n, 0.0, th, &g.brushes.muted);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_labels_read_like_eguis() {
        let row = Row {
            kind: RowKind::Direct,
            id: Some("dm".into()),
            name: "Maya".into(),
            note: None,
            own: false,
            unread: true,
            count: Some(3),
            color: 0,
            initials: "M".into(),
            active: false,
        };
        assert_eq!(row.label(), "Maya, 3 unread messages");
        let row = Row {
            count: None,
            note: Some("Request sent".into()),
            ..row
        };
        assert_eq!(row.label(), "Maya Request sent, unread activity");
        let row = Row {
            unread: false,
            note: Some("you".into()),
            ..row
        };
        assert_eq!(row.label(), "Maya you");
        let row = Row {
            kind: RowKind::Request,
            note: Some("@maya".into()),
            ..row
        };
        assert_eq!(row.label(), "Message request from Maya @maya");
    }
}
