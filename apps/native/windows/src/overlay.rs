//! Popups over the conversation, each on its own composition surface: the
//! message menu that the hover toolbar's "More" opens, and the modal dialogs
//! for editing and deleting a message and for its history (`versions`). They look and behave like the egui
//! client's: the same items in the same order, the same titles, notes,
//! buttons and errors.
use crate::composer::{Composer, LINE};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::theme::*;
use caper_client::model as core;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::core::*;

/// (x, y, width, height) in client DIPs.
pub type Rect = (f32, f32, f32, f32);

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

fn layout(
    g: &Gfx,
    text: &str,
    format: &IDWriteTextFormat,
    width: f32,
) -> Result<IDWriteTextLayout> {
    let l = g.layout(&gfx::wide(text), format, width)?;
    crate::emoji::apply(&l, text, 14.0, 1.0)?;
    Ok(l)
}

fn height(l: &IDWriteTextLayout) -> f32 {
    gfx::metrics(l).height
}

fn width(l: &IDWriteTextLayout) -> f32 {
    gfx::metrics(l).widthIncludingTrailingWhitespace
}

// ---------------------------------------------------------------------------
// The message menu.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Reply,
    Edit,
    History,
    Pin,
    Unpin,
    /// "Saving…" while a pin change is in flight; not clickable.
    PinSaving,
    CopyText,
    CopyId,
    Delete,
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Self::Reply => "Reply",
            Self::Edit => "Edit message",
            Self::History => "Message history",
            Self::Pin => "Pin message",
            Self::Unpin => "Unpin message",
            Self::PinSaving => "Saving\u{2026}",
            Self::CopyText => "Copy text",
            Self::CopyId => "Copy message ID",
            Self::Delete => "Delete message",
        }
    }

    pub fn enabled(self) -> bool {
        self != Self::PinSaving
    }

    fn icon(self) -> Icon {
        match self {
            Self::Reply => Icon::Reply,
            Self::Edit => Icon::Pencil,
            Self::History => Icon::History,
            Self::Pin | Self::PinSaving => Icon::Pin,
            Self::Unpin => Icon::PinOff,
            Self::CopyText => Icon::Copy,
            Self::CopyId => Icon::Hash,
            Self::Delete => Icon::Trash,
        }
    }
}

/// egui's popup: 244 DIPs of content inside a 6-DIP margin, a 30-DIP
/// "Message actions" heading with its close button, then 36-DIP items.
const MENU_PAD: f32 = 6.0;
const MENU_W: f32 = 244.0 + 2.0 * MENU_PAD;
const MENU_HEAD: f32 = 30.0 + 4.0;
const ITEM_H: f32 = 36.0;
/// Danger items (Delete message), as in the egui client.
const DANGER: u32 = 0xFF8C9A;

/// The actions menu of message `row`, at (x, y) in client DIPs.
pub struct Menu {
    pub row: usize,
    pub id: String,
    pub items: Vec<Item>,
    pub x: f32,
    pub y: f32,
    pub hover: Option<usize>,
    pub close_hover: bool,
}

impl Menu {
    pub fn size(&self) -> (f32, f32) {
        (
            MENU_W,
            2.0 * MENU_PAD + MENU_HEAD + ITEM_H * self.items.len() as f32,
        )
    }

    /// Opens below (x, bottom) right-aligned to x, or above `top` when it
    /// would leave the window.
    pub fn new(
        row: usize,
        id: String,
        items: Vec<Item>,
        right: f32,
        top: f32,
        bottom: f32,
        window_h: f32,
    ) -> Self {
        let mut m = Self {
            row,
            id,
            items,
            x: 0.0,
            y: 0.0,
            hover: None,
            close_hover: false,
        };
        let (w, h) = m.size();
        m.x = (right - w).max(4.0);
        m.y = if bottom + 4.0 + h > window_h - 4.0 {
            (top - 4.0 - h).max(4.0)
        } else {
            bottom + 4.0
        };
        m
    }

    pub fn rect(&self) -> Rect {
        let (w, h) = self.size();
        (self.x, self.y, w, h)
    }

    pub fn close_rect(&self) -> Rect {
        (
            self.x + MENU_W - MENU_PAD - 24.0,
            self.y + MENU_PAD + 3.0,
            24.0,
            24.0,
        )
    }

    pub fn item_rect(&self, k: usize) -> Rect {
        (
            self.x + MENU_PAD,
            self.y + MENU_PAD + MENU_HEAD + ITEM_H * k as f32,
            MENU_W - 2.0 * MENU_PAD,
            ITEM_H,
        )
    }

    pub fn hit(&self, x: f32, y: f32) -> Option<usize> {
        (0..self.items.len()).find(|&k| contains(self.item_rect(k), x, y))
    }

    /// Draws the menu with its top-left at the surface origin.
    pub fn draw(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        let (w, h) = self.size();
        unsafe {
            dc.Clear(Some(&D2D1_COLOR_F::default()));
            let body = rect(0.0, 0.0, w, h);
            fill_round(dc, body, RADIUS, &g.brushes.surface);
            stroke_round(dc, body, RADIUS, &g.brushes.border);
            let title = g.layout(&gfx::wide("Message actions"), &g.formats.menu_title, 1e6)?;
            crate::text::draw(
                dc,
                &title,
                MENU_PAD,
                MENU_PAD + (30.0 - height(&title)) / 2.0,
                &g.brushes.text,
            );
            let (cx, cy, cw, ch) = self.close_rect();
            let (cx, cy) = (cx - self.x, cy - self.y);
            if self.close_hover {
                fill_round(dc, rect(cx, cy, cx + cw, cy + ch), 6.0, &g.brushes.hover);
            }
            icons::draw(
                dc,
                Icon::Close,
                rect(cx + 3.0, cy + 3.0, cx + cw - 3.0, cy + ch - 3.0),
                if self.close_hover { TEXT } else { TEXT_MUTED },
                1.0,
            );
            for (k, item) in self.items.iter().enumerate() {
                let (ix, iy, iw, ih) = self.item_rect(k);
                let (ix, iy) = (ix - self.x, iy - self.y);
                if self.hover == Some(k) && item.enabled() {
                    fill_round(dc, rect(ix, iy, ix + iw, iy + ih), 4.0, &g.brushes.border);
                }
                let (color, alpha) = match item {
                    _ if !item.enabled() => (TEXT, 0.5),
                    Item::Delete => (DANGER, 1.0),
                    _ => (TEXT, 1.0),
                };
                let cy = iy + ih / 2.0;
                icons::draw(
                    dc,
                    item.icon(),
                    rect(ix + 8.0, cy - 8.0, ix + 24.0, cy + 8.0),
                    color,
                    alpha,
                );
                let l = layout(g, item.label(), &g.formats.menu_item, iw - 40.0)?;
                let brush = g.solid(color);
                brush.SetOpacity(alpha);
                crate::text::draw(dc, &l, ix + 32.0, cy - height(&l) / 2.0, &brush);
                brush.SetOpacity(1.0);
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Dialogs.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Edit,
    Delete,
    /// Message history: its retained versions, compared word by word.
    History,
    /// "New direct message": an exact username, found, then opened.
    StartDirect,
    /// "Block {name}?", confirming a block.
    Block,
    /// "Leave {space}?" and "Leave #{channel}?", from their menus.
    LeaveSpace,
    LeaveChannel,
    /// Browse channels: the open space's channels under a search.
    Browse,
    /// Join with invite link: a pasted link, then its space's preview.
    JoinInvite,
    /// "You\u{2019}re invited!" to a space, or a private channel's
    /// invitation: Decline or Accept invitation.
    Invitation,
    /// "Create a space" and "Create a channel": a name, and a
    /// channel's privacy.
    CreateSpace,
    CreateChannel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Title,
    Note,
    Field,
    Error,
    /// "Discard draft and load latest", after an edit failed.
    Reload,
    Cancel,
    Primary,
    /// The history's parts: its Close button; a comparison column
    /// (`versions::Dialog::column`); "View previous versions" and the
    /// earlier version `k` to compare; the hint that an older page is
    /// needed; "Loading versions…"; Retry; "Load older versions".
    Close,
    Column(u8),
    Choose,
    Version(usize),
    Hint,
    Loading,
    Retry,
    Older,
    /// The New DM field's suggested person `k`.
    Suggest(usize),
    /// Browse's shown channel `k`.
    Channel(usize),
    /// The invite link's space, who invited you and what joining opens.
    Preview,
    /// A field's label above it, Create a channel's guidance, and its
    /// Private channel switch.
    Label,
    Guide,
    Toggle,
}

impl Part {
    /// A stable number per part, for UI Automation's runtime ids.
    pub fn index(self) -> usize {
        match self {
            Self::Title => 0,
            Self::Note => 1,
            Self::Field => 2,
            Self::Error => 3,
            Self::Reload => 4,
            Self::Cancel => 5,
            Self::Primary => 6,
            Self::Close => 7,
            Self::Choose => 8,
            Self::Hint => 9,
            Self::Loading => 10,
            Self::Retry => 11,
            Self::Older => 12,
            Self::Preview => 13,
            Self::Label => 14,
            Self::Guide => 15,
            Self::Toggle => 16,
            Self::Column(k) => 16 + k as usize,
            Self::Version(k) => 32 + k,
            Self::Suggest(k) => 10_000 + k,
            Self::Channel(k) => 20_000 + k,
        }
    }

    /// Whether the part scrolls with the history's body (all but its head).
    fn in_body(self) -> bool {
        !matches!(self, Self::Title | Self::Note | Self::Close)
    }
}

pub(crate) const CARD_PAD: f32 = 24.0;
const BUTTON_H: f32 = 36.0;
pub(crate) const FIELD_PAD_X: f32 = 10.0;
const FIELD_PAD_Y: f32 = 8.0;
/// The edit field shows this many lines, then scrolls.
const FIELD_LINES: f32 = 6.0;
pub(crate) const DIRECT_FIELD_H: f32 = 42.0;
/// A suggested person's row, and the highlighted row's fill (egui's).
const SUGGEST_H: f32 = 44.0;
const SUGGEST_SELECTED: u32 = 0x43241E;

/// What a Leave dialog leaves: a space, or one of its channels; its name,
/// and whether leaving loses access (a private channel you don't own).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leave {
    pub space: String,
    pub channel: Option<String>,
    pub name: String,
    pub private: bool,
}

pub struct Dialog {
    pub kind: Kind,
    /// The message as it was when the dialog opened (an edit's original);
    /// None for the New DM dialog.
    pub message: Option<Box<core::Message>>,
    /// The edit's draft.
    pub field: Composer,
    pub busy: bool,
    pub error: Option<String>,
    /// The request whose reply this dialog waits for.
    pub request: u64,
    pub hover: Option<Part>,
    pub pressed: Option<Part>,
    /// Client DIP origin of the field's text, before it scrolls.
    pub field_origin: (f32, f32),
    /// The history's versions, while `kind` is History.
    pub history: Option<Box<crate::versions::Versions>>,
    /// How far the history's body is scrolled, and where it shows.
    pub scroll: f32,
    pub body: Rect,
    /// The New DM dialog's person for the typed username (someone you
    /// know, or the lookup's answer), the username it is for, and whether
    /// the conversation is opening.
    pub recipient: Option<core::Person>,
    pub resolved: String,
    pub creating: bool,
    /// The New DM field's people suggestions (egui's `people_field`): the
    /// rows for the typed text, the highlighted one, and whether Escape or
    /// a choice hid them until the text changes.
    pub suggestions: Vec<core::Person>,
    pub selected: usize,
    pub hidden: bool,
    /// Whom the Block dialog blocks, and the request it declines.
    pub block: Option<(core::BlockedAccount, Option<String>)>,
    /// What the Leave dialog leaves.
    pub leave: Option<Leave>,
    /// What Browse channels lists.
    pub browse: Option<Box<crate::browse::Browse>>,
    /// The invite link's preview, once the pasted link checked out.
    pub invite: Option<Box<caper_client::invites::InvitePreview>>,
    /// The invitation answered, and whether Decline was the answer.
    pub invitation: Option<Box<crate::live::Invitation>>,
    pub declining: bool,
    /// Create a channel's privacy, and the space it is created in.
    pub private: bool,
    pub space_name: String,
    pub(crate) parts: Vec<(Part, Rect)>,
    pub(crate) card: Rect,
}

impl Dialog {
    pub fn edit(message: core::Message) -> Self {
        let mut field = Composer::default();
        field.text = gfx::wide(&message.content.text);
        field.focused = true;
        field.caret_on = true;
        // Continue where the message ends, as on web.
        field.caret = field.text.len();
        field.anchor = field.caret;
        Self::new(Kind::Edit, Some(message), field)
    }

    pub fn delete(message: core::Message) -> Self {
        Self::new(Kind::Delete, Some(message), Composer::default())
    }

    pub fn start_direct() -> Self {
        let field = Composer {
            focused: true,
            caret_on: true,
            ..Default::default()
        };
        Self::new(Kind::StartDirect, None, field)
    }

    pub fn block(account: core::BlockedAccount, request: Option<String>) -> Self {
        let mut d = Self::new(Kind::Block, None, Composer::default());
        d.block = Some((account, request));
        d
    }

    pub fn leave(leave: Leave) -> Self {
        let kind = if leave.channel.is_some() {
            Kind::LeaveChannel
        } else {
            Kind::LeaveSpace
        };
        let mut d = Self::new(kind, None, Composer::default());
        d.leave = Some(leave);
        d
    }

    pub(crate) fn new(kind: Kind, message: Option<core::Message>, field: Composer) -> Self {
        Self {
            kind,
            message: message.map(Box::new),
            field,
            busy: false,
            error: None,
            request: 0,
            hover: None,
            pressed: None,
            field_origin: (0.0, 0.0),
            history: None,
            scroll: 0.0,
            body: (0.0, 0.0, 0.0, 0.0),
            recipient: None,
            resolved: String::new(),
            creating: false,
            suggestions: Vec::new(),
            selected: 0,
            hidden: false,
            block: None,
            leave: None,
            browse: None,
            invite: None,
            invitation: None,
            declining: false,
            private: false,
            space_name: String::new(),
            parts: Vec::new(),
            card: (0.0, 0.0, 0.0, 0.0),
        }
    }

    pub fn draft(&self) -> String {
        String::from_utf16_lossy(&self.field.text)
    }

    pub fn title(&self) -> String {
        match self.kind {
            Kind::Edit => "Edit message".into(),
            Kind::Delete => "Delete message?".into(),
            Kind::History => "Message history".into(),
            Kind::StartDirect => "New direct message".into(),
            Kind::Block => format!(
                "Block {}?",
                self.block
                    .as_ref()
                    .map_or("", |(a, _)| a.display_name.as_str())
            ),
            Kind::Browse => "Browse channels".into(),
            Kind::JoinInvite | Kind::Invitation => self.invite_title().into(),
            Kind::CreateSpace => "Create a space".into(),
            Kind::CreateChannel => "Create a channel".into(),
            Kind::LeaveSpace | Kind::LeaveChannel => {
                let name = self.leave.as_ref().map_or("", |l| l.name.as_str());
                if self.kind == Kind::LeaveChannel {
                    format!("Leave #{name}?")
                } else {
                    format!("Leave {name}?")
                }
            }
        }
    }

    pub fn note(&self) -> String {
        match self.kind {
            Kind::Edit => {
                "Previous versions remain visible to people who can read this message.".into()
            }
            Kind::Delete => {
                "This removes the message for everyone. This can\u{2019}t be undone.".into()
            }
            Kind::History => format!(
                "{} \u{b7} Previous versions are retained.",
                self.message.as_ref().map_or("", |m| m.author.name.as_str())
            ),
            Kind::StartDirect => "Enter an exact username.".into(),
            Kind::Block => "You won't see their messages unless you choose to, and they can't send you DMs or requests.".into(),
            Kind::Browse => format!(
                "Find conversations in {}. Previewing a channel doesn\u{2019}t join it.",
                self.browse.as_ref().map_or("", |b| b.space_name.as_str())
            ),
            Kind::JoinInvite | Kind::Invitation => self.invite_note(),
            Kind::CreateSpace | Kind::CreateChannel => String::new(),
            Kind::LeaveSpace => "You will lose access to its channels and conversations. An owner can add you again later.".into(),
            Kind::LeaveChannel if self.leave.as_ref().is_some_and(|l| l.private) => {
                "You\u{2019}ll lose access and need another invitation to return. You\u{2019}ll disconnect from this channel\u{2019}s voice call.".into()
            }
            Kind::LeaveChannel => "It will leave your sidebar. You can preview and rejoin from Browse channels. You\u{2019}ll disconnect from this channel\u{2019}s voice call.".into(),
        }
    }

    /// Who the New DM dialog will open, as egui shows them.
    pub fn recipient_label(&self) -> Option<String> {
        let p = self.recipient.as_ref()?;
        Some(format!("{} \u{b7} @{}", p.display_name, p.username))
    }

    /// The New DM field's text changed: it keeps only username characters
    /// (egui's `normalize_username`), and a new username drops the person
    /// found for the old one, matching someone you know instead.
    fn username_changed(&mut self, live: Option<&crate::live::Live>) {
        let typed = self.draft();
        let name = caper_client::names::normalize_username(&typed);
        if name != typed {
            self.field.text = gfx::wide(&name);
            self.field.caret = self.field.caret.min(self.field.text.len());
            self.field.anchor = self.field.anchor.min(self.field.text.len());
        }
        if name != self.resolved {
            self.recipient = live.and_then(|l| l.person_named(&name)).cloned();
            self.error = None;
            self.selected = 0;
            self.hidden = false;
            self.resolved = name;
        }
        // Ranked as `@` suggestions are, while the field is focused and
        // nothing is in flight.
        self.suggestions = match live {
            Some(live) if self.field.focused && !self.hidden && !self.busy => {
                let people: Vec<_> = live
                    .people()
                    .iter()
                    .map(caper_client::mentions::Person::from)
                    .collect();
                caper_client::mentions::field_suggestions(&people, &self.resolved)
                    .into_iter()
                    .filter_map(|p| live.person_named(&p.username).cloned())
                    .collect()
            }
            _ => Vec::new(),
        };
        self.selected = self.selected.min(self.suggestions.len().saturating_sub(1));
    }

    /// Puts suggestion `k`'s exact username in the field with the caret
    /// after it, and hides the list until the text changes. It never
    /// submits.
    fn choose(&mut self, k: usize, live: Option<&crate::live::Live>) {
        let Some(p) = self.suggestions.get(k) else {
            return;
        };
        self.field.text = gfx::wide(&p.username);
        self.field.caret = self.field.text.len();
        self.field.anchor = self.field.caret;
        self.field.focused = true;
        self.username_changed(live);
        self.hidden = true;
        self.suggestions.clear();
    }

    pub fn primary_label(&self) -> &'static str {
        match (self.kind, self.busy) {
            (Kind::Edit, false) => "Save changes",
            (Kind::Edit, true) => "Saving\u{2026}",
            (Kind::Delete, false) => "Delete",
            (Kind::Delete, true) => "Deleting\u{2026}",
            (Kind::History, _) => "Close",
            (Kind::StartDirect, true) if self.creating => "Opening\u{2026}",
            (Kind::StartDirect, true) => "Finding\u{2026}",
            (Kind::StartDirect, false) if self.recipient.is_some() => "Open conversation",
            (Kind::StartDirect, false) => "Find person",
            (Kind::Block, false) => "Block",
            (Kind::Block, true) => "Blocking\u{2026}",
            (Kind::Browse, _) => "Close",
            (Kind::LeaveSpace, false) => "Leave space",
            (Kind::LeaveChannel, false) => "Leave channel",
            (Kind::LeaveSpace | Kind::LeaveChannel, true) => "Leaving\u{2026}",
            (Kind::JoinInvite | Kind::Invitation, _) => self.invite_primary(),
            (Kind::CreateSpace, false) => "Create space",
            (Kind::CreateChannel, false) => "Create channel",
            (Kind::CreateSpace | Kind::CreateChannel, true) => "Creating\u{2026}",
        }
    }

    pub fn counter(&self) -> String {
        let n = self.draft().chars().count();
        let n = if n >= 1000 {
            format!("{},{:03}", n / 1000, n % 1000)
        } else {
            n.to_string()
        };
        format!("{n} / 4,000")
    }

    pub fn enabled(&self, part: Part) -> bool {
        match part {
            Part::Primary => {
                !self.busy
                    && match self.kind {
                        Kind::Edit => caper_client::edits::valid_text(&self.draft()),
                        Kind::Delete | Kind::Block | Kind::LeaveSpace | Kind::LeaveChannel => true,
                        Kind::History | Kind::Browse => false,
                        Kind::StartDirect => self.draft().len() >= 3,
                        Kind::JoinInvite => {
                            self.invite.is_some() || !self.draft().trim().is_empty()
                        }
                        Kind::Invitation => true,
                        Kind::CreateSpace | Kind::CreateChannel => !self.create_name().is_empty(),
                    }
            }
            Part::Cancel
            | Part::Reload
            | Part::Field
            | Part::Suggest(_)
            | Part::Channel(_)
            | Part::Toggle => !self.busy,
            Part::Retry | Part::Older => self.history.as_ref().is_some_and(|v| !v.loading),
            Part::Title
            | Part::Note
            | Part::Error
            | Part::Close
            | Part::Column(_)
            | Part::Choose
            | Part::Version(_)
            | Part::Hint
            | Part::Loading
            | Part::Preview
            | Part::Label
            | Part::Guide => true,
        }
    }

    /// The parts shown, in reading order.
    pub fn parts(&self) -> impl Iterator<Item = Part> + '_ {
        self.parts.iter().map(|(p, _)| *p)
    }

    pub fn part_rect(&self, part: Part) -> Option<Rect> {
        self.parts.iter().find(|(p, _)| *p == part).map(|(_, r)| *r)
    }

    /// The clickable part under (x, y).
    pub fn hit(&self, x: f32, y: f32) -> Option<Part> {
        self.parts
            .iter()
            .find(|(p, r)| {
                matches!(
                    p,
                    Part::Field
                        | Part::Reload
                        | Part::Cancel
                        | Part::Primary
                        | Part::Close
                        | Part::Version(_)
                        | Part::Retry
                        | Part::Older
                        | Part::Suggest(_)
                        | Part::Channel(_)
                        | Part::Toggle
                ) && contains(*r, x, y)
                    && match self.kind {
                        Kind::History => !p.in_body() || contains(self.body, x, y),
                        Kind::Browse => !matches!(p, Part::Channel(_)) || contains(self.body, x, y),
                        _ => true,
                    }
            })
            .map(|(p, _)| *p)
    }

    /// Whether (x, y) is on the card (clicks outside close it, as on web).
    pub fn on_card(&self, x: f32, y: f32) -> bool {
        contains(self.card, x, y)
    }

    /// The field's caret position for a point.
    pub fn field_hit(&self, x: f32, y: f32) -> usize {
        let (ox, oy) = self.field_origin;
        self.field.hit(x - ox, y - oy)
    }

    /// Client DIP origin of the field's text layout as drawn (scrolled), for
    /// the IME window and UI Automation.
    pub fn text_origin(&self) -> (f32, f32) {
        let (ox, oy) = self.field_origin;
        (ox, oy - self.field.scroll())
    }

    /// Lays out the card for a `w` × `h` window: the field's text and every
    /// part's rectangle.
    pub fn relayout(&mut self, g: &Gfx, w: f32, h: f32) -> Result<()> {
        if self.kind == Kind::History {
            return self.relayout_history(g, w, h);
        }
        if self.kind == Kind::Browse {
            return self.relayout_browse(g, w, h);
        }
        let card_w = match self.kind {
            Kind::Edit => 500.0_f32,
            _ => 420.0,
        }
        .min(w - 48.0)
        .max(200.0);
        let inner = card_w - 2.0 * CARD_PAD;
        let mut parts = Vec::new();
        let mut y = 0.0;
        let title = layout(g, &self.title(), &g.formats.dialog_title, inner)?;
        parts.push((Part::Title, (0.0, y, inner, height(&title))));
        y += height(&title) + 4.0;
        if self.kind == Kind::Invitation {
            y += 8.0;
            let l = layout(g, &self.hint_text(), &g.formats.direct_name, inner)?;
            parts.push((Part::Hint, (0.0, y, inner, height(&l))));
            y += height(&l) + 4.0;
        }
        let note_format = match self.kind {
            Kind::Delete => &g.formats.note,
            _ => &g.formats.dialog_note,
        };
        let note_text = self.note();
        if !note_text.is_empty() {
            let note = layout(g, &note_text, note_format, inner)?;
            parts.push((Part::Note, (0.0, y, inner, height(&note))));
            y += height(&note);
        }
        if self.kind == Kind::JoinInvite {
            y += 12.0;
            if self.invite.is_some() {
                let h = self.preview_height(g, inner)?;
                parts.push((Part::Preview, (0.0, y, inner, h)));
                y += h;
            } else {
                self.field.relayout_with(
                    g,
                    &g.formats.composer,
                    15.0,
                    inner - 2.0 * FIELD_PAD_X,
                )?;
                parts.push((Part::Field, (0.0, y, inner, DIRECT_FIELD_H)));
                y += DIRECT_FIELD_H;
            }
        }
        if matches!(self.kind, Kind::CreateSpace | Kind::CreateChannel) {
            y = self.relayout_create(g, inner, y, &mut parts)?;
        }
        if self.kind == Kind::Edit {
            y += 12.0;
            self.field
                .relayout_with(g, &g.formats.composer, 15.0, inner - 2.0 * FIELD_PAD_X)?;
            let fh = FIELD_LINES * LINE + 2.0 * FIELD_PAD_Y;
            parts.push((Part::Field, (0.0, y, inner, fh)));
            y += fh + 4.0 + 16.0;
        }
        if self.kind == Kind::StartDirect {
            // One line, egui's 42-DIP username field, then who it finds.
            y += 12.0;
            self.field
                .relayout_with(g, &g.formats.composer, 15.0, inner - 2.0 * FIELD_PAD_X)?;
            parts.push((Part::Field, (0.0, y, inner, DIRECT_FIELD_H)));
            y += DIRECT_FIELD_H;
            // The people list under the field, 44-DIP rows in a 4-DIP frame.
            if !self.suggestions.is_empty() {
                y += 8.0;
                for k in 0..self.suggestions.len() {
                    parts.push((Part::Suggest(k), (4.0, y, inner - 8.0, SUGGEST_H)));
                    y += SUGGEST_H;
                }
                y += 4.0;
            }
            if let Some(label) = self.recipient_label() {
                y += 8.0;
                let l = layout(g, &label, &g.formats.dialog_note, inner)?;
                parts.push((Part::Hint, (0.0, y, inner, height(&l))));
                y += height(&l);
            }
        }
        if let Some(e) = &self.error {
            y += 8.0;
            let l = layout(g, e, &g.formats.error, inner)?;
            parts.push((Part::Error, (0.0, y, inner, height(&l))));
            y += height(&l);
            if self.kind == Kind::Edit {
                y += 6.0;
                let l = layout(g, "Discard draft and load latest", &g.formats.button, 1e6)?;
                parts.push((Part::Reload, (0.0, y, width(&l) + 24.0, 30.0)));
                y += 30.0;
            }
        }
        y += 20.0;
        let primary = layout(g, self.primary_label(), &g.formats.button, 1e6)?;
        let pw = (width(&primary) + 24.0).max(112.0);
        let cancel = layout(g, self.cancel_label(), &g.formats.button, 1e6)?;
        let cw = width(&cancel) + 24.0;
        parts.push((Part::Cancel, (inner - pw - 8.0 - cw, y, cw, BUTTON_H)));
        parts.push((Part::Primary, (inner - pw, y, pw, BUTTON_H)));
        y += BUTTON_H;
        let card_h = y + 2.0 * CARD_PAD;
        let (cx, cy) = (
            ((w - card_w) / 2.0).round(),
            ((h - card_h) / 2.0).max(24.0).round(),
        );
        self.card = (cx, cy, card_w, card_h);
        for (_, r) in &mut parts {
            r.0 += cx + CARD_PAD;
            r.1 += cy + CARD_PAD;
        }
        self.parts = parts;
        if let Some((fx, fy, _, fh)) = self.part_rect(Part::Field) {
            let pad = match self.kind {
                Kind::StartDirect | Kind::JoinInvite | Kind::CreateSpace | Kind::CreateChannel => {
                    (fh - LINE) / 2.0
                }
                _ => FIELD_PAD_Y,
            };
            // Create a channel's field starts after its # or lock.
            let icon = if self.kind == Kind::CreateChannel {
                crate::manage::ICON_W
            } else {
                0.0
            };
            self.field_origin = (fx + FIELD_PAD_X + icon, fy + pad);
        }
        Ok(())
    }

    /// Draws the scrim and the card over a `w` × `h` window. Call after
    /// [`Dialog::relayout`].
    pub fn draw(&self, g: &Gfx, dc: &ID2D1DeviceContext, w: f32, h: f32) -> Result<()> {
        unsafe {
            dc.Clear(Some(&gfx::color(BLACKOUT, 0.6)));
            let _ = (w, h);
            let (cx, cy, cw, ch) = self.card;
            let card = rect(cx, cy, cx + cw, cy + ch);
            fill_round(dc, card, RADIUS, &g.brushes.surface);
            stroke_round(dc, card, RADIUS, &g.brushes.border);
            if self.kind == Kind::History {
                return self.draw_history(g, dc);
            }
            if self.kind == Kind::Browse {
                return self.draw_browse(g, dc);
            }
            for (part, (x, y, pw, ph)) in &self.parts {
                let (x, y, pw, ph) = (*x, *y, *pw, *ph);
                match part {
                    Part::Title => {
                        let l = layout(g, &self.title(), &g.formats.dialog_title, pw)?;
                        crate::text::draw(dc, &l, x, y, &g.brushes.text);
                    }
                    Part::Note => {
                        let format = match self.kind {
                            Kind::Delete => &g.formats.note,
                            _ => &g.formats.dialog_note,
                        };
                        let l = layout(g, &self.note(), format, pw)?;
                        crate::text::draw(dc, &l, x, y, &g.brushes.muted);
                    }
                    Part::Suggest(k) => {
                        if *k == 0 {
                            // The list's frame, around every row.
                            let n = self.suggestions.len() as f32;
                            let frame = rect(x - 4.0, y - 4.0, x + pw + 4.0, y + n * ph + 4.0);
                            fill_round(dc, frame, RADIUS, &g.brushes.composer);
                            stroke_round(dc, frame, RADIUS, &g.brushes.border);
                        }
                        self.draw_suggestion(g, dc, *k, (x, y, pw, ph))?;
                    }
                    Part::Field => {
                        let body = rect(x, y, x + pw, y + ph);
                        let stroke = if self.field.focused {
                            g.solid(TERRACOTTA)
                        } else if self.hover == Some(Part::Field) {
                            g.solid(TERRACOTTA_BRIGHT)
                        } else {
                            g.brushes.border.clone()
                        };
                        fill_round(dc, body, 7.0, &g.brushes.composer);
                        stroke_round(dc, body, 7.0, &stroke);
                        dc.PushAxisAlignedClip(
                            &rect(x + 2.0, y + 2.0, x + pw - 2.0, y + ph - 2.0),
                            D2D1_ANTIALIAS_MODE_ALIASED,
                        );
                        let (ox, oy) = self.text_origin();
                        let hint = match self.kind {
                            Kind::StartDirect => {
                                Some(layout(g, "Username", &g.formats.composer, 1e6)?)
                            }
                            Kind::CreateSpace => {
                                Some(layout(g, "Gaming nights", &g.formats.composer, 1e6)?)
                            }
                            Kind::CreateChannel => {
                                Some(layout(g, "project-updates", &g.formats.composer, 1e6)?)
                            }
                            // web's and egui's example under an empty field.
                            Kind::JoinInvite => Some(layout(
                                g,
                                "it looks like \u{2192} caper.chat/invite/AbCdE12345",
                                &g.formats.composer,
                                1e6,
                            )?),
                            _ => None,
                        };
                        self.field.draw_text(
                            g,
                            dc,
                            ox,
                            oy,
                            hint.as_ref().map(|l| (l, &g.brushes.muted)),
                        );
                        dc.PopAxisAlignedClip();
                        if self.kind == Kind::CreateChannel {
                            self.draw_channel_icon(dc, (x, y, pw, ph));
                        }
                        if self.kind == Kind::Edit {
                            let counter = layout(g, &self.counter(), &g.formats.quote_meta, 1e6)?;
                            crate::text::draw(
                                dc,
                                &counter,
                                x + pw - width(&counter),
                                y + ph + 4.0,
                                &g.brushes.muted,
                            );
                        }
                    }
                    Part::Hint => {
                        let format = match self.kind {
                            Kind::Invitation => &g.formats.direct_name,
                            Kind::CreateChannel => &g.formats.direct_note,
                            _ => &g.formats.dialog_note,
                        };
                        let brush = match self.kind {
                            Kind::CreateChannel => &g.brushes.muted,
                            _ => &g.brushes.text,
                        };
                        let l = layout(g, &self.hint_text(), format, pw)?;
                        crate::text::draw(dc, &l, x, y, brush);
                    }
                    Part::Label | Part::Guide | Part::Toggle => {
                        self.draw_create_part(g, dc, *part, (x, y, pw, ph))?
                    }
                    Part::Preview => self.draw_preview(g, dc, (x, y, pw, ph))?,
                    Part::Error => {
                        let l = layout(
                            g,
                            self.error.as_deref().unwrap_or_default(),
                            &g.formats.error,
                            pw,
                        )?;
                        crate::text::draw(dc, &l, x, y, &g.brushes.error);
                    }
                    Part::Reload => self.button(
                        g,
                        dc,
                        *part,
                        "Discard draft and load latest",
                        (x, y, pw, ph),
                        Tone::Secondary,
                    )?,
                    Part::Cancel => self.button(
                        g,
                        dc,
                        *part,
                        self.cancel_label(),
                        (x, y, pw, ph),
                        Tone::Secondary,
                    )?,
                    Part::Primary => {
                        let tone = match self.kind {
                            // Leaving a channel is neutral, as on web.
                            Kind::Delete | Kind::Block | Kind::LeaveSpace => Tone::Danger,
                            _ => Tone::Primary,
                        };
                        self.button(g, dc, *part, self.primary_label(), (x, y, pw, ph), tone)?;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Suggested person `k`: highlighted or hovered, a 24-DIP avatar, the
    /// name over a muted @username.
    fn draw_suggestion(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        k: usize,
        (x, y, w, h): Rect,
    ) -> Result<()> {
        let Some(p) = self.suggestions.get(k) else {
            return Ok(());
        };
        let r = rect(x, y, x + w, y + h);
        if self.selected == k {
            fill_round(dc, r, 6.0, &g.solid(SUGGEST_SELECTED));
        } else if self.hover == Some(Part::Suggest(k)) {
            fill_round(dc, r, 6.0, &g.brushes.hover);
        }
        let face = crate::live::person(&p.id, &p.display_name);
        let (cx, cy) = (x + 8.0 + 12.0, y + h / 2.0);
        unsafe {
            dc.FillEllipse(
                &D2D1_ELLIPSE {
                    point: windows_numerics::Vector2 { X: cx, Y: cy },
                    radiusX: 12.0,
                    radiusY: 12.0,
                },
                &g.solid(face.color),
            );
        }
        let i = g.layout(&gfx::wide(&face.initials), &g.formats.direct_initials, 1e6)?;
        crate::text::draw(
            dc,
            &i,
            cx - width(&i) / 2.0,
            cy - height(&i) / 2.0,
            &g.brushes.text,
        );
        let tx = x + 8.0 + 24.0 + 12.0;
        let tw = (w - (tx - x) - 8.0).max(1.0);
        let name = layout(g, &p.display_name, &g.formats.direct_name, tw)?;
        crate::text::draw(dc, &name, tx, y + 5.0, &g.brushes.text);
        let user = layout(g, &format!("@{}", p.username), &g.formats.direct_note, tw)?;
        crate::text::draw(dc, &user, tx, y + 24.0, &g.brushes.muted);
        Ok(())
    }

    /// Suggestion `k`'s accessible name.
    pub fn suggestion_name(&self, k: usize) -> String {
        self.suggestions.get(k).map_or_else(String::new, |p| {
            format!("{} @{}", p.display_name, p.username)
        })
    }

    pub(crate) fn button(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        part: Part,
        label: &str,
        r: Rect,
        tone: Tone,
    ) -> Result<()> {
        let enabled = self.enabled(part);
        let state = if !enabled {
            0
        } else if self.pressed == Some(part) && self.hover == Some(part) {
            3
        } else if self.hover == Some(part) {
            2
        } else {
            1
        };
        draw_button(g, dc, label, r, tone, state)
    }
}

/// A button in `tone` and `state` (0 disabled, 1 rest, 2 hover, 3 pressed).
pub(crate) fn draw_button(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    label: &str,
    r: Rect,
    tone: Tone,
    state: u8,
) -> Result<()> {
    let (fill, border, text) = tone.colors(state);
    let (x, y, w, h) = r;
    let body = rect(x, y, x + w, y + h);
    fill_round(dc, body, 7.0, &g.solid(fill));
    stroke_round(dc, body, 7.0, &g.solid(border));
    let l = layout(g, label, &g.formats.button, w)?;
    crate::text::draw(
        dc,
        &l,
        x + (w - width(&l)) / 2.0,
        y + (h - height(&l)) / 2.0,
        &g.solid(text),
    );
    Ok(())
}

/// The egui client's button tones (web's `.primary`, `.secondary` and
/// danger buttons): fill, border and label at rest, hovered and pressed.
#[derive(Clone, Copy)]
pub(crate) enum Tone {
    Primary,
    Secondary,
    Danger,
}

impl Tone {
    /// `state`: 0 disabled, 1 rest, 2 hover, 3 pressed.
    fn colors(self, state: u8) -> (u32, u32, u32) {
        if state == 0 {
            // Disabled steps back to neutral chrome.
            return (SURFACE_RAISED, BORDER, TEXT_MUTED);
        }
        match (self, state) {
            (Self::Primary, 1) => (ACTION, ACTION, BLACKOUT),
            (Self::Primary, 2) => (ACTION_HOVER, ACTION_HOVER, BLACKOUT),
            (Self::Primary, _) => (ACTION_PRESS, ACTION_PRESS, BLACKOUT),
            (Self::Secondary, 1) => (SURFACE_RAISED, BORDER, TEXT),
            (Self::Secondary, 2) => (BORDER, TEXT_MUTED, TEXT),
            (Self::Secondary, _) => (0x39231E, TERRACOTTA_BRIGHT, TEXT),
            (Self::Danger, 1) => (0xB93646, 0xB93646, 0xFFFFFF),
            (Self::Danger, 2) => (0xD44355, 0xD44355, 0xFFFFFF),
            (Self::Danger, _) => (0xA02E3C, 0xFF8C9A, 0xFFFFFF),
        }
    }
}

// ---------------------------------------------------------------------------
// The window's side: opening, input, drawing and replies.

use crate::{App, EV_FOCUS, EV_SCREEN, EV_TEXT};
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

impl App {
    /// Opens row `i`'s actions menu under its toolbar.
    pub fn open_menu(&mut self, i: usize) {
        let Some(live) = &self.live else { return };
        let Some(m) = self.c.messages.get(i).and_then(|r| live.message(&r.id)) else {
            return;
        };
        let mut items = Vec::new();
        if live.can_react() && !m.is_deleted() {
            items.push(Item::Reply);
        }
        if live.can_edit(&m) {
            items.push(Item::Edit);
        }
        // As in egui: an edited message's history, after Edit message.
        if m.forward.is_none() && m.revision > 1 && !m.is_deleted() {
            items.push(Item::History);
        }
        if live.can_pin(&m) {
            items.push(if live.pin_pending(&m.id) {
                Item::PinSaving
            } else if self.c.messages[i].pinned_by.is_some() {
                Item::Unpin
            } else {
                Item::Pin
            });
        }
        if !m.is_deleted() && !m.content.text.is_empty() {
            items.push(Item::CopyText);
        }
        items.push(Item::CopyId);
        if live.can_delete(&m) {
            items.push(Item::Delete);
        }
        let (tx, ty, tw, th) = self.bar_rect_for(i);
        self.close_picker();
        self.menu = Some(Menu::new(i, m.id, items, tx + tw, ty, ty + th, self.h));
        self.overlay_changed();
    }

    pub fn close_menu(&mut self) {
        if self.menu.take().is_some() {
            self.overlay_changed();
        }
    }

    /// Clicks, Enter and UIA Invoke on a menu item.
    pub fn menu_activate(&mut self, k: usize) {
        let Some(menu) = self.menu.take() else { return };
        self.overlay_changed();
        let (Some(item), Some(live)) = (menu.items.get(k).copied(), &mut self.live) else {
            return;
        };
        let Some(m) = live.message(&menu.id) else {
            return;
        };
        match item {
            Item::Reply => return self.select_reply(menu.row),
            Item::Edit if live.can_edit(&m) => self.open_dialog(Dialog::edit(m)),
            Item::Delete if live.can_delete(&m) => self.open_dialog(Dialog::delete(m)),
            Item::History => return self.open_history(m),
            Item::Pin => live.pin(&m, true),
            Item::Unpin => live.pin(&m, false),
            Item::CopyText => crate::set_clipboard(self.hwnd, &gfx::wide(&m.content.text)),
            Item::CopyId => crate::set_clipboard(self.hwnd, &gfx::wide(&m.id)),
            _ => {}
        }
        if let Err(e) = self.drain_live() {
            eprintln!("message action failed: {e:?}");
        }
    }

    pub(crate) fn open_dialog(&mut self, d: Dialog) {
        self.set_composer_focus(false);
        let editing = matches!(
            d.kind,
            Kind::Edit
                | Kind::StartDirect
                | Kind::Browse
                | Kind::JoinInvite
                | Kind::CreateSpace
                | Kind::CreateChannel
        );
        self.dialog = Some(d);
        if editing {
            unsafe {
                SetTimer(
                    Some(self.hwnd),
                    crate::TIMER_CARET,
                    GetCaretBlinkTime(),
                    None,
                );
            }
        }
        self.dialog_changed();
        self.events |= EV_SCREEN | EV_FOCUS;
    }

    pub fn close_dialog(&mut self) {
        if self.dialog.take().is_some() {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), crate::TIMER_CARET);
            }
            self.overlay_changed();
            self.events |= EV_SCREEN | EV_FOCUS;
        }
    }

    fn overlay_changed(&mut self) {
        self.dirty.overlay = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// After the dialog's text, caret, error or busy state changed.
    pub fn dialog_changed(&mut self) {
        let (g, w, h) = (&self.g, self.w, self.h);
        if let Some(d) = &mut self.dialog {
            if d.kind == Kind::StartDirect {
                d.username_changed(self.live.as_ref());
            }
            // Editing the link clears why the last one failed.
            if matches!(d.kind, Kind::CreateSpace | Kind::CreateChannel) {
                d.normalize_name();
            }
            if matches!(
                d.kind,
                Kind::JoinInvite | Kind::CreateSpace | Kind::CreateChannel
            ) {
                let typed = d.draft();
                if typed != d.resolved {
                    d.error = None;
                    d.resolved = typed;
                }
            }
            d.field.caret_on = true;
            if let Err(e) = d.relayout(g, w, h) {
                eprintln!("dialog layout failed: {e:?}");
            }
        }
        self.dirty.overlay = true;
        self.events |= EV_TEXT;
        self.need = true;
        self.ime_position();
    }

    /// Clicks, Enter and UIA Invoke on a dialog part.
    pub fn dialog_activate(&mut self, part: Part) {
        let Some(d) = &mut self.dialog else { return };
        if !d.enabled(part) {
            return;
        }
        match part {
            Part::Primary | Part::Cancel if d.kind == Kind::Invitation => {
                let (Some(live), Some(invitation)) = (&mut self.live, d.invitation.as_deref())
                else {
                    return;
                };
                d.busy = true;
                d.declining = part == Part::Cancel;
                d.error = None;
                live.answer_invitation(invitation, part == Part::Primary);
            }
            Part::Toggle => {
                d.private = !d.private;
                if let Some(live) = &mut self.live {
                    live.effects.toggle(d.private);
                }
            }
            Part::Primary if matches!(d.kind, Kind::CreateSpace | Kind::CreateChannel) => {
                let Some(live) = &mut self.live else { return };
                // Web validates on submit and says why.
                let name = d.create_name();
                let channel = d.kind == Kind::CreateChannel;
                let invalid = if channel {
                    caper_client::names::channel_name_error(&name)
                } else {
                    caper_client::names::space_name_error(&name)
                };
                match invalid {
                    Some(e) => d.error = Some(e.into()),
                    None => {
                        d.busy = true;
                        d.error = None;
                        live.create(name, channel.then_some(d.private));
                    }
                }
            }
            Part::Primary if d.kind == Kind::JoinInvite => {
                let Some(live) = &mut self.live else { return };
                d.error = None;
                match d.invite.as_ref().map(|p| p.code.clone()) {
                    Some(code) => {
                        d.busy = true;
                        d.creating = true;
                        d.request = live.join_invite(code);
                    }
                    None => match caper_client::invites::invite_code(&d.draft()) {
                        Some(code) => {
                            d.busy = true;
                            d.request = live.preview_invite(code);
                        }
                        None => d.error = Some(caper_client::invites::INVALID_LINK.into()),
                    },
                }
            }
            Part::Cancel | Part::Close => return self.close_dialog(),
            Part::Version(k) => {
                if let Some(v) = &mut d.history {
                    let revision = v.list.get(k).map(|v| v.revision);
                    v.selected = if v.selected == revision {
                        None
                    } else {
                        revision
                    };
                }
            }
            Part::Retry | Part::Older => {
                let older = d.history.as_ref().is_some_and(|v| !v.list.is_empty());
                return self.history_load(older);
            }
            Part::Field => {
                d.field.focused = true;
                self.events |= EV_FOCUS;
            }
            Part::Suggest(k) => {
                d.choose(k, self.live.as_ref());
                self.events |= EV_FOCUS;
            }
            Part::Channel(k) => return self.browse_open(k),
            Part::Primary if d.kind == Kind::StartDirect => {
                let Some(live) = &mut self.live else { return };
                d.busy = true;
                d.error = None;
                // A known person opens at once; anyone else is looked up first.
                match &d.recipient {
                    Some(p) => {
                        d.creating = true;
                        live.create_direct(p.username.clone());
                    }
                    None => d.request = live.lookup_direct(d.draft()),
                }
            }
            Part::Primary if d.kind == Kind::Block => {
                let (Some(live), Some((account, request))) = (&mut self.live, d.block.clone())
                else {
                    return;
                };
                d.busy = true;
                d.error = None;
                live.block(account, request);
            }
            Part::Primary if matches!(d.kind, Kind::LeaveSpace | Kind::LeaveChannel) => {
                let (Some(live), Some(leave)) = (&mut self.live, d.leave.clone()) else {
                    return;
                };
                d.busy = true;
                d.error = None;
                live.leave(&leave);
            }
            Part::Primary | Part::Reload => {
                let Some(live) = &mut self.live else { return };
                d.busy = true;
                d.error = None;
                d.request = match (d.kind, part, d.message.as_deref()) {
                    (Kind::Edit, Part::Reload, Some(m)) => live.reload_message(m),
                    (Kind::Edit, _, Some(m)) => live.edit(m, d.draft()),
                    (Kind::Delete, _, Some(m)) => live.delete(m),
                    _ => d.request,
                };
            }
            Part::Title
            | Part::Note
            | Part::Error
            | Part::Column(_)
            | Part::Choose
            | Part::Hint
            | Part::Loading
            | Part::Preview
            | Part::Label
            | Part::Guide => {}
        }
        self.dialog_changed();
        if let Err(e) = self.drain_live() {
            eprintln!("dialog action failed: {e:?}");
        }
    }

    /// The reply to an edit or reload: a save closes the dialog, a reload
    /// puts the latest text in it, a failure shows in it.
    pub fn edited(
        &mut self,
        request: u64,
        reloaded: bool,
        result: std::result::Result<Box<core::Message>, String>,
    ) {
        let Some(d) = self
            .dialog
            .as_mut()
            .filter(|d| d.kind == Kind::Edit && d.request == request)
        else {
            return;
        };
        d.busy = false;
        match result {
            Ok(m)
                if d.message
                    .as_ref()
                    .is_some_and(|o| o.author.id != m.author.id) =>
            {
                d.error = Some("Message author mismatch.".into())
            }
            Ok(m) if reloaded => {
                d.field.text = gfx::wide(&m.content.text);
                d.field.caret = d.field.text.len();
                d.field.anchor = d.field.caret;
                d.field.focused = true;
                d.message = Some(m);
            }
            Ok(_) => return self.close_dialog(),
            Err(e) => {
                d.error = Some(e);
                d.field.focused = true;
            }
        }
        self.dialog_changed();
    }

    /// Opens "New direct message" (egui's `open_start_direct`).
    pub fn open_start_direct(&mut self) {
        if let Some(live) = &self.live {
            live.load_people();
        }
        self.open_dialog(Dialog::start_direct());
    }

    /// The New DM lookup's answer: the person to open, or why not.
    pub fn direct_found(
        &mut self,
        request: u64,
        result: std::result::Result<core::Person, String>,
    ) {
        let Some(d) = self.dialog.as_mut().filter(|d| {
            d.kind == Kind::StartDirect && d.busy && !d.creating && d.request == request
        }) else {
            return;
        };
        d.busy = false;
        d.field.focused = true;
        match result {
            Ok(p) => d.recipient = Some(p),
            Err(e) => d.error = Some(e),
        }
        self.dialog_changed();
    }

    /// A DM you asked for opened (None) or couldn't: the New DM dialog
    /// closes or says why; without it (your own DM) the header says why.
    pub fn direct_created(&mut self, error: Option<String>) {
        match (
            self.dialog.as_mut().filter(|d| d.kind == Kind::StartDirect),
            error,
        ) {
            (Some(_), None) => self.close_dialog(),
            (Some(d), Some(e)) => {
                d.busy = false;
                d.creating = false;
                d.field.focused = true;
                d.error = Some(e);
                self.dialog_changed();
            }
            (None, Some(e)) => {
                self.status = Some((e, true));
                self.dirty.header = true;
                self.need = true;
            }
            (None, None) => {}
        }
    }

    /// The Block dialog's block finished: it closes, or says why not.
    pub fn blocked(&mut self, error: Option<String>) {
        let Some(d) = self
            .dialog
            .as_mut()
            .filter(|d| d.kind == Kind::Block && d.busy)
        else {
            return;
        };
        match error {
            None => return self.close_dialog(),
            Some(e) => {
                d.busy = false;
                d.error = Some(e);
            }
        }
        self.dialog_changed();
    }

    /// The Leave or invitation dialog's request finished: it closes (the
    /// app has moved on), or says why not.
    pub fn left(&mut self, error: Option<String>) {
        let Some(d) = self.dialog.as_mut().filter(|d| {
            matches!(
                d.kind,
                Kind::LeaveSpace
                    | Kind::LeaveChannel
                    | Kind::Invitation
                    | Kind::CreateSpace
                    | Kind::CreateChannel
            ) && d.busy
        }) else {
            return;
        };
        match error {
            None => return self.close_dialog(),
            Some(e) => {
                d.busy = false;
                d.declining = false;
                d.error = Some(e);
            }
        }
        self.dialog_changed();
    }

    pub fn deleted(&mut self, request: u64, error: Option<String>) {
        let Some(d) = self
            .dialog
            .as_mut()
            .filter(|d| d.kind == Kind::Delete && d.request == request)
        else {
            return;
        };
        match error {
            None => return self.close_dialog(),
            Some(e) => {
                d.busy = false;
                d.error = Some(if e.trim().is_empty() {
                    "Couldn\u{2019}t delete the message. Try again.".into()
                } else {
                    e
                });
            }
        }
        self.dialog_changed();
    }

    /// Input while the dialog or menu is open. None leaves the message to
    /// the normal handler (size, timers, IME).
    pub fn overlay_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        if self.dialog.is_some() {
            return self.dialog_input(msg, wp, x, y, ctrl, shift);
        }
        let done = Some(Some(LRESULT(0)));
        let menu = self.menu.as_mut()?;
        match msg {
            WM_MOUSEMOVE => {
                self.pointer = Some((x, y));
                let hover = menu.hit(x, y);
                let close_hover = contains(menu.close_rect(), x, y);
                if hover != menu.hover || close_hover != menu.close_hover {
                    menu.hover = hover;
                    menu.close_hover = close_hover;
                    self.dirty.overlay = true;
                    self.need = true;
                }
                None
            }
            WM_LBUTTONDOWN => {
                match menu.hit(x, y) {
                    Some(k) if menu.items[k].enabled() => self.menu_activate(k),
                    Some(_) => {}
                    None if contains(menu.close_rect(), x, y) => self.close_menu(),
                    None if contains(menu.rect(), x, y) => {}
                    // A click outside closes it, as egui's popups do.
                    None => self.close_menu(),
                }
                done
            }
            WM_KEYDOWN => {
                let n = menu.items.len();
                match VIRTUAL_KEY(wp.0 as u16) {
                    VK_ESCAPE => self.close_menu(),
                    VK_DOWN => menu.hover = Some(menu.hover.map_or(0, |k| (k + 1) % n)),
                    VK_UP => menu.hover = Some(menu.hover.map_or(n - 1, |k| (k + n - 1) % n)),
                    VK_RETURN | VK_SPACE => {
                        if let Some(k) = menu.hover {
                            self.menu_activate(k);
                        }
                    }
                    _ => return done,
                }
                self.dirty.overlay = true;
                self.need = true;
                done
            }
            WM_MOUSEWHEEL => {
                self.close_menu();
                None
            }
            _ => None,
        }
    }

    fn dialog_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let hwnd = self.hwnd;
        let d = self.dialog.as_mut()?;
        match msg {
            WM_MOUSEMOVE => {
                self.pointer = Some((x, y));
                if d.pressed == Some(Part::Field) {
                    let p = d.field_hit(x, y);
                    if p != d.field.caret {
                        d.field.move_to(p, true);
                        self.dialog_changed();
                    }
                } else {
                    let hover = d.hit(x, y);
                    if hover != d.hover {
                        d.hover = hover;
                        // Moving over a suggestion selects it, as web's
                        // `onMouseMove`, so Enter takes what it shows.
                        if let Some(Part::Suggest(k)) = hover {
                            d.selected = k;
                        }
                        self.dirty.overlay = true;
                        self.need = true;
                    }
                }
                done
            }
            WM_LBUTTONDOWN => {
                unsafe {
                    SetCapture(hwnd);
                }
                let part = d.hit(x, y);
                d.pressed = part;
                if part == Some(Part::Field) && !d.busy {
                    let p = d.field_hit(x, y);
                    d.field.move_to(p, shift);
                    d.field.focused = true;
                    self.events |= EV_FOCUS;
                    self.dialog_changed();
                } else if part.is_none() && !d.on_card(x, y) && !d.busy {
                    self.close_dialog();
                } else {
                    self.dirty.overlay = true;
                    self.need = true;
                }
                done
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                let pressed = d.pressed.take();
                let released = d.hit(x, y);
                self.dirty.overlay = true;
                self.need = true;
                if let Some(p) = pressed.filter(|p| Some(*p) == released && *p != Part::Field) {
                    self.dialog_activate(p);
                }
                done
            }
            WM_MOUSEWHEEL if matches!(d.kind, Kind::History | Kind::Browse) => {
                let notches = (wp.0 >> 16) as u16 as i16 as f32 / 120.0;
                d.scroll -= notches * 3.0 * LINE;
                self.dialog_changed();
                done
            }
            WM_LBUTTONDBLCLK | WM_MOUSEWHEEL => done,
            WM_SETCURSOR => {
                let cursor = match self.pointer.and_then(|(x, y)| d.hit(x, y)) {
                    Some(Part::Field) => IDC_IBEAM,
                    Some(p) if d.enabled(p) => IDC_HAND,
                    _ => IDC_ARROW,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                Some(Some(LRESULT(1)))
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                match vk {
                    // An open people list takes its keys first; Escape hides
                    // it, not the dialog.
                    VK_DOWN | VK_UP | VK_RETURN | VK_ESCAPE
                        if d.kind == Kind::StartDirect && !d.suggestions.is_empty() && !ctrl =>
                    {
                        let n = d.suggestions.len();
                        match vk {
                            VK_DOWN => d.selected = (d.selected + 1) % n,
                            VK_UP => d.selected = (d.selected + n - 1) % n,
                            VK_RETURN => {
                                let k = d.selected;
                                self.dialog_activate(Part::Suggest(k));
                                return done;
                            }
                            _ => d.hidden = true,
                        }
                        self.dialog_changed();
                    }
                    VK_ESCAPE if !d.busy => self.close_dialog(),
                    // The history's body scrolls from the keyboard too.
                    VK_UP | VK_DOWN | VK_PRIOR | VK_NEXT | VK_HOME | VK_END
                        if d.kind == Kind::History =>
                    {
                        let page = d.body.3 - 2.0 * LINE;
                        d.scroll += match vk {
                            VK_UP => -LINE,
                            VK_DOWN => LINE,
                            VK_PRIOR => -page,
                            VK_NEXT => page,
                            VK_HOME => -f32::MAX / 2.0,
                            _ => f32::MAX / 2.0,
                        };
                        self.dialog_changed();
                    }
                    // Ctrl+Enter saves, as in the egui client; Enter finds or
                    // opens the New DM's person.
                    VK_RETURN
                        if ctrl
                            || matches!(
                                d.kind,
                                Kind::StartDirect
                                    | Kind::JoinInvite
                                    | Kind::CreateSpace
                                    | Kind::CreateChannel
                            ) =>
                    {
                        self.dialog_activate(Part::Primary)
                    }
                    VK_RETURN if d.kind == Kind::Edit && d.field.focused && !d.busy => {
                        d.field.insert(&['\n' as u16]);
                        self.dialog_changed();
                    }
                    _ if ctrl && vk == VIRTUAL_KEY('C' as u16) => {
                        let (a, b) = d.field.selection();
                        if b > a {
                            crate::set_clipboard(hwnd, &d.field.text[a..b]);
                        }
                    }
                    _ if d.field.focused
                        && !d.busy
                        && d.field
                            .edit_key(hwnd, vk, ctrl, shift, d.kind == Kind::Edit) =>
                    {
                        self.dialog_changed();
                    }
                    _ => return Some(None),
                }
                done
            }
            WM_CHAR => {
                let c = wp.0 as u16;
                if matches!(
                    d.kind,
                    Kind::Edit
                        | Kind::StartDirect
                        | Kind::Browse
                        | Kind::JoinInvite
                        | Kind::CreateSpace
                        | Kind::CreateChannel
                ) && d.field.focused
                    && !d.busy
                    && !ctrl
                    && c >= 0x20
                    && c != 0x7f
                {
                    d.field.insert(&[c]);
                    self.dialog_changed();
                }
                done
            }
            _ => None,
        }
    }

    /// Redraws the menu and dialog surfaces when they changed.
    pub fn draw_overlays(&mut self) -> windows::core::Result<()> {
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        if !std::mem::take(&mut self.dirty.overlay) {
            return Ok(());
        }
        let g = &self.g;
        match &self.menu {
            Some(menu) => {
                let (w, h) = menu.size();
                self.menu_p.ensure(g, px(w), px(h))?;
                self.menu_p
                    .offset((menu.x * s).round(), (menu.y * s).round())?;
                self.menu_p.draw(s, |dc| menu.draw(g, dc))?;
            }
            None => self.menu_p.offset(-10_000.0, 0.0)?,
        }
        match &self.dialog {
            Some(d) => {
                self.dialog_p.ensure(g, px(self.w), px(self.h))?;
                self.dialog_p.offset(0.0, 0.0)?;
                let (w, h) = (self.w, self.h);
                self.dialog_p.draw(s, |dc| d.draw(g, dc, w, h))?;
            }
            None => self.dialog_p.offset(-100_000.0, 0.0)?,
        }
        Ok(())
    }
}
