//! Invite people, as in the egui client and web's invite-only Manage
//! space: the space's invite link, prepared as the dialog opens, with Copy
//! and More invite options (Regenerate link and Deactivate link, each
//! confirmed first); the space's pending invitations with Cancel invite;
//! and "Or invite by username" with the New DM field's people suggestions.
use crate::composer::{Composer, LINE};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::live::Live;
use crate::overlay::{
    CARD_PAD, Dialog, FIELD_PAD_X, Kind, Part, Rect, Tone, height, layout, width,
};
use crate::theme::*;
use crate::{App, text};
use caper_client::invites::{self, INVITATION_SENT, InviteLink};
use caper_client::model::Member;
use caper_client::worker::{AccountOperation, AdminOperation};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::{DWRITE_WORD_WRAPPING_NO_WRAP, IDWriteTextFormat};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows::core::Result;

/// "Copied" reads "Copy" again after two seconds, as on web.
pub const TIMER_COPIED: usize = 17;
const COPIED_MS: u32 = 2_000;

const WIDTH: f32 = 480.0;
const CLOSE: f32 = 32.0;
const CONTROL_H: f32 = 38.0;
const BUTTON_H: f32 = 36.0;
const ROW_H: f32 = 48.0;
const SUGGEST_H: f32 = 44.0;
const MENU_W: f32 = 300.0;

/// `Part::Heading`s.
pub const SHARE: u8 = 0;
pub const PENDING: u8 = 1;
pub const BY_NAME: u8 = 2;
/// `Part::Line`s: the link's guidance, the note without public channels,
/// the link's summary, the note without a link, why a link request
/// failed, the username rules, "Invitation sent…", and the confirmation's
/// lines.
pub const GUIDE: u8 = 0;
pub const NO_CHANNEL: u8 = 1;
pub const SUMMARY: u8 = 2;
pub const MISSING: u8 = 3;
pub const LINK_ERROR: u8 = 4;
pub const HELP: u8 = 5;
pub const STATUS: u8 = 6;
pub const STOPS: u8 = 7;
pub const REPLACEMENT: u8 = 8;
pub const CURRENT: u8 = 9;
/// `Part::LinkAction`s, More invite options' items.
pub const REGENERATE: u8 = 0;
pub const DEACTIVATE: u8 = 1;

const USERNAME_RULE: &str = "Use 3\u{2013}32 lowercase letters, numbers, or underscores.";
const REGENERATE_NOTE: &str =
    "Replaces this link with 30 days and 100 joins. The old link stops working.";

/// The open dialog's space, its links and its invitations.
pub struct InvitePeople {
    pub space: String,
    pub space_name: String,
    /// The public channel whose menu opened it: the link opens there.
    pub channel: Option<String>,
    /// The space's public channels (id, name), where links can open.
    pub public: Vec<(String, String)>,
    /// The shareable link before its code (`https://caper.chat/invite/`).
    prefix: String,
    pub default: Option<InviteLink>,
    pub links: Vec<InviteLink>,
    /// A link request is in flight (preparing, regenerating, deactivating).
    pub pending: bool,
    regenerating: bool,
    pub link_error: Option<String>,
    /// The code whose Copy reads "Copied".
    pub copied: Option<String>,
    /// More invite options is open.
    pub menu: bool,
    /// The code to regenerate (true) or deactivate, awaiting confirmation.
    pub confirm: Option<(String, bool)>,
    pub members: Vec<Member>,
    pub invitations: Vec<Member>,
    pub status: Option<&'static str>,
}

impl InvitePeople {
    pub fn new(
        space: String,
        space_name: String,
        channel: Option<String>,
        public: Vec<(String, String)>,
        prefix: String,
        members: Vec<Member>,
    ) -> Self {
        Self {
            space,
            space_name,
            channel,
            public,
            prefix,
            default: None,
            links: Vec::new(),
            pending: false,
            regenerating: false,
            link_error: None,
            copied: None,
            menu: false,
            confirm: None,
            members,
            invitations: Vec::new(),
            status: None,
        }
    }

    pub fn url(&self, code: &str) -> String {
        format!("{}{code}", self.prefix)
    }
}

/// What the worker answered for the dialog.
pub enum Answer {
    Default {
        space: String,
        request: u64,
        result: std::result::Result<InviteLink, String>,
    },
    Links {
        space: String,
        request: u64,
        result: std::result::Result<Vec<InviteLink>, String>,
    },
    Revoked {
        space: String,
        request: u64,
        code: String,
        result: std::result::Result<(), String>,
    },
    Invitations(Vec<Member>),
    Invited(Member),
    Cancelled(String),
}

impl Dialog {
    pub fn invite_people(inviting: InvitePeople) -> Self {
        let field = Composer {
            focused: true,
            caret_on: true,
            ..Default::default()
        };
        let mut d = Self::new(Kind::InvitePeople, None, field);
        d.inviting = Some(Box::new(inviting));
        d
    }

    fn confirming(&self) -> bool {
        self.inviting
            .as_ref()
            .is_some_and(|ip| ip.confirm.is_some())
    }

    pub(crate) fn invite_people_title(&self) -> String {
        match self.inviting.as_deref() {
            Some(InvitePeople {
                confirm: Some((_, true)),
                ..
            }) => "Regenerate invite link?".into(),
            Some(InvitePeople {
                confirm: Some(_), ..
            }) => "Deactivate invite link?".into(),
            Some(ip) => format!("Invite people to {}", ip.space_name),
            None => String::new(),
        }
    }

    pub(crate) fn invite_people_note(&self) -> String {
        if self.confirming() {
            String::new()
        } else {
            "Share a link or invite someone by username.".into()
        }
    }

    pub(crate) fn invite_people_primary(&self) -> &'static str {
        match self.inviting.as_ref().and_then(|ip| ip.confirm.as_ref()) {
            Some((_, true)) => "Regenerate link",
            Some(_) => "Deactivate link",
            None => "Invite",
        }
    }

    /// Whether the primary button (or Copy and More) can be used now.
    pub(crate) fn invite_enabled(&self, part: Part) -> bool {
        let Some(ip) = self.inviting.as_deref() else {
            return false;
        };
        match part {
            Part::Primary if ip.confirm.is_some() => !ip.pending,
            Part::Copy | Part::More => !ip.pending && ip.default.is_some(),
            Part::Retry => !ip.pending,
            _ => !self.busy,
        }
    }

    pub fn invite_heading(&self, k: u8) -> String {
        match k {
            SHARE => "Share an invite link".into(),
            PENDING => "Pending invitations".into(),
            _ => "Or invite by username".into(),
        }
    }

    pub fn invite_line(&self, k: u8) -> String {
        let Some(ip) = self.inviting.as_deref() else {
            return String::new();
        };
        match k {
            GUIDE => "Anyone with this link can join this space. New links allow 100 joins for 30 days.".into(),
            NO_CHANNEL => "People will join the space without opening a channel.".into(),
            SUMMARY => ip.default.as_ref().map_or_else(String::new, |l| {
                invites::link_meta(l, chrono::Utc::now())
            }),
            MISSING if ip.link_error.is_some() => "Couldn\u{2019}t prepare the invite link.".into(),
            MISSING => "No active invite link.".into(),
            LINK_ERROR => ip.link_error.clone().unwrap_or_default(),
            HELP => "3\u{2013}32 lowercase letters, numbers, or underscores. Invitations expire after 7 days.".into(),
            STATUS => ip.status.unwrap_or_default().into(),
            STOPS => "This shared URL will stop accepting new joins.".into(),
            REPLACEMENT => "The replacement allows 100 joins for 30 days.".into(),
            _ => ip.confirm.as_ref().map_or_else(String::new, |(code, _)| {
                format!("Current link: {}", ip.url(code))
            }),
        }
    }

    /// The read-only link box's accessible name.
    pub fn invite_link_name(&self) -> String {
        let url = self
            .inviting
            .as_deref()
            .and_then(|ip| ip.default.as_ref().map(|l| ip.url(&l.code)));
        format!("Invite link: {}", url.unwrap_or_default())
    }

    pub fn copy_label(&self) -> &'static str {
        match self.inviting.as_deref() {
            Some(ip)
                if ip.copied.is_some()
                    && ip.copied == ip.default.as_ref().map(|l| l.code.clone()) =>
            {
                "Copied"
            }
            _ => "Copy",
        }
    }

    pub fn retry_label(&self) -> &'static str {
        if self
            .inviting
            .as_ref()
            .is_some_and(|ip| ip.link_error.is_some())
        {
            "Try again"
        } else {
            "Create another link"
        }
    }

    pub fn link_action_label(k: u8) -> &'static str {
        if k == REGENERATE {
            "Regenerate link"
        } else {
            "Deactivate link"
        }
    }

    /// Pending invitation `k`'s row, as egui and web word it.
    pub fn invitee_name(&self, k: usize) -> String {
        self.inviting
            .as_deref()
            .and_then(|ip| ip.invitations.get(k))
            .map_or_else(String::new, |m| {
                format!("{} @{} \u{b7} Invited", m.display_name, m.username)
            })
    }

    /// Whether a suggested username already has a pending invitation.
    pub(crate) fn already_invited(&self, username: &str) -> bool {
        self.inviting
            .as_deref()
            .is_some_and(|ip| ip.invitations.iter().any(|m| m.username == username))
    }

    /// More invite options or a confirmation is showing (Escape and an
    /// outside click close it first).
    pub(crate) fn invite_layered(&self) -> bool {
        self.inviting
            .as_ref()
            .is_some_and(|ip| ip.menu || ip.confirm.is_some())
    }

    pub(crate) fn invite_back(&mut self) {
        if let Some(ip) = self.inviting.as_deref_mut() {
            if ip.menu {
                ip.menu = false;
            } else if ip.confirm.take().is_some() {
                self.field.focused = true;
            }
        }
    }

    /// Asks for the space's invite link (web's `prepareDefault`): the
    /// default one for its channel, made if there is none.
    pub(crate) fn prepare_link(&mut self, live: &Live) {
        let Some(ip) = self.inviting.as_deref_mut().filter(|ip| !ip.pending) else {
            return;
        };
        self.request += 1;
        ip.pending = true;
        ip.link_error = None;
        live.account(AccountOperation::EnsureDefaultInviteLink {
            space: ip.space.clone(),
            channel: ip.channel.clone(),
            request: self.request,
        });
    }

    /// Web's checks before inviting: the username to invite, or None with
    /// why not in `error`.
    pub(crate) fn invite_submit(&mut self) -> Option<String> {
        let ip = self.inviting.as_deref_mut()?;
        let name = String::from_utf16_lossy(&self.field.text);
        ip.status = None;
        self.error = if name.len() < 3 {
            Some(USERNAME_RULE.into())
        } else {
            invites::invite_error(&name, &ip.members, &ip.invitations, false).map(Into::into)
        };
        self.error.is_none().then_some(name)
    }

    /// Applies the worker's answer; returns the space whose links reload.
    pub(crate) fn invite_answer(&mut self, answer: Answer) -> Option<String> {
        let request = self.request;
        let ip = self.inviting.as_deref_mut()?;
        match answer {
            Answer::Default {
                space,
                request: r,
                result,
            } if space == ip.space && r == request => {
                ip.pending = false;
                match result {
                    Ok(link) => {
                        // A regenerated link replaces the old one.
                        if let Some(previous) = ip.default.replace(link.clone())
                            && ip.regenerating
                        {
                            ip.links.retain(|l| l.code != previous.code);
                        }
                        ip.links.retain(|l| l.code != link.code);
                        ip.links.insert(0, link);
                        ip.link_error = None;
                        ip.copied = None;
                    }
                    Err(e) => ip.link_error = Some(e),
                }
                ip.regenerating = false;
                return Some(space);
            }
            Answer::Links {
                space,
                request: r,
                result,
            } if space == ip.space && r == request => match result {
                Ok(links) => {
                    if ip
                        .default
                        .as_ref()
                        .is_some_and(|d| !links.iter().any(|l| l.code == d.code))
                    {
                        ip.default = None;
                    }
                    ip.links = links;
                }
                Err(e) => ip.link_error = Some(e),
            },
            Answer::Revoked {
                space,
                request: r,
                code,
                result,
            } if space == ip.space && r == request => {
                ip.pending = false;
                match result {
                    Ok(()) => {
                        ip.links.retain(|l| l.code != code);
                        if ip.default.as_ref().is_some_and(|l| l.code == code) {
                            ip.default = None;
                        }
                    }
                    // It may have expired or gone elsewhere: reload.
                    Err(e) => {
                        ip.link_error = Some(e);
                        return Some(space);
                    }
                }
            }
            Answer::Invitations(members) => ip.invitations = members,
            Answer::Invited(member) => {
                ip.invitations.retain(|m| m.id != member.id);
                ip.invitations.push(member);
                ip.status = Some(INVITATION_SENT);
                // Ready for the next person.
                self.busy = false;
                self.error = None;
                self.field.text.clear();
                self.field.caret = 0;
                self.field.anchor = 0;
                self.resolved.clear();
            }
            Answer::Cancelled(id) => {
                ip.invitations.retain(|m| m.id != id);
                self.busy = false;
            }
            _ => {}
        }
        None
    }

    fn line_style<'g>(
        &self,
        g: &'g Gfx,
        k: u8,
    ) -> (&'g IDWriteTextFormat, &'g ID2D1SolidColorBrush) {
        match k {
            LINK_ERROR => (&g.formats.error, &g.brushes.error),
            STOPS | REPLACEMENT => (&g.formats.dialog_note, &g.brushes.text),
            _ => (&g.formats.direct_note, &g.brushes.muted),
        }
    }

    /// The heading, then the link section, the pending invitations and
    /// the username field, in a body that scrolls once it fills the room;
    /// or, while confirming, the confirmation alone.
    pub(crate) fn relayout_invite(&mut self, g: &Gfx, w: f32, h: f32) -> Result<()> {
        let Some(ip) = self.inviting.as_deref() else {
            return Ok(());
        };
        let card_w = WIDTH.min(w - 48.0).max(260.0);
        let inner = card_w - 2.0 * CARD_PAD;
        let mut parts = Vec::new();
        let mut menu = Vec::new();
        let line = |parts: &mut Vec<(Part, Rect)>, part: Part, text: &str, format, y: f32| {
            let h = height(&layout(g, text, format, inner)?);
            parts.push((part, (0.0, y, inner, h)));
            Ok::<f32, windows::core::Error>(h)
        };
        if let Some((_, regenerate)) = &ip.confirm {
            let title = layout(g, &self.title(), &g.formats.dialog_title, inner)?;
            parts.push((Part::Title, (0.0, 0.0, inner, height(&title))));
            let mut y = height(&title) + 12.0;
            for k in [STOPS, REPLACEMENT, CURRENT] {
                if k == REPLACEMENT && !regenerate {
                    continue;
                }
                let (format, _) = self.line_style(g, k);
                y += line(&mut parts, Part::Line(k), &self.invite_line(k), format, y)? + 8.0;
            }
            y += 12.0;
            let primary = layout(g, self.primary_label(), &g.formats.button, 1e6)?;
            let pw = width(&primary) + 24.0;
            let cancel = layout(g, "Cancel", &g.formats.button, 1e6)?;
            let cw = width(&cancel) + 24.0;
            parts.push((Part::Cancel, (inner - pw - 8.0 - cw, y, cw, BUTTON_H)));
            parts.push((Part::Primary, (inner - pw, y, pw, BUTTON_H)));
            let card_h = y + BUTTON_H + 2.0 * CARD_PAD;
            return self.place(parts, card_w, card_h, w, h, (0.0, card_h));
        }
        parts.push((Part::Close, (inner - CLOSE, 0.0, CLOSE, CLOSE)));
        let title = layout(
            g,
            &self.title(),
            &g.formats.dialog_title,
            inner - CLOSE - 8.0,
        )?;
        parts.push((Part::Title, (0.0, 0.0, inner - CLOSE - 8.0, height(&title))));
        let mut y = height(&title).max(CLOSE) + 2.0;
        y += line(
            &mut parts,
            Part::Note,
            &self.note(),
            &g.formats.dialog_note,
            y,
        )? + 16.0;
        let body_top = y;
        y += line(
            &mut parts,
            Part::Heading(SHARE),
            &self.invite_heading(SHARE),
            &g.formats.direct_name,
            y,
        )? + 6.0;
        y += line(
            &mut parts,
            Part::Line(GUIDE),
            &self.invite_line(GUIDE),
            &g.formats.direct_note,
            y,
        )? + 4.0;
        if ip.public.is_empty() {
            let text = self.invite_line(NO_CHANNEL);
            y += line(
                &mut parts,
                Part::Line(NO_CHANNEL),
                &text,
                &g.formats.direct_note,
                y,
            )? + 4.0;
        }
        y += 8.0;
        if ip.default.is_some() {
            let copied = layout(g, "Copied", &g.formats.button, 1e6)?;
            let copy_w = width(&copied) + 28.0;
            let link_w = (inner - copy_w - 1.0 - CONTROL_H - 8.0).max(40.0);
            parts.push((Part::Link, (0.0, y, link_w, CONTROL_H)));
            parts.push((Part::Copy, (link_w + 8.0, y, copy_w, CONTROL_H)));
            let more_x = link_w + 8.0 + copy_w + 1.0;
            parts.push((Part::More, (more_x, y, CONTROL_H, CONTROL_H)));
            if ip.menu {
                // A popup under More, right-aligned with it.
                let mw = MENU_W.min(inner);
                let mx = more_x + CONTROL_H - mw;
                let mut my = y + CONTROL_H + 4.0 + 6.0;
                let note = layout(g, REGENERATE_NOTE, &g.formats.direct_note, mw - 32.0)?;
                let rh = 10.0 + 20.0 + height(&note) + 10.0;
                menu.push((Part::LinkAction(REGENERATE), (mx + 6.0, my, mw - 12.0, rh)));
                my += rh;
                menu.push((
                    Part::LinkAction(DEACTIVATE),
                    (mx + 6.0, my, mw - 12.0, 36.0),
                ));
            }
            y += CONTROL_H + 6.0;
            y += line(
                &mut parts,
                Part::Line(SUMMARY),
                &self.invite_line(SUMMARY),
                &g.formats.direct_note,
                y,
            )?;
        } else if ip.pending {
            y += line(
                &mut parts,
                Part::Loading,
                "Preparing invite link\u{2026}",
                &g.formats.direct_note,
                y,
            )?;
        } else {
            y += line(
                &mut parts,
                Part::Line(MISSING),
                &self.invite_line(MISSING),
                &g.formats.direct_note,
                y,
            )? + 8.0;
            let l = layout(g, self.retry_label(), &g.formats.button, 1e6)?;
            parts.push((Part::Retry, (0.0, y, width(&l) + 24.0, BUTTON_H)));
            y += BUTTON_H;
        }
        if ip.link_error.is_some() {
            y += 6.0;
            y += line(
                &mut parts,
                Part::Line(LINK_ERROR),
                &self.invite_line(LINK_ERROR),
                &g.formats.error,
                y,
            )?;
        }
        y += 22.0;
        if !ip.invitations.is_empty() {
            y += line(
                &mut parts,
                Part::Heading(PENDING),
                &self.invite_heading(PENDING),
                &g.formats.direct_name,
                y,
            )? + 6.0;
            let l = layout(g, "Cancel invite", &g.formats.button, 1e6)?;
            let cw = width(&l) + 24.0;
            for k in 0..ip.invitations.len() {
                parts.push((Part::Invitee(k), (0.0, y, inner - cw - 12.0, ROW_H)));
                parts.push((
                    Part::CancelInvite(k),
                    (inner - cw, y + (ROW_H - 32.0) / 2.0, cw, 32.0),
                ));
                y += ROW_H;
            }
            y += 16.0;
        }
        y += line(
            &mut parts,
            Part::Heading(BY_NAME),
            &self.invite_heading(BY_NAME),
            &g.formats.direct_name,
            y,
        )? + 8.0;
        let invite = layout(g, "Invite", &g.formats.button, 1e6)?;
        let invite_w = (width(&invite) + 24.0).max(72.0);
        let field_w = inner - invite_w - 8.0;
        self.field
            .relayout_with(g, &g.formats.composer, 15.0, field_w - 2.0 * FIELD_PAD_X)?;
        parts.push((Part::Field, (0.0, y, field_w, CONTROL_H)));
        parts.push((Part::Primary, (field_w + 8.0, y, invite_w, CONTROL_H)));
        y += CONTROL_H;
        if !self.suggestions.is_empty() {
            y += 8.0;
            for k in 0..self.suggestions.len() {
                parts.push((Part::Suggest(k), (4.0, y, field_w - 8.0, SUGGEST_H)));
                y += SUGGEST_H;
            }
            y += 4.0;
        }
        y += 8.0;
        y += line(
            &mut parts,
            Part::Line(HELP),
            &self.invite_line(HELP),
            &g.formats.direct_note,
            y,
        )?;
        if let Some(e) = &self.error {
            y += 8.0;
            y += line(&mut parts, Part::Error, e, &g.formats.error, y)?;
        } else if ip.status.is_some() {
            y += 8.0;
            y += line(
                &mut parts,
                Part::Line(STATUS),
                &self.invite_line(STATUS),
                &g.formats.direct_note,
                y,
            )?;
        }
        // The menu is hit first, and drawn last.
        menu.append(&mut parts);
        let content = y - body_top;
        let room = (h - 48.0 - 2.0 * CARD_PAD - body_top).max(ROW_H);
        let view = content.min(room);
        self.scroll = self.scroll.clamp(0.0, (content - view).max(0.0));
        let card_h = body_top + view + 2.0 * CARD_PAD;
        self.place(menu, card_w, card_h, w, h, (body_top, view))
    }

    /// Centres the card, moves the parts onto it (the body's scrolled),
    /// and sets where the field's text starts.
    fn place(
        &mut self,
        mut parts: Vec<(Part, Rect)>,
        card_w: f32,
        card_h: f32,
        w: f32,
        h: f32,
        (body_top, view): (f32, f32),
    ) -> Result<()> {
        let (cx, cy) = (
            ((w - card_w) / 2.0).round(),
            ((h - card_h) / 2.0).max(24.0).round(),
        );
        self.card = (cx, cy, card_w, card_h);
        let scroll = if self.confirming() { 0.0 } else { self.scroll };
        for (p, r) in &mut parts {
            r.0 += cx + CARD_PAD;
            r.1 += cy + CARD_PAD;
            if p.in_body() {
                r.1 -= scroll;
            }
        }
        self.body = (
            cx + CARD_PAD,
            cy + CARD_PAD + body_top,
            card_w - 2.0 * CARD_PAD,
            view,
        );
        self.parts = parts;
        if let Some((fx, fy, _, fh)) = self.part_rect(Part::Field) {
            self.field_origin = (fx + FIELD_PAD_X, fy + (fh - LINE) / 2.0);
        }
        Ok(())
    }

    pub(crate) fn draw_invite(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        for (part, r) in self.parts.iter().filter(|(p, _)| !p.in_body()) {
            self.draw_invite_part(g, dc, *part, *r)?;
        }
        let (bx, by, bw, bh) = self.body;
        unsafe {
            dc.PushAxisAlignedClip(&rect(bx, by, bx + bw, by + bh), D2D1_ANTIALIAS_MODE_ALIASED);
        }
        let drawn = (|| {
            let body = self.parts.iter().filter(|(p, _)| p.in_body());
            for (part, r) in body
                .clone()
                .filter(|(p, _)| !matches!(p, Part::LinkAction(_)))
            {
                self.draw_invite_part(g, dc, *part, *r)?;
            }
            // More invite options, over everything else.
            let items: Vec<_> = body
                .filter(|(p, _)| matches!(p, Part::LinkAction(_)))
                .collect();
            if let (Some((_, first)), Some((_, last))) = (items.first(), items.last()) {
                let frame = rect(
                    first.0 - 6.0,
                    first.1 - 6.0,
                    first.0 + first.2 + 6.0,
                    last.1 + last.3 + 6.0,
                );
                fill_round(dc, frame, RADIUS, &g.brushes.raised);
                stroke_round(dc, frame, RADIUS, &g.brushes.border);
                for (part, r) in items {
                    self.draw_invite_part(g, dc, *part, *r)?;
                }
            }
            Ok(())
        })();
        unsafe { dc.PopAxisAlignedClip() };
        drawn
    }

    fn draw_invite_part(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        part: Part,
        (x, y, w, h): Rect,
    ) -> Result<()> {
        let Some(ip) = self.inviting.as_deref() else {
            return Ok(());
        };
        let draw = |s: &str, format, brush, tx: f32, ty: f32, tw: f32| -> Result<()> {
            let l = layout(g, s, format, tw.max(1.0))?;
            text::draw(dc, &l, tx, ty, brush);
            Ok(())
        };
        match part {
            Part::Title => draw(
                &self.title(),
                &g.formats.dialog_title,
                &g.brushes.text,
                x,
                y,
                w,
            )?,
            Part::Note => draw(
                &self.note(),
                &g.formats.dialog_note,
                &g.brushes.muted,
                x,
                y,
                w,
            )?,
            Part::Close => {
                let lit = self.hover == Some(Part::Close);
                if lit {
                    fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
                }
                icons::draw(
                    dc,
                    Icon::Close,
                    rect(x + 8.0, y + 8.0, x + w - 8.0, y + h - 8.0),
                    if lit { TEXT } else { TEXT_MUTED },
                    1.0,
                );
            }
            Part::Heading(k) => {
                draw(
                    &self.invite_heading(k),
                    &g.formats.direct_name,
                    &g.brushes.text,
                    x,
                    y,
                    w,
                )?;
                if k == PENDING {
                    let n = ip.invitations.len().to_string();
                    let l = layout(g, &n, &g.formats.direct_note, 1e6)?;
                    text::draw(dc, &l, x + w - width(&l), y, &g.brushes.muted);
                }
            }
            Part::Line(k) => {
                let (format, brush) = self.line_style(g, k);
                draw(&self.invite_line(k), format, brush, x, y, w)?;
            }
            Part::Loading => draw(
                "Preparing invite link\u{2026}",
                &g.formats.direct_note,
                &g.brushes.muted,
                x,
                y,
                w,
            )?,
            Part::Error => draw(
                self.error.as_deref().unwrap_or_default(),
                &g.formats.error,
                &g.brushes.error,
                x,
                y,
                w,
            )?,
            Part::Link => {
                let body = rect(x, y, x + w, y + h);
                fill_round(dc, body, 7.0, &g.brushes.composer);
                stroke_round(dc, body, 7.0, &g.brushes.border);
                let url = ip
                    .default
                    .as_ref()
                    .map_or_else(String::new, |l| ip.url(&l.code));
                let l = g.layout(&gfx::wide(&url), &g.formats.composer, 1e6)?;
                unsafe {
                    l.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                    dc.PushAxisAlignedClip(
                        &rect(x + 2.0, y + 2.0, x + w - 8.0, y + h - 2.0),
                        D2D1_ANTIALIAS_MODE_ALIASED,
                    );
                }
                text::draw(
                    dc,
                    &l,
                    x + FIELD_PAD_X,
                    y + (h - height(&l)) / 2.0,
                    &g.brushes.text,
                );
                unsafe { dc.PopAxisAlignedClip() };
            }
            Part::Copy => {
                self.button(g, dc, part, self.copy_label(), (x, y, w, h), Tone::Primary)?
            }
            Part::More => {
                self.button(g, dc, part, "", (x, y, w, h), Tone::Primary)?;
                let (cx, cy) = (x + w / 2.0, y + h / 2.0);
                let enabled = self.invite_enabled(Part::More);
                icons::draw(
                    dc,
                    Icon::ChevronDown,
                    rect(cx - 8.0, cy - 8.0, cx + 8.0, cy + 8.0),
                    if enabled { BLACKOUT } else { TEXT_MUTED },
                    1.0,
                );
            }
            Part::LinkAction(k) => {
                if self.hover == Some(part) {
                    fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
                }
                let brush = if k == DEACTIVATE {
                    &g.brushes.error
                } else {
                    &g.brushes.text
                };
                let label = layout(
                    g,
                    Self::link_action_label(k),
                    &g.formats.dialog_note,
                    w - 20.0,
                )?;
                if k == REGENERATE {
                    text::draw(dc, &label, x + 10.0, y + 10.0, brush);
                    draw(
                        REGENERATE_NOTE,
                        &g.formats.direct_note,
                        &g.brushes.muted,
                        x + 10.0,
                        y + 30.0,
                        w - 20.0,
                    )?;
                } else {
                    text::draw(dc, &label, x + 10.0, y + (h - height(&label)) / 2.0, brush);
                }
            }
            Part::Retry => {
                self.button(g, dc, part, self.retry_label(), (x, y, w, h), Tone::Primary)?
            }
            Part::Invitee(k) => {
                let Some(m) = ip.invitations.get(k) else {
                    return Ok(());
                };
                let face = crate::live::person(&m.id, &m.display_name);
                let (cx, cy) = (x + 15.0, y + h / 2.0);
                unsafe {
                    dc.FillEllipse(
                        &D2D1_ELLIPSE {
                            point: windows_numerics::Vector2 { X: cx, Y: cy },
                            radiusX: 15.0,
                            radiusY: 15.0,
                        },
                        &g.solid(face.color),
                    );
                }
                let i = g.layout(&gfx::wide(&face.initials), &g.formats.direct_initials, 1e6)?;
                text::draw(
                    dc,
                    &i,
                    cx - width(&i) / 2.0,
                    cy - height(&i) / 2.0,
                    &g.brushes.text,
                );
                let tx = x + 30.0 + 12.0;
                let tw = w - (tx - x);
                draw(
                    &m.display_name,
                    &g.formats.direct_name,
                    &g.brushes.text,
                    tx,
                    y + 6.0,
                    tw,
                )?;
                let user = format!("@{} \u{b7} Invited", m.username);
                draw(
                    &user,
                    &g.formats.direct_note,
                    &g.brushes.muted,
                    tx,
                    y + 26.0,
                    tw,
                )?;
            }
            Part::CancelInvite(_) => {
                self.button(g, dc, part, "Cancel invite", (x, y, w, h), Tone::Secondary)?
            }
            Part::Field => self.draw_field(g, dc, (x, y, w, h))?,
            Part::Suggest(k) => {
                if k == 0 {
                    let n = self.suggestions.len() as f32;
                    let frame = rect(x - 4.0, y - 4.0, x + w + 4.0, y + n * h + 4.0);
                    fill_round(dc, frame, RADIUS, &g.brushes.composer);
                    stroke_round(dc, frame, RADIUS, &g.brushes.border);
                }
                self.draw_suggestion(g, dc, k, (x, y, w, h))?;
            }
            Part::Cancel => self.button(g, dc, part, "Cancel", (x, y, w, h), Tone::Secondary)?,
            Part::Primary => {
                let tone = if self.confirming() {
                    Tone::Danger
                } else {
                    Tone::Primary
                };
                self.button(g, dc, part, self.primary_label(), (x, y, w, h), tone)?;
            }
            _ => {}
        }
        Ok(())
    }
}

impl App {
    /// The space menu's or a public channel's "Invite people…" (owners):
    /// the link is prepared at once and the space's invitations load.
    pub fn open_invite_people(&mut self, channel: Option<String>) {
        let Some(live) = &self.live else { return };
        let Some(inviting) = live.invite_people(channel) else {
            return;
        };
        live.load_people();
        live.admin(AdminOperation::LoadInvitations {
            space: inviting.space.clone(),
        });
        let mut d = Dialog::invite_people(inviting);
        d.prepare_link(live);
        self.open_dialog(d);
    }

    pub fn invite_people_answer(&mut self, answer: Answer) {
        let Some(d) = self
            .dialog
            .as_mut()
            .filter(|d| d.kind == Kind::InvitePeople)
        else {
            return;
        };
        let request = d.request;
        if let (Some(space), Some(live)) = (d.invite_answer(answer), &self.live) {
            live.account(AccountOperation::LoadInviteLinks { space, request });
        }
        self.dialog_changed();
    }

    /// Clicks and UIA Invoke on the dialog's own parts.
    pub(crate) fn invite_people_activate(&mut self, part: Part) {
        let hwnd = self.hwnd;
        let (Some(d), Some(live)) = (self.dialog.as_mut(), self.live.as_ref()) else {
            return;
        };
        if part == Part::Retry {
            d.prepare_link(live);
        } else if part == Part::Primary && !d.confirming() {
            if let Some(username) = d.invite_submit()
                && let Some(ip) = &d.inviting
            {
                d.busy = true;
                d.hidden = true;
                d.suggestions.clear();
                live.admin(AdminOperation::AddMember {
                    space: ip.space.clone(),
                    channel: None,
                    username,
                });
            }
        } else if let Some(ip) = d.inviting.as_deref_mut() {
            let menu = std::mem::take(&mut ip.menu);
            match part {
                Part::More => ip.menu = !menu,
                Part::Copy => {
                    if let Some(link) = &ip.default {
                        crate::set_clipboard(hwnd, &gfx::wide(&ip.url(&link.code)));
                        ip.copied = Some(link.code.clone());
                        unsafe {
                            SetTimer(Some(hwnd), TIMER_COPIED, COPIED_MS, None);
                        }
                    }
                }
                Part::LinkAction(k) => {
                    if let Some(link) = &ip.default {
                        ip.confirm = Some((link.code.clone(), k == REGENERATE));
                        d.field.focused = false;
                    }
                }
                Part::Cancel => {
                    ip.confirm = None;
                    d.field.focused = true;
                }
                Part::Primary => {
                    if let Some((code, regenerate)) = ip.confirm.take() {
                        d.request += 1;
                        ip.pending = true;
                        ip.regenerating = regenerate;
                        ip.link_error = None;
                        d.field.focused = true;
                        let (space, request) = (ip.space.clone(), d.request);
                        live.account(if regenerate {
                            AccountOperation::RegenerateInviteLink {
                                space,
                                code,
                                request,
                            }
                        } else {
                            AccountOperation::RevokeInviteLink {
                                space,
                                request,
                                code,
                            }
                        });
                    }
                }
                Part::CancelInvite(k) => {
                    if let Some(m) = ip.invitations.get(k) {
                        d.busy = true;
                        d.error = None;
                        ip.status = None;
                        live.admin(AdminOperation::CancelInvitation {
                            space: ip.space.clone(),
                            user: m.id.clone(),
                        });
                    }
                }
                _ => {}
            }
        }
        self.dialog_changed();
        if let Err(e) = self.drain_live() {
            eprintln!("invite action failed: {e:?}");
        }
    }

    pub fn copied_tick(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_COPIED);
        }
        if let Some(ip) = self.dialog.as_mut().and_then(|d| d.inviting.as_deref_mut()) {
            ip.copied = None;
        }
        self.dialog_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: &str, username: &str) -> Member {
        serde_json::from_value(serde_json::json!({
            "id": id, "username": username, "displayName": username.to_uppercase(), "owner": false,
        }))
        .unwrap()
    }

    fn link(code: &str, uses: u32) -> InviteLink {
        serde_json::from_value(serde_json::json!({
            "code": code,
            "channel": { "id": "c1", "name": "general" },
            "createdBy": { "username": "owner", "displayName": "Owner" },
            "uses": uses, "maxUses": 100, "expiresAt": null,
            "createdAt": "2026-11-07T00:00:00Z",
        }))
        .unwrap()
    }

    fn dialog() -> Dialog {
        let ip = InvitePeople::new(
            "s1".into(),
            "Studio".into(),
            None,
            Vec::new(),
            "https://caper.chat/invite/".into(),
            vec![member("u1", "maya")],
        );
        let mut d = Dialog::invite_people(ip);
        d.request = 4;
        d.inviting.as_mut().unwrap().pending = true;
        d
    }

    fn typed(d: &mut Dialog, text: &str) {
        d.field.text = gfx::wide(text);
    }

    #[test]
    fn the_link_is_shown_regenerated_and_stale_answers_dropped() {
        let mut d = dialog();
        assert_eq!(d.title(), "Invite people to Studio");
        assert_eq!(
            d.invite_line(NO_CHANNEL),
            "People will join the space without opening a channel."
        );
        // An answer for an older request changes nothing.
        let stale = Answer::Default {
            space: "s1".into(),
            request: 3,
            result: Ok(link("OLD0000001", 0)),
        };
        assert_eq!(d.invite_answer(stale), None);
        assert!(d.inviting.as_ref().unwrap().default.is_none());
        let fresh = Answer::Default {
            space: "s1".into(),
            request: 4,
            result: Ok(link("AbCdE12345", 7)),
        };
        assert_eq!(d.invite_answer(fresh).as_deref(), Some("s1"));
        assert_eq!(
            d.invite_link_name(),
            "Invite link: https://caper.chat/invite/AbCdE12345"
        );
        assert_eq!(
            d.invite_line(SUMMARY),
            "#general \u{b7} 93 of 100 joins remaining \u{b7} Never expires"
        );
        // Regenerating replaces it, and the old code leaves the list.
        let ip = d.inviting.as_mut().unwrap();
        ip.confirm = Some(("AbCdE12345".into(), true));
        assert_eq!(
            (d.title().as_str(), d.primary_label()),
            ("Regenerate invite link?", "Regenerate link")
        );
        assert_eq!(
            d.invite_line(CURRENT),
            "Current link: https://caper.chat/invite/AbCdE12345"
        );
        let ip = d.inviting.as_mut().unwrap();
        ip.confirm = None;
        ip.regenerating = true;
        d.request = 5;
        let next = Answer::Default {
            space: "s1".into(),
            request: 5,
            result: Ok(link("ZzZzZ67890", 0)),
        };
        d.invite_answer(next);
        let ip = d.inviting.as_ref().unwrap();
        let codes: Vec<_> = ip.links.iter().map(|l| l.code.as_str()).collect();
        assert_eq!(codes, ["ZzZzZ67890"]);
        // Deactivated, it says there is none and offers another.
        d.request = 6;
        d.invite_answer(Answer::Revoked {
            space: "s1".into(),
            request: 6,
            code: "ZzZzZ67890".into(),
            result: Ok(()),
        });
        assert!(d.inviting.as_ref().unwrap().default.is_none());
        assert_eq!(
            (d.invite_line(MISSING).as_str(), d.retry_label()),
            ("No active invite link.", "Create another link")
        );
    }

    #[test]
    fn inviting_by_username_checks_first_then_says_it_was_sent() {
        let mut d = dialog();
        d.invite_answer(Answer::Invitations(vec![member("u2", "alex")]));
        typed(&mut d, "ab");
        assert_eq!(d.invite_submit(), None);
        assert_eq!(d.error.as_deref(), Some(USERNAME_RULE));
        typed(&mut d, "maya");
        assert_eq!(d.invite_submit(), None);
        assert_eq!(
            d.error.as_deref(),
            Some("This person is already in the space.")
        );
        typed(&mut d, "alex");
        assert_eq!(d.invite_submit(), None);
        assert_eq!(
            d.error.as_deref(),
            Some("This person already has a pending invitation.")
        );
        assert!(d.already_invited("alex") && !d.already_invited("sam"));
        typed(&mut d, "sam");
        assert_eq!(d.invite_submit().as_deref(), Some("sam"));
        d.busy = true;
        d.invite_answer(Answer::Invited(member("u3", "sam")));
        assert!(!d.busy && d.field.text.is_empty());
        assert_eq!(d.invite_line(STATUS), INVITATION_SENT);
        assert_eq!(d.invitee_name(1), "SAM @sam \u{b7} Invited");
        d.invite_answer(Answer::Cancelled("u2".into()));
        assert_eq!(d.invitee_name(0), "SAM @sam \u{b7} Invited");
    }
}
