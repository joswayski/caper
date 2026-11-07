//! Invite people's Custom links, as web's "Create a separate link": Expire
//! after (an amount and its unit, or Never), Max uses (empty for no
//! limit), Opens in (a public channel), Create link, then the space's live
//! links, each with Copy and Deactivate (confirmed first).
use crate::composer::{Composer, LINE};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::invite_people::*;
use crate::live::Live;
use crate::overlay::{Dialog, FIELD_PAD_X, Part, Rect, Tone, height, layout, width};
use crate::text;
use crate::theme::*;
use caper_client::invites::{self, EXPIRY_UNITS, ExpiryUnit};
use caper_client::worker::AccountOperation;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP;
use windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY;
use windows::Win32::UI::WindowsAndMessaging::SetTimer;
use windows::core::Result;

/// `Part::Number`s.
pub const AMOUNT: u8 = 0;
pub const USES: u8 = 1;
/// Manage space's and Channel settings' name field.
pub const NAME: u8 = 2;

const AMOUNT_W: f32 = 72.0;
const CONTROL_H: f32 = 38.0;
const BUTTON_H: f32 = 36.0;
const ROW_H: f32 = 52.0;
const CHOICE_H: f32 = 36.0;

pub const CUSTOM_GUIDE_TEXT: &str =
    "Choose different limits or a landing channel. Existing links keep their settings.";

/// A number field holding `text`, its caret at the end.
pub fn number(text: &str) -> Composer {
    let text = gfx::wide(text);
    Composer {
        caret: text.len(),
        anchor: text.len(),
        text,
        ..Default::default()
    }
}

fn digit(c: &u16) -> bool {
    (u16::from(b'0')..=u16::from(b'9')).contains(c)
}

/// "caper.chat/invite/AbCdE12345", as web lists links.
fn bare(url: &str) -> &str {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
}

impl Part {
    /// A popup's item, drawn over the body and hit before it.
    pub(crate) fn popup(self) -> bool {
        matches!(self, Part::LinkAction(_) | Part::LandingChoice(_))
    }

    /// One of Custom links' controls.
    pub(crate) fn custom(self) -> bool {
        matches!(
            self,
            Part::Custom
                | Part::Number(_)
                | Part::Unit(_)
                | Part::Landing
                | Part::LandingChoice(_)
                | Part::CreateLink
                | Part::LinkCopy(_)
                | Part::LinkRevoke(_)
        )
    }
}

impl InvitePeople {
    /// Where a new link opens: the chosen public channel, else the first.
    pub fn landing_channel(&self) -> Option<String> {
        self.landing
            .clone()
            .filter(|id| self.public.iter().any(|(p, _)| p == id))
            .or_else(|| self.public.first().map(|(id, _)| id.clone()))
    }

    pub fn landing_label(&self) -> String {
        let id = self.landing_channel();
        self.public
            .iter()
            .find(|(p, _)| Some(p) == id.as_ref())
            .map_or_else(|| "No public channels".into(), |(_, n)| format!("#{n}"))
    }
}

impl Dialog {
    /// The number field with the keyboard, if one has it.
    pub(crate) fn number_focus(&self) -> Option<u8> {
        self.inviting.as_ref().and_then(|ip| ip.focus)
    }

    /// Whether the focused number field takes no edits now: the name while
    /// it saves, the others while a link request is in flight.
    pub(crate) fn number_locked(&self) -> bool {
        match self.number_focus() {
            Some(NAME) => self.busy,
            _ => self.inviting.as_ref().is_some_and(|ip| ip.pending),
        }
    }

    pub(crate) fn number_mut(&mut self) -> Option<&mut Composer> {
        let ip = self.inviting.as_deref_mut()?;
        let k = ip.focus?;
        Some(&mut ip.numbers[k as usize])
    }

    /// The editable text behind a part: a number field's, else the field's.
    pub(crate) fn field_for(&self, part: Part) -> &Composer {
        match (part, self.inviting.as_deref()) {
            (Part::Number(k), Some(ip)) => &ip.numbers[k as usize],
            _ => &self.field,
        }
    }

    pub(crate) fn field_for_mut(&mut self, part: Part) -> &mut Composer {
        match (part, self.inviting.as_deref_mut()) {
            (Part::Number(k), Some(ip)) => &mut ip.numbers[k as usize],
            _ => &mut self.field,
        }
    }

    /// A focused number field's editing keys (and Ctrl+C); false for other
    /// keys or while a link request is in flight.
    pub(crate) fn number_key(
        &mut self,
        hwnd: HWND,
        vk: VIRTUAL_KEY,
        ctrl: bool,
        shift: bool,
    ) -> bool {
        let pending = self.number_locked();
        let Some(f) = self.number_mut().filter(|_| !pending) else {
            return false;
        };
        if ctrl && vk == VIRTUAL_KEY(u16::from(b'C')) {
            let (a, b) = f.selection();
            if b > a {
                crate::set_clipboard(hwnd, &f.text[a..b]);
            }
            return true;
        }
        f.edit_key(hwnd, vk, ctrl, shift, false)
    }

    /// A typed character for the focused number field.
    pub(crate) fn number_char(&mut self, c: u16) -> bool {
        let pending = self.number_locked();
        match self.number_mut().filter(|_| !pending) {
            Some(f) => {
                f.insert(&[c]);
                true
            }
            None => false,
        }
    }

    /// After any change: the username field and a number field don't both
    /// keep the keyboard; number fields keep six digits, as on web, and
    /// editing one clears why the last link couldn't be made.
    pub(crate) fn invite_changed(&mut self) {
        let typing = self.field.focused;
        let Some(ip) = self.inviting.as_deref_mut() else {
            return;
        };
        if typing {
            ip.focus = None;
        }
        let focus = ip.focus;
        let mut changed = false;
        for (k, (f, seen)) in ip.numbers.iter_mut().zip(&mut ip.seen).enumerate() {
            // The name keeps what `space_settings` lets it.
            if k as u8 == NAME {
                f.focused = focus == Some(NAME);
                f.caret_on = true;
                continue;
            }
            if f.text.iter().any(|c| !digit(c)) || f.text.len() > 6 {
                let caret = f.text[..f.caret.min(f.text.len())]
                    .iter()
                    .filter(|c| digit(c))
                    .count()
                    .min(6);
                f.text = f.text.iter().copied().filter(digit).take(6).collect();
                f.caret = caret;
                f.anchor = caret;
            }
            f.focused = focus == Some(k as u8);
            f.caret_on = true;
            if f.text != *seen {
                *seen = f.text.clone();
                changed = true;
            }
        }
        if changed {
            ip.form_error = None;
        }
    }

    /// The new link's `maxAge` and `maxUses`, or None with why not.
    pub(crate) fn custom_link(&mut self) -> Option<(Option<u32>, Option<u32>)> {
        let ip = self.inviting.as_deref_mut()?;
        let amount = String::from_utf16_lossy(&ip.numbers[AMOUNT as usize].text);
        let uses = String::from_utf16_lossy(&ip.numbers[USES as usize].text);
        let parsed = invites::expiry_seconds(&amount, ip.unit)
            .and_then(|age| Ok((age, invites::max_uses(&uses)?)));
        match parsed {
            Ok(limits) => {
                ip.form_error = None;
                Some(limits)
            }
            Err(e) => {
                ip.form_error = Some(e.into());
                None
            }
        }
    }

    pub(crate) fn custom_line(&self, k: u8) -> String {
        match k {
            CUSTOM_GUIDE => CUSTOM_GUIDE_TEXT.into(),
            EXPIRE_LABEL => "Expire after".into(),
            USES_LABEL => "Max uses".into(),
            OPENS_LABEL => "Opens in".into(),
            _ => self
                .inviting
                .as_ref()
                .and_then(|ip| ip.form_error.clone())
                .unwrap_or_default(),
        }
    }

    /// Custom links' parts' accessible names, as web labels its controls.
    pub fn custom_name(&self, part: Part) -> String {
        let Some(ip) = self.inviting.as_deref() else {
            return String::new();
        };
        let link = |k: usize| ip.links.get(k);
        match part {
            Part::Custom => "Custom links".into(),
            Part::Number(AMOUNT) => "Expire after amount".into(),
            Part::Number(NAME) => self.manage_label().into(),
            Part::Number(_) => "Max uses".into(),
            Part::Unit(k) => EXPIRY_UNITS
                .get(k as usize)
                .map_or_else(String::new, |(u, l)| {
                    let selected = if *u == ip.unit { ", selected" } else { "" };
                    format!("Expire after unit: {l}{selected}")
                }),
            Part::Landing => format!("Opens in: {}", ip.landing_label()),
            Part::LandingChoice(k) => ip.public.get(k).map_or_else(String::new, |(id, n)| {
                let selected = if ip.landing_channel().as_ref() == Some(id) {
                    ", selected"
                } else {
                    ""
                };
                format!("#{n}{selected}")
            }),
            Part::CreateLink => self.create_link_label().into(),
            Part::LinkRow(k) => link(k).map_or_else(String::new, |l| {
                format!(
                    "{}, {}",
                    bare(&ip.url(&l.code)),
                    invites::link_meta(l, chrono::Utc::now())
                )
            }),
            Part::LinkCopy(k) => self.link_copy_label(k).into(),
            Part::LinkRevoke(_) => "Deactivate".into(),
            _ => String::new(),
        }
    }

    fn create_link_label(&self) -> &'static str {
        if self.inviting.as_ref().is_some_and(|ip| ip.pending) {
            "Creating\u{2026}"
        } else {
            "Create link"
        }
    }

    fn link_copy_label(&self, k: usize) -> &'static str {
        match self.inviting.as_deref() {
            Some(ip)
                if ip
                    .links
                    .get(k)
                    .is_some_and(|l| ip.copied.as_ref() == Some(&l.code)) =>
            {
                "Copied"
            }
            _ => "Copy",
        }
    }

    /// Whether a click at (x, y) on `part` only closes a popup or, off the
    /// card, a confirmation.
    pub(crate) fn invite_click_off(&self, part: Option<Part>, x: f32, y: f32) -> bool {
        let Some(ip) = self.inviting.as_deref() else {
            return false;
        };
        let own = matches!(
            part,
            Some(Part::More | Part::Landing | Part::LinkAction(_) | Part::LandingChoice(_))
        );
        (ip.menu || ip.landing_open) && !own
            || (ip.confirm.is_some() || (ip.removing.is_some() || ip.deleting) && !self.busy)
                && part.is_none()
                && !self.on_card(x, y)
    }

    /// The Custom links button (web's summary), then, open, the form and
    /// the space's links, from `y`; the Opens in choices go to `popups`.
    pub(crate) fn relayout_custom(
        &self,
        g: &Gfx,
        inner: f32,
        mut y: f32,
        parts: &mut Vec<(Part, Rect)>,
        popups: &mut Vec<(Part, Rect)>,
    ) -> Result<f32> {
        let Some(ip) = self.inviting.as_deref() else {
            return Ok(y);
        };
        // Manage space always offers Custom links, as web's disclosure.
        if ip.manage.is_none() && !ip.settings_open && (ip.default.is_some() || ip.pending) {
            return Ok(y);
        }
        let text = |s: &str, format| -> Result<f32> { Ok(height(&layout(g, s, format, inner)?)) };
        y += 14.0;
        let l = layout(g, "Custom links", &g.formats.button, 1e6)?;
        parts.push((Part::Custom, (0.0, y, width(&l) + 24.0 + 22.0, BUTTON_H)));
        y += BUTTON_H;
        if !ip.settings_open {
            return Ok(y);
        }
        y += 14.0;
        let h = text(&self.invite_heading(CUSTOM_HEAD), &g.formats.direct_name)?;
        parts.push((Part::Heading(CUSTOM_HEAD), (0.0, y, inner, h)));
        y += h + 6.0;
        let h = text(CUSTOM_GUIDE_TEXT, &g.formats.direct_note)?;
        parts.push((Part::Line(CUSTOM_GUIDE), (0.0, y, inner, h)));
        y += h + 12.0;
        // Expire after: the amount (none for Never) and its unit.
        let h = text("Expire after", &g.formats.direct_note)?;
        parts.push((Part::Line(EXPIRE_LABEL), (0.0, y, inner, h)));
        y += h + 6.0;
        let mut x = 0.0;
        if ip.unit != ExpiryUnit::Never {
            parts.push((Part::Number(AMOUNT), (0.0, y, AMOUNT_W, CONTROL_H)));
            x = AMOUNT_W + 8.0;
        }
        for (k, (_, label)) in EXPIRY_UNITS.iter().enumerate() {
            let bw = width(&layout(g, label, &g.formats.button, 1e6)?) + 20.0;
            parts.push((Part::Unit(k as u8), (x, y, bw, CONTROL_H)));
            x += bw + 4.0;
        }
        y += CONTROL_H + 12.0;
        // Max uses and Opens in, side by side.
        let colw = ((inner - 12.0) / 2.0).floor();
        let h = text("Max uses", &g.formats.direct_note)?;
        parts.push((Part::Line(USES_LABEL), (0.0, y, colw, h)));
        parts.push((Part::Line(OPENS_LABEL), (colw + 12.0, y, colw, h)));
        y += h + 6.0;
        parts.push((Part::Number(USES), (0.0, y, colw, CONTROL_H)));
        parts.push((Part::Landing, (colw + 12.0, y, colw, CONTROL_H)));
        if ip.landing_open {
            let mut py = y + CONTROL_H + 4.0 + 6.0;
            for k in 0..ip.public.len() {
                popups.push((
                    Part::LandingChoice(k),
                    (colw + 18.0, py, colw - 12.0, CHOICE_H),
                ));
                py += CHOICE_H;
            }
        }
        y += CONTROL_H + 12.0;
        let bw = width(&layout(g, "Creating\u{2026}", &g.formats.button, 1e6)?) + 24.0;
        let bw = bw.max(112.0);
        parts.push((Part::CreateLink, (inner - bw, y, bw, BUTTON_H)));
        y += BUTTON_H;
        if let Some(e) = &ip.form_error {
            y += 8.0;
            let h = text(e, &g.formats.error)?;
            parts.push((Part::Line(FORM_ERROR), (0.0, y, inner, h)));
            y += h;
        }
        let copy_w = width(&layout(g, "Copied", &g.formats.button, 1e6)?) + 24.0;
        let revoke_w = width(&layout(g, "Deactivate", &g.formats.button, 1e6)?) + 24.0;
        // A summary wraps beside the buttons, and its row grows to fit it.
        let tw = (inner - 44.0 - revoke_w - copy_w).max(1.0);
        let now = chrono::Utc::now();
        for (k, link) in ip.links.iter().enumerate() {
            y += 8.0;
            let meta = layout(
                g,
                &invites::link_meta(link, now),
                &g.formats.direct_note,
                tw,
            )?;
            let name = layout(g, bare(&ip.url(&link.code)), &g.formats.direct_name, 1e6)?;
            let row_h = (height(&name) + height(&meta) + 16.0).max(ROW_H);
            parts.push((Part::LinkRow(k), (0.0, y, inner, row_h)));
            let by = y + (row_h - 32.0) / 2.0;
            let rx = inner - 12.0 - revoke_w;
            parts.push((Part::LinkCopy(k), (rx - 8.0 - copy_w, by, copy_w, 32.0)));
            parts.push((Part::LinkRevoke(k), (rx, by, revoke_w, 32.0)));
            y += row_h;
        }
        Ok(y)
    }

    /// The number fields' layouts, before the rest is laid out.
    pub(crate) fn relayout_numbers(&mut self, g: &Gfx, inner: f32) -> Result<()> {
        let colw = ((inner - 12.0) / 2.0).floor();
        let name_w = self.name_field_w(g, inner)?;
        if let Some(ip) = self.inviting.as_deref_mut() {
            for (k, w) in [(AMOUNT, AMOUNT_W), (USES, colw), (NAME, name_w)] {
                ip.numbers[k as usize].relayout_with(
                    g,
                    &g.formats.composer,
                    15.0,
                    w - 2.0 * FIELD_PAD_X,
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn draw_custom_part(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        part: Part,
        (x, y, w, h): Rect,
    ) -> Result<()> {
        let Some(ip) = self.inviting.as_deref() else {
            return Ok(());
        };
        let enabled = self.invite_enabled(part);
        let lit = enabled && self.hover == Some(part);
        let boxed = |stroke: &ID2D1SolidColorBrush| {
            let body = rect(x, y, x + w, y + h);
            fill_round(dc, body, 7.0, &g.brushes.composer);
            stroke_round(dc, body, 7.0, stroke);
        };
        let border = |focused: bool| {
            if focused {
                g.solid(TERRACOTTA)
            } else if lit {
                g.solid(TERRACOTTA_BRIGHT)
            } else {
                g.brushes.border.clone()
            }
        };
        match part {
            Part::Custom => {
                self.button(g, dc, part, "", (x, y, w, h), Tone::Secondary)?;
                let l = layout(g, "Custom links", &g.formats.button, 1e6)?;
                let color = if enabled {
                    &g.brushes.text
                } else {
                    &g.brushes.muted
                };
                text::draw(dc, &l, x + 12.0, y + (h - height(&l)) / 2.0, color);
                let (cx, cy) = (x + w - 20.0, y + h / 2.0);
                let r = rect(cx - 7.0, cy - 7.0, cx + 7.0, cy + 7.0);
                icons::draw(dc, Icon::ChevronDown, r, TEXT_MUTED, 1.0);
            }
            Part::Number(k) => {
                let f = &ip.numbers[k as usize];
                boxed(&border(f.focused));
                unsafe {
                    dc.PushAxisAlignedClip(
                        &rect(x + 2.0, y + 2.0, x + w - 2.0, y + h - 2.0),
                        D2D1_ANTIALIAS_MODE_ALIASED,
                    );
                }
                let hint = match k {
                    USES => Some("No limit"),
                    NAME if self.settings_channel().is_some() => Some("project-updates"),
                    NAME => Some("Gaming nights"),
                    _ => None,
                };
                let hint = hint
                    .map(|s| layout(g, s, &g.formats.composer, 1e6))
                    .transpose()?;
                f.draw_text(
                    g,
                    dc,
                    x + FIELD_PAD_X,
                    y + (h - LINE) / 2.0,
                    hint.as_ref().map(|l| (l, &g.brushes.muted)),
                );
                unsafe { dc.PopAxisAlignedClip() };
            }
            Part::Unit(k) => {
                let selected = EXPIRY_UNITS
                    .get(k as usize)
                    .is_some_and(|(u, _)| *u == ip.unit);
                let body = rect(x, y, x + w, y + h);
                let fill = if selected || lit {
                    &g.brushes.raised
                } else {
                    &g.brushes.composer
                };
                fill_round(dc, body, 7.0, fill);
                stroke_round(dc, body, 7.0, &border(selected));
                let label = EXPIRY_UNITS.get(k as usize).map_or("", |(_, l)| *l);
                let l = layout(g, label, &g.formats.button, w)?;
                let brush = if selected {
                    &g.brushes.text
                } else {
                    &g.brushes.muted
                };
                text::draw(
                    dc,
                    &l,
                    x + (w - width(&l)) / 2.0,
                    y + (h - height(&l)) / 2.0,
                    brush,
                );
            }
            Part::Landing => {
                boxed(&border(ip.landing_open));
                let l = layout(
                    g,
                    &ip.landing_label(),
                    &g.formats.composer,
                    (w - 40.0).max(1.0),
                )?;
                unsafe { l.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
                let brush = if enabled {
                    &g.brushes.text
                } else {
                    &g.brushes.muted
                };
                text::draw(dc, &l, x + FIELD_PAD_X, y + (h - height(&l)) / 2.0, brush);
                let (cx, cy) = (x + w - 18.0, y + h / 2.0);
                icons::draw(
                    dc,
                    Icon::ChevronDown,
                    rect(cx - 7.0, cy - 7.0, cx + 7.0, cy + 7.0),
                    TEXT_MUTED,
                    if enabled { 1.0 } else { 0.5 },
                );
            }
            Part::LandingChoice(k) => {
                if lit {
                    fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
                }
                let Some((id, name)) = ip.public.get(k) else {
                    return Ok(());
                };
                if ip.landing_channel().as_ref() == Some(id) {
                    icons::draw(
                        dc,
                        Icon::Check,
                        rect(x + 8.0, y + h / 2.0 - 7.0, x + 22.0, y + h / 2.0 + 7.0),
                        TEXT,
                        1.0,
                    );
                }
                let l = layout(g, &format!("#{name}"), &g.formats.dialog_note, w - 36.0)?;
                text::draw(
                    dc,
                    &l,
                    x + 30.0,
                    y + (h - height(&l)) / 2.0,
                    &g.brushes.text,
                );
            }
            Part::CreateLink => self.button(
                g,
                dc,
                part,
                self.create_link_label(),
                (x, y, w, h),
                Tone::Primary,
            )?,
            Part::LinkRow(k) => {
                let Some(link) = ip.links.get(k) else {
                    return Ok(());
                };
                let body = rect(x, y, x + w, y + h);
                fill_round(dc, body, RADIUS, &g.brushes.raised);
                stroke_round(dc, body, RADIUS, &g.brushes.border);
                // The text stops before the row's buttons.
                let right = self
                    .part_rect(Part::LinkCopy(k))
                    .map_or(x + w, |(bx, ..)| bx)
                    - 12.0;
                let tw = (right - (x + 12.0)).max(1.0);
                let url = ip.url(&link.code);
                let name = layout(g, bare(&url), &g.formats.direct_name, tw)?;
                let meta = invites::link_meta(link, chrono::Utc::now());
                let meta = layout(g, &meta, &g.formats.direct_note, tw)?;
                unsafe {
                    name.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                    dc.PushAxisAlignedClip(&rect(x, y, right, y + h), D2D1_ANTIALIAS_MODE_ALIASED);
                }
                let top = y + (h - height(&name) - height(&meta)) / 2.0;
                text::draw(dc, &name, x + 12.0, top, &g.brushes.text);
                text::draw(dc, &meta, x + 12.0, top + height(&name), &g.brushes.muted);
                unsafe { dc.PopAxisAlignedClip() };
            }
            Part::LinkCopy(k) => self.button(
                g,
                dc,
                part,
                self.link_copy_label(k),
                (x, y, w, h),
                Tone::Secondary,
            )?,
            Part::LinkRevoke(_) => {
                self.button(g, dc, part, "Deactivate", (x, y, w, h), Tone::Secondary)?
            }
            _ => {}
        }
        Ok(())
    }

    /// Clicks and UIA Invoke on Custom links' controls. `landing` is
    /// whether Opens in's choices were open (the click closed them).
    pub(crate) fn custom_activate(&mut self, part: Part, live: &Live, hwnd: HWND, landing: bool) {
        if !matches!(part, Part::Number(_))
            && let Some(ip) = self.inviting.as_deref_mut()
        {
            ip.focus = None;
        }
        if part == Part::CreateLink {
            let Some((max_age, max_uses)) = self.custom_link() else {
                return;
            };
            let Some(ip) = self.inviting.as_deref_mut() else {
                return;
            };
            self.request += 1;
            ip.pending = true;
            live.account(AccountOperation::CreateInviteLink {
                space: ip.space.clone(),
                request: self.request,
                channel: ip.landing_channel(),
                max_age,
                max_uses,
            });
            return;
        }
        let Some(ip) = self.inviting.as_deref_mut() else {
            return;
        };
        match part {
            Part::Custom => ip.settings_open = !ip.settings_open,
            Part::Number(k) => {
                ip.focus = Some(k);
                let f = &mut ip.numbers[k as usize];
                f.caret = f.text.len();
                f.anchor = f.caret;
                self.field.focused = false;
            }
            Part::Unit(k) => {
                if let Some((unit, _)) = EXPIRY_UNITS.get(k as usize) {
                    ip.unit = *unit;
                    ip.form_error = None;
                }
            }
            Part::Landing => ip.landing_open = !landing,
            Part::LandingChoice(k) => {
                if let Some((id, _)) = ip.public.get(k) {
                    ip.landing = Some(id.clone());
                }
            }
            Part::LinkCopy(k) => {
                if let Some(link) = ip.links.get(k) {
                    crate::set_clipboard(hwnd, &gfx::wide(&ip.url(&link.code)));
                    ip.copied = Some(link.code.clone());
                    unsafe {
                        SetTimer(Some(hwnd), TIMER_COPIED, COPIED_MS, None);
                    }
                }
            }
            Part::LinkRevoke(k) => {
                if let Some(link) = ip.links.get(k) {
                    ip.confirm = Some((link.code.clone(), false));
                    self.field.focused = false;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dialog() -> Dialog {
        let ip = InvitePeople::new(
            "s1".into(),
            "Studio".into(),
            Some("c2".into()),
            vec![
                ("c1".into(), "general".into()),
                ("c2".into(), "feedback".into()),
            ],
            "https://caper.chat/invite/".into(),
            Vec::new(),
        );
        let mut d = Dialog::invite_people(ip);
        d.field.focused = false;
        d
    }

    fn set(d: &mut Dialog, k: u8, text: &str) {
        d.inviting.as_mut().unwrap().numbers[k as usize] = number(text);
        d.invite_changed();
    }

    #[test]
    fn a_custom_link_starts_with_web_s_defaults_and_checks_its_limits() {
        let mut d = dialog();
        // 30 days, 100 joins, in the first public channel even when opened
        // from another, as on web.
        assert_eq!(d.custom_link(), Some((Some(2_592_000), Some(100))));
        let ip = d.inviting.as_ref().unwrap();
        assert_eq!(ip.landing_channel().as_deref(), Some("c1"));
        assert_eq!(d.custom_name(Part::Landing), "Opens in: #general");
        assert_eq!(
            d.custom_name(Part::Unit(2)),
            "Expire after unit: days, selected"
        );
        // Only digits stay, six at most; the caret stays after what was typed.
        let f = &mut d.inviting.as_mut().unwrap().numbers[USES as usize];
        f.text = gfx::wide("1a2b3");
        f.caret = 4;
        d.invite_changed();
        let f = &d.inviting.as_ref().unwrap().numbers[USES as usize];
        assert_eq!(
            (String::from_utf16_lossy(&f.text).as_str(), f.caret),
            ("123", 2)
        );
        set(&mut d, USES, "123456789");
        assert_eq!(d.inviting.as_ref().unwrap().numbers[1].text.len(), 6);
        // Out of range says why, and editing again clears it.
        set(&mut d, USES, "20000");
        assert_eq!(d.custom_link(), None);
        assert_eq!(d.custom_line(FORM_ERROR), invites::USES_ERROR);
        set(&mut d, USES, "");
        assert!(d.inviting.as_ref().unwrap().form_error.is_none());
        set(&mut d, AMOUNT, "0");
        assert_eq!(d.custom_link(), None);
        assert_eq!(d.custom_line(FORM_ERROR), invites::EXPIRY_ERROR);
        // Never sends no expiry, whatever the amount says; empty uses is no limit.
        d.inviting.as_mut().unwrap().unit = ExpiryUnit::Never;
        assert_eq!(d.custom_link(), Some((None, None)));
    }

    #[test]
    fn a_created_link_leads_the_list_and_stale_answers_are_dropped() {
        let mut d = dialog();
        d.request = 7;
        d.inviting.as_mut().unwrap().pending = true;
        let link = |code: &str| -> caper_client::invites::InviteLink {
            serde_json::from_value(serde_json::json!({
                "code": code, "channel": null,
                "createdBy": { "username": "owner", "displayName": "Owner" },
                "uses": 2, "maxUses": null, "expiresAt": null,
                "createdAt": "2026-11-07T00:00:00Z",
            }))
            .unwrap()
        };
        d.invite_answer(Answer::Created {
            space: "s1".into(),
            request: 6,
            result: Ok(link("OLD0000001")),
        });
        assert!(d.inviting.as_ref().unwrap().links.is_empty());
        d.invite_answer(Answer::Created {
            space: "s1".into(),
            request: 7,
            result: Ok(link("AbCdE12345")),
        });
        let ip = d.inviting.as_ref().unwrap();
        assert!(!ip.pending);
        assert_eq!(
            d.custom_name(Part::LinkRow(0)),
            "caper.chat/invite/AbCdE12345, 2 uses \u{b7} Never expires"
        );
        assert_eq!(d.custom_name(Part::CreateLink), "Create link");
    }
}
