//! Invitations and invite links, as in the egui client and web: the rail's
//! tiles for spaces you're invited to and its "Join with invite link"
//! button under them, the sidebar's "Invitations" to the open space's
//! private channels, and their dialogs. An invitation opens "You're
//! invited!" (or "Private channel invitation") with Decline and Accept
//! invitation; Join with invite link takes a pasted link, shows its space,
//! who invited you and what joining opens, then joins or opens it.
use crate::composer::Composer;
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::live::Invitation;
use crate::overlay::{Dialog, Kind, Rect};
use crate::theme::*;
use crate::{App, EV_SCREEN, text};
use caper_client::invites::{self, InvitePreview};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::{DWRITE_WORD_WRAPPING_NO_WRAP, IDWriteTextLayout};
use windows::core::Result;

/// The sidebar section's gap above, label and rows (the DM section's).
const GAP: f32 = 14.0;
const LABEL_H: f32 = 28.0;
const ROW_H: f32 = 28.0;
const PITCH: f32 = 30.0;

/// A membership control: an invited space's rail tile, the rail's
/// Create space and Join with invite link, the Channels heading's
/// Create channel (owners), or an open space's private channel
/// invitation in the sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Spot {
    Space(usize),
    Create,
    Join,
    NewChannel,
    Channel(usize),
}

impl Spot {
    /// A stable number per spot, for UI Automation's runtime ids.
    pub fn index(self) -> usize {
        match self {
            Self::Join => 0,
            Self::Create => 1,
            Self::NewChannel => 2,
            Self::Space(k) => 10 + 2 * k,
            Self::Channel(k) => 11 + 2 * k,
        }
    }

    /// Whether the spot is in the sidebar (else the rail).
    pub fn in_sidebar(self) -> bool {
        matches!(self, Self::NewChannel | Self::Channel(_))
    }
}

impl Dialog {
    pub fn join_invite() -> Self {
        let field = Composer {
            focused: true,
            caret_on: true,
            ..Default::default()
        };
        Self::new(Kind::JoinInvite, None, field)
    }

    pub fn invitation(invitation: Invitation) -> Self {
        let mut d = Self::new(Kind::Invitation, None, Composer::default());
        d.invitation = Some(Box::new(invitation));
        d
    }

    pub(crate) fn invite_title(&self) -> &'static str {
        match (self.kind, self.invitation.as_deref()) {
            (Kind::Invitation, Some(Invitation::Channel(_))) => "Private channel invitation",
            (Kind::Invitation, _) => "You\u{2019}re invited!",
            _ if self.invite.as_ref().is_some_and(|p| p.already_member) => "Open space",
            _ => "Join with invite link",
        }
    }

    pub(crate) fn invite_note(&self) -> String {
        match self.invitation.as_deref() {
            Some(Invitation::Space(s)) => s.inviter.as_ref().map_or_else(String::new, |i| {
                format!("{} (@{}) invited you.", i.display_name, i.username)
            }),
            Some(Invitation::Channel(c)) => format!(
                "{} (@{}) invited you to private channel #{}.",
                c.inviter.display_name, c.inviter.username, c.channel.name
            ),
            None if self.invite.is_none() => "Paste it below to join!".into(),
            None => String::new(),
        }
    }

    /// The invitation dialog's heading, or the New DM dialog's person.
    pub fn hint_text(&self) -> String {
        if self.kind == Kind::CreateChannel {
            return self.privacy_note();
        }
        match self.invitation.as_deref() {
            Some(Invitation::Space(s)) => format!("Join {}?", s.name),
            Some(Invitation::Channel(c)) => format!("Join #{}?", c.channel.name),
            None => self.recipient_label().unwrap_or_default(),
        }
    }

    pub(crate) fn invite_primary(&self) -> &'static str {
        if self.kind == Kind::Invitation {
            return if self.busy && !self.declining {
                "Accepting\u{2026}"
            } else {
                "Accept invitation"
            };
        }
        match (self.invite.as_ref().map(|p| p.already_member), self.busy) {
            (None, true) => "Checking\u{2026}",
            (None, false) => "Continue",
            (Some(true), true) => "Opening\u{2026}",
            (Some(true), false) => "Open space",
            (Some(false), true) => "Joining\u{2026}",
            (Some(false), false) => "Join space",
        }
    }

    pub fn cancel_label(&self) -> &'static str {
        match (self.kind, self.busy && self.declining) {
            (Kind::Invitation, true) => "Declining\u{2026}",
            (Kind::Invitation, false) => "Decline",
            (Kind::Update, _) => "Later",
            _ => "Cancel",
        }
    }

    /// The preview card's lines under the space's name, as egui lists them.
    fn preview_lines(p: &InvitePreview) -> Vec<String> {
        [
            Some(format!("Invited by {}", p.inviter.display_name)),
            p.already_member
                .then(|| "You\u{2019}re already a member".into()),
            Some(invites::plural(p.member_count, "member")),
            p.channel.as_ref().map(|c| format!("Opens in #{}", c.name)),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    /// The preview card's accessible name: its space, then its lines.
    pub fn preview_label(&self) -> String {
        self.invite.as_ref().map_or_else(String::new, |p| {
            std::iter::once(p.space.name.clone())
                .chain(Self::preview_lines(p))
                .collect::<Vec<_>>()
                .join(", ")
        })
    }

    fn preview_layouts(
        &self,
        g: &Gfx,
        w: f32,
    ) -> Result<Option<(IDWriteTextLayout, IDWriteTextLayout)>> {
        let Some(p) = &self.invite else {
            return Ok(None);
        };
        let w = (w - 28.0).max(1.0);
        let name = g.layout(&gfx::wide(&p.space.name), &g.formats.dialog_title, w)?;
        let lines = Self::preview_lines(p).join("\n");
        let lines = g.layout(&gfx::wide(&lines), &g.formats.dialog_note, w)?;
        Ok(Some((name, lines)))
    }

    pub(crate) fn preview_height(&self, g: &Gfx, w: f32) -> Result<f32> {
        Ok(match self.preview_layouts(g, w)? {
            Some((name, lines)) => {
                12.0 + gfx::metrics(&name).height + 6.0 + gfx::metrics(&lines).height + 12.0
            }
            None => 0.0,
        })
    }

    /// The raised card: the space's name over its muted lines.
    pub(crate) fn draw_preview(&self, g: &Gfx, dc: &ID2D1DeviceContext, r: Rect) -> Result<()> {
        let (x, y, w, h) = r;
        let Some((name, lines)) = self.preview_layouts(g, w)? else {
            return Ok(());
        };
        let card = rect(x, y, x + w, y + h);
        fill_round(dc, card, RADIUS, &g.brushes.raised);
        stroke_round(dc, card, RADIUS, &g.brushes.border);
        text::draw(dc, &name, x + 14.0, y + 12.0, &g.brushes.text);
        let ly = y + 12.0 + gfx::metrics(&name).height + 6.0;
        text::draw(dc, &lines, x + 14.0, ly, &g.brushes.muted);
        Ok(())
    }
}

/// The rail's invitation tiles, Create space (dimmed at the account's
/// limits) and Join with invite link, after its `first` spaces
/// (rail-local DIPs; the rail sits at the window's left).
pub fn draw_rail(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    names: &[String],
    first: usize,
    hover: Option<Spot>,
    create: bool,
) -> Result<()> {
    for (k, name) in names.iter().enumerate() {
        let (x, y, w, h) = App::space_rect(first + k);
        let r = rect(x, y, x + w, y + h);
        let lit = hover == Some(Spot::Space(k));
        fill_round(dc, r, 12.0, &g.brushes.composer);
        // An invited space's tile is outlined in terracotta, as on web.
        stroke_round(
            dc,
            r,
            12.0,
            if lit {
                &g.brushes.terracotta_bright
            } else {
                &g.brushes.terracotta
            },
        );
        let initial: String = name
            .trim()
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_default();
        let l = g.layout(&gfx::wide(&initial), &g.formats.rail, w)?;
        unsafe { l.SetMaxHeight(h)? };
        text::draw(dc, &l, r.left, r.top, &g.brushes.terracotta_bright);
    }
    let (x, y, w, h) = App::space_rect(first + names.len());
    let r = rect(x, y, x + w, y + h);
    let lit = create && hover == Some(Spot::Create);
    fill_round(
        dc,
        r,
        12.0,
        if lit {
            &g.brushes.raised
        } else {
            &g.brushes.composer
        },
    );
    stroke_round(dc, r, 12.0, &g.brushes.border);
    let c = (r.left + w / 2.0, r.top + h / 2.0);
    icons::draw(
        dc,
        Icon::Plus,
        rect(c.0 - 9.0, c.1 - 9.0, c.0 + 9.0, c.1 + 9.0),
        if lit { TEXT } else { TEXT_MUTED },
        if create { 1.0 } else { 0.4 },
    );
    let (x, y, w, h) = App::space_rect(first + names.len() + 1);
    let r = rect(x, y, x + w, y + h);
    let lit = hover == Some(Spot::Join);
    fill_round(
        dc,
        r,
        12.0,
        if lit {
            &g.brushes.raised
        } else {
            &g.brushes.composer
        },
    );
    stroke_round(dc, r, 12.0, &g.brushes.border);
    let c = (r.left + w / 2.0, r.top + h / 2.0);
    icons::draw(
        dc,
        Icon::Link,
        rect(c.0 - 9.0, c.1 - 9.0, c.0 + 9.0, c.1 + 9.0),
        if lit { TEXT } else { TEXT_MUTED },
        1.0,
    );
    Ok(())
}

/// The Channels heading's Create channel (sidebar-local DIPs), for an
/// owner: Some(enabled).
pub fn draw_channel_plus(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    plus: Option<bool>,
    hover: Option<Spot>,
) {
    let Some(enabled) = plus else { return };
    let b = rect(
        SIDEBAR_W - 40.0,
        SECTION_TOP - 2.0,
        SIDEBAR_W - 12.0,
        SECTION_TOP + 26.0,
    );
    let lit = enabled && hover == Some(Spot::NewChannel);
    if lit {
        fill_round(dc, b, 6.0, &g.brushes.hover);
    }
    icons::draw(
        dc,
        Icon::Plus,
        rect(b.left + 6.0, b.top + 6.0, b.right - 6.0, b.bottom - 6.0),
        if lit { TEXT } else { TEXT_MUTED },
        if enabled { 1.0 } else { 0.4 },
    );
}

/// The sidebar's "Invitations" (sidebar-local DIPs), its gap at `top`.
pub fn draw_sidebar(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    names: &[String],
    top: f32,
    hover: Option<Spot>,
) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    let label = g.layout(&gfx::wide("Invitations"), &g.formats.section, 10_000.0)?;
    text::draw(dc, &label, 20.0, top + GAP + 4.0, &g.brushes.muted);
    let invited = g.layout(&gfx::wide("Invited"), &g.formats.direct_note, 10_000.0)?;
    let iw = gfx::metrics(&invited).widthIncludingTrailingWhitespace;
    for (k, name) in names.iter().enumerate() {
        let y = top + GAP + LABEL_H + PITCH * k as f32;
        let r = rect(12.0, y, SIDEBAR_W - 12.0, y + ROW_H);
        if hover == Some(Spot::Channel(k)) {
            fill_round(dc, r, 6.0, &g.brushes.hover);
        }
        let cy = y + ROW_H / 2.0;
        icons::draw(
            dc,
            Icon::Lock,
            rect(24.0, cy - 7.0, 38.0, cy + 7.0),
            TEXT_MUTED,
            1.0,
        );
        let right = r.right - 8.0 - iw - 8.0;
        let l = g.layout(
            &gfx::wide(name),
            &g.formats.direct_name,
            (right - 46.0).max(1.0),
        )?;
        unsafe { l.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
        text::draw(
            dc,
            &l,
            46.0,
            cy - gfx::metrics(&l).height / 2.0,
            &g.brushes.text,
        );
        text::draw(
            dc,
            &invited,
            r.right - 8.0 - iw,
            cy - gfx::metrics(&invited).height / 2.0,
            &g.brushes.muted,
        );
    }
    Ok(())
}

impl App {
    /// The rail's invited spaces and the open space's private channels
    /// you're invited to (none without a live account).
    pub fn invite_names(&self) -> (Vec<String>, Vec<String>) {
        match self.live.as_ref().filter(|_| self.c.is_live()) {
            Some(l) => (
                l.invitations.iter().map(|s| s.name.clone()).collect(),
                l.channel_invitations()
                    .iter()
                    .map(|i| i.channel.name.clone())
                    .collect(),
            ),
            None => Default::default(),
        }
    }

    /// Every invitation control shown, in reading order.
    pub fn invite_spots(&self) -> Vec<Spot> {
        if !self.live.as_ref().is_some_and(|_| self.c.is_live()) {
            return Vec::new();
        }
        let (spaces, channels) = self.invite_names();
        let owner = self.live.as_ref().is_some_and(|l| l.owns_space());
        (0..spaces.len())
            .map(Spot::Space)
            .chain([Spot::Create, Spot::Join])
            .chain(owner.then_some(Spot::NewChannel))
            .chain((0..channels.len()).map(Spot::Channel))
            .collect()
    }

    /// The sidebar section's top, under the channels.
    pub fn invites_top(&self) -> f32 {
        CHANNEL_TOP + CHANNEL_H * self.c.channels.len() as f32
    }

    /// The sidebar section's height (none without channel invitations).
    pub fn invites_h(&self) -> f32 {
        match self.invite_names().1.len() {
            0 => 0.0,
            n => GAP + LABEL_H + PITCH * n as f32,
        }
    }

    /// A spot's rectangle in client DIPs.
    pub fn invite_rect(&self, spot: Spot) -> Rect {
        let spaces = self.c.spaces.len();
        match spot {
            Spot::Space(k) => Self::space_rect(spaces + k),
            Spot::Create => Self::space_rect(spaces + self.invite_names().0.len()),
            Spot::Join => Self::space_rect(spaces + self.invite_names().0.len() + 1),
            Spot::NewChannel => (RAIL_W + SIDEBAR_W - 40.0, SECTION_TOP - 2.0, 28.0, 28.0),
            Spot::Channel(k) => (
                RAIL_W + 12.0,
                self.invites_top() + GAP + LABEL_H + PITCH * k as f32,
                SIDEBAR_W - 24.0,
                ROW_H,
            ),
        }
    }

    /// Whether a spot can be used now (Create is off at a limit).
    pub fn invite_enabled(&self, spot: Spot) -> bool {
        match spot {
            Spot::Create | Spot::NewChannel => self
                .live
                .as_ref()
                .is_some_and(|l| l.can_create(spot == Spot::Create)),
            _ => true,
        }
    }

    pub fn invite_at(&self, x: f32, y: f32) -> Option<Spot> {
        self.invite_spots().into_iter().find(|&s| {
            let (l, t, w, h) = self.invite_rect(s);
            x >= l && x < l + w && y >= t && y < t + h
        })
    }

    /// A spot's accessible name, as web names its buttons.
    pub fn invite_name(&self, spot: Spot) -> String {
        let (spaces, channels) = self.invite_names();
        match spot {
            Spot::Space(k) => spaces
                .get(k)
                .map_or_else(String::new, |n| format!("Invitation to {n}")),
            Spot::Join => "Join with invite link".into(),
            // web's titles, with why it is unavailable at a limit.
            Spot::Create | Spot::NewChannel => self
                .live
                .as_ref()
                .map_or_else(String::new, |l| l.create_label(spot == Spot::Create)),
            Spot::Channel(k) => channels
                .get(k)
                .map_or_else(String::new, |n| format!("Invitation to #{n}, private")),
        }
    }

    /// Clicks and UIA Invoke: an invitation's dialog, or Join with invite
    /// link.
    pub fn invite_activate(&mut self, spot: Spot) {
        let Some(live) = &self.live else { return };
        let invitation = match spot {
            Spot::Join => return self.open_dialog(Dialog::join_invite()),
            Spot::Create if live.can_create(true) => {
                return self.open_dialog(Dialog::create_space());
            }
            Spot::NewChannel if live.can_create(false) => {
                let name = live.space_name();
                return self.open_dialog(Dialog::create_channel(name));
            }
            Spot::Create | Spot::NewChannel => return,
            Spot::Space(k) => live.invitations.get(k).cloned().map(Invitation::Space),
            Spot::Channel(k) => live
                .channel_invitations()
                .get(k)
                .cloned()
                .map(Invitation::Channel),
        };
        if let Some(invitation) = invitation {
            self.open_dialog(Dialog::invitation(invitation));
        }
    }

    /// The Join with invite link dialog's answer: the link's preview, a
    /// join (the space is opening, so it closes), or why not.
    pub fn invite_link(
        &mut self,
        request: u64,
        result: std::result::Result<Option<InvitePreview>, String>,
    ) {
        let Some(d) = self
            .dialog
            .as_mut()
            .filter(|d| d.kind == Kind::JoinInvite && d.busy && d.request == request)
        else {
            return;
        };
        d.busy = false;
        d.creating = false;
        match result {
            Ok(Some(preview)) => {
                d.invite = Some(Box::new(preview));
                d.field.focused = false;
            }
            Ok(None) => return self.close_dialog(),
            Err(e) => d.error = Some(e),
        }
        self.dialog_changed();
        self.events |= EV_SCREEN;
    }

    /// Invitations changed: the rail and sidebar redraw, and a dialog for
    /// one no longer offered closes (unless it is answering).
    pub fn invitations_changed(&mut self) {
        self.dirty.rail = true;
        self.dirty.sidebar = true;
        self.events |= EV_SCREEN;
        self.need = true;
        let gone = match (&self.dialog, &self.live) {
            (Some(d), Some(live)) => {
                d.kind == Kind::Invitation
                    && !d.busy
                    && d.invitation.as_deref().is_some_and(|i| !live.offered(i))
            }
            _ => false,
        };
        if gone {
            self.close_dialog();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(already_member: bool, channel: bool) -> InvitePreview {
        serde_json::from_value(serde_json::json!({
            "code": "AbC123xYz9",
            "space": { "name": "Studio" },
            "channel": channel.then(|| serde_json::json!({ "name": "feedback" })),
            "inviter": { "username": "maya", "displayName": "Maya" },
            "memberCount": 1,
            "expiresAt": null,
            "alreadyMember": already_member,
        }))
        .unwrap()
    }

    #[test]
    fn join_invite_says_what_the_link_does_as_egui_does() {
        let mut d = Dialog::join_invite();
        assert_eq!(d.invite_title(), "Join with invite link");
        assert_eq!(d.invite_note(), "Paste it below to join!");
        assert_eq!(d.invite_primary(), "Continue");
        d.busy = true;
        assert_eq!(d.invite_primary(), "Checking\u{2026}");
        d.busy = false;
        d.invite = Some(Box::new(preview(false, true)));
        assert_eq!(
            (
                d.invite_title(),
                d.invite_note().as_str(),
                d.invite_primary()
            ),
            ("Join with invite link", "", "Join space")
        );
        assert_eq!(
            d.preview_label(),
            "Studio, Invited by Maya, 1 member, Opens in #feedback"
        );
        // A member opens the space instead.
        d.invite = Some(Box::new(preview(true, false)));
        d.busy = true;
        assert_eq!(
            (d.invite_title(), d.invite_primary()),
            ("Open space", "Opening\u{2026}")
        );
        assert_eq!(
            d.preview_label(),
            "Studio, Invited by Maya, You\u{2019}re already a member, 1 member"
        );
    }
}
