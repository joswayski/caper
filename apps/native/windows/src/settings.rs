//! Settings, as in the egui client: a card over the conversation with
//! Startup (launch at login), Notifications ("Notify me about" and voice
//! starting), Privacy (who can start a DM, blocked accounts) and Delete
//! account. Account choices save as soon as they change and revert with an
//! inline error (the shared `Notifications` and the core's Account
//! operations, kept in `Live`). Also the sidebar's account footer: your
//! avatar and name, and its gear's menu (Settings…, Log out).
//!
//! The same card shows the notification inbox (egui's `activity_dialog`):
//! the filters, Unread only with the count, each notification (opening it
//! closes the inbox over its conversation; its check marks it read), and
//! Load more. The shared `caper_client::activity::Inbox` keeps its state.
//! And the Audio test (`audio`).
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::overlay::{Rect, Tone};
use crate::theme::*;
use crate::{App, EV_FOCUS, EV_SCREEN, text};
use audio::Mic;
use caper_client::activity::Filter;
use caper_client::model::{DELETE_ACCOUNT_URL, DM_PRIVACY, NotificationLevel};
use caper_client::notifications::{Change, Scope, account_level_label};
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

pub mod audio;

const CARD_W: f32 = 560.0;
const PAD: f32 = 24.0;
/// The title row above the scrolling body.
const HEAD_H: f32 = 64.0;
const ROW_H: f32 = 26.0;
const BUTTON_H: f32 = 30.0;
/// An inbox filter's chip.
const CHIP_H: f32 = 28.0;
/// An inbox row's title width leaves this much for its time and check.
const ROW_SIDE: f32 = 104.0;
/// Refreshes the open inbox's times and polls it, as egui's repaint.
pub const TIMER_INBOX: usize = 11;
/// The gap around a section's separator.
const SECTION_GAP: f32 = 18.0;
pub const LEVELS: [NotificationLevel; 3] = [
    NotificationLevel::All,
    NotificationLevel::Mentions,
    NotificationLevel::Nothing,
];
/// The sidebar's account footer and its gear's menu.
pub const FOOT_H: f32 = 52.0;
const MENU_W: f32 = 232.0;
const MENU_ITEM_H: f32 = 32.0;
pub const MENU_ITEMS: [&str; 3] = ["Settings\u{2026}", "Audio test", "Log out"];

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SPart {
    Close,
    /// A line of text, by its place in the body.
    Text(usize),
    Launch,
    DismissStartup,
    Level(usize),
    VoiceStarted,
    RetryNotifications,
    Privacy(usize),
    RetryPrivacy,
    Unblock(usize),
    RetryBlocks,
    Delete,
    /// The inbox: a filter (`Filter::ALL`), Unread only, a shown row
    /// (opens it), a row's mark read check, Load more and Retry.
    Filter(usize),
    UnreadOnly,
    Open(usize),
    MarkRead(usize),
    LoadMore,
    RetryInbox,
    /// The Audio test: a microphone or speaker choice (the system
    /// default first), Refresh devices, the three sliders, Test
    /// speakers, Test microphone, the input meter and a sample's Play.
    Input(usize),
    Output(usize),
    RefreshDevices,
    MicVolume,
    SpeakerVolume,
    Enhancement,
    SpeakerTest,
    MicTest,
    Meter,
    /// The natural (false) or enhanced sample.
    Sample(bool),
}

impl SPart {
    /// A stable number per part, for UI Automation's runtime ids.
    pub fn index(self) -> usize {
        match self {
            Self::Close => 0,
            Self::Launch => 1,
            Self::DismissStartup => 2,
            Self::VoiceStarted => 3,
            Self::RetryNotifications => 4,
            Self::RetryPrivacy => 5,
            Self::RetryBlocks => 6,
            Self::Delete => 7,
            Self::Level(k) => 8 + k,
            Self::Privacy(k) => 12 + k,
            Self::Filter(k) => 20 + k,
            Self::UnreadOnly => 30,
            Self::LoadMore => 31,
            Self::RetryInbox => 32,
            Self::RefreshDevices => 33,
            Self::MicVolume => 34,
            Self::SpeakerVolume => 35,
            Self::Enhancement => 36,
            Self::SpeakerTest => 37,
            Self::MicTest => 38,
            Self::Meter => 39,
            Self::Sample(e) => 40 + usize::from(e),
            Self::Input(k) => 2000 + k,
            Self::Output(k) => 3000 + k,
            Self::Text(k) => 100 + k,
            Self::Unblock(k) => 1000 + k,
            Self::Open(k) => 1_000_000 + 2 * k,
            Self::MarkRead(k) => 1_000_001 + 2 * k,
        }
    }

    pub fn is_radio(self) -> bool {
        matches!(
            self,
            Self::Level(_) | Self::Privacy(_) | Self::Filter(_) | Self::Input(_) | Self::Output(_)
        )
    }

    pub fn is_slider(self) -> bool {
        matches!(
            self,
            Self::MicVolume | Self::SpeakerVolume | Self::Enhancement
        )
    }

    pub fn is_toggle(self) -> bool {
        matches!(self, Self::Launch | Self::VoiceStarted | Self::UnreadOnly)
    }

    fn is_button(self) -> bool {
        matches!(
            self,
            Self::DismissStartup
                | Self::RetryNotifications
                | Self::RetryPrivacy
                | Self::RetryBlocks
                | Self::Unblock(_)
                | Self::LoadMore
                | Self::RetryInbox
                | Self::RefreshDevices
                | Self::SpeakerTest
                | Self::MicTest
                | Self::Sample(_)
        )
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Style {
    Section,
    Label,
    Detail,
    Error,
    /// A blocked account: its avatar (or initials on this colour) before
    /// the name.
    Person(u32),
    /// An inbox row's title: who it is from.
    Title,
}

struct Item {
    part: SPart,
    /// Body-local, before scrolling.
    rect: Rect,
    text: String,
    style: Style,
    /// A person's initials and avatar.
    face: Option<(String, crate::avatars::Avatar)>,
}

/// What Settings shows, gathered from the app and the account each layout.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Snapshot {
    /// Launch at login as the OS has it; None when it can't change here.
    pub launch: Option<bool>,
    pub startup_error: Option<String>,
    /// Signed in: the account sections show.
    pub account: bool,
    pub level: Option<NotificationLevel>,
    pub voice_started: bool,
    pub notify_saving: bool,
    pub notify_load_error: Option<String>,
    pub notify_error: Option<String>,
    pub privacy: Option<String>,
    pub privacy_error: Option<String>,
    /// Each blocked account's name, @username, colour, initials and avatar.
    pub blocks: Option<Vec<(String, String, u32, String, crate::avatars::Avatar)>>,
    pub blocks_error: Option<String>,
    pub block_busy: bool,
    pub block_error: Option<String>,
    /// The inbox page's state.
    pub inbox: Option<InboxSnap>,
    /// The Audio test's state.
    pub audio: Option<audio::AudioSnap>,
}

/// What the inbox shows (`Live::inbox_snapshot`).
#[derive(Clone, Default, PartialEq, Debug)]
pub struct InboxSnap {
    /// The chosen filter's place in `Filter::ALL`.
    pub filter: usize,
    pub unread_only: bool,
    pub unread_count: u64,
    pub rows: Vec<InboxRow>,
    pub loading: bool,
    /// The rows are the previous filter's, dimmed and inert until its page
    /// answers.
    pub stale: bool,
    pub error: Option<String>,
    /// Older notifications exist (Load more).
    pub more: bool,
    /// Retry can run: nothing loads and no read is in flight.
    pub retry: bool,
}

/// One notification as the inbox shows it.
#[derive(Clone, PartialEq, Debug)]
pub struct InboxRow {
    pub actor: String,
    pub unread: bool,
    /// Its open is in flight: only it is disabled.
    pub opening: bool,
    pub time: String,
    /// What happened, for reactions, requests and invitations.
    pub description: Option<String>,
    pub body: String,
    /// Its space and channel.
    pub place: String,
}

/// Which card is shown.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Settings,
    Inbox,
    Audio,
}

pub struct Settings {
    pub page: Page,
    items: Vec<Item>,
    separators: Vec<f32>,
    /// Boxes behind parts of the body (body-local) and their borders.
    frames: Vec<(Rect, u32)>,
    pub snap: Snapshot,
    pub scroll: f32,
    content_h: f32,
    pub hover: Option<SPart>,
    pub pressed: Option<SPart>,
    /// Client DIPs: the card and its scrolling body.
    card: Rect,
    body: Rect,
    /// Launch at login as last read or set, and why reading or changing it
    /// failed.
    pub launch: bool,
    pub startup_error: Option<String>,
}

impl Settings {
    pub fn new(launch: bool, startup_error: Option<String>) -> Self {
        Self {
            page: Page::Settings,
            items: Vec::new(),
            separators: Vec::new(),
            frames: Vec::new(),
            snap: Snapshot::default(),
            scroll: 0.0,
            content_h: 0.0,
            hover: None,
            pressed: None,
            card: (0.0, 0.0, 0.0, 0.0),
            body: (0.0, 0.0, 0.0, 0.0),
            launch,
            startup_error,
        }
    }

    /// The notification inbox.
    pub fn inbox() -> Self {
        Self {
            page: Page::Inbox,
            ..Self::new(false, None)
        }
    }

    pub fn enabled(&self, part: SPart) -> bool {
        let s = &self.snap;
        let inbox = s.inbox.as_ref();
        let audio = s.audio.as_ref();
        match part {
            SPart::Launch => s.launch.is_some(),
            SPart::Level(_) | SPart::VoiceStarted => s.level.is_some() && !s.notify_saving,
            SPart::Privacy(_) => s.privacy.is_some(),
            SPart::Unblock(_) => !s.block_busy,
            SPart::Open(k) => {
                inbox.is_some_and(|i| !i.stale && i.rows.get(k).is_some_and(|r| !r.opening))
            }
            SPart::MarkRead(_) => inbox.is_some_and(|i| !i.stale),
            SPart::LoadMore => inbox.is_some_and(|i| !i.loading),
            SPart::RetryInbox => inbox.is_some_and(|i| i.retry),
            SPart::Input(_) | SPart::Output(_) => audio.is_some_and(|a| a.mic == Mic::Idle),
            SPart::RefreshDevices => audio.is_some_and(|a| !a.refreshing),
            SPart::Enhancement => audio.is_some_and(|a| a.mic != Mic::Recording),
            SPart::MicTest => {
                audio.is_some_and(|a| !matches!(a.mic, Mic::Preparing | Mic::Processing))
            }
            SPart::Sample(e) => {
                audio.is_some_and(|a| a.mic == Mic::Ready || a.mic == Mic::Playing(e))
            }
            SPart::Meter => false,
            _ => true,
        }
    }

    /// Whether a radio is chosen or a switch or box is on.
    pub fn checked(&self, part: SPart) -> bool {
        let s = &self.snap;
        match part {
            SPart::Launch => s.launch.unwrap_or(false),
            SPart::VoiceStarted => s.voice_started,
            SPart::Level(k) => s.level == Some(LEVELS[k]),
            SPart::Privacy(k) => s.privacy.as_deref() == Some(DM_PRIVACY[k].0),
            SPart::Filter(k) => s.inbox.as_ref().is_some_and(|i| i.filter == k),
            SPart::UnreadOnly => s.inbox.as_ref().is_some_and(|i| i.unread_only),
            SPart::Input(k) => s.audio.as_ref().is_some_and(|a| a.input == Some(k)),
            SPart::Output(k) => s.audio.as_ref().is_some_and(|a| a.output == Some(k)),
            _ => false,
        }
    }

    /// The parts shown, in reading order, Close first.
    pub fn parts(&self) -> Vec<SPart> {
        std::iter::once(SPart::Close)
            .chain(self.items.iter().map(|i| i.part))
            .collect()
    }

    /// A part's accessible name: its text or label.
    pub fn name(&self, part: SPart) -> String {
        match part {
            SPart::Close if self.page == Page::Inbox => "Close notifications".into(),
            SPart::Close if self.page == Page::Audio => "Close audio test".into(),
            SPart::Close => "Close settings".into(),
            _ => self
                .items
                .iter()
                .find(|i| i.part == part)
                .map(|i| i.text.clone())
                .unwrap_or_default(),
        }
    }

    /// A part's rectangle in client DIPs, as scrolled.
    pub fn part_rect(&self, part: SPart) -> Option<Rect> {
        if part == SPart::Close {
            let (cx, cy, cw, _) = self.card;
            return Some((cx + cw - PAD - 28.0 + 6.0, cy + 18.0, 28.0, 28.0));
        }
        let item = self.items.iter().find(|i| i.part == part)?;
        Some(self.client(item.rect))
    }

    fn client(&self, (x, y, w, h): Rect) -> Rect {
        let (bx, by, _, _) = self.body;
        (bx + PAD + x, by + y - self.scroll, w, h)
    }

    /// The enabled control under (x, y).
    pub fn hit(&self, x: f32, y: f32) -> Option<SPart> {
        if self
            .part_rect(SPart::Close)
            .is_some_and(|r| contains(r, x, y))
        {
            return Some(SPart::Close);
        }
        if !contains(self.body, x, y) {
            return None;
        }
        // Last first: an inbox row's check lies on the row.
        self.items
            .iter()
            .rev()
            .filter(|i| !matches!(i.part, SPart::Text(_) | SPart::Meter) && self.enabled(i.part))
            .find(|i| contains(self.client(i.rect), x, y))
            .map(|i| i.part)
    }

    pub fn on_card(&self, x: f32, y: f32) -> bool {
        contains(self.card, x, y)
    }

    fn max_scroll(&self) -> f32 {
        (self.content_h - self.body.3).max(0.0)
    }

    pub fn scroll_by(&mut self, dy: f32) {
        self.scroll = (self.scroll + dy).clamp(0.0, self.max_scroll());
    }

    /// Lays the card out for a `w` × `h` window.
    pub fn relayout(&mut self, g: &Gfx, snap: Snapshot, w: f32, h: f32) -> Result<()> {
        let card_w = CARD_W.min(w - 48.0).max(280.0);
        let inner = card_w - 2.0 * PAD;
        let mut b = Builder {
            g,
            inner,
            y: 0.0,
            items: Vec::new(),
            separators: Vec::new(),
            frames: Vec::new(),
            texts: 0,
        };
        if let (Page::Inbox, Some(inbox)) = (self.page, &snap.inbox) {
            b.inbox(inbox)?;
        } else if let (Page::Audio, Some(a)) = (self.page, &snap.audio) {
            b.audio(a)?;
        } else if self.page == Page::Settings {
            b.settings(&snap)?;
        }
        let content_h = b.y;
        let card_h = (HEAD_H + content_h + PAD).min(h - 48.0).max(HEAD_H + 40.0);
        let (cx, cy) = (
            ((w - card_w) / 2.0).round(),
            ((h - card_h) / 2.0).max(24.0).round(),
        );
        self.card = (cx, cy, card_w, card_h);
        self.body = (cx, cy + HEAD_H, card_w, card_h - HEAD_H - PAD);
        self.items = b.items;
        self.separators = b.separators;
        self.frames = b.frames;
        self.content_h = content_h;
        self.snap = snap;
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
        Ok(())
    }
}

impl Builder<'_> {
    /// Settings' sections.
    fn settings(&mut self, snap: &Snapshot) -> Result<()> {
        let b = self;
        let inner = b.inner;
        b.text("Startup", Style::Section)?;
        b.y += 6.0;
        // The switch row: its title and note on the left, the switch right.
        let title = b.measure("Launch at login", Style::Label, inner - 56.0)?;
        let note = b.measure(
            "Open Caper when you sign in to your computer.",
            Style::Detail,
            inner - 56.0,
        )?;
        b.items.push(Item {
            part: SPart::Launch,
            rect: (0.0, b.y, inner, (title + 2.0 + note).max(ROW_H)),
            text: "Launch at login".into(),
            style: Style::Label,
            face: None,
        });
        b.y += (title + 2.0 + note).max(ROW_H);
        if let Some(e) = &snap.startup_error {
            b.y += 6.0;
            b.text(e, Style::Error)?;
            b.y += 4.0;
            b.button(SPart::DismissStartup, "Dismiss startup error")?;
        }
        if snap.account {
            b.separator();
            b.text("Notifications", Style::Section)?;
            b.y += 6.0;
            b.text("Notify me about", Style::Label)?;
            for (k, level) in LEVELS.iter().enumerate() {
                b.row(SPart::Level(k), account_level_label(*level));
            }
            b.row(SPart::VoiceStarted, "When voice starts in a space");
            if snap.level.is_none() {
                match &snap.notify_load_error {
                    None => b.text("Loading\u{2026}", Style::Detail)?,
                    Some(e) => {
                        b.text(e, Style::Error)?;
                        b.button(SPart::RetryNotifications, "Try again")?;
                    }
                }
            }
            if let Some(e) = &snap.notify_error {
                b.text(e, Style::Error)?;
            }
            b.separator();
            b.text("Privacy", Style::Section)?;
            b.y += 6.0;
            b.text("Who can start a DM with you", Style::Label)?;
            for (k, (_, label, detail)) in DM_PRIVACY.iter().enumerate() {
                b.row(SPart::Privacy(k), label);
                if let Some(detail) = detail {
                    b.text(detail, Style::Detail)?;
                }
            }
            if snap.privacy.is_none() && snap.privacy_error.is_none() {
                b.text("Loading\u{2026}", Style::Detail)?;
            }
            if let Some(e) = &snap.privacy_error {
                b.text(e, Style::Error)?;
                if snap.privacy.is_none() {
                    b.button(SPart::RetryPrivacy, "Try again")?;
                }
            }
            b.y += 14.0;
            b.text("Blocked accounts", Style::Label)?;
            b.y += 6.0;
            match &snap.blocks {
                None if snap.blocks_error.is_none() => b.text("Loading\u{2026}", Style::Detail)?,
                // Only a successful load can say the list is empty.
                None => {}
                Some(list) if list.is_empty() => {
                    if snap.blocks_error.is_none() {
                        b.text("You haven't blocked anyone.", Style::Detail)?;
                    }
                }
                Some(list) => {
                    for (k, (name, username, color, initials, avatar)) in list.iter().enumerate() {
                        b.person(k, name, username, *color, initials, avatar)?;
                    }
                }
            }
            if let Some(e) = &snap.blocks_error {
                b.text(e, Style::Error)?;
                b.button(SPart::RetryBlocks, "Try again")?;
            }
            if let Some(e) = &snap.block_error {
                b.text(e, Style::Error)?;
            }
            b.y += 14.0;
            let dh = b.measure("Delete account", Style::Label, inner)?;
            let dw = b.width("Delete account", Style::Label)?;
            b.items.push(Item {
                part: SPart::Delete,
                rect: (0.0, b.y, dw, dh),
                text: "Delete account".into(),
                style: Style::Error,
                face: None,
            });
            b.y += dh;
        }
        Ok(())
    }

    /// The inbox, as egui's `activity_dialog` stacks it.
    fn inbox(&mut self, s: &InboxSnap) -> Result<()> {
        // The filters wrap like egui's selectable row.
        let mut x = 0.0;
        for (k, filter) in Filter::ALL.iter().enumerate() {
            let w = self.width(filter.label(), Style::Label)? + 20.0;
            if x > 0.0 && x + w > self.inner {
                x = 0.0;
                self.y += CHIP_H + 6.0;
            }
            self.push(SPart::Filter(k), (x, self.y, w, CHIP_H), filter.label());
            x += w + 6.0;
        }
        self.y += CHIP_H + 10.0;
        let unread = format!("Unread only ({})", s.unread_count);
        self.push(
            SPart::UnreadOnly,
            (0.0, self.y, self.inner.min(220.0), ROW_H),
            &unread,
        );
        self.y += ROW_H + 10.0;
        if let Some(e) = &s.error {
            self.text(e, Style::Error)?;
            self.button(SPart::RetryInbox, "Retry")?;
            self.y += 6.0;
        }
        if s.rows.is_empty() {
            if s.loading {
                self.text("Loading notifications\u{2026}", Style::Detail)?;
            } else if s.error.is_none() {
                self.text("No notifications match these filters.", Style::Detail)?;
            }
        }
        for (k, row) in s.rows.iter().enumerate() {
            let top = self.y;
            // The row opens it; its title line ends in the time and, while
            // unread, the check.
            let open = self.items.len();
            let title = format!("{}{}", if row.unread { "\u{25cf} " } else { "" }, row.actor);
            let th = self.measure(&title, Style::Title, self.inner - ROW_SIDE)?;
            self.items.push(Item {
                part: SPart::Open(k),
                rect: (0.0, top, self.inner, 0.0),
                text: title,
                style: Style::Title,
                face: None,
            });
            let line = (th + 8.0).max(28.0);
            let mark = if row.unread { 32.0 } else { 0.0 };
            let tw = self.width(&row.time, Style::Detail)?;
            let tm = self.measure(&row.time, Style::Detail, 1e6)?;
            self.items.push(Item {
                part: SPart::Text(self.texts),
                rect: (self.inner - mark - tw, top + (line - tm) / 2.0, tw, tm),
                text: row.time.clone(),
                style: Style::Detail,
                face: None,
            });
            self.texts += 1;
            if row.unread {
                let text = format!("Mark notification from {} read", row.actor);
                self.push(
                    SPart::MarkRead(k),
                    (self.inner - 28.0, top + (line - 28.0) / 2.0, 28.0, 28.0),
                    &text,
                );
            }
            self.y = top + line;
            if let Some(d) = &row.description {
                self.text(d, Style::Detail)?;
            }
            if !row.body.is_empty() {
                self.text(&row.body, Style::Label)?;
            }
            if !row.place.is_empty() {
                self.text(&row.place, Style::Detail)?;
            }
            self.items[open].rect.3 = self.y - top + 4.0;
            self.y += 10.0;
            self.separators.push(self.y);
            self.y += 10.0;
        }
        if s.more {
            self.button(SPart::LoadMore, "Load more")?;
        }
        Ok(())
    }

    fn push(&mut self, part: SPart, rect: Rect, text: &str) {
        self.items.push(Item {
            part,
            rect,
            text: text.into(),
            style: Style::Label,
            face: None,
        });
    }
}

impl Settings {
    /// Draws the scrim and the card over a `w` × `h` window.
    pub fn draw(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        unsafe {
            dc.Clear(Some(&gfx::color(BLACKOUT, 0.6)));
            let (cx, cy, cw, ch) = self.card;
            let card = rect(cx, cy, cx + cw, cy + ch);
            fill_round(dc, card, RADIUS, &g.brushes.surface);
            stroke_round(dc, card, RADIUS, &g.brushes.border);
            let heading = match self.page {
                Page::Settings => "Settings",
                Page::Inbox => "Notifications",
                Page::Audio => "Audio test",
            };
            let title = g.layout(&gfx::wide(heading), &g.formats.dialog_title, 1e6)?;
            text::draw(dc, &title, cx + PAD, cy + 22.0, &g.brushes.text);
            if let Some((x, y, w, h)) = self.part_rect(SPart::Close) {
                let on = self.hover == Some(SPart::Close);
                if on {
                    fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
                }
                icons::draw(
                    dc,
                    Icon::Close,
                    rect(x + 5.0, y + 5.0, x + w - 5.0, y + h - 5.0),
                    if on { TEXT } else { TEXT_MUTED },
                    1.0,
                );
            }
            let (bx, by, bw, bh) = self.body;
            dc.PushAxisAlignedClip(&rect(bx, by, bx + bw, by + bh), D2D1_ANTIALIAS_MODE_ALIASED);
            for &s in &self.separators {
                let y = by + s - self.scroll;
                dc.FillRectangle(
                    &rect(bx + PAD, y, bx + bw - PAD, y + 1.0),
                    &g.brushes.border,
                );
            }
            self.draw_frames(g, dc);
            for item in &self.items {
                let r = self.client(item.rect);
                if r.1 + r.3 < by || r.1 > by + bh {
                    continue;
                }
                self.draw_item(g, dc, item, r)?;
            }
            // A filter's page is loading: the previous rows stay, dimmed.
            let stale = self.snap.inbox.as_ref().is_some_and(|i| i.stale);
            if let Some(first) = self.items.iter().find(|i| i.part == SPart::Open(0))
                && stale
            {
                let top = self.client(first.rect).1.max(by);
                let veil = dc.CreateSolidColorBrush(&gfx::color(0x151719, 0.5), None)?;
                dc.FillRectangle(&rect(bx, top, bx + bw, by + bh), &veil);
            }
            dc.PopAxisAlignedClip();
        }
        Ok(())
    }

    fn draw_item(&self, g: &Gfx, dc: &ID2D1DeviceContext, item: &Item, r: Rect) -> Result<()> {
        let (x, y, w, h) = r;
        let enabled = self.enabled(item.part);
        let hover = self.hover == Some(item.part);
        let on = self.checked(item.part);
        let label_brush = if enabled {
            &g.brushes.text
        } else {
            &g.brushes.muted
        };
        match item.part {
            SPart::Text(_) => {
                let (format, brush) = style(g, item.style);
                let mut tx = x;
                if let (Style::Person(color), Some((initials, avatar))) = (item.style, &item.face) {
                    crate::avatars::circle(
                        g,
                        dc,
                        avatar,
                        (x + 12.0, y + h / 2.0, 12.0),
                        color,
                        initials,
                    )?;
                    tx += 32.0;
                }
                let l = g.layout(&gfx::wide(&item.text), format, (w - (tx - x)).max(1.0))?;
                crate::emoji::apply(&l, &item.text, 13.0, 1.0)?;
                let m = gfx::metrics(&l);
                let ty = if matches!(item.style, Style::Person(_)) {
                    y + (h - m.height) / 2.0
                } else {
                    y
                };
                text::draw(dc, &l, tx, ty, brush);
            }
            SPart::Launch => {
                let tw = w - 56.0;
                let t = g.layout(&gfx::wide("Launch at login"), &g.formats.dialog_note, tw)?;
                text::draw(dc, &t, x, y, label_brush);
                let th = gfx::metrics(&t).height;
                let n = g.layout(
                    &gfx::wide("Open Caper when you sign in to your computer."),
                    &g.formats.direct_note,
                    tw,
                )?;
                text::draw(dc, &n, x, y + th + 2.0, &g.brushes.muted);
                switch(
                    g,
                    dc,
                    x + w - 40.0,
                    y + (h - 22.0) / 2.0,
                    enabled,
                    on,
                    hover,
                );
            }
            SPart::UnreadOnly => {
                let l = g.layout(&gfx::wide(&item.text), &g.formats.dialog_note, w - 52.0)?;
                let m = gfx::metrics(&l);
                text::draw(dc, &l, x, y + (h - m.height) / 2.0, label_brush);
                switch(
                    g,
                    dc,
                    x + w - 40.0,
                    y + (h - 22.0) / 2.0,
                    enabled,
                    on,
                    hover,
                );
            }
            SPart::Filter(_) => {
                let r = rect(x, y, x + w, y + h);
                if on {
                    fill_round(dc, r, 6.0, &g.brushes.selected);
                } else if hover {
                    fill_round(dc, r, 6.0, &g.brushes.hover);
                }
                let l = g.layout(&gfx::wide(&item.text), &g.formats.dialog_note, 1e6)?;
                let m = gfx::metrics(&l);
                let brush = if on || hover {
                    &g.brushes.text
                } else {
                    &g.brushes.muted
                };
                text::draw(
                    dc,
                    &l,
                    x + (w - m.widthIncludingTrailingWhitespace) / 2.0,
                    y + (h - m.height) / 2.0,
                    brush,
                );
            }
            SPart::Open(_) => {
                if hover {
                    fill_round(
                        dc,
                        rect(x - 8.0, y - 4.0, x + w + 8.0, y + h),
                        6.0,
                        &g.brushes.hover,
                    );
                }
                let l = g.layout(
                    &gfx::wide(&item.text),
                    &g.formats.direct_unread,
                    (w - ROW_SIDE).max(1.0),
                )?;
                crate::emoji::apply(&l, &item.text, 13.0, 1.0)?;
                let m = gfx::metrics(&l);
                let line = (m.height + 8.0).max(28.0);
                text::draw(dc, &l, x, y + (line - m.height) / 2.0, label_brush);
            }
            SPart::MarkRead(_) => {
                if hover {
                    fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
                }
                icons::draw(
                    dc,
                    Icon::Check,
                    rect(x + 6.0, y + 6.0, x + w - 6.0, y + h - 6.0),
                    if hover { TEXT } else { TEXT_MUTED },
                    1.0,
                );
            }
            p if p.is_slider() => self.draw_slider(g, dc, item, r)?,
            SPart::Meter => self.draw_meter(g, dc, r)?,
            SPart::Level(_)
            | SPart::Privacy(_)
            | SPart::VoiceStarted
            | SPart::Input(_)
            | SPart::Output(_) => {
                let (mx, my) = (x + 8.0, y + h / 2.0);
                let ring = if !enabled {
                    BORDER
                } else if on || hover {
                    TERRACOTTA_BRIGHT
                } else {
                    TEXT_MUTED
                };
                if item.part.is_radio() {
                    let e = D2D1_ELLIPSE {
                        point: windows_numerics::Vector2 { X: mx, Y: my },
                        radiusX: 7.5,
                        radiusY: 7.5,
                    };
                    unsafe { dc.DrawEllipse(&e, &g.solid(ring), 1.5, None) };
                    if on {
                        let dot = D2D1_ELLIPSE {
                            radiusX: 4.0,
                            radiusY: 4.0,
                            ..e
                        };
                        unsafe { dc.FillEllipse(&dot, &g.solid(TERRACOTTA_BRIGHT)) };
                    }
                } else {
                    let b = rect(mx - 8.0, my - 8.0, mx + 8.0, my + 8.0);
                    if on {
                        fill_round(dc, b, 4.0, &g.solid(TERRACOTTA));
                        icons::draw(
                            dc,
                            Icon::Check,
                            rect(b.left + 2.0, b.top + 2.0, b.right - 2.0, b.bottom - 2.0),
                            TEXT,
                            1.0,
                        );
                    } else {
                        stroke_round(dc, b, 4.0, &g.solid(ring));
                    }
                }
                let l = g.layout(&gfx::wide(&item.text), &g.formats.dialog_note, w - 26.0)?;
                let m = gfx::metrics(&l);
                text::draw(dc, &l, x + 26.0, my - m.height / 2.0, label_brush);
            }
            SPart::Delete => {
                let l = g.layout(&gfx::wide(&item.text), &g.formats.dialog_note, 1e6)?;
                if hover {
                    unsafe {
                        l.SetUnderline(
                            true,
                            DWRITE_TEXT_RANGE {
                                startPosition: 0,
                                length: item.text.len() as u32,
                            },
                        )?;
                    }
                }
                text::draw(dc, &l, x, y, &g.brushes.error);
            }
            part if part.is_button() => {
                let state = if !enabled {
                    0
                } else if self.pressed == Some(part) && hover {
                    3
                } else if hover {
                    2
                } else {
                    1
                };
                let (label, tone) = match part {
                    SPart::Sample(e) => (self.sample_label(e), Tone::Secondary),
                    SPart::MicTest => (item.text.as_str(), Tone::Primary),
                    _ => (item.text.as_str(), Tone::Secondary),
                };
                crate::overlay::draw_button(g, dc, label, r, tone, state)?;
            }
            _ => {}
        }
        Ok(())
    }
}

/// A switch at (x, y): terracotta while on, its knob at the end.
pub(crate) fn switch(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    x: f32,
    y: f32,
    enabled: bool,
    on: bool,
    hover: bool,
) {
    let track = rect(x, y, x + 40.0, y + 22.0);
    let fill = match (enabled, on) {
        (false, _) => SURFACE_RAISED,
        (true, true) => TERRACOTTA,
        (true, false) => BORDER,
    };
    fill_round(dc, track, 11.0, &g.solid(fill));
    if hover {
        stroke_round(dc, track, 11.0, &g.solid(TERRACOTTA_BRIGHT));
    }
    let e = D2D1_ELLIPSE {
        point: windows_numerics::Vector2 {
            X: if on { x + 29.0 } else { x + 11.0 },
            Y: y + 11.0,
        },
        radiusX: 8.0,
        radiusY: 8.0,
    };
    let knob = if enabled { TEXT } else { TEXT_MUTED };
    unsafe { dc.FillEllipse(&e, &g.solid(knob)) };
}

fn style(g: &Gfx, s: Style) -> (&IDWriteTextFormat, &ID2D1SolidColorBrush) {
    match s {
        Style::Title => (&g.formats.direct_unread, &g.brushes.text),
        Style::Section => (&g.formats.section, &g.brushes.muted),
        Style::Label | Style::Person(_) => (&g.formats.dialog_note, &g.brushes.text),
        Style::Detail => (&g.formats.direct_note, &g.brushes.muted),
        Style::Error => (&g.formats.error, &g.brushes.error),
    }
}

/// Stacks the body's items from the top.
struct Builder<'a> {
    g: &'a Gfx,
    inner: f32,
    y: f32,
    items: Vec<Item>,
    separators: Vec<f32>,
    frames: Vec<(Rect, u32)>,
    texts: usize,
}

impl Builder<'_> {
    fn measure(&self, text: &str, s: Style, w: f32) -> Result<f32> {
        let l = self.g.layout(&gfx::wide(text), style(self.g, s).0, w)?;
        Ok(gfx::metrics(&l).height)
    }

    fn width(&self, text: &str, s: Style) -> Result<f32> {
        let l = self.g.layout(&gfx::wide(text), style(self.g, s).0, 1e6)?;
        Ok(gfx::metrics(&l).widthIncludingTrailingWhitespace)
    }

    fn text(&mut self, text: &str, s: Style) -> Result<()> {
        // Notes under a choice line up with its label.
        let indent = if s == Style::Detail && self.after_choice() {
            26.0
        } else {
            0.0
        };
        let h = self.measure(text, s, self.inner - indent)?;
        self.items.push(Item {
            part: SPart::Text(self.texts),
            rect: (indent, self.y, self.inner - indent, h),
            text: text.into(),
            style: s,
            face: None,
        });
        self.texts += 1;
        self.y += h + 2.0;
        Ok(())
    }

    fn after_choice(&self) -> bool {
        self.items.last().is_some_and(|i| i.part.is_radio())
    }

    fn row(&mut self, part: SPart, label: &str) {
        self.items.push(Item {
            part,
            rect: (0.0, self.y, self.inner, ROW_H),
            text: label.into(),
            style: Style::Label,
            face: None,
        });
        self.y += ROW_H;
    }

    fn button(&mut self, part: SPart, label: &str) -> Result<()> {
        let w = self.width(label, Style::Label)? + 24.0;
        self.items.push(Item {
            part,
            rect: (0.0, self.y, w, BUTTON_H),
            text: label.into(),
            style: Style::Label,
            face: None,
        });
        self.y += BUTTON_H + 4.0;
        Ok(())
    }

    fn person(
        &mut self,
        k: usize,
        name: &str,
        username: &str,
        color: u32,
        initials: &str,
        avatar: &crate::avatars::Avatar,
    ) -> Result<()> {
        let unblock = self.width("Unblock", Style::Label)? + 24.0;
        let label = if username.is_empty() {
            name.to_owned()
        } else {
            format!("{name} @{username}")
        };
        self.items.push(Item {
            part: SPart::Text(self.texts),
            rect: (0.0, self.y, self.inner - unblock - 8.0, 32.0),
            text: label,
            style: Style::Person(color),
            face: Some((initials.into(), avatar.clone())),
        });
        self.texts += 1;
        self.items.push(Item {
            part: SPart::Unblock(k),
            rect: (self.inner - unblock, self.y + 1.0, unblock, BUTTON_H),
            text: "Unblock".into(),
            style: Style::Label,
            face: None,
        });
        self.y += 36.0;
        Ok(())
    }

    fn separator(&mut self) {
        self.y += SECTION_GAP;
        self.separators.push(self.y);
        self.y += SECTION_GAP;
    }
}

/// The gear's menu over the account footer.
pub struct AccountMenu {
    pub hover: Option<usize>,
}

/// Whether Launch at login can change here: only against the production
/// server, as egui keeps fixture runs off the OS registration (it is shared
/// with the installed Caper).
fn launch_allowed() -> bool {
    crate::live::production()
}

impl App {
    /// Opens Settings (egui's `open_settings`): reads launch at login and
    /// loads the account's choices.
    pub fn open_settings(&mut self) {
        self.close_settings();
        self.account_menu = None;
        self.close_dm_menu();
        self.close_menu();
        self.close_dialog();
        self.close_gifs(false);
        self.set_composer_focus(false);
        let (launch, error) = if launch_allowed() {
            match caper_client::startup::enabled() {
                Ok(on) => (on, None),
                Err(e) => (false, Some(format!("Could not read startup settings: {e}"))),
            }
        } else {
            (false, None)
        };
        self.settings = Some(Settings::new(launch, error));
        if let Some(live) = &mut self.live {
            live.load_settings();
        }
        self.settings_changed();
        self.events |= EV_SCREEN | EV_FOCUS;
    }

    /// Opens the notification inbox (egui's Notifications button): its
    /// first page loads, and it refreshes every second while open.
    pub fn open_inbox(&mut self) {
        self.close_settings();
        self.account_menu = None;
        self.close_dm_menu();
        self.close_menu();
        self.close_dialog();
        self.close_gifs(false);
        self.set_composer_focus(false);
        self.settings = Some(Settings::inbox());
        if let Some(live) = &mut self.live {
            live.open_activity();
        }
        unsafe { SetTimer(Some(self.hwnd), TIMER_INBOX, 1000, None) };
        self.dirty.sidebar = true;
        self.settings_changed();
        self.events |= EV_SCREEN | EV_FOCUS;
    }

    /// The open inbox's second: it polls when due and redraws when what it
    /// shows changed (a relative time, say).
    pub fn inbox_tick(&mut self) {
        if let Some(live) = &mut self.live {
            live.poll_activity();
        }
        let snap = self.settings_snapshot();
        if self
            .settings
            .as_ref()
            .is_some_and(|s| s.page == Page::Inbox && s.snap != snap)
        {
            self.settings_changed();
        }
    }

    /// The inbox's rows or count changed, or it closed to open one.
    pub fn inbox_changed(&mut self) {
        let shown = self.live.as_ref().is_some_and(|l| l.activity.shown);
        if self
            .settings
            .as_ref()
            .is_some_and(|s| s.page == Page::Inbox)
        {
            if shown {
                self.settings_changed();
            } else {
                self.close_settings();
            }
        }
        self.dirty.sidebar = true;
        self.need = true;
    }

    pub fn close_settings(&mut self) {
        if let Some(s) = self.settings.take() {
            if s.page == Page::Audio {
                self.audio_closed();
            }
            if s.page == Page::Inbox {
                if let Some(live) = &mut self.live {
                    live.close_activity();
                }
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), TIMER_INBOX);
                }
                self.dirty.sidebar = true;
            }
            self.dirty.settings = true;
            self.need = true;
            self.events |= EV_SCREEN | EV_FOCUS;
        }
    }

    fn settings_snapshot(&self) -> Snapshot {
        let s = self.settings.as_ref();
        let mut snap = Snapshot {
            launch: launch_allowed().then(|| s.is_some_and(|s| s.launch)),
            startup_error: s.and_then(|s| s.startup_error.clone()),
            ..Snapshot::default()
        };
        if let Some(live) = self.live.as_ref().filter(|_| self.c.is_live()) {
            live.settings_snapshot(&mut snap);
            if s.is_some_and(|s| s.page == Page::Inbox) {
                snap.inbox = Some(live.inbox_snapshot());
            }
            if s.is_some_and(|s| s.page == Page::Audio) {
                snap.audio = Some(audio::snapshot(&live.voice));
            }
        }
        snap
    }

    /// Settings' state or the account's choices changed: laid out again.
    pub fn settings_changed(&mut self) {
        let snap = self.settings_snapshot();
        let (g, w, h) = (&self.g, self.w, self.h);
        if let Some(s) = &mut self.settings
            && let Err(e) = s.relayout(g, snap, w, h)
        {
            eprintln!("settings layout failed: {e:?}");
        }
        if let Some(s) = self.settings.as_ref().filter(|s| s.page == Page::Audio) {
            self.arm_audio_timer(
                s.snap
                    .audio
                    .as_ref()
                    .is_some_and(|a| a.mic == Mic::Recording),
            );
        }
        self.dirty.settings = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// Clicks, Enter and UIA Invoke or Toggle on a Settings part.
    pub fn settings_activate(&mut self, part: SPart) {
        let Some(s) = &mut self.settings else { return };
        if !s.enabled(part) {
            return;
        }
        match part {
            SPart::Close => return self.close_settings(),
            SPart::Input(_)
            | SPart::Output(_)
            | SPart::RefreshDevices
            | SPart::SpeakerTest
            | SPart::MicTest
            | SPart::Sample(_) => return self.audio_activate(part),
            // Sliders move by dragging or RangeValue.
            p if p.is_slider() => return,
            SPart::Launch => {
                let wanted = !s.launch;
                match caper_client::startup::set_enabled(wanted) {
                    Ok(()) => {
                        s.launch = wanted;
                        s.startup_error = None;
                    }
                    Err(e) => {
                        s.startup_error = Some(format!("Could not change startup settings: {e}"))
                    }
                }
            }
            SPart::DismissStartup => s.startup_error = None,
            SPart::Delete => crate::open_link(DELETE_ACCOUNT_URL),
            SPart::Text(_) => {}
            _ => {
                let voice = !s.snap.voice_started;
                let Some(live) = &mut self.live else { return };
                match part {
                    SPart::Level(k) => {
                        live.change_notifications(Scope::Account, Change::Level(Some(LEVELS[k])))
                    }
                    SPart::VoiceStarted => {
                        live.change_notifications(Scope::Account, Change::VoiceStarted(Some(voice)))
                    }
                    SPart::RetryNotifications => live.load_notifications(),
                    SPart::Privacy(k) => live.save_privacy(DM_PRIVACY[k].0),
                    SPart::RetryPrivacy => live.load_privacy(),
                    SPart::Unblock(k) => live.unblock(k),
                    SPart::RetryBlocks => live.load_blocks(),
                    SPart::Filter(k) => live.set_activity_filter(Filter::ALL[k]),
                    SPart::UnreadOnly => live.toggle_unread_only(),
                    SPart::Open(k) => live.activity_action(k, true),
                    SPart::MarkRead(k) => live.activity_action(k, false),
                    SPart::LoadMore => live.load_activity(true, false),
                    SPart::RetryInbox => live.load_activity(false, true),
                    _ => {}
                }
                // A read changes the sidebar's count.
                self.dirty.sidebar = true;
            }
        }
        self.settings_changed();
    }

    /// Input while Settings is open: it is modal, as egui's dialog.
    pub fn settings_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let s = self.settings.as_mut()?;
        match msg {
            WM_MOUSEMOVE => {
                self.pointer = Some((x, y));
                if let Some(p) = s.pressed.filter(|p| p.is_slider()) {
                    if let Some(v) = s.gauge_at(p, x) {
                        self.set_gauge(p, v, false);
                    }
                    return done;
                }
                let hover = s.hit(x, y);
                if hover != s.hover {
                    s.hover = hover;
                    self.dirty.settings = true;
                    self.need = true;
                }
                done
            }
            WM_LBUTTONDOWN => {
                let part = s.hit(x, y);
                s.pressed = part;
                // A slider follows the pointer until it lets go.
                if let Some(p) = part.filter(|p| p.is_slider()) {
                    unsafe { SetCapture(self.hwnd) };
                    if let Some(v) = s.gauge_at(p, x) {
                        self.set_gauge(p, v, false);
                    }
                    return done;
                }
                if part.is_none() && !s.on_card(x, y) {
                    self.close_settings();
                } else {
                    self.dirty.settings = true;
                    self.need = true;
                }
                done
            }
            WM_LBUTTONUP => {
                let pressed = s.pressed.take();
                if let Some(p) = pressed.filter(|p| p.is_slider()) {
                    unsafe {
                        let _ = ReleaseCapture();
                    }
                    if let Some(v) = s.gauge_at(p, x) {
                        self.set_gauge(p, v, true);
                    }
                    return done;
                }
                self.dirty.settings = true;
                self.need = true;
                if let Some(p) = pressed.filter(|p| s.hit(x, y) == Some(*p)) {
                    self.settings_activate(p);
                }
                done
            }
            WM_MOUSEWHEEL => {
                let notches = (wp.0 >> 16) as u16 as i16 as f32 / 120.0;
                s.scroll_by(-notches * 48.0);
                s.hover = self.pointer.and_then(|(x, y)| s.hit(x, y));
                self.dirty.settings = true;
                self.events |= EV_SCREEN;
                self.need = true;
                done
            }
            WM_LBUTTONDBLCLK => done,
            WM_SETCURSOR => {
                let hand = self.pointer.and_then(|(x, y)| s.hit(x, y)).is_some();
                unsafe {
                    SetCursor(LoadCursorW(None, if hand { IDC_HAND } else { IDC_ARROW }).ok());
                }
                Some(Some(LRESULT(1)))
            }
            WM_KEYDOWN => {
                let page = s.body.3 - 48.0;
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_ESCAPE => {
                        self.close_settings();
                        return done;
                    }
                    VK_UP => s.scroll_by(-48.0),
                    VK_DOWN => s.scroll_by(48.0),
                    VK_PRIOR => s.scroll_by(-page),
                    VK_NEXT => s.scroll_by(page),
                    VK_HOME => s.scroll_by(-f32::MAX / 2.0),
                    VK_END => s.scroll_by(f32::MAX / 2.0),
                    _ => return done,
                }
                self.dirty.settings = true;
                self.events |= EV_SCREEN;
                self.need = true;
                done
            }
            WM_CHAR => done,
            _ => None,
        }
    }

    // ---- the account footer and its menu ----

    /// The footer's gear (client DIPs).
    pub fn gear_rect(&self) -> Rect {
        (
            RAIL_W + SIDEBAR_W - 12.0 - 28.0,
            self.h - FOOT_H + 12.0,
            28.0,
            28.0,
        )
    }

    pub fn foot_shown(&self) -> bool {
        self.live.as_ref().is_some_and(|l| l.account_id.is_some()) && self.c.is_live()
    }

    pub fn on_gear(&self, x: f32, y: f32) -> bool {
        self.foot_shown() && contains(self.gear_rect(), x, y)
    }

    /// The gear's menu (client DIPs), above the gear.
    pub fn account_menu_rect(&self) -> Rect {
        let h = MENU_ITEMS.len() as f32 * MENU_ITEM_H + 12.0;
        let (gx, gy, gw, _) = self.gear_rect();
        (
            (gx + gw - MENU_W).max(RAIL_W + 4.0),
            gy - 6.0 - h,
            MENU_W,
            h,
        )
    }

    pub fn account_item_rect(&self, k: usize) -> Rect {
        let (x, y, w, _) = self.account_menu_rect();
        (
            x + 6.0,
            y + 6.0 + k as f32 * MENU_ITEM_H,
            w - 12.0,
            MENU_ITEM_H,
        )
    }

    pub fn toggle_account_menu(&mut self) {
        self.account_menu = match self.account_menu {
            Some(_) => None,
            None => Some(AccountMenu { hover: None }),
        };
        self.dirty.settings = true;
        self.dirty.sidebar = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    pub fn account_activate(&mut self, k: usize) {
        self.account_menu = None;
        self.dirty.settings = true;
        self.need = true;
        match k {
            0 => self.open_settings(),
            1 => self.open_audio(),
            2 => self.logout(),
            _ => {}
        }
    }

    /// Input while the gear's menu is open.
    pub fn account_menu_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let hit = (0..MENU_ITEMS.len()).find(|&k| contains(self.account_item_rect(k), x, y));
        let inside = contains(self.account_menu_rect(), x, y);
        let gear = self.on_gear(x, y);
        let m = self.account_menu.as_mut()?;
        match msg {
            WM_MOUSEMOVE => {
                if hit != m.hover {
                    m.hover = hit;
                    self.dirty.settings = true;
                    self.need = true;
                }
                None
            }
            WM_LBUTTONDOWN => {
                if let Some(k) = hit {
                    self.account_activate(k);
                    return done;
                }
                if inside {
                    return done;
                }
                self.toggle_account_menu();
                // The gear's own click only closes it.
                if gear { done } else { None }
            }
            WM_KEYDOWN => {
                let n = MENU_ITEMS.len();
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_ESCAPE => self.toggle_account_menu(),
                    VK_DOWN => m.hover = Some(m.hover.map_or(0, |k| (k + 1) % n)),
                    VK_UP => m.hover = Some(m.hover.map_or(n - 1, |k| (k + n - 1) % n)),
                    VK_RETURN | VK_SPACE => {
                        if let Some(k) = m.hover {
                            self.account_activate(k);
                        }
                    }
                    _ => return done,
                }
                self.dirty.settings = true;
                self.need = true;
                done
            }
            WM_SETCURSOR if hit.is_some() => {
                unsafe { SetCursor(LoadCursorW(None, IDC_HAND).ok()) };
                Some(Some(LRESULT(1)))
            }
            _ => None,
        }
    }

    /// Signs out (egui's `logout`) and shows the sign-in form.
    pub fn logout(&mut self) {
        self.close_settings();
        self.close_dialog();
        self.close_menu();
        self.close_dm_menu();
        self.close_gifs(false);
        self.cancel_reply();
        self.close_thread();
        if let Some(live) = self.live.take() {
            self.live = Some(live.logout());
        }
        self.drafts.clear();
        self.composer.set_text(Vec::new());
        self.composer_changed();
        self.status = None;
        let mut s = crate::signin::SignIn::new();
        s.restoring = false;
        self.signin = Some(s);
        self.signin_screen_changed();
        self.set_signin_focus(true);
    }

    /// Draws the footer in the sidebar (sidebar-local DIPs).
    pub fn draw_foot(&self, g: &Gfx, dc: &ID2D1DeviceContext, h: f32) -> Result<()> {
        if !self.foot_shown() {
            return Ok(());
        }
        let Some(live) = &self.live else {
            return Ok(());
        };
        self.draw_update_dock(g, dc)?;
        let top = h - FOOT_H;
        unsafe {
            dc.FillRectangle(&rect(0.0, top, SIDEBAR_W, h), &g.solid(SURFACE_SIDEBAR));
            dc.FillRectangle(&rect(0.0, top, SIDEBAR_W, top + 1.0), &g.brushes.border);
        }
        let me = live.account_id.as_deref().unwrap_or_default();
        let p = crate::live::person(me, &live.account_name, live.account_avatar.clone());
        let (cx, cy) = (12.0 + 15.0, top + FOOT_H / 2.0);
        crate::avatars::circle(g, dc, &p.avatar, (cx, cy, 15.0), p.color, &p.initials)?;
        let name_w = SIDEBAR_W - 12.0 - 28.0 - 8.0 - (cx + 21.0);
        let name = g.layout(
            &gfx::wide(&live.account_name),
            &g.formats.direct_unread,
            name_w.max(1.0),
        )?;
        unsafe { name.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
        crate::emoji::apply(&name, &live.account_name, 13.0, 1.0)?;
        let nm = gfx::metrics(&name);
        text::draw(dc, &name, cx + 21.0, cy - nm.height / 2.0, &g.brushes.text);
        let (gx, gy, gw, gh) = self.gear_rect();
        let gx = gx - RAIL_W;
        let lit = self.gear_hover || self.account_menu.is_some();
        if lit {
            fill_round(dc, rect(gx, gy, gx + gw, gy + gh), 6.0, &g.brushes.hover);
        }
        icons::draw(
            dc,
            Icon::Settings,
            rect(gx + 5.0, gy + 5.0, gx + gw - 5.0, gy + gh - 5.0),
            if lit { TEXT } else { TEXT_MUTED },
            1.0,
        );
        Ok(())
    }

    /// Redraws Settings or the gear's menu when they changed.
    pub fn draw_settings(&mut self) -> Result<()> {
        if !std::mem::take(&mut self.dirty.settings) {
            return Ok(());
        }
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let g = &self.g;
        if let Some(st) = &self.settings {
            self.settings_p.ensure(g, px(self.w), px(self.h))?;
            self.settings_p.offset(0.0, 0.0)?;
            return self.settings_p.draw(s, |dc| st.draw(g, dc));
        }
        let Some(m) = &self.account_menu else {
            return self.settings_p.offset(-100_000.0, 0.0);
        };
        let (mx, my, mw, mh) = self.account_menu_rect();
        let items: Vec<Rect> = (0..MENU_ITEMS.len())
            .map(|k| {
                let (x, y, w, h) = self.account_item_rect(k);
                (x - mx, y - my, w, h)
            })
            .collect();
        self.settings_p.ensure(g, px(mw), px(mh))?;
        self.settings_p.offset((mx * s).round(), (my * s).round())?;
        self.settings_p.draw(s, |dc| {
            unsafe { dc.Clear(Some(&D2D1_COLOR_F::default())) };
            let r = rect(0.5, 0.5, mw - 0.5, mh - 0.5);
            fill_round(dc, r, RADIUS, &g.brushes.surface);
            stroke_round(dc, r, RADIUS, &g.brushes.border);
            for (k, (x, y, w, h)) in items.iter().enumerate() {
                if m.hover == Some(k) {
                    fill_round(dc, rect(*x, *y, x + w, y + h), 6.0, &g.brushes.hover);
                }
                let l = g.layout(&gfx::wide(MENU_ITEMS[k]), &g.formats.dialog_note, *w)?;
                let lm = gfx::metrics(&l);
                text::draw(dc, &l, x + 10.0, y + (h - lm.height) / 2.0, &g.brushes.text);
            }
            Ok(())
        })
    }
}
