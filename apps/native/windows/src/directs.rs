//! Direct messages, as in the egui client: the sidebar's "Direct messages"
//! section under the channels ("Message requests" with its count, opening
//! to the requests, then your own DM, then each conversation with its
//! avatar, unread count or dot and its Blocked / Request sent note),
//! opening a DM in place of the channel (no member list), what an empty
//! conversation says, the bar that replaces the composer in a request
//! (Accept, Decline or Block) or a DM you blocked (Unblock), and the
//! "Waiting for @x to accept" note above your own request's composer. The
//! row rules are the shared `caper_client::model::DirectConversation` ones.
//! Each conversation's options button opens its notification menu (on or
//! off, mute for a while or unmute), and a muted one shows a bell-slash.
//! The same menu serves the space (the sidebar header's button: when voice
//! starts, the level, mute) and each channel (its row's button: the level,
//! mute). The section's first row opens the notification inbox, with its
//! unread count.
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::live::Live;
use crate::overlay::{Rect, Tone};
use crate::theme::*;
use crate::{App, text};
use caper_client::model::{DirectConversation, DirectStatus, NotificationLevel};
use caper_client::notifications::Scope;
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
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
/// "New direct message" button, a row, or a row's options button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spot {
    Heading,
    New,
    Row(usize),
    Options(usize),
}

/// The options menu's entries, as egui's `notification_items` words them;
/// egui's submenus are headings here, with their choices under them.
#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem {
    /// "When voice starts" (spaces), heading Default (account), On, Off.
    VoiceHeading,
    /// A voice choice, and whether it is the current one.
    Voice(Option<bool>, bool),
    /// "Notifications" (spaces and channels), heading the levels.
    LevelHeading,
    /// A level (None: "Default (…)", named for what it inherits), its
    /// label, and whether it is the current one.
    Level(Option<NotificationLevel>, String, bool),
    /// A DM's "Turn off notifications" while they are on, else "Turn on…".
    Toggle(bool),
    /// "Mute space", "Mute channel" or "Mute conversation", heading the
    /// mute presets.
    MuteHeading(&'static str),
    /// `caper_client::notifications::MUTE_PRESETS` entry `k`.
    Mute(usize),
    Unmute(&'static str),
    /// Text only: when the mute ends, "Loading…", or an error (true).
    Note(String, bool),
    /// The menu's last item: "Leave space…" (true, in red) or "Leave
    /// channel", opening its confirmation.
    Leave(bool),
    /// The space menu's first item: "Browse channels".
    Browse,
    /// "Invite people…", for the open space's owner (the space's menu
    /// and its public channels').
    Invite,
    /// The owner's "Space settings" and "Channel settings".
    SpaceSettings,
    ChannelSettings,
}

impl MenuItem {
    pub fn label(&self) -> String {
        match self {
            Self::VoiceHeading => "When voice starts".into(),
            Self::Voice(None, _) => "Default (account)".into(),
            Self::Voice(Some(true), _) => "On".into(),
            Self::Voice(Some(false), _) => "Off".into(),
            Self::LevelHeading => "Notifications".into(),
            Self::Level(_, label, _) => label.clone(),
            Self::Toggle(true) => "Turn off notifications".into(),
            Self::Toggle(false) => "Turn on notifications".into(),
            Self::MuteHeading(noun) => format!("Mute {noun}"),
            Self::Mute(k) => caper_client::notifications::MUTE_PRESETS
                .get(*k)
                .map_or("", |p| p.0)
                .into(),
            Self::Unmute(noun) => format!("Unmute {noun}"),
            Self::Note(text, _) => text.clone(),
            Self::Leave(true) => "Leave space\u{2026}".into(),
            Self::Leave(false) => "Leave channel".into(),
            Self::Browse => "Browse channels".into(),
            Self::Invite => "Invite people\u{2026}".into(),
            Self::SpaceSettings => "Space settings".into(),
            Self::ChannelSettings => "Channel settings".into(),
        }
    }

    /// Whether choosing it changes something.
    pub fn action(&self) -> bool {
        matches!(
            self,
            Self::Toggle(_)
                | Self::Mute(_)
                | Self::Unmute(_)
                | Self::Voice(..)
                | Self::Level(..)
                | Self::Leave(_)
                | Self::Browse
                | Self::Invite
                | Self::SpaceSettings
                | Self::ChannelSettings
        )
    }

    /// Whether it can be chosen now: the notification choices once the
    /// settings are ready (`enabled`), Browse and Leave always.
    pub fn usable(&self, enabled: bool) -> bool {
        self.action()
            && (enabled
                || matches!(
                    self,
                    Self::Leave(_)
                        | Self::Browse
                        | Self::Invite
                        | Self::SpaceSettings
                        | Self::ChannelSettings
                ))
    }

    /// Whether it is one of a heading's choices (indented under it).
    pub fn choice(&self) -> bool {
        matches!(self, Self::Mute(_) | Self::Voice(..) | Self::Level(..))
    }

    /// Whether it is the current choice (ticked).
    pub fn chosen(&self) -> bool {
        matches!(self, Self::Voice(_, true) | Self::Level(_, _, true))
    }
}

/// The open notification menu (a DM's, a channel's or the space's), by
/// its scope, and the item under the pointer or keyboard.
pub struct DmMenu {
    pub scope: Scope,
    pub hover: Option<usize>,
}

const MENU_W: f32 = 232.0;
const MENU_ITEM_H: f32 = 32.0;

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

/// What a row of the section is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    Direct,
    /// "Notifications", opening the inbox, with its unread count.
    Notifications,
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
    /// The bell-slash label while muted, and why the last notification
    /// change failed.
    pub muted: Option<String>,
    pub error: Option<String>,
}

impl Row {
    /// Whether it has an options menu: another person's conversation.
    pub fn menu(&self) -> bool {
        self.kind == RowKind::Direct && !self.own && self.id.is_some()
    }

    /// Its accessible name, as egui's `channel_button` words it.
    pub fn label(&self) -> String {
        match self.kind {
            RowKind::Requests { .. } => return self.name.clone(),
            RowKind::Notifications => {
                return match self.count {
                    Some(n) => format!("{} \u{b7} {n}", self.name),
                    None => self.name.clone(),
                };
            }
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
                muted: None,
                error: None,
            }
        };
        // egui's Notifications button, under the section's heading.
        let unread = self.activity.unread_count;
        let mut out = vec![Row {
            kind: RowKind::Notifications,
            id: None,
            name: "Notifications".into(),
            note: None,
            own: false,
            unread: false,
            count: (unread > 0).then_some(unread),
            color: 0,
            initials: String::new(),
            active: self.activity.shown,
            muted: None,
            error: None,
        }];
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
                muted: None,
                error: None,
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
                .map(|d| {
                    // A muted or blocked DM shows no unread, as in egui.
                    let muted = self.direct_muted(&d.id);
                    let blocked = self.is_blocked(&d.peer.id);
                    let unread = muted.is_none() && !d.blocked && !blocked && d.has_unread();
                    Row {
                        note: d.row_suffix(blocked).map(str::to_owned),
                        unread,
                        count: d.unread_count.filter(|n| unread && *n > 0),
                        muted,
                        error: self.direct_notify_error(&d.id),
                        ..row(Some(d), &d.peer.display_name, &d.peer.id)
                    }
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
    menu: Option<usize>,
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
        let lit = matches!(hover, Some(Spot::Row(j) | Spot::Options(j)) if j == k);
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
        if row.kind == RowKind::Notifications {
            draw_inbox(g, dc, row, r, lit)?;
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
        // From the right: the options button while revealed, the unread
        // count or dot, the bell-slash, then the name.
        let mut right = r.right - 8.0;
        if row.menu() && (lit || menu == Some(k)) {
            let b = rect(r.right - 32.0, cy - 12.0, r.right - 8.0, cy + 12.0);
            if hover == Some(Spot::Options(k)) || menu == Some(k) {
                fill_round(dc, b, 6.0, &g.brushes.hover);
            }
            crate::icons::draw(
                dc,
                crate::icons::Icon::More,
                rect(b.left + 4.0, b.top + 4.0, b.right - 4.0, b.bottom - 4.0),
                TEXT,
                1.0,
            );
            right = b.left - 4.0;
        }
        if let Some(n) = row.count {
            right = badge(g, dc, n, right, cy)? - 8.0;
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
        if row.muted.is_some() {
            crate::icons::draw(
                dc,
                crate::icons::Icon::BellOff,
                rect(right - 14.0, cy - 7.0, right, cy + 7.0),
                TEXT_MUTED,
                1.0,
            );
            right -= 20.0;
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
        // A failed notification change shows in place of the note.
        if let Some(note) = row.error.as_ref().or(row.note.as_ref()) {
            let nx = x + m.widthIncludingTrailingWhitespace.min(right - x) + 5.0;
            if nx < right {
                let l = g.layout(&gfx::wide(note), &g.formats.direct_note, right - nx)?;
                let nm = gfx::metrics(&l);
                let brush = if row.error.is_some() {
                    &g.brushes.error
                } else {
                    &g.brushes.muted
                };
                text::draw(dc, &l, nx, cy - 1.0 - nm.height / 2.0, brush);
            }
        }
    }
    Ok(())
}

/// A terracotta count ending at `right`, centred on `cy`; returns its
/// left edge.
fn badge(g: &Gfx, dc: &ID2D1DeviceContext, n: u64, right: f32, cy: f32) -> Result<f32> {
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
    Ok(b.left)
}

/// "Notifications": a bell, the label, and the unread count.
fn draw_inbox(g: &Gfx, dc: &ID2D1DeviceContext, row: &Row, r: D2D_RECT_F, lit: bool) -> Result<()> {
    let cy = (r.top + r.bottom) / 2.0;
    let on = lit || row.active;
    crate::icons::draw(
        dc,
        crate::icons::Icon::Bell,
        rect(r.left + 10.0, cy - 7.5, r.left + 25.0, cy + 7.5),
        if on { TEXT } else { TEXT_MUTED },
        1.0,
    );
    let mut right = r.right - 8.0;
    if let Some(n) = row.count {
        right = badge(g, dc, n, right, cy)? - 8.0;
    }
    let x = r.left + 32.0;
    let name = g.layout(
        &gfx::wide(&row.name),
        &g.formats.direct_name,
        (right - x).max(1.0),
    )?;
    let m = gfx::metrics(&name);
    let brush = if on {
        &g.brushes.text
    } else {
        &g.brushes.muted
    };
    text::draw(dc, &name, x, cy - 1.0 - m.height / 2.0, brush);
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
    /// A channel you haven't joined, open from Browse: Join channel.
    Preview,
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
    Join,
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
            BarKind::Preview => {
                format!("Preview\nJoin #{} to interact with people here", self.name)
            }
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
            BarPart::Join if self.busy.is_some() => "Joining\u{2026}".into(),
            BarPart::Join => "Join channel".into(),
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
        if self.kind == BarKind::Preview {
            // "Preview" and the channel's name stand out, as on web.
            let name = gfx::wide(&format!("#{}", self.name)).len() as u32;
            for range in [
                DWRITE_TEXT_RANGE {
                    startPosition: 0,
                    length: 7,
                },
                DWRITE_TEXT_RANGE {
                    startPosition: 13,
                    length: name,
                },
            ] {
                unsafe {
                    l.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, range)?;
                    l.SetDrawingEffect(&g.brushes.text, range)?;
                }
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
            BarKind::Preview => {
                parts.push((BarPart::Text, (0.0, 0.0, w, m.height)));
                y = m.height + 8.0;
                let bw = self.button_w(g, BarPart::Join)?;
                parts.push((BarPart::Join, (0.0, y, bw, BAR_BUTTON_H)));
                y += BAR_BUTTON_H;
            }
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
            BarPart::Accept | BarPart::Decline | BarPart::Block | BarPart::Join => {
                self.busy.is_none()
            }
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
                    let tone = if matches!(part, BarPart::Accept | BarPart::Join) {
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
                BarPart::Join => live.join_channel(),
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

    /// The section's top, under the channels and channel invitations.
    pub fn directs_top(&self) -> f32 {
        // Under the channels and their invitations.
        self.invites_top() + self.invites_h()
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
        let rows = self.direct_rows();
        let n = rows.len();
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
        if let Some(k) = (0..n).find(|&k| rows[k].menu() && inside(self.direct_options_rect(k))) {
            return Some(Spot::Options(k));
        }
        (0..n).find(|&k| inside(self.direct_rect(k))).map(Spot::Row)
    }

    /// Row `k`'s options button (client DIPs), shown while it is hovered.
    pub fn direct_options_rect(&self, k: usize) -> Rect {
        let (x, y, w, _) = self.direct_rect(k);
        (x + w - 32.0, y + 2.0, 24.0, 24.0)
    }

    /// The row whose options menu is open.
    pub fn dm_menu_row(&self) -> Option<usize> {
        let Scope::Direct(id) = &self.dm_menu.as_ref()?.scope else {
            return None;
        };
        self.direct_rows()
            .iter()
            .position(|r| r.id.as_ref() == Some(id))
    }

    /// The open menu's items and whether they can change now.
    pub fn dm_menu_items(&self) -> (bool, Vec<MenuItem>) {
        let (Some(m), Some(live)) = (&self.dm_menu, &self.live) else {
            return (false, Vec::new());
        };
        let (enabled, mut items) = live.notify_menu(&m.scope);
        // The space's opens with Browse channels and ends with leaving it,
        // as egui's (an owner can't leave a space).
        if let Scope::Space(_) = &m.scope {
            items.insert(0, MenuItem::Browse);
            if live.can_invite(&m.scope) {
                items.insert(1, MenuItem::Invite);
            }
        }
        if matches!(m.scope, Scope::Channel { .. }) && live.can_invite(&m.scope) {
            items.push(MenuItem::Invite);
        }
        // The owner's Channel settings come before Leave channel, and Space
        // settings where others have Leave space, as on web.
        let manage = live.can_manage(&m.scope);
        match &m.scope {
            Scope::Channel { .. } => {
                if manage {
                    items.push(MenuItem::ChannelSettings);
                }
                items.push(MenuItem::Leave(false));
            }
            Scope::Space(_) if live.can_leave_space() => items.push(MenuItem::Leave(true)),
            Scope::Space(_) if manage => items.push(MenuItem::SpaceSettings),
            _ => {}
        }
        (enabled, items)
    }

    /// The menu (client DIPs), beside its row, kept in the window.
    pub fn dm_menu_rect(&self) -> Rect {
        let h = self.dm_menu_items().1.len() as f32 * MENU_ITEM_H + 12.0;
        let y = self.dm_menu_anchor().unwrap_or(0.0);
        (
            RAIL_W + SIDEBAR_W - 8.0,
            y.min(self.h - 8.0 - h).max(8.0),
            MENU_W,
            h,
        )
    }

    pub fn dm_item_rect(&self, k: usize) -> Rect {
        let (x, y, w, _) = self.dm_menu_rect();
        (
            x + 6.0,
            y + 6.0 + k as f32 * MENU_ITEM_H,
            w - 12.0,
            MENU_ITEM_H,
        )
    }

    /// Opens or closes row `k`'s options menu.
    pub fn toggle_dm_menu(&mut self, k: usize) {
        let scope = self
            .direct_rows()
            .get(k)
            .filter(|r| r.menu())
            .and_then(|r| r.id.clone())
            .map(Scope::Direct);
        self.toggle_scope_menu(scope);
    }

    /// Opens `scope`'s notification menu, or closes it when it is the open
    /// one; opening loads the settings, as egui's menus do.
    pub fn toggle_scope_menu(&mut self, scope: Option<Scope>) {
        let same = scope.is_some() && self.dm_menu.as_ref().map(|m| &m.scope) == scope.as_ref();
        self.dm_menu = match scope {
            Some(scope) if !same => {
                if let Some(live) = &mut self.live {
                    live.load_notifications();
                }
                Some(DmMenu { scope, hover: None })
            }
            _ => None,
        };
        self.dm_menu_changed();
    }

    /// The open space's notification scope, in a live account.
    pub fn space_scope(&self) -> Option<Scope> {
        self.live.as_ref()?;
        if !self.c.is_live() {
            return None;
        }
        Some(Scope::Space(self.ids.spaces.get(self.c.space)?.clone()))
    }

    /// Space `i`'s bell-slash label while it is muted (the rail's badge).
    pub fn space_muted(&self, i: usize) -> Option<String> {
        let live = self.live.as_ref().filter(|_| self.c.is_live())?;
        live.muted(&Scope::Space(self.ids.spaces.get(i)?.clone()))
    }

    /// Channel `i`'s notification scope, in a live account.
    pub fn channel_scope(&self, i: usize) -> Option<Scope> {
        let Some(Scope::Space(space)) = self.space_scope() else {
            return None;
        };
        Some(Scope::Channel {
            space,
            channel: self.ids.channels.get(i)?.clone(),
        })
    }

    /// The sidebar header's space options button (client DIPs).
    pub fn space_options_rect(&self) -> Rect {
        (RAIL_W + SIDEBAR_W - 40.0, 13.0, 28.0, 28.0)
    }

    /// Channel `i`'s options button (client DIPs), shown while its row is
    /// hovered.
    pub fn channel_options_rect(i: usize) -> Rect {
        let (x, y, w, h) = App::channel_rect(i);
        (x + w - 30.0, y + (h - 24.0) / 2.0, 24.0, 24.0)
    }

    /// The top of what the open menu sits beside: its DM or channel row,
    /// or the sidebar header; None once that is no longer shown.
    fn dm_menu_anchor(&self) -> Option<f32> {
        let here = |space: &String| self.ids.spaces.get(self.c.space) == Some(space);
        match &self.dm_menu.as_ref()?.scope {
            Scope::Direct(_) => self.dm_menu_row().map(|k| self.direct_rect(k).1),
            Scope::Channel { space, channel } if here(space) => self
                .ids
                .channels
                .iter()
                .position(|c| c == channel)
                .map(|i| App::channel_rect(i).1),
            Scope::Space(space) if here(space) => Some(HEADER_H - 6.0),
            _ => None,
        }
    }

    /// Whether (x, y) is on the button that opened the open menu.
    fn on_menu_button(&self, x: f32, y: f32) -> bool {
        match self.dm_menu.as_ref().map(|m| &m.scope) {
            Some(Scope::Direct(_)) => {
                matches!(self.direct_at(x, y), Some(Spot::Options(k)) if Some(k) == self.dm_menu_row())
            }
            Some(Scope::Channel { channel, .. }) => self
                .ids
                .channels
                .iter()
                .position(|c| c == channel)
                .is_some_and(|i| contains(Self::channel_options_rect(i), x, y)),
            Some(Scope::Space(_)) => contains(self.space_options_rect(), x, y),
            _ => false,
        }
    }

    pub fn close_dm_menu(&mut self) {
        if self.dm_menu.take().is_some() {
            self.dm_menu_changed();
        }
    }

    pub fn dm_menu_changed(&mut self) {
        self.dirty.dm_menu = true;
        self.dirty.sidebar = true;
        self.events |= crate::EV_SCREEN;
        self.need = true;
    }

    /// Chooses item `k`; the menu closes, as egui's does.
    pub fn dm_menu_activate(&mut self, k: usize) {
        let (enabled, items) = self.dm_menu_items();
        let Some(item) = items.get(k).filter(|i| i.usable(enabled)) else {
            return;
        };
        let Some(scope) = self.dm_menu.as_ref().map(|m| m.scope.clone()) else {
            return;
        };
        if *item == MenuItem::Browse {
            self.dm_menu = None;
            self.dm_menu_changed();
            return self.open_browse();
        }
        if *item == MenuItem::Invite {
            self.dm_menu = None;
            self.dm_menu_changed();
            let channel = match scope {
                Scope::Channel { channel, .. } => Some(channel),
                _ => None,
            };
            return self.open_invite_people(channel);
        }
        if *item == MenuItem::SpaceSettings {
            self.dm_menu = None;
            self.dm_menu_changed();
            return self.open_manage_space();
        }
        if *item == MenuItem::ChannelSettings {
            self.dm_menu = None;
            self.dm_menu_changed();
            if let Scope::Channel { channel, .. } = &scope {
                self.open_channel_settings(channel);
            }
            return;
        }
        if let MenuItem::Leave(_) = item {
            self.dm_menu = None;
            self.dm_menu_changed();
            if let Some(leave) = self.live.as_ref().and_then(|l| l.leave_target(&scope)) {
                self.open_dialog(crate::overlay::Dialog::leave(leave));
            }
            return;
        }
        if let Some(live) = &mut self.live {
            live.menu_choose(&scope, item);
        }
        self.dm_menu = None;
        self.dm_menu_changed();
    }

    /// Input while a conversation's options menu is open.
    pub fn dm_menu_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let (enabled, items) = self.dm_menu_items();
        let hit = (0..items.len())
            .find(|&k| contains(self.dm_item_rect(k), x, y))
            .filter(|&k| items[k].usable(enabled));
        let inside = contains(self.dm_menu_rect(), x, y);
        // The open row's own button only closes it.
        let own = self.on_menu_button(x, y);
        let m = self.dm_menu.as_mut()?;
        match msg {
            WM_MOUSEMOVE => {
                if hit != m.hover {
                    m.hover = hit;
                    self.dirty.dm_menu = true;
                    self.need = true;
                }
                None
            }
            WM_LBUTTONDOWN => {
                if let Some(k) = hit {
                    self.dm_menu_activate(k);
                    return done;
                }
                if inside {
                    return done;
                }
                self.close_dm_menu();
                if own { done } else { None }
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                let choices: Vec<usize> = (0..items.len()).filter(|&k| items[k].action()).collect();
                let at = m.hover.and_then(|h| choices.iter().position(|&c| c == h));
                let n = choices.len();
                match vk {
                    VK_ESCAPE => {
                        self.close_dm_menu();
                        return done;
                    }
                    VK_DOWN if n > 0 => m.hover = Some(choices[at.map_or(0, |p| (p + 1) % n)]),
                    VK_UP if n > 0 => {
                        m.hover = Some(choices[at.map_or(n - 1, |p| (p + n - 1) % n)])
                    }
                    VK_RETURN | VK_SPACE => {
                        if let Some(k) = m.hover {
                            self.dm_menu_activate(k);
                        }
                        return done;
                    }
                    _ => return done,
                }
                self.dirty.dm_menu = true;
                self.need = true;
                done
            }
            WM_MOUSEWHEEL => {
                self.close_dm_menu();
                None
            }
            WM_SETCURSOR if hit.is_some() => {
                unsafe { SetCursor(LoadCursorW(None, IDC_HAND).ok()) };
                Some(Some(LRESULT(1)))
            }
            _ => None,
        }
    }

    /// Redraws the options menu when it or the sidebar changed.
    pub fn draw_dm_menu(&mut self) -> Result<()> {
        if !(std::mem::take(&mut self.dirty.dm_menu) || self.dirty.sidebar) {
            return Ok(());
        }
        if self.dm_menu_anchor().is_none() {
            return self.dm_menu_p.offset(-100_000.0, 0.0);
        }
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let (enabled, items) = self.dm_menu_items();
        let hover = self.dm_menu.as_ref().and_then(|m| m.hover);
        let (mx, my, mw, mh) = self.dm_menu_rect();
        let g = &self.g;
        self.dm_menu_p.ensure(g, px(mw), px(mh))?;
        self.dm_menu_p.offset((mx * s).round(), (my * s).round())?;
        self.dm_menu_p.draw(s, |dc| {
            unsafe { dc.Clear(Some(&D2D1_COLOR_F::default())) };
            let r = rect(0.5, 0.5, mw - 0.5, mh - 0.5);
            fill_round(dc, r, RADIUS, &g.brushes.surface);
            stroke_round(dc, r, RADIUS, &g.brushes.border);
            for (k, item) in items.iter().enumerate() {
                let (x, y, w, h) = (6.0, 6.0 + k as f32 * MENU_ITEM_H, mw - 12.0, MENU_ITEM_H);
                let live = item.usable(enabled);
                if live && hover == Some(k) {
                    fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
                }
                // Choices sit under their heading; the current one is ticked.
                let indent = if item.choice() { 26.0 } else { 10.0 };
                if item.chosen() {
                    crate::icons::draw(
                        dc,
                        crate::icons::Icon::Check,
                        rect(x + 8.0, y + h / 2.0 - 7.0, x + 22.0, y + h / 2.0 + 7.0),
                        TEXT,
                        if live { 1.0 } else { 0.5 },
                    );
                }
                let l = g.layout(
                    &gfx::wide(&item.label()),
                    &g.formats.dialog_note,
                    (w - indent - 6.0).max(1.0),
                )?;
                let lm = gfx::metrics(&l);
                let brush = match item {
                    MenuItem::Note(_, true) | MenuItem::Leave(true) => &g.brushes.error,
                    _ if !live => &g.brushes.muted,
                    _ => &g.brushes.text,
                };
                text::draw(dc, &l, x + indent, y + (h - lm.height) / 2.0, brush);
            }
            Ok(())
        })
    }

    pub fn direct_click(&mut self, spot: Spot) {
        match spot {
            Spot::New => self.open_start_direct(),
            Spot::Row(k) => self.open_direct_row(k),
            Spot::Options(k) => self.toggle_dm_menu(k),
            Spot::Heading => {}
        }
    }

    /// Opens row `k`'s DM (your own is created first when it is new), or
    /// opens or closes "Message requests".
    pub fn open_direct_row(&mut self, k: usize) {
        let Some(row) = self.direct_rows().into_iter().nth(k) else {
            return;
        };
        if row.kind == RowKind::Notifications {
            return self.open_inbox();
        }
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
            muted: None,
            error: None,
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
        let row = Row {
            kind: RowKind::Notifications,
            name: "Notifications".into(),
            count: Some(2),
            ..row
        };
        assert_eq!(row.label(), "Notifications \u{b7} 2");
    }
}
