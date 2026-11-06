//! Direct messages, as in the egui client: the sidebar's "Direct messages"
//! section under the channels (your own DM first, then each conversation
//! with its avatar, unread count or dot and its Blocked / Request sent
//! note), opening a DM in place of the channel (no member list), and what
//! an empty conversation says. The row rules are the shared
//! `caper_client::model::DirectConversation` ones; incoming requests are
//! left to their own list, as in egui.
use crate::gfx::{self, Gfx, fill_round, rect};
use crate::live::Live;
use crate::overlay::Rect;
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

/// One row of the section.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The conversation; None for your own DM before it exists.
    pub id: Option<String>,
    pub name: String,
    /// "you" on your own DM, else Blocked or Request sent.
    pub note: Option<&'static str>,
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
        let name = match self.note {
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
    /// The section's rows: your own DM, then the others in the server's
    /// order, without incoming requests.
    pub fn direct_rows(&self) -> Vec<Row> {
        let Some(me) = &self.account_id else {
            return Vec::new();
        };
        let open = self.direct.as_deref();
        let row = |d: Option<&DirectConversation>, name: &str, id: &str| {
            let p = crate::live::person(id, name);
            let unread = d.is_some_and(|d| !d.blocked && d.has_unread());
            Row {
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
        let own = self.directs.iter().find(|d| &d.peer.id == me);
        let mut out = vec![Row {
            note: Some("you"),
            own: true,
            // egui's own row shows only a dot.
            count: None,
            ..row(own, &self.account_name, me)
        }];
        out.extend(
            self.directs
                .iter()
                .filter(|d| &d.peer.id != me && d.status != DirectStatus::Incoming)
                .map(|d| Row {
                    note: d.row_suffix(false),
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
        if let Some(note) = row.note {
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

impl App {
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

    /// Opens row `k`'s DM (your own is created first when it is new).
    pub fn open_direct_row(&mut self, k: usize) {
        let Some(row) = self.direct_rows().into_iter().nth(k) else {
            return;
        };
        let Some(live) = &mut self.live else { return };
        match (row.own, row.id) {
            (true, _) => live.open_self_direct(),
            (false, Some(id)) => live.open_direct(id),
            (false, None) => {}
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
            note: Some("Request sent"),
            ..row
        };
        assert_eq!(row.label(), "Maya Request sent, unread activity");
        let row = Row {
            unread: false,
            note: Some("you"),
            ..row
        };
        assert_eq!(row.label(), "Maya you");
    }
}
