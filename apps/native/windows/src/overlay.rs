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

pub(crate) fn layout(
    g: &Gfx,
    text: &str,
    format: &IDWriteTextFormat,
    width: f32,
) -> Result<IDWriteTextLayout> {
    let l = g.layout(&gfx::wide(text), format, width)?;
    crate::emoji::apply(&l, text, 14.0, 1.0)?;
    Ok(l)
}

pub(crate) fn height(l: &IDWriteTextLayout) -> f32 {
    gfx::metrics(l).height
}

pub(crate) fn width(l: &IDWriteTextLayout) -> f32 {
    gfx::metrics(l).widthIncludingTrailingWhitespace
}

// ---------------------------------------------------------------------------
// The message menu.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    /// A right-click while message text is selected, first as in egui.
    CopySelection,
    /// A right-click on a link: its own menu.
    OpenLink,
    CopyLink,
    /// A right-click on an image: its own menu, as in egui. Copy image
    /// keeps the menu open and shows "Copying…" until it lands.
    CopyImage,
    CopyingImage,
    CopyImageLink,
    OpenImage,
    Reply,
    Thread,
    Forward,
    Edit,
    History,
    Pin,
    Unpin,
    /// "Saving…" while a pin change is in flight; not clickable.
    PinSaving,
    ViewReactions,
    CopyText,
    CopyId,
    Delete,
}

impl Item {
    pub fn label(self) -> &'static str {
        match self {
            Self::CopySelection => "Copy selected text",
            Self::OpenLink => "Open link",
            Self::CopyLink => "Copy link",
            Self::CopyImage => "Copy image",
            Self::CopyingImage => "Copying\u{2026}",
            Self::CopyImageLink => "Copy image link",
            Self::OpenImage => "Open image in browser",
            Self::Reply => "Reply",
            Self::Thread => "Reply in thread",
            Self::Forward => "Forward",
            Self::Edit => "Edit message",
            Self::History => "Message history",
            Self::Pin => "Pin message",
            Self::Unpin => "Unpin message",
            Self::PinSaving => "Saving\u{2026}",
            Self::ViewReactions => "View reactions",
            Self::CopyText => "Copy text",
            Self::CopyId => "Copy message ID",
            Self::Delete => "Delete message",
        }
    }

    pub fn enabled(self) -> bool {
        !matches!(self, Self::PinSaving | Self::CopyingImage)
    }

    fn icon(self) -> Icon {
        match self {
            Self::CopySelection | Self::CopyLink | Self::CopyImage | Self::CopyingImage => {
                Icon::Copy
            }
            Self::CopyImageLink => Icon::Link,
            Self::OpenLink | Self::OpenImage => Icon::ExternalLink,
            Self::Reply => Icon::Reply,
            Self::Thread => Icon::Thread,
            Self::Forward => Icon::ExternalLink,
            Self::Edit => Icon::Pencil,
            Self::History => Icon::History,
            Self::Pin | Self::PinSaving => Icon::Pin,
            Self::Unpin => Icon::PinOff,
            Self::ViewReactions => Icon::Smile,
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
/// An image menu's copy status, a muted line under the items.
const STATUS_H: f32 = 28.0;

/// The actions menu of message `row`, at (x, y) in client DIPs.
pub struct Menu {
    pub row: usize,
    pub id: String,
    pub items: Vec<Item>,
    /// "Message actions", or "Link" for a link's menu.
    pub title: &'static str,
    /// Opened on a thread reply: `row` is the thread pane's.
    pub in_thread: bool,
    /// What Copy selected text and the link items act on.
    pub selection: Vec<u16>,
    pub link: String,
    /// The image an image menu acts on, its copy in flight, and the copy's
    /// status line under the items.
    pub image: Option<core::Attachment>,
    pub(crate) copying: Option<std::sync::mpsc::Receiver<crate::viewer::CopyResult>>,
    pub status: Option<String>,
    pub x: f32,
    pub y: f32,
    pub hover: Option<usize>,
    pub close_hover: bool,
}

impl Menu {
    pub fn size(&self) -> (f32, f32) {
        let status = if self.status.is_some() { STATUS_H } else { 0.0 };
        (
            MENU_W,
            2.0 * MENU_PAD + MENU_HEAD + ITEM_H * self.items.len() as f32 + status,
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
            title: "Message actions",
            in_thread: false,
            selection: Vec::new(),
            link: String::new(),
            image: None,
            copying: None,
            status: None,
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

    /// Moves it to a right-click at (x, y): its top-left at the pointer,
    /// flipped left or up where it would leave the window.
    pub fn place_at(&mut self, x: f32, y: f32, window_w: f32, window_h: f32) {
        let (w, h) = self.size();
        self.x = (if x + w > window_w - 4.0 { x - w } else { x }).max(4.0);
        self.y = (if y + h > window_h - 4.0 { y - h } else { y }).max(4.0);
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
            let title = g.layout(&gfx::wide(self.title), &g.formats.menu_title, 1e6)?;
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
                if self.close_hover {
                    crate::theme::text()
                } else {
                    crate::theme::text_muted()
                },
                1.0,
            );
            for (k, item) in self.items.iter().enumerate() {
                let (ix, iy, iw, ih) = self.item_rect(k);
                let (ix, iy) = (ix - self.x, iy - self.y);
                if self.hover == Some(k) && item.enabled() {
                    fill_round(dc, rect(ix, iy, ix + iw, iy + ih), 4.0, &g.brushes.border);
                }
                let (color, alpha) = match item {
                    _ if !item.enabled() => (crate::theme::text(), 0.5),
                    Item::Delete => (crate::theme::error(), 1.0),
                    _ => (crate::theme::text(), 1.0),
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
                let brush = g.solid(color)?;
                brush.SetOpacity(alpha);
                crate::text::draw(dc, &l, ix + 32.0, cy - height(&l) / 2.0, &brush);
                brush.SetOpacity(1.0);
            }
            if let Some(status) = &self.status {
                let top = MENU_PAD + MENU_HEAD + ITEM_H * self.items.len() as f32;
                let l = layout(
                    g,
                    status,
                    &g.formats.menu_item,
                    MENU_W - 2.0 * MENU_PAD - 16.0,
                )?;
                crate::text::draw(
                    dc,
                    &l,
                    MENU_PAD + 8.0,
                    top + (STATUS_H - height(&l)) / 2.0,
                    &g.brushes.muted,
                );
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
    /// "Invite people to {space}": its invite link and invitations.
    InvitePeople,
    /// The update review (`updates`): what's new and Restart to update.
    Update,
    /// Your private name and picture draft.
    Profile,
    Reactions,
    Forward,
    ForwardConversation,
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
    /// Invite people's section headings and lines (`invite_people`'s
    /// numbers), the read-only link with Copy and More invite options,
    /// that menu's items, and pending invitation `k` with its Cancel
    /// invite.
    Heading(u8),
    Line(u8),
    Link,
    Copy,
    More,
    LinkAction(u8),
    Invitee(usize),
    CancelInvite(usize),
    /// Custom links (`custom_links`): its disclosure button, the
    /// Expire after and Max uses fields, Expire after's units, Opens in
    /// and its choices, Create link, and live link `k` with its Copy and
    /// Deactivate.
    Custom,
    Number(u8),
    Unit(u8),
    Landing,
    LandingChoice(usize),
    CreateLink,
    LinkRow(usize),
    LinkCopy(usize),
    LinkRevoke(usize),
    /// Manage space and Channel settings (`space_settings`): Save name,
    /// member `k`, and the save bar's text, Reset and Save changes; and
    /// Browse's Manage on shown channel `k`.
    SaveName,
    Member(usize),
    Unsaved,
    Reset,
    SaveChanges,
    ManageRow(usize),
    /// Member `k`'s Remove.
    RemoveMember(usize),
    /// The danger zone's Delete space or Delete channel.
    DeleteOpen,
    /// The update review's "Version 2026.11.19.3", which opens its commit.
    Release,
    /// "See what changed on GitHub" under a review without highlights.
    Changes,
    Profile(crate::profile::editor::Control),
    ReactorTab(usize),
    ReactorPerson(usize),
    Forward(crate::forward::Control),
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
            Self::Heading(k) => 30_000 + k as usize,
            Self::Line(k) => 30_100 + k as usize,
            Self::Link => 30_200,
            Self::Copy => 30_201,
            Self::More => 30_202,
            Self::LinkAction(k) => 30_300 + k as usize,
            Self::Invitee(k) => 40_000 + k,
            Self::CancelInvite(k) => 50_000 + k,
            Self::Landing => 30_203,
            Self::CreateLink => 30_204,
            Self::Custom => 30_205,
            Self::Number(k) => 30_400 + k as usize,
            Self::Unit(k) => 30_500 + k as usize,
            Self::LandingChoice(k) => 60_000 + k,
            Self::LinkRow(k) => 70_000 + k,
            Self::LinkCopy(k) => 80_000 + k,
            Self::LinkRevoke(k) => 90_000 + k,
            Self::SaveName => 30_206,
            Self::Unsaved => 30_207,
            Self::Reset => 30_208,
            Self::SaveChanges => 30_209,
            Self::Member(k) => 100_000 + k,
            Self::ManageRow(k) => 110_000 + k,
            Self::RemoveMember(k) => 120_000 + k,
            Self::DeleteOpen => 30_210,
            Self::Release => 30_211,
            Self::Changes => 30_212,
            Self::Profile(control) => 130_000 + control.index(),
            Self::ReactorTab(k) => 140_000 + k,
            Self::ReactorPerson(k) => 150_000 + k,
            // Forward controls have their own multi-component UIA identities.
            Self::Forward(_) => 160_000,
        }
    }

    /// Whether the part scrolls with the history's or Invite people's
    /// body (all but its head).
    pub(crate) fn in_body(self) -> bool {
        !matches!(
            self,
            Self::Title
                | Self::Note
                | Self::Close
                | Self::Unsaved
                | Self::Reset
                | Self::SaveChanges
        )
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
    /// The update review's version link, then its changes link: how far
    /// each hover has come (0 to 1, before easing), toward which end, and
    /// when it last stepped.
    pub release_glow: [std::cell::Cell<(f32, bool, std::time::Instant)>; 2],
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
    /// Invite people's space, links and invitations.
    pub inviting: Option<Box<crate::invite_people::InvitePeople>>,
    /// The update reviewed, whether restarting would end a call, and the
    /// body's lines as laid out (cut to the window).
    pub update: Option<Box<caper_client::updates::Available>>,
    pub in_call: bool,
    pub update_shown: Vec<(crate::updates::Line, String)>,
    pub profile_edit: Option<Box<crate::profile::editor::Editor>>,
    pub reactors: Option<Box<crate::reactors::View>>,
    pub forwarding: Option<Box<crate::forward::Forward>>,
    /// A confirmation's keyboard focus, Cancel or its action: Cancel
    /// first, as web's dialog autofocuses it.
    pub button: Option<Part>,
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
            release_glow: std::array::from_fn(|_| {
                std::cell::Cell::new((0.0, false, std::time::Instant::now()))
            }),
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
            inviting: None,
            update: None,
            in_call: false,
            update_shown: Vec::new(),
            profile_edit: None,
            reactors: None,
            forwarding: None,
            button: matches!(
                kind,
                Kind::Delete | Kind::Block | Kind::LeaveSpace | Kind::LeaveChannel
            )
            .then_some(Part::Cancel),
            parts: Vec::new(),
            card: (0.0, 0.0, 0.0, 0.0),
        }
    }

    pub fn draft(&self) -> String {
        String::from_utf16_lossy(&self.field.text)
    }

    pub fn title(&self) -> String {
        match self.kind {
            Kind::Profile => "Edit profile".into(),
            Kind::Reactions => "Reactions".into(),
            Kind::Forward => "Forward message".into(),
            Kind::ForwardConversation => "Forwarded conversation".into(),
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
            Kind::JoinInvite | Kind::Invitation => self.invite_title(),
            Kind::CreateSpace => "Create a space".into(),
            Kind::CreateChannel => "Create a channel".into(),
            Kind::InvitePeople => self.invite_people_title(),
            Kind::Update => Self::update_title().into(),
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
            Kind::CreateSpace | Kind::CreateChannel | Kind::Profile | Kind::Reactions | Kind::Forward | Kind::ForwardConversation => String::new(),
            Kind::InvitePeople => self.invite_people_note(),
            Kind::Update => self.update_note(),
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
            if let Some(ip) = self.inviting.as_deref_mut() {
                ip.status = None;
            }
            self.selected = 0;
            self.hidden = false;
            self.resolved = name;
        }
        // Ranked as `@` suggestions are, while the field is focused and
        // nothing is in flight.
        self.suggestions = match live {
            Some(live) if self.field.focused && !self.hidden && !self.busy => {
                // Invite people's: known people outside the space, then
                // pending invitees (egui's and web's `invitees`).
                let people: Vec<_> = match self.inviting.as_deref() {
                    Some(ip) => match ip.channel() {
                        // A private channel's: the space's members without
                        // access, once its own have loaded (web's).
                        Some(c) if c.loaded => caper_client::mentions::grantees(
                            &c.space_members,
                            Some(&c.owner),
                            &ip.members,
                            &ip.invitations,
                        ),
                        Some(_) => Vec::new(),
                        None => caper_client::mentions::invitees(
                            live.people(),
                            &ip.members,
                            &ip.invitations,
                        ),
                    },
                    None => live
                        .people()
                        .iter()
                        .map(caper_client::mentions::Person::from)
                        .collect(),
                };
                let invited = self
                    .inviting
                    .as_deref()
                    .map_or(&[][..], |ip| &ip.invitations);
                let granted = self
                    .inviting
                    .as_deref()
                    .and_then(|ip| ip.channel())
                    .map_or(&[][..], |c| &c.space_members);
                caper_client::mentions::field_suggestions(&people, &self.resolved)
                    .into_iter()
                    .filter_map(|p| {
                        live.person_named(&p.username).cloned().or_else(|| {
                            invited
                                .iter()
                                .chain(granted)
                                .find(|m| m.username == p.username)
                                .map(|m| core::Person {
                                    admin: m.admin,
                                    id: m.id.clone(),
                                    username: m.username.clone(),
                                    display_name: m.display_name.clone(),
                                    avatar_id: m.avatar_id,
                                    avatar_asset: m.avatar_asset.clone(),
                                    avatar_url: m.avatar_url.clone(),
                                })
                        })
                    })
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
            (Kind::History | Kind::Reactions, _) => "Close",
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
            (Kind::InvitePeople, _) => self.invite_people_primary(),
            (Kind::Update, _) => self.update_primary(),
            (Kind::Profile, _) => "Save profile",
            (Kind::Forward, _) => "Forward",
            (Kind::ForwardConversation, _) => "Close",
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
        if self.forwarding.is_some() {
            return self.forward_enabled(part);
        }
        if self.kind == Kind::Profile {
            return self.profile_enabled(part);
        }
        match part {
            Part::Primary => {
                !self.busy
                    && match self.kind {
                        Kind::Edit => caper_client::edits::valid_text(&self.draft()),
                        Kind::Delete | Kind::Block | Kind::LeaveSpace | Kind::LeaveChannel => true,
                        Kind::History | Kind::Browse | Kind::Reactions => false,
                        Kind::StartDirect => self.draft().len() >= 3,
                        // Only a link the shared parser accepts can continue.
                        Kind::JoinInvite => {
                            self.invite.is_some()
                                || caper_client::invites::invite_code(&self.draft()).is_some()
                        }
                        Kind::Invitation => true,
                        Kind::CreateSpace | Kind::CreateChannel => !self.create_name().is_empty(),
                        Kind::InvitePeople => self.invite_enabled(part),
                        Kind::Update => true,
                        Kind::Profile => self.profile_enabled(part),
                        Kind::Forward | Kind::ForwardConversation => self.forward_enabled(part),
                    }
            }
            Part::Copy
            | Part::More
            | Part::Custom
            | Part::Number(_)
            | Part::Unit(_)
            | Part::Landing
            | Part::LandingChoice(_)
            | Part::CreateLink
            | Part::LinkCopy(_)
            | Part::LinkRevoke(_) => self.invite_enabled(part),
            Part::Retry if self.kind == Kind::InvitePeople => self.invite_enabled(part),
            Part::Retry if self.kind == Kind::Reactions => self.part_rect(part).is_some(),
            Part::ReactorTab(_) | Part::ReactorPerson(_) | Part::Forward(_) => true,
            Part::CancelInvite(_) => !self.busy,
            Part::SaveName => self.can_save_name(),
            Part::Reset | Part::SaveChanges | Part::RemoveMember(_) | Part::DeleteOpen => {
                !self.busy
            }
            Part::Member(_) | Part::Unsaved | Part::ManageRow(_) | Part::Profile(_) => true,
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
            | Part::Guide
            | Part::Heading(_)
            | Part::Line(_)
            | Part::Link
            | Part::LinkAction(_)
            | Part::Invitee(_)
            | Part::LinkRow(_) => true,
            Part::Release => self.update_commit().is_some(),
            Part::Changes => self.update_changes().is_some(),
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
        if self.forwarding.is_some() {
            return self.forward_hit(x, y);
        }
        if self.kind == Kind::Profile {
            return self.profile_hit(x, y);
        }
        self.parts
            .iter()
            .find(|(p, r)| {
                matches!(
                    p,
                    Part::Field
                        | Part::Reload
                        | Part::Cancel
                        | Part::Primary
                        | Part::Release
                        | Part::Changes
                        | Part::Close
                        | Part::Version(_)
                        | Part::Retry
                        | Part::Older
                        | Part::Suggest(_)
                        | Part::Channel(_)
                        | Part::Toggle
                        | Part::Copy
                        | Part::More
                        | Part::LinkAction(_)
                        | Part::CancelInvite(_)
                        | Part::Custom
                        | Part::Number(_)
                        | Part::Unit(_)
                        | Part::Landing
                        | Part::LandingChoice(_)
                        | Part::CreateLink
                        | Part::LinkCopy(_)
                        | Part::LinkRevoke(_)
                        | Part::SaveName
                        | Part::Reset
                        | Part::SaveChanges
                        | Part::ManageRow(_)
                        | Part::RemoveMember(_)
                        | Part::DeleteOpen
                        | Part::ReactorTab(_)
                ) && contains(*r, x, y)
                    && match self.kind {
                        Kind::History | Kind::InvitePeople | Kind::Reactions => {
                            !p.in_body() || contains(self.body, x, y)
                        }
                        Kind::Browse => {
                            !matches!(p, Part::Channel(_) | Part::ManageRow(_))
                                || contains(self.body, x, y)
                        }
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
        if let Some(f) = &self.forwarding {
            return self.forward_origin(f.focus);
        }
        if let Some(e) = &self.profile_edit {
            return self.profile_field_origin(e.focus);
        }
        let (ox, oy) = self.field_origin;
        (ox, oy - self.field.scroll())
    }

    /// Lays out the card for a `w` × `h` window: the field's text and every
    /// part's rectangle.
    pub fn relayout(&mut self, g: &Gfx, w: f32, h: f32) -> Result<()> {
        if self.forwarding.is_some() {
            return self.relayout_forward(g, w, h);
        }
        if self.kind == Kind::Profile {
            return self.relayout_profile(g, w, h);
        }
        if self.kind == Kind::Reactions {
            return self.relayout_reactors(g, w, h);
        }
        if self.kind == Kind::History {
            return self.relayout_history(g, w, h);
        }
        if self.kind == Kind::Browse {
            return self.relayout_browse(g, w, h);
        }
        if self.kind == Kind::InvitePeople {
            return self.relayout_invite(g, w, h);
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
            if self.update_commit().is_some() {
                let l = layout(g, &self.update_version(), note_format, 1e6)?;
                parts.push((
                    Part::Release,
                    (0.0, y, width(&l) + crate::updates::RELEASE_ICON, height(&l)),
                ));
            }
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
        if self.kind == Kind::Update {
            // The notes give up room so the actions stay on screen, as
            // egui's scrolling notes do; the footer line always shows.
            let footer = self.update_footer();
            let reserve = 20.0 + BUTTON_H + if footer.is_some() { 32.0 } else { 0.0 };
            let budget = (h - 48.0 - 2.0 * CARD_PAD - reserve).max(y + 64.0);
            self.update_shown.clear();
            let lines = self.update_lines();
            let total = lines.len();
            for (k, (line, s)) in lines.into_iter().enumerate() {
                let gap = match line {
                    crate::updates::Line::Failed | crate::updates::Line::Heading => 14.0,
                    crate::updates::Line::Title if k > 0 => 12.0,
                    _ => 4.0,
                };
                let l = layout(g, &s, crate::updates::line_format(g, line), inner)?;
                if y + gap + height(&l) > budget && k + 1 < total {
                    let more = "More changes are in this update.".to_owned();
                    let l = layout(g, &more, &g.formats.dialog_note, inner)?;
                    y += 10.0;
                    let n = self.update_shown.len() as u8;
                    parts.push((Part::Line(n), (0.0, y, inner, height(&l))));
                    self.update_shown.push((crate::updates::Line::Body, more));
                    y += height(&l);
                    break;
                }
                y += gap;
                let n = self.update_shown.len() as u8;
                parts.push((Part::Line(n), (0.0, y, inner, height(&l))));
                self.update_shown.push((line, s));
                y += height(&l);
            }
            if self.update_changes().is_some() {
                let label = caper_client::updates::CHANGES_LABEL;
                let l = layout(g, label, &g.formats.dialog_note, 1e6)?;
                y += 8.0;
                parts.push((
                    Part::Changes,
                    (0.0, y, width(&l) + crate::updates::RELEASE_ICON, height(&l)),
                ));
                y += height(&l);
            }
            if let Some(s) = footer {
                let l = layout(g, &s, &g.formats.dialog_note, inner)?;
                y += 16.0;
                let n = self.update_shown.len() as u8;
                parts.push((Part::Line(n), (0.0, y, inner, height(&l))));
                self.update_shown.push((crate::updates::Line::Body, s));
                y += height(&l);
            }
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
        // The review lists its error among its lines.
        if let Some(e) = self.error.as_ref().filter(|_| self.kind != Kind::Update) {
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

    /// Draws the card over a `w` × `h` window; its scrim is a separate
    /// stationary surface so the entrance never uncovers a window edge. Call after
    /// [`Dialog::relayout`].
    pub fn draw(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        w: f32,
        h: f32,
        media: Option<&mut crate::files::Ctx>,
    ) -> Result<()> {
        unsafe {
            dc.Clear(None);
            let _ = (w, h);
            let (cx, cy, cw, ch) = self.card;
            let card = rect(cx, cy, cx + cw, cy + ch);
            fill_round(dc, card, RADIUS, &g.brushes.surface);
            stroke_round(dc, card, RADIUS, &g.brushes.border);
            if self.forwarding.is_some() {
                return self.draw_forward(g, dc, media);
            }
            if self.kind == Kind::Profile {
                return self.draw_profile(g, dc);
            }
            if self.kind == Kind::Reactions {
                return self.draw_reactors(g, dc);
            }
            if self.kind == Kind::History {
                return self.draw_history(g, dc);
            }
            if self.kind == Kind::Browse {
                return self.draw_browse(g, dc);
            }
            if self.kind == Kind::InvitePeople {
                return self.draw_invite(g, dc);
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
                        let note = self.note();
                        // The review's version is a link (`Part::Release`, drawn
                        // on its own); the rest of the line follows it.
                        let lead = match self.kind {
                            Kind::Update if self.update_commit().is_some() => {
                                let link = layout(g, &self.update_version(), format, 1e6)?;
                                Some((
                                    self.update_version().len(),
                                    width(&link) + crate::updates::RELEASE_ICON,
                                ))
                            }
                            _ => None,
                        };
                        let (skip, dx) = lead.unwrap_or((0, 0.0));
                        let l = layout(g, &note[skip..], format, pw - dx)?;
                        crate::text::draw(dc, &l, x + dx, y, &g.brushes.muted);
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
                        self.draw_field(g, dc, (x, y, pw, ph))?;
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
                    Part::Line(k) if self.kind == Kind::Update => {
                        if let Some((line, s)) = self.update_shown.get(*k as usize) {
                            let l = layout(g, s, crate::updates::line_format(g, *line), pw)?;
                            crate::text::draw(dc, &l, x, y, crate::updates::line_brush(g, *line));
                        }
                    }
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
                    Part::Release | Part::Changes => {
                        self.draw_release(g, dc, (x, y, pw, ph), *part)?
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// The one-line or edit field's box, its text and caret, or its
    /// placeholder while empty.
    pub(crate) fn draw_field(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        (x, y, pw, ph): Rect,
    ) -> Result<()> {
        let body = rect(x, y, x + pw, y + ph);
        let stroke = if self.field.focused {
            g.solid(crate::theme::terracotta())?
        } else if self.hover == Some(Part::Field) {
            g.solid(crate::theme::terracotta_bright())?
        } else {
            g.brushes.border.clone()
        };
        fill_round(dc, body, 7.0, &g.brushes.composer);
        stroke_round(dc, body, 7.0, &stroke);
        unsafe {
            dc.PushAxisAlignedClip(
                &rect(x + 2.0, y + 2.0, x + pw - 2.0, y + ph - 2.0),
                D2D1_ANTIALIAS_MODE_ALIASED,
            );
        }
        let (ox, oy) = self.text_origin();
        let hint = match self.kind {
            Kind::StartDirect => Some("Username"),
            Kind::CreateSpace => Some("Gaming nights"),
            Kind::CreateChannel => Some("project-updates"),
            Kind::InvitePeople => Some("Exact username"),
            // web's and egui's example under an empty field.
            Kind::JoinInvite => Some("it looks like \u{2192} caper.chat/invite/AbCdE12345"),
            _ => None,
        };
        let hint = hint
            .map(|h| layout(g, h, &g.formats.composer, 1e6))
            .transpose()?;
        self.field
            .draw_text(g, dc, ox, oy, hint.as_ref().map(|l| (l, &g.brushes.muted)));
        unsafe { dc.PopAxisAlignedClip() };
        Ok(())
    }

    /// Suggested person `k`: highlighted or hovered, a 24-DIP avatar, the
    /// name over a muted @username.
    pub(crate) fn draw_suggestion(
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
            fill_round(dc, r, 6.0, &g.solid(crate::theme::accent_surface())?);
        } else if self.hover == Some(Part::Suggest(k)) {
            fill_round(dc, r, 6.0, &g.brushes.hover);
        }
        let face = crate::live::person(&p.id, &p.display_name, crate::avatars::Face::avatar(p));
        let (cx, cy) = (x + 8.0 + 12.0, y + h / 2.0);
        crate::avatars::circle(
            g,
            dc,
            &face.avatar,
            (cx, cy, 12.0),
            face.color,
            &face.initials,
        )?;
        let tx = x + 8.0 + 24.0 + 12.0;
        let tw = (w - (tx - x) - 8.0).max(1.0);
        let name = layout(g, &p.display_name, &g.formats.direct_name, tw)?;
        crate::text::draw(dc, &name, tx, y + 5.0, &g.brushes.text);
        let user = layout(g, &self.suggestion_note(p), &g.formats.direct_note, tw)?;
        crate::text::draw(dc, &user, tx, y + 24.0, &g.brushes.muted);
        Ok(())
    }

    /// Under a suggestion's name: its @username, and whether it is
    /// already invited (Invite people), as egui says.
    fn suggestion_note(&self, p: &core::Person) -> String {
        if self.already_invited(&p.username) {
            format!("@{} \u{b7} Already invited", p.username)
        } else {
            format!("@{}", p.username)
        }
    }

    /// Suggestion `k`'s accessible name.
    pub fn suggestion_name(&self, k: usize) -> String {
        self.suggestions.get(k).map_or_else(String::new, |p| {
            format!("{} {}", p.display_name, self.suggestion_note(p))
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
        draw_button(g, dc, label, r, tone, state)?;
        if self.button == Some(part) {
            let (x, y, w, h) = r;
            stroke_round(
                dc,
                rect(x - 3.0, y - 3.0, x + w + 3.0, y + h + 3.0),
                RADIUS + 3.0,
                &g.brushes.terracotta,
            );
        }
        Ok(())
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
    let radius = match tone {
        Tone::Primary => (h / 2.0).min(16.0),
        _ => 9.0,
    };
    let body = rect(x, y, x + w, y + h);
    fill_round(dc, body, radius, &g.solid(fill)?);
    // Tactile top specular rim highlight for non-disabled buttons.
    if state != 0 && state != 3 {
        let rim = rect(x + 1.0, y + 1.0, x + w - 1.0, y + h - 1.0);
        let rim_alpha = match tone {
            Tone::Primary => {
                if crate::theme::is_light() {
                    0.16
                } else {
                    0.36
                }
            }
            Tone::Danger => 0.24,
            _ => {
                if crate::theme::is_light() {
                    0.75
                } else {
                    0.12
                }
            }
        };
        let rim_brush =
            unsafe { dc.CreateSolidColorBrush(&gfx::color(0xFFFFFF, rim_alpha), None)? };
        stroke_round(dc, rim, (radius - 1.0).max(2.0), &rim_brush);
    }
    stroke_round(dc, body, radius, &g.solid(border)?);
    let l = layout(g, label, &g.formats.button, w)?;
    crate::text::draw(
        dc,
        &l,
        x + (w - width(&l)) / 2.0,
        y + (h - height(&l)) / 2.0,
        &g.solid(text)?,
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
    /// `.danger-outline`: the danger zone's opener, on the card's surface.
    DangerOutline,
}

impl Tone {
    /// `state`: 0 disabled, 1 rest, 2 hover, 3 pressed.
    fn colors(self, state: u8) -> (u32, u32, u32) {
        let p = crate::theme::palette();
        if state == 0 {
            // Disabled steps back to neutral chrome.
            return (p.surface_raised, p.border, p.text_muted);
        }
        let light = crate::theme::is_light();
        match (self, state) {
            (Self::Primary, 1) => (p.action, p.action, p.on_action),
            (Self::Primary, 2) => (p.action_hover, p.action_hover, p.on_action),
            (Self::Primary, _) => (p.action_press, p.action_press, p.on_action),
            (Self::Secondary, 1) => (p.surface_raised, p.border, p.text),
            (Self::Secondary, 2) => (p.border, p.text_muted, p.text),
            (Self::Secondary, _) => (p.accent_surface, p.terracotta_bright, p.text),
            (Self::Danger, 1) => (0xB93646, 0xB93646, 0xFFFFFF),
            (Self::Danger, 2) => (0xD44355, 0xD44355, 0xFFFFFF),
            (Self::Danger, _) => (0xA02E3C, 0xFF8C9A, 0xFFFFFF),
            // `--danger-wash` over the card on hover.
            (Self::DangerOutline, 1) => {
                (if light { p.surface } else { 0x151719 }, 0xB93646, p.error)
            }
            (Self::DangerOutline, 2) => {
                (if light { 0xFCEBED } else { 0x2F1C20 }, 0xD44355, p.error)
            }
            (Self::DangerOutline, _) => {
                (if light { 0xF9D7DC } else { 0x3C1E24 }, 0xFF8C9A, p.error)
            }
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
        let Some(row) = self.c.messages.get(i) else {
            return;
        };
        let Some((id, items)) = self.message_items(&row.id, row.pinned_by.is_some(), false) else {
            return;
        };
        let (tx, ty, tw, th) = self.bar_rect_for(i);
        self.close_picker();
        self.menu = Some(Menu::new(i, id, items, tx + tw, ty, ty + th, self.h));
        self.menu_opened();
    }

    /// Message `id`'s actions in egui's order. The thread pane offers Reply
    /// in its own composer, but never starts a nested thread.
    fn message_items(
        &self,
        id: &str,
        pinned: bool,
        in_thread: bool,
    ) -> Option<(String, Vec<Item>)> {
        let live = self.live.as_ref()?;
        let m = live.message(id)?;
        let mut items = Vec::new();
        if live.can_react() && !m.is_deleted() {
            items.push(Item::Reply);
        }
        if !in_thread && (!m.is_deleted() || m.thread.is_some() || m.thread_root_id.is_some()) {
            items.push(Item::Thread);
        }
        if live.forward_session().is_some() && crate::forward::forwardable(&m) {
            items.push(Item::Forward);
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
            } else if pinned {
                Item::Unpin
            } else {
                Item::Pin
            });
        }
        if live
            .reactor_message(id)
            .is_some_and(|m| m.reactions.iter().any(|r| !r.author_ids.is_empty()))
        {
            items.push(Item::ViewReactions);
        }
        if !m.is_deleted() && !m.content.text.is_empty() {
            items.push(Item::CopyText);
        }
        items.push(Item::CopyId);
        if live.can_delete(&m) {
            items.push(Item::Delete);
        }
        Some((m.id, items))
    }

    /// A right-click at (x, y) on thread reply `i`: a link's menu on a
    /// link, otherwise the reply's actions at the pointer.
    pub fn open_thread_context_menu(&mut self, i: usize, x: f32, y: f32) {
        self.open_thread_menu(i, x, y, true);
    }

    /// The toolbar and keyboard always open message actions, even when
    /// their anchor happens to cover a link in another row.
    pub fn open_thread_menu(&mut self, i: usize, x: f32, y: f32, links: bool) {
        let Some(row) = self.thread.as_ref().and_then(|t| t.c.messages.get(i)) else {
            return;
        };
        let (id, pinned) = (row.id.clone(), row.pinned_by.is_some());
        let mut menu = if links && let Some((_, link)) = self.thread_text_link_at(x, y) {
            let mut menu = Menu::new(i, id, vec![Item::OpenLink, Item::CopyLink], x, y, y, self.h);
            menu.title = "Link";
            menu.link = link;
            menu
        } else {
            let Some((id, items)) = self.message_items(&id, pinned, true) else {
                return;
            };
            Menu::new(i, id, items, x, y, y, self.h)
        };
        let selected = self.thread_selected_text();
        if menu.title == "Message actions" && !selected.is_empty() {
            menu.items.insert(0, Item::CopySelection);
            menu.selection = selected;
        }
        menu.in_thread = true;
        menu.place_at(x, y, self.w, self.h);
        self.close_picker();
        self.menu = Some(menu);
        self.menu_opened();
    }

    /// A right-click at (x, y) on row `i`, as in egui: a link opens its own
    /// menu (Open link, Copy link); anywhere else the message's actions
    /// open at the pointer, with Copy selected text first while text is
    /// selected.
    pub fn open_context_menu(&mut self, i: usize, x: f32, y: f32) {
        let image = self.file_at(x, y).and_then(|(i, k, _)| {
            let m = self.c.messages.get(i)?;
            let a = m.attachments.get(k)?;
            (a.kind == core::AttachmentKind::Image
                && crate::files::clickable(a)
                && !crate::files::hidden(a, &m.revealed))
            .then(|| a.clone())
        });
        if let Some(image) = image {
            return self.open_image_menu(image, x, y);
        } else if let Some(link) = self.link_at(x, y) {
            let Some(id) = self.c.messages.get(i).map(|m| m.id.clone()) else {
                return;
            };
            self.close_picker();
            let mut menu = Menu::new(i, id, vec![Item::OpenLink, Item::CopyLink], x, y, y, self.h);
            menu.title = "Link";
            menu.link = link;
            self.menu = Some(menu);
        } else {
            self.open_menu(i);
        }
        let selection = self.selected_text();
        let (w, h) = (self.w, self.h);
        let Some(menu) = &mut self.menu else { return };
        if menu.title == "Message actions" && !selection.is_empty() {
            menu.items.insert(0, Item::CopySelection);
            menu.selection = selection;
        }
        menu.place_at(x, y, w, h);
        self.menu_opened();
    }

    /// History attachments and profile pictures share these actions and the
    /// copy worker. A picture need not belong to any loaded message.
    pub fn open_image_menu(&mut self, image: core::Attachment, x: f32, y: f32) {
        self.close_picker();
        self.suggest_dismiss();
        let items = vec![Item::CopyImage, Item::CopyImageLink, Item::OpenImage];
        let mut menu = Menu::new(0, String::new(), items, x, y, y, self.h);
        menu.title = "Image";
        menu.image = Some(image);
        menu.place_at(x, y, self.w, self.h);
        self.menu = Some(menu);
        self.menu_opened();
    }

    /// An image menu's finished copy: onto the clipboard, then its status.
    pub fn poll_menu_copy(&mut self) {
        let Some(menu) = &mut self.menu else { return };
        let Some(Ok(result)) = menu.copying.as_ref().map(|rx| rx.try_recv()) else {
            return;
        };
        menu.copying = None;
        menu.status = Some(crate::viewer::put_copied(self.hwnd, result));
        for item in &mut menu.items {
            if *item == Item::CopyingImage {
                *item = Item::CopyImage;
            }
        }
        self.overlay_changed();
    }

    pub fn close_menu(&mut self) {
        if self.menu.take().is_some() {
            self.overlay_changed();
            self.events |= EV_FOCUS;
        }
    }

    /// A menu opened: it takes focus (`uia::focused`).
    fn menu_opened(&mut self) {
        self.overlay_changed();
        self.events |= EV_FOCUS;
    }

    /// Clicks, Enter and UIA Invoke on a menu item.
    pub fn menu_activate(&mut self, k: usize) {
        // Copy image keeps the menu open to show its progress, as in egui.
        if let Some(menu) = &self.menu
            && menu.items.get(k) == Some(&Item::CopyImage)
            && let Some(image) = menu.image.clone()
        {
            let started = self.start_image_copy(&image);
            let Some(menu) = &mut self.menu else { return };
            match started {
                Ok(rx) => {
                    menu.copying = Some(rx);
                    menu.items[k] = Item::CopyingImage;
                    menu.status = None;
                }
                Err(e) => menu.status = Some(e),
            }
            return self.overlay_changed();
        }
        let Some(menu) = self.menu.take() else { return };
        self.overlay_changed();
        self.events |= EV_FOCUS;
        let (Some(item), Some(live)) = (menu.items.get(k).copied(), &mut self.live) else {
            return;
        };
        if matches!(item, Item::CopyImageLink | Item::OpenImage) {
            if let Some(image) = &menu.image {
                if item == Item::CopyImageLink {
                    if let Some(url) = live.media.resolve(image).url {
                        crate::set_clipboard(self.hwnd, &gfx::wide(&url));
                    }
                } else {
                    let unix = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_secs() as i64);
                    // An expiring URL opens once refreshed (`Update::Open`).
                    if let Some(url) = live.media.open(image, unix) {
                        crate::open_link(&url);
                    }
                }
            }
            return;
        }
        let Some(m) = live.message(&menu.id) else {
            return;
        };
        match item {
            Item::Reply if menu.in_thread => return self.select_thread_reply(&m.id),
            Item::Reply => return self.select_reply(menu.row),
            Item::Thread => return self.reply_in_thread(&m.id),
            Item::Forward => return self.open_forward(&m.id, false),
            Item::Edit if live.can_edit(&m) => self.open_dialog(Dialog::edit(m)),
            Item::Delete if live.can_delete(&m) => self.open_dialog(Dialog::delete(m)),
            Item::History => return self.open_history(m),
            Item::Pin => live.pin(&m, true),
            Item::Unpin => live.pin(&m, false),
            Item::ViewReactions => return self.open_reactors(&m.id),
            Item::CopyText => crate::set_clipboard(self.hwnd, &gfx::wide(&m.content.text)),
            Item::CopyId => crate::set_clipboard(self.hwnd, &gfx::wide(&m.id)),
            Item::CopySelection => crate::set_clipboard(self.hwnd, &menu.selection),
            Item::OpenLink => crate::open_link(&menu.link),
            Item::CopyLink => crate::set_clipboard(self.hwnd, &gfx::wide(&menu.link)),
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
                | Kind::InvitePeople
                | Kind::Profile
                | Kind::Forward
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
        if let Some(d) = self.dialog.take() {
            // Closing the review is Later; the dock stays.
            if d.kind == Kind::Update {
                self.updates.dismiss();
            }
            unsafe {
                let _ = KillTimer(Some(self.hwnd), crate::TIMER_CARET);
            }
            self.overlay_changed();
            self.events |= EV_SCREEN | EV_FOCUS;
        }
    }

    /// The reader dismissed the dialog (Close, Cancel, Escape, a click
    /// outside) or what it showed went away: the keyboard goes back to the
    /// composer rather than to nothing.
    pub fn dismiss_dialog(&mut self) {
        self.close_dialog();
        self.set_composer_focus(true);
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
            if d.kind == Kind::Profile {
                d.profile_changed();
            }
            if matches!(d.kind, Kind::StartDirect | Kind::InvitePeople) {
                d.username_changed(self.live.as_ref());
            }
            if d.kind == Kind::InvitePeople {
                d.invite_changed();
                d.settings_name_changed();
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
            if let Some(f) = &mut d.forwarding {
                f.note.caret_on = true;
            }
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
        if d.forwarding.is_some() {
            return self.forward_dialog_activate(part);
        }
        if d.kind == Kind::Profile {
            return self.profile_edit_activate(part);
        }
        if d.kind == Kind::Reactions {
            return self.reactors_activate(part);
        }
        if d.kind == Kind::Update {
            return self.update_activate(part);
        }
        if d.kind == Kind::InvitePeople
            && !matches!(
                part,
                Part::Field | Part::Suggest(_) | Part::Close | Part::Toggle
            )
        {
            return self.invite_people_activate(part);
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
            Part::Cancel | Part::Close => return self.dismiss_dialog(),
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
            Part::ManageRow(k) => return self.browse_manage(k),
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
            | Part::Guide
            | Part::Heading(_)
            | Part::Line(_)
            | Part::Link
            | Part::Copy
            | Part::More
            | Part::LinkAction(_)
            | Part::Invitee(_)
            | Part::CancelInvite(_)
            | Part::LinkRow(_)
            | Part::Custom
            | Part::Number(_)
            | Part::Unit(_)
            | Part::Landing
            | Part::LandingChoice(_)
            | Part::CreateLink
            | Part::LinkCopy(_)
            | Part::LinkRevoke(_)
            | Part::SaveName
            | Part::Member(_)
            | Part::Unsaved
            | Part::Reset
            | Part::SaveChanges
            | Part::RemoveMember(_)
            | Part::DeleteOpen
            | Part::Release
            | Part::Changes
            | Part::ReactorTab(_)
            | Part::ReactorPerson(_)
            | Part::Forward(_)
            | Part::Profile(_) => {}
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
        // Invite people's invite or Cancel invite failed: it says why,
        // in web's words.
        if let (Some(d), Some(e)) = (
            self.dialog
                .as_mut()
                .filter(|d| d.kind == Kind::InvitePeople),
            &error,
        ) {
            // A save, or a channel's members: why shows by the name.
            if d.manage_failed(e) {
                return self.dialog_changed();
            }
            if d.busy {
                d.busy = false;
                d.error = Some(caper_client::invites::admin_error(e.clone()));
                return self.dialog_changed();
            }
        }
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
                self.events |= EV_FOCUS;
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
        if self.dialog.as_ref().is_some_and(|d| d.forwarding.is_some()) {
            return self.forward_dialog_input(msg, wp, x, y, ctrl, shift);
        }
        if self
            .dialog
            .as_ref()
            .is_some_and(|d| d.kind == Kind::Profile)
        {
            return self.profile_edit_input(msg, wp, x, y, ctrl, shift);
        }
        let done = Some(Some(LRESULT(0)));
        let hwnd = self.hwnd;
        let d = self.dialog.as_mut()?;
        if d.kind == Kind::Reactions && msg == WM_KEYDOWN {
            self.reactors_key(VIRTUAL_KEY(wp.0 as u16), shift);
            return done;
        }
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
                // A click off More invite options closes it, and one off
                // the card closes a confirmation, before anything else.
                if d.kind == Kind::InvitePeople && d.invite_click_off(part, x, y) {
                    d.invite_back();
                    self.dialog_changed();
                    return done;
                }
                d.pressed = part;
                if part == Some(Part::Field) && !d.busy {
                    let p = d.field_hit(x, y);
                    d.field.move_to(p, shift);
                    d.field.focused = true;
                    self.events |= EV_FOCUS;
                    self.dialog_changed();
                } else if part.is_none() && !d.on_card(x, y) && !d.busy && !d.unsaved() {
                    self.dismiss_dialog();
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
            WM_MOUSEWHEEL
                if matches!(
                    d.kind,
                    Kind::History | Kind::Browse | Kind::InvitePeople | Kind::Reactions
                ) =>
            {
                let notches = (wp.0 >> 16) as u16 as i16 as f32 / 120.0;
                d.scroll -= notches * 3.0 * LINE;
                self.dialog_changed();
                done
            }
            WM_LBUTTONDBLCLK | WM_MOUSEWHEEL => done,
            WM_SETCURSOR => {
                let cursor = match self.pointer.and_then(|(x, y)| d.hit(x, y)) {
                    Some(Part::Field | Part::Number(_)) => IDC_IBEAM,
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
                        if matches!(d.kind, Kind::StartDirect | Kind::InvitePeople)
                            && !d.suggestions.is_empty()
                            && !ctrl =>
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
                    VK_ESCAPE if d.kind == Kind::InvitePeople && d.invite_layered() => {
                        d.invite_back();
                        self.dialog_changed();
                    }
                    VK_ESCAPE if !d.busy => self.dismiss_dialog(),
                    // A confirmation's Tab and arrows move between Cancel and
                    // its action; Enter and Space press the focused one.
                    VK_TAB | VK_LEFT | VK_RIGHT if d.button.is_some() => {
                        d.button = Some(if d.button == Some(Part::Cancel) {
                            Part::Primary
                        } else {
                            Part::Cancel
                        });
                        self.events |= crate::EV_FOCUS;
                        self.dialog_changed();
                    }
                    VK_RETURN | VK_SPACE if d.button.is_some() => {
                        if let Some(part) = d.button.filter(|p| d.enabled(*p)) {
                            self.dialog_activate(part);
                        }
                    }
                    // Enter invites the typed username.
                    VK_RETURN if d.kind == Kind::InvitePeople => {
                        // In Expire after or Max uses it creates the link,
                        // and in the name it saves, as web's forms submit.
                        let part = match d.number_focus() {
                            Some(crate::custom_links::NAME) if d.settings_channel().is_some() => {
                                Some(Part::SaveChanges)
                            }
                            Some(crate::custom_links::NAME) => Some(Part::SaveName),
                            Some(_) => Some(Part::CreateLink),
                            None => d.field.focused.then_some(Part::Primary),
                        };
                        if let Some(part) = part {
                            self.dialog_activate(part);
                        }
                    }
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
                    _ if d.kind == Kind::InvitePeople && d.number_key(hwnd, vk, ctrl, shift) => {
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
                // Expire after and Max uses take their own characters.
                if d.kind == Kind::InvitePeople
                    && !ctrl
                    && c >= 0x20
                    && c != 0x7f
                    && d.number_char(c)
                {
                    self.dialog_changed();
                    return done;
                }
                if matches!(
                    d.kind,
                    Kind::Edit
                        | Kind::StartDirect
                        | Kind::Browse
                        | Kind::JoinInvite
                        | Kind::CreateSpace
                        | Kind::CreateChannel
                        | Kind::InvitePeople
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
        if self.viewer.is_some() {
            self.dialog_card_p.hide()?;
            self.dirty.overlay = false;
            return self.draw_viewer();
        }
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
        if let Some(e) = &self.attachment_editor {
            self.dialog_card_p.hide()?;
            self.dialog_p.ensure(g, px(self.w), px(self.h))?;
            self.dialog_p.offset(0.0, 0.0)?;
            self.dialog_p.draw(s, |dc| e.draw(g, dc, self.w))?;
            return Ok(());
        }
        match &self.dialog {
            Some(d) => {
                self.dialog_p.ensure(g, px(self.w), px(self.h))?;
                self.dialog_p.offset(0.0, 0.0)?;
                self.dialog_p.draw(s, |dc| {
                    unsafe {
                        let (scrim, scrim_alpha) = crate::theme::scrim();
                        dc.Clear(Some(&gfx::color(scrim, scrim_alpha)));
                    }
                    Ok(())
                })?;
                self.dialog_card_p.ensure(g, px(self.w), px(self.h))?;
                self.dialog_card_p.offset(0.0, 0.0)?;
                let (w, h) = (self.w, self.h);
                // The forward chooser's original draws its pictures.
                let mut tex = crate::files::D2d(g.dc.clone());
                let mut media = self
                    .live
                    .as_mut()
                    .filter(|_| d.forwarding.is_some())
                    .map(|l| crate::files::Ctx::new(&mut l.media, &mut tex));
                self.dialog_card_p
                    .draw(s, |dc| d.draw(g, dc, w, h, media.as_mut()))?;
                // Each frame of the version link's hover fade.
                unsafe {
                    if d.release_moving() {
                        SetTimer(Some(self.hwnd), crate::updates::TIMER_RELEASE, 16, None);
                    } else {
                        let _ = KillTimer(Some(self.hwnd), crate::updates::TIMER_RELEASE);
                    }
                }
                if media.is_some_and(|m| m.media.busy()) {
                    unsafe {
                        SetTimer(Some(self.hwnd), crate::TIMER_LIVE, 100, None);
                    }
                }
            }
            None => {
                self.dialog_p.hide()?;
                self.dialog_card_p.hide()?;
            }
        }
        Ok(())
    }
}
