//! Manage space and Channel settings, as web's dialogs (egui's
//! `space_dialog` and `channel_dialog`): Invite people's dialog, opened by
//! the space's owner. Manage space adds the Space name, saved on its own,
//! lists the members, and makes links only through Custom links. Channel
//! settings hold the channel's name and privacy, saved together from the
//! save bar, and, once it is saved private, who has access to it.
use crate::custom_links::{NAME, number};
use crate::gfx::{self, Gfx, rect};
use crate::invite_people::{Answer, InvitePeople, NAME_ERROR};
use crate::overlay::{Dialog, Part, Rect, Tone, height, layout, width};
use crate::text;
use caper_client::model::Member;
use caper_client::names;
use caper_client::worker::{AccountOperation, AdminOperation};
use windows::Win32::Graphics::Direct2D::*;
use windows::core::Result;

const CONTROL_H: f32 = 38.0;
const BUTTON_H: f32 = 36.0;
const ROW_H: f32 = 48.0;
const TOGGLE_H: f32 = 26.0;
/// The save bar under a channel's settings while they're unsaved.
pub const FOOTER_H: f32 = 64.0;

pub const OWNER_NOTE: &str = "Only the owner can change this space and its membership.";
pub const UNSAVED: &str = "You have unsaved changes.";

/// What the dialog manages, beyond inviting people.
pub enum Manage {
    Space,
    Channel(ChannelSettings),
}

/// A channel's settings: the channel, its saved privacy (the dialog's
/// `private` holds the draft), whether who has access to it has loaded,
/// and whom it can be granted to: the space's members but its owner.
pub struct ChannelSettings {
    pub id: String,
    pub private: bool,
    pub loaded: bool,
    pub space_members: Vec<Member>,
    pub owner: String,
}

impl InvitePeople {
    /// Manage space: its name to change, and its members.
    pub fn manage_space(mut self) -> Self {
        self.manage = Some(Manage::Space);
        let name = self.space_name.clone();
        self.rename(&name);
        self
    }

    /// Channel `id`'s settings (`name`, `private`); the space's members
    /// (`members`, as built) become whom it can be granted to.
    pub fn channel_settings(
        mut self,
        id: String,
        name: &str,
        private: bool,
        owner: String,
    ) -> Self {
        let space_members = std::mem::take(&mut self.members);
        self.manage = Some(Manage::Channel(ChannelSettings {
            id,
            private,
            loaded: !private,
            space_members,
            owner,
        }));
        self.rename(name);
        self
    }

    /// The saved name, and the name field showing it.
    pub(crate) fn rename(&mut self, name: &str) {
        self.saved = name.to_owned();
        self.numbers[NAME as usize] = number(name);
        self.seen[NAME as usize] = self.numbers[NAME as usize].text.clone();
    }

    pub fn channel(&self) -> Option<&ChannelSettings> {
        match &self.manage {
            Some(Manage::Channel(c)) => Some(c),
            _ => None,
        }
    }
}

impl Dialog {
    /// Manage space or Channel settings; neither field takes the keyboard
    /// at first, as on web.
    pub fn manage(ip: InvitePeople) -> Self {
        let private = ip.channel().is_some_and(|c| c.private);
        let space_name = ip.space_name.clone();
        let mut d = Self::invite_people(ip);
        d.field.focused = false;
        d.private = private;
        d.space_name = space_name;
        d
    }

    pub(crate) fn managing(&self) -> Option<&Manage> {
        self.inviting.as_ref()?.manage.as_ref()
    }

    pub(crate) fn settings_channel(&self) -> Option<&ChannelSettings> {
        self.inviting.as_ref()?.channel()
    }

    pub fn name_text(&self) -> String {
        self.inviting.as_ref().map_or_else(String::new, |ip| {
            String::from_utf16_lossy(&ip.numbers[NAME as usize].text)
        })
    }

    /// Changes not saved yet (web's `dirty`): the space's name, or the
    /// channel's name or privacy.
    pub fn unsaved(&self) -> bool {
        let Some(ip) = self.inviting.as_deref() else {
            return false;
        };
        let name = self.name_text();
        match &ip.manage {
            Some(Manage::Space) => name != ip.saved,
            Some(Manage::Channel(c)) => name != ip.saved || self.private != c.private,
            None => false,
        }
    }

    /// Whether the channel's save bar shows.
    pub(crate) fn save_bar(&self) -> bool {
        self.settings_channel().is_some() && self.unsaved()
    }

    /// Whether the invitations and members show: always for the space,
    /// for a channel once it is saved private.
    pub(crate) fn members_shown(&self) -> bool {
        self.settings_channel().is_none_or(|c| c.private)
    }

    /// Whether who has access is still loading.
    pub(crate) fn members_loading(&self) -> bool {
        self.settings_channel().is_some_and(|c| !c.loaded)
    }

    pub fn manage_label(&self) -> &'static str {
        if self.settings_channel().is_some() {
            "Channel name"
        } else {
            "Space name"
        }
    }

    pub fn save_changes_label(&self) -> &'static str {
        if self.busy {
            "Saving\u{2026}"
        } else {
            "Save changes"
        }
    }

    pub fn loading_text(&self) -> &'static str {
        if self.settings_channel().is_some() {
            "Loading members\u{2026}"
        } else {
            "Preparing invite link\u{2026}"
        }
    }

    /// Whether Save name can save: something to save, nothing in flight.
    pub(crate) fn can_save_name(&self) -> bool {
        !self.busy
            && self
                .inviting
                .as_ref()
                .is_some_and(|ip| self.name_text().trim() != ip.saved)
    }

    /// Member `k`'s row, as web shows it.
    pub fn member_name(&self, k: usize) -> String {
        self.inviting
            .as_deref()
            .and_then(|ip| ip.members.get(k))
            .map_or_else(String::new, |m| {
                let owner = if m.owner { " \u{b7} Owner" } else { "" };
                format!("{} @{}{owner}", m.display_name, m.username)
            })
    }

    /// After any change: the name is kept as web keeps it while typing,
    /// and changing it clears why it couldn't be saved.
    pub(crate) fn settings_name_changed(&mut self) {
        let channel = self.settings_channel().is_some();
        let Some(ip) = self
            .inviting
            .as_deref_mut()
            .filter(|ip| ip.manage.is_some())
        else {
            return;
        };
        let f = &mut ip.numbers[NAME as usize];
        if let Some((text, caret)) = crate::manage::normalized(&f.text, f.caret, channel) {
            f.text = text;
            f.caret = caret;
            f.anchor = caret;
        }
        if f.text != ip.seen[NAME as usize] {
            ip.seen[NAME as usize] = f.text.clone();
            ip.name_error = None;
        }
    }

    /// The name's label and field (Manage space's with Save name beside
    /// it; a channel's with the Private channel switch and what it means
    /// under it), then why it couldn't be saved, from `y`.
    pub(crate) fn relayout_manage_head(
        &self,
        g: &Gfx,
        inner: f32,
        mut y: f32,
        parts: &mut Vec<(Part, Rect)>,
    ) -> Result<f32> {
        let text_h = |s: &str, format| -> Result<f32> { Ok(height(&layout(g, s, format, inner)?)) };
        let h = text_h(self.manage_label(), &g.formats.dialog_note)?;
        parts.push((Part::Label, (0.0, y, inner, h)));
        y += h + 6.0;
        if self.settings_channel().is_some() {
            parts.push((Part::Number(NAME), (0.0, y, inner, CONTROL_H)));
            y += CONTROL_H + 14.0;
            parts.push((Part::Toggle, (0.0, y, inner, TOGGLE_H)));
            y += TOGGLE_H + 4.0;
            let h = text_h(&self.privacy_note(), &g.formats.direct_note)?;
            parts.push((Part::Hint, (0.0, y, inner, h)));
            y += h;
        } else {
            let sw = save_name_w(g)?;
            parts.push((Part::Number(NAME), (0.0, y, inner - sw - 8.0, CONTROL_H)));
            parts.push((Part::SaveName, (inner - sw, y, sw, CONTROL_H)));
            y += CONTROL_H;
        }
        if let Some(e) = self.inviting.as_ref().and_then(|ip| ip.name_error.as_ref()) {
            y += 8.0;
            let h = text_h(e, &g.formats.error)?;
            parts.push((Part::Line(NAME_ERROR), (0.0, y, inner, h)));
            y += h;
        }
        Ok(y)
    }

    /// The name field's width, for its layout.
    pub(crate) fn name_field_w(&self, g: &Gfx, inner: f32) -> Result<f32> {
        Ok(if self.settings_channel().is_some() {
            inner
        } else {
            inner - save_name_w(g)? - 8.0
        })
    }

    /// The members' rows, from `y`.
    pub(crate) fn relayout_members(
        &self,
        inner: f32,
        mut y: f32,
        parts: &mut Vec<(Part, Rect)>,
    ) -> f32 {
        let Some(ip) = self.inviting.as_deref() else {
            return y;
        };
        if ip.manage.is_none() || self.members_loading() || ip.members.is_empty() {
            return y;
        }
        y += 12.0;
        for k in 0..ip.members.len() {
            parts.push((Part::Member(k), (0.0, y, inner, ROW_H)));
            y += ROW_H;
        }
        y
    }

    /// The save bar from `top`: what it says, Reset and Save changes.
    pub(crate) fn relayout_footer(
        &self,
        g: &Gfx,
        inner: f32,
        top: f32,
        parts: &mut Vec<(Part, Rect)>,
    ) -> Result<()> {
        let y = top + (FOOTER_H - BUTTON_H) / 2.0 + 4.0;
        let save = width(&layout(g, "Save changes", &g.formats.button, 1e6)?) + 24.0;
        let reset = width(&layout(g, "Reset", &g.formats.button, 1e6)?) + 24.0;
        parts.push((
            Part::Unsaved,
            (0.0, top, inner - save - reset - 16.0, FOOTER_H),
        ));
        parts.push((
            Part::Reset,
            (inner - save - 8.0 - reset, y, reset, BUTTON_H),
        ));
        parts.push((Part::SaveChanges, (inner - save, y, save, BUTTON_H)));
        Ok(())
    }

    pub(crate) fn draw_manage_part(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        part: Part,
        (x, y, w, h): Rect,
    ) -> Result<()> {
        let draw = |s: &str, format, brush, ty: f32| -> Result<()> {
            let l = layout(g, s, format, w.max(1.0))?;
            text::draw(dc, &l, x, ty, brush);
            Ok(())
        };
        match part {
            Part::Label => draw(
                self.manage_label(),
                &g.formats.dialog_note,
                &g.brushes.text,
                y,
            )?,
            Part::Hint => draw(
                &self.privacy_note(),
                &g.formats.direct_note,
                &g.brushes.muted,
                y,
            )?,
            Part::Toggle => self.draw_create_part(g, dc, part, (x, y, w, h))?,
            Part::SaveName => {
                self.button(g, dc, part, "Save name", (x, y, w, h), Tone::Secondary)?
            }
            Part::Member(k) => {
                let Some(m) = self.inviting.as_deref().and_then(|ip| ip.members.get(k)) else {
                    return Ok(());
                };
                let owner = if m.owner { " \u{b7} Owner" } else { "" };
                let sub = format!("@{}{owner}", m.username);
                draw_person(g, dc, m, &sub, (x, y, w, h))?;
            }
            Part::Unsaved => {
                // A rule across the card above the bar, as web's footer.
                let (cx, _, cw, _) = self.card;
                unsafe {
                    dc.FillRectangle(&rect(cx, y, cx + cw, y + 1.0), &g.brushes.border);
                }
                let l = layout(g, UNSAVED, &g.formats.dialog_note, w.max(1.0))?;
                let ty = y + (h - height(&l)) / 2.0 + 4.0;
                text::draw(dc, &l, x, ty, &g.brushes.text);
            }
            Part::Reset => self.button(g, dc, part, "Reset", (x, y, w, h), Tone::Secondary)?,
            Part::SaveChanges => self.button(
                g,
                dc,
                part,
                self.save_changes_label(),
                (x, y, w, h),
                Tone::Primary,
            )?,
            _ => {}
        }
        Ok(())
    }

    /// Save name, Reset and Save changes, checked first as web's forms
    /// check them: what to send, if anything.
    pub(crate) fn manage_activate(&mut self, part: Part) -> Option<AdminOperation> {
        let name = self.name_text();
        let (private, dirty) = (self.private, self.unsaved());
        let ip = self.inviting.as_deref_mut()?;
        let operation = match (part, &ip.manage) {
            (Part::Reset, Some(Manage::Channel(c))) => {
                self.private = c.private;
                let saved = ip.saved.clone();
                ip.rename(&saved);
                ip.name_error = None;
                return None;
            }
            (Part::SaveName, Some(Manage::Space)) => match names::space_name_error(&name) {
                Some(e) => {
                    ip.name_error = Some(e.into());
                    return None;
                }
                None => AdminOperation::UpdateSpace {
                    space: ip.space.clone(),
                    name: names::space_name(&name).to_owned(),
                },
            },
            (Part::SaveChanges, Some(Manage::Channel(c))) if dirty => {
                // The dash left while typing isn't saved.
                let name = name.strip_suffix('-').unwrap_or(&name);
                match names::channel_name_error(name) {
                    Some(e) => {
                        ip.name_error = Some(e.into());
                        return None;
                    }
                    None => AdminOperation::UpdateChannel {
                        space: ip.space.clone(),
                        channel: c.id.clone(),
                        name: name.to_owned(),
                        private,
                    },
                }
            }
            _ => return None,
        };
        ip.saving = true;
        ip.name_error = None;
        ip.focus = None;
        self.busy = true;
        Some(operation)
    }

    /// A failed save says why by the name; members that couldn't load
    /// say why there too. False when it wasn't one of these.
    pub(crate) fn manage_failed(&mut self, error: &str) -> bool {
        let busy = self.busy;
        let Some(ip) = self.inviting.as_deref_mut() else {
            return false;
        };
        let loading = matches!(&ip.manage, Some(Manage::Channel(c)) if !c.loaded);
        if ip.saving {
            ip.saving = false;
            self.busy = false;
        } else if loading && !busy {
            if let Some(Manage::Channel(c)) = &mut ip.manage {
                c.loaded = true;
            }
        } else {
            return false;
        }
        ip.name_error = Some(error.to_owned());
        true
    }

    /// Applies a save or a channel's members. A saved dialog stays open on
    /// the saved values, as web's does.
    pub(crate) fn manage_answer(&mut self, answer: Answer) {
        let channel = self.settings_channel().map(|c| c.id.clone());
        let Some(ip) = self.inviting.as_deref_mut() else {
            return;
        };
        match answer {
            Answer::SpaceSaved(space) if space.id == ip.space => {
                ip.saving = false;
                self.busy = false;
                ip.space_name = space.name.clone();
                self.space_name = space.name.clone();
                ip.rename(&space.name);
            }
            Answer::ChannelSaved(saved) if Some(&saved.id) == channel.as_ref() => {
                ip.saving = false;
                self.busy = false;
                ip.rename(&saved.name);
                self.private = saved.private;
                if let Some(Manage::Channel(c)) = &mut ip.manage {
                    // Made private: who has access loads.
                    if saved.private && !c.private {
                        c.loaded = false;
                    }
                    c.private = saved.private;
                }
            }
            Answer::Members {
                channel: id,
                members,
                invitations,
            } if Some(&id) == channel.as_ref() => {
                ip.members = members;
                ip.invitations = invitations;
                if let Some(Manage::Channel(c)) = &mut ip.manage {
                    c.loaded = true;
                }
            }
            Answer::ChannelInvited {
                channel: id,
                member,
            } if Some(&id) == channel.as_ref() => {
                self.invite_answer(Answer::Invited(member));
            }
            Answer::ChannelRemoved {
                channel: id,
                member,
            } if Some(&id) == channel.as_ref() => {
                ip.members.retain(|m| m.id != member);
                self.invite_answer(Answer::Cancelled(member));
            }
            _ => {}
        }
    }
}

fn save_name_w(g: &Gfx) -> Result<f32> {
    Ok(width(&layout(g, "Save name", &g.formats.button, 1e6)?) + 24.0)
}

/// A person's row: their initials' circle, name, and `sub` under it.
pub(crate) fn draw_person(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    m: &Member,
    sub: &str,
    (x, y, w, h): Rect,
) -> Result<()> {
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
    let tw = (w - (tx - x)).max(1.0);
    let name = layout(g, &m.display_name, &g.formats.direct_name, tw)?;
    text::draw(dc, &name, tx, y + 6.0, &g.brushes.text);
    let sub = layout(g, sub, &g.formats.direct_note, tw)?;
    text::draw(dc, &sub, tx, y + 26.0, &g.brushes.muted);
    Ok(())
}

impl crate::App {
    /// The space menu's Space settings (owners): its invitations and
    /// links load, and the people the field suggests.
    pub fn open_manage_space(&mut self) {
        let Some(live) = &self.live else { return };
        let Some(ip) = live.invite_people(None) else {
            return;
        };
        live.load_people();
        live.admin(AdminOperation::LoadInvitations {
            space: ip.space.clone(),
        });
        let mut d = Dialog::manage(ip.manage_space());
        d.request += 1;
        if let Some(ip) = d.inviting.as_deref_mut() {
            ip.pending = true;
            live.account(AccountOperation::LoadInviteLinks {
                space: ip.space.clone(),
                request: d.request,
            });
        }
        self.open_dialog(d);
    }

    /// Channel `id`'s settings (its menu's, Browse's Manage, or a new
    /// private channel's); a private channel's members load.
    pub fn open_channel_settings(&mut self, id: &str) {
        let Some(live) = &self.live else { return };
        let Some(ip) = live.channel_settings(id) else {
            return;
        };
        if let Some(c) = ip.channel().filter(|c| c.private) {
            live.admin(AdminOperation::LoadMembers {
                space: ip.space.clone(),
                channel: Some(c.id.clone()),
            });
        }
        self.open_dialog(Dialog::manage(ip));
    }

    /// After a save: the rail, sidebar and header show the new name and
    /// privacy.
    pub(crate) fn settings_saved(&mut self, answer: &Answer) {
        match answer {
            Answer::SpaceSaved(space) => {
                if let Some(i) = self.ids.spaces.iter().position(|id| *id == space.id)
                    && let Some(s) = self.c.spaces.get_mut(i)
                {
                    s.name = space.name.clone();
                }
            }
            Answer::ChannelSaved(channel) => {
                if let Some(i) = self.ids.channels.iter().position(|id| *id == channel.id)
                    && let Some(c) = self.c.channels.get_mut(i)
                {
                    c.name = channel.name.clone();
                    c.private = channel.private;
                }
            }
            _ => return,
        }
        self.dirty.rail = true;
        self.dirty.sidebar = true;
        self.dirty.header = true;
        self.dirty.composer = true;
        self.events |= crate::EV_SCREEN;
        self.need = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caper_client::model::{Channel, Space};

    fn member(id: &str, username: &str, owner: bool) -> Member {
        serde_json::from_value(serde_json::json!({
            "id": id, "username": username, "displayName": username.to_uppercase(), "owner": owner,
        }))
        .unwrap()
    }

    fn base() -> InvitePeople {
        InvitePeople::new(
            "s1".into(),
            "Studio".into(),
            None,
            vec![("c1".into(), "general".into())],
            "https://caper.chat/invite/".into(),
            vec![member("u0", "owner", true), member("u1", "maya", false)],
        )
    }

    fn typed(d: &mut Dialog, text: &str) {
        let ip = d.inviting.as_mut().unwrap();
        ip.numbers[NAME as usize].text = gfx::wide(text);
        ip.numbers[NAME as usize].caret = text.encode_utf16().count();
        d.settings_name_changed();
    }

    #[test]
    fn a_channel_s_name_and_privacy_save_together_and_stay_open() {
        let ip = base().channel_settings("c9".into(), "design", false, "u0".into());
        let mut d = Dialog::manage(ip);
        assert_eq!(d.title(), "Channel settings");
        assert!(!d.unsaved() && !d.save_bar() && !d.members_shown());
        // The channel's members are its own; the space's are whom to grant.
        let c = d.settings_channel().unwrap();
        assert_eq!(c.space_members.len(), 2);
        assert!(d.inviting.as_ref().unwrap().members.is_empty());
        // Typed as on web: normalized, the trailing dash kept until saved.
        typed(&mut d, "Brand Refresh ");
        assert_eq!(d.name_text(), "brand-refresh-");
        assert!(d.save_bar());
        d.private = true;
        // Reset goes back to what's saved.
        d.manage_activate(Part::Reset);
        assert!(!d.unsaved() && !d.private);
        assert_eq!(d.name_text(), "design");
        // Saving checks the name first, as web's form does.
        typed(&mut d, "-");
        assert!(d.manage_activate(Part::SaveChanges).is_none());
        assert!(d.inviting.as_ref().unwrap().name_error.is_some());
        typed(&mut d, "brand-");
        d.private = true;
        let sent = d.manage_activate(Part::SaveChanges);
        assert!(matches!(
            sent,
            Some(AdminOperation::UpdateChannel { ref name, private: true, .. }) if name == "brand"
        ));
        assert!(d.busy);
        // Saved private: the dialog stays, on the saved channel, and who
        // has access starts loading.
        let saved: Channel = serde_json::from_value(serde_json::json!({
            "id": "c9", "spaceId": "s1", "name": "brand", "private": true, "joined": true,
        }))
        .unwrap();
        d.manage_answer(Answer::ChannelSaved(saved));
        assert!(!d.busy && !d.unsaved() && d.private);
        assert_eq!(d.name_text(), "brand");
        assert!(d.members_shown() && d.members_loading());
        assert_eq!(d.loading_text(), "Loading members\u{2026}");
        // Its members arrive; an invitation for another channel doesn't land.
        d.manage_answer(Answer::Members {
            channel: "c9".into(),
            members: vec![member("u1", "maya", false)],
            invitations: Vec::new(),
        });
        assert!(!d.members_loading());
        assert_eq!(d.member_name(0), "MAYA @maya");
        d.manage_answer(Answer::ChannelInvited {
            channel: "c8".into(),
            member: member("u2", "sam", false),
        });
        assert!(d.inviting.as_ref().unwrap().invitations.is_empty());
        typed_field(&mut d, "maya");
        assert_eq!(d.invite_submit(), None);
        assert_eq!(
            d.error.as_deref(),
            Some("This person already has access to this channel.")
        );
        d.manage_answer(Answer::ChannelInvited {
            channel: "c9".into(),
            member: member("u2", "sam", false),
        });
        assert_eq!(d.invitee_name(0), "SAM @sam \u{b7} Invited");
    }

    fn typed_field(d: &mut Dialog, text: &str) {
        d.field.text = gfx::wide(text);
    }

    #[test]
    fn the_space_s_name_saves_alone_and_failures_say_why_by_it() {
        let mut d = Dialog::manage(base().manage_space());
        assert_eq!(
            (d.title().as_str(), d.note().as_str()),
            ("Manage space", OWNER_NOTE)
        );
        assert_eq!(d.invite_heading(crate::invite_people::BY_NAME), "Members");
        assert_eq!(d.member_name(0), "OWNER @owner \u{b7} Owner");
        assert!(!d.can_save_name() && !d.save_bar());
        // Only surrounding spaces typed: nothing to save.
        typed(&mut d, " Studio ");
        assert!(!d.can_save_name() && d.unsaved());
        typed(&mut d, "  Studio B ");
        assert!(d.can_save_name());
        // Sent trimmed, as web sends it.
        let sent = d.manage_activate(Part::SaveName);
        assert!(matches!(
            sent,
            Some(AdminOperation::UpdateSpace { ref name, .. }) if name == "Studio B"
        ));
        assert!(d.manage_failed("space name taken"));
        assert!(!d.busy);
        assert_eq!(
            d.inviting.as_ref().unwrap().name_error.as_deref(),
            Some("space name taken")
        );
        // Editing the name clears it.
        typed(&mut d, "Studio C");
        assert!(d.inviting.as_ref().unwrap().name_error.is_none());
        let saved: Space = serde_json::from_value(serde_json::json!({
            "id": "s1", "name": "Studio C", "ownerId": "u0",
        }))
        .unwrap();
        d.manage_answer(Answer::SpaceSaved(saved));
        assert!(!d.unsaved());
        assert_eq!(d.space_name, "Studio C");
        // Another failure while nothing of the dialog's is in flight isn't
        // the dialog's.
        assert!(!d.manage_failed("other"));
    }
}
