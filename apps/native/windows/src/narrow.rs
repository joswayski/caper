//! The shared 760-DIP shell: Browse and the conversation take turns;
//! members overlay below the header and a thread replaces the conversation.
use crate::call::{Card, VPart};
use crate::gfx::{self, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::overlay::Rect;
use crate::theme::*;
use crate::{App, Dirty, EV_FOCUS, EV_SCREEN, text};
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

pub const BREAKPOINT: f32 = 760.0;
const STRIP_H: f32 = 56.0;

#[derive(Default)]
pub struct State {
    pub browsing: bool,
    pub members: bool,
    pub menu: bool,
    pub focus: Option<Part>,
    hover: Option<Part>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Browse,
    CloseBrowse,
    Heading,
    Pins,
    Members,
    CloseMembers,
    Call,
    VoiceAction,
}

pub fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

impl App {
    pub fn is_narrow(&self) -> bool {
        self.w <= BREAKPOINT
    }

    pub fn browsing(&self) -> bool {
        self.is_narrow() && (self.narrow.browsing || !self.c.has_conversation())
    }

    pub fn navigation_shown(&self) -> bool {
        !self.is_narrow() || self.browsing()
    }

    pub fn chat_shown(&self) -> bool {
        if self.is_narrow() {
            !self.browsing() && self.thread.is_none()
        } else {
            self.chat_width() > 0.0
        }
    }

    /// egui clips content to the space remaining after its side panels. Keep
    /// measurement usable without letting it paint or hit through a neighbor.
    pub fn chat_width(&self) -> f32 {
        if self.is_narrow() {
            self.w
        } else {
            (self.w - self.list_x() - self.members_reserved() - self.thread_w()).max(0.0)
        }
    }

    pub fn thread_shown(&self) -> bool {
        self.thread.is_some() && !self.browsing()
    }

    pub fn members_tiled(&self) -> bool {
        !self.is_narrow() && self.w - self.list_x() >= 540.0
    }

    pub fn members_reserved(&self) -> f32 {
        if self.members_tiled() {
            self.members_w()
        } else {
            0.0
        }
    }

    pub fn members_y(&self) -> f32 {
        if self.members_tiled() {
            0.0
        } else if self.is_narrow() {
            62.0
        } else {
            HEADER_H
        }
    }

    pub fn members_h(&self) -> f32 {
        (self.h - self.members_y() - if self.is_narrow() { 8.0 } else { 0.0 }).max(0.0)
    }

    pub fn on_member_overlay(&self, x: f32, y: f32) -> bool {
        !self.members_tiled()
            && self.members_w() > 0.0
            && contains(
                (
                    self.members_x(),
                    self.members_y(),
                    self.members_w(),
                    self.members_h(),
                ),
                x,
                y,
            )
    }

    pub fn history_y(&self) -> f32 {
        HEADER_H + if self.strip_shown() { STRIP_H } else { 0.0 }
    }

    fn strip_shown(&self) -> bool {
        self.is_narrow()
            && self.chat_shown()
            && self.c.has_conversation()
            && (matches!(self.voice_card(), Some(Card::Call(_)))
                || (self.c.direct.is_none() && self.channel_voice(self.c.channel).is_some()))
    }

    fn strip_action(&self) -> VPart {
        if matches!(self.voice_card(), Some(Card::Call(_))) {
            VPart::Mute
        } else {
            VPart::Join(self.c.channel)
        }
    }

    pub fn narrow_parts(&self) -> Vec<Part> {
        use Part::*;
        let mut parts = Vec::new();
        if self.is_narrow() {
            if self.browsing() {
                if self.c.has_conversation() {
                    parts.push(CloseBrowse);
                }
            } else if self.thread.is_none() {
                parts.extend([Browse, Heading]);
                if self.strip_shown() {
                    parts.extend([Call, VoiceAction]);
                }
                if self.narrow.menu {
                    parts.push(Pins);
                    if self.c.direct.is_none() && !self.c.members.is_empty() {
                        parts.push(Members);
                    }
                }
            }
        }
        if self.members_w() > 0.0 && !self.members_tiled() {
            parts.push(CloseMembers);
        }
        parts
    }

    pub fn narrow_rect(&self, p: Part) -> Option<Rect> {
        if !self.narrow_parts().contains(&p) {
            return None;
        }
        use Part::*;
        Some(match p {
            Browse => (8.0, 5.0, 44.0, 44.0),
            CloseBrowse => (self.w - 52.0, 5.0, 44.0, 44.0),
            Heading => (60.0, 5.0, (self.w - 68.0).max(0.0), 44.0),
            Pins => (8.0, HEADER_H + 8.0, self.w - 16.0, 44.0),
            Members => (8.0, HEADER_H + 52.0, self.w - 16.0, 44.0),
            CloseMembers => (
                self.members_x() + self.member_width() - 40.0,
                self.members_y() + 5.0,
                36.0,
                44.0,
            ),
            Call => (0.0, HEADER_H, self.w - 64.0, STRIP_H),
            VoiceAction => (self.w - 56.0, HEADER_H + 6.0, 44.0, 44.0),
        })
    }

    pub fn narrow_name(&self, p: Part) -> String {
        use Part::*;
        match p {
            Browse => "Browse".into(),
            CloseBrowse => "Close navigation".into(),
            Heading => format!("{} options", self.c.title()),
            Pins => "Pinned messages".into(),
            Members => if self.narrow.members {
                "Hide member list"
            } else {
                "Show member list"
            }
            .into(),
            CloseMembers => "Close member list".into(),
            Call => match self.voice_card() {
                Some(Card::Call(c)) => format!("Voice in #{}: show the call", c.channel),
                _ => "Browse voice channels".into(),
            },
            VoiceAction => self.voice_name(self.strip_action()),
        }
    }

    pub fn narrow_enabled(&self, p: Part) -> bool {
        p != Part::VoiceAction || self.voice_enabled(self.strip_action())
    }

    pub fn focus_narrow(&mut self, p: Part) {
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        self.set_member_search_focus(false);
        self.clear_pane_focus();
        self.set_kb(None);
        self.narrow.focus = self.narrow_rect(p).map(|_| p);
        self.dirty.narrow = true;
        self.dirty.header = true;
        self.events |= EV_FOCUS;
        self.need = true;
    }

    pub fn set_browsing(&mut self, open: bool) {
        if !self.is_narrow() {
            return;
        }
        self.narrow.browsing = open;
        self.narrow.menu = false;
        self.narrow.members = false;
        self.stop_autoscroll();
        self.close_drop();
        self.close_menu();
        self.close_picker();
        self.close_gifs(false);
        self.close_member_search();
        self.pointer = None;
        self.update_hover();
        self.focus_narrow(if open {
            Part::CloseBrowse
        } else {
            Part::Browse
        });
        if !open && self.thread.is_some() {
            self.narrow.focus = None;
            self.set_thread_focus(true);
        }
        self.dirty = Dirty::all();
        self.events |= EV_SCREEN;
    }

    pub fn narrow_activate(&mut self, p: Part) {
        if !self.narrow_enabled(p) {
            return;
        }
        use Part::*;
        match p {
            Browse | Call => self.set_browsing(true),
            CloseBrowse => self.set_browsing(false),
            Heading => {
                self.narrow.menu = !self.narrow.menu;
                self.focus_narrow(if self.narrow.menu { Pins } else { Heading });
            }
            Pins => {
                self.narrow.menu = false;
                self.open_pins();
            }
            Members | CloseMembers => {
                self.narrow.menu = false;
                self.toggle_members();
                self.focus_narrow(if self.members_w() > 0.0 {
                    CloseMembers
                } else {
                    Heading
                });
            }
            VoiceAction => self.voice_activate(self.strip_action()),
        }
        self.dirty.narrow = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// Pointer routing precedes the underlying history. Drawers never pass a
    /// click, wheel or key to the conversation they cover.
    pub fn narrow_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<LRESULT> {
        if self
            .narrow
            .focus
            .is_some_and(|p| self.narrow_rect(p).is_none())
        {
            self.narrow.focus = None;
        }
        let pointer = matches!(
            msg,
            WM_MOUSEMOVE
                | WM_SETCURSOR
                | WM_LBUTTONDOWN
                | WM_LBUTTONUP
                | WM_LBUTTONDBLCLK
                | WM_RBUTTONDOWN
                | WM_CONTEXTMENU
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL
                | WM_MBUTTONDOWN
                | WM_MBUTTONUP
        );
        let hit = self
            .narrow_parts()
            .into_iter()
            .rev()
            .filter(|p| {
                !self.narrow.menu
                    || matches!(p, Part::Browse | Part::Heading | Part::Pins | Part::Members)
            })
            .find(|p| self.narrow_rect(*p).is_some_and(|r| contains(r, x, y)));
        if msg == WM_MOUSEMOVE {
            if self.narrow.hover != hit {
                self.narrow.hover = hit;
                self.dirty.narrow = true;
                self.dirty.header = true;
                self.need = true;
            }
        } else if msg == WM_SETCURSOR && hit.is_some() {
            unsafe {
                SetCursor(LoadCursorW(None, IDC_HAND).ok());
            }
            return Some(LRESULT(1));
        } else if msg == WM_LBUTTONDOWN {
            if let Some(p) = hit {
                self.focus_narrow(p);
                self.narrow_activate(p);
                return Some(LRESULT(0));
            }
            self.narrow.focus = None;
        }
        if msg == WM_KEYDOWN && !ctrl {
            match VIRTUAL_KEY(wp.0 as u16) {
                VK_ESCAPE if self.narrow.menu => {
                    self.narrow.menu = false;
                    self.focus_narrow(Part::Heading);
                }
                VK_ESCAPE
                    if self.members_w() > 0.0
                        && !self.members_tiled()
                        && self.member_search.is_none() =>
                {
                    self.narrow_activate(Part::CloseMembers);
                }
                VK_ESCAPE if self.browsing() && self.c.has_conversation() => {
                    self.set_browsing(false)
                }
                VK_RETURN | VK_SPACE if self.narrow.focus.is_some() => {
                    self.narrow_activate(self.narrow.focus?);
                }
                VK_TAB if self.is_narrow() && self.members_w() > 0.0 && !self.narrow.menu => {
                    if self.narrow.focus == Some(Part::CloseMembers) {
                        self.narrow.focus = None;
                        self.open_member_search();
                        self.set_member_search_focus(true);
                    } else {
                        self.focus_narrow(Part::CloseMembers);
                    }
                }
                VK_TAB
                    if self.is_narrow()
                        && (self.narrow.focus.is_some()
                            || (self.composer.focused && !shift)
                            || self.browsing()) =>
                {
                    let parts: Vec<_> = self
                        .narrow_parts()
                        .into_iter()
                        .filter(|p| !self.narrow.menu || matches!(p, Part::Pins | Part::Members))
                        .collect();
                    let pos = self
                        .narrow
                        .focus
                        .and_then(|p| parts.iter().position(|q| *q == p));
                    let next = match (pos, shift) {
                        (Some(i), true) => i.checked_sub(1),
                        (Some(i), false) => (i + 1 < parts.len()).then_some(i + 1),
                        (None, true) => parts.len().checked_sub(1),
                        _ => Some(0),
                    };
                    if let Some(p) = next.and_then(|i| parts.get(i)) {
                        self.focus_narrow(*p);
                    } else if self.narrow.menu || self.browsing() {
                        self.focus_narrow(*parts.first()?);
                    } else {
                        self.narrow.focus = None;
                        self.set_composer_focus(true);
                    }
                }
                _ => return None,
            }
            return Some(LRESULT(0));
        }
        if self.narrow.menu && pointer {
            if msg == WM_LBUTTONDOWN {
                self.narrow.menu = false;
                self.focus_narrow(Part::Heading);
            }
            return Some(LRESULT(0));
        }
        let at = if matches!(msg, WM_MOUSEWHEEL | WM_MOUSEHWHEEL) {
            self.pointer.unwrap_or((x, y))
        } else {
            (x, y)
        };
        if pointer && self.members_w() > 0.0 && !self.members_tiled() && at.1 >= HEADER_H {
            if self.on_member_overlay(at.0, at.1) {
                if let Some(r) = self.member_search_input(msg, wp, at.0, at.1, ctrl, shift) {
                    return r;
                }
                self.pointer = Some(at);
                self.update_hover();
                if msg == WM_LBUTTONDOWN
                    && let Some(i) = self.hover_member
                {
                    self.open_member_profile(i, at.0, at.1);
                }
                if msg == WM_SETCURSOR {
                    unsafe {
                        SetCursor(
                            LoadCursorW(
                                None,
                                if self.hover_member.is_some() {
                                    IDC_HAND
                                } else {
                                    IDC_ARROW
                                },
                            )
                            .ok(),
                        );
                    }
                }
                return Some(LRESULT(0));
            }
            if self.is_narrow() {
                if msg == WM_LBUTTONDOWN {
                    self.narrow_activate(Part::CloseMembers);
                }
                return Some(LRESULT(0));
            }
        }
        if self.browsing() && pointer {
            self.pointer = Some(at);
            self.update_hover();
            if let Some(r) = self.call_input(msg, wp, at.0, at.1, shift) {
                return Some(r);
            }
            match msg {
                WM_LBUTTONDOWN => {
                    if self.on_gear(at.0, at.1) {
                        self.toggle_account_menu();
                    } else if self.on_update_dock(at.0, at.1) {
                        self.open_update_review();
                    } else if let Some(p) = self.voice_at(at.0, at.1) {
                        self.voice_activate(p);
                    } else if self.on_call(at.0, at.1) {
                    } else if let Some(p) = self.hover_invite {
                        self.invite_activate(p);
                    } else if self.space_options_hover {
                        self.toggle_scope_menu(self.space_scope());
                    } else if let Some(i) = self.hover_channel {
                        if self.hover_channel_options {
                            self.toggle_scope_menu(self.channel_scope(i));
                        } else {
                            self.select_channel(i);
                        }
                    } else if let Some(p) = self.hover_direct {
                        self.direct_click(p);
                    } else if let Some(i) = self.hover_space {
                        self.select_space(i);
                    }
                }
                WM_SETCURSOR => unsafe {
                    SetCursor(LoadCursorW(None, IDC_HAND).ok());
                },
                _ => {}
            }
            return Some(LRESULT(0));
        }
        None
    }

    fn narrow_button(
        &self,
        dc: &ID2D1DeviceContext,
        p: Part,
        icon: Icon,
        label: Option<&str>,
    ) -> Result<()> {
        let Some((x, y, w, h)) = self.narrow_rect(p) else {
            return Ok(());
        };
        let g = &self.g;
        if self.narrow.hover == Some(p) {
            fill_round(dc, rect(x, y, x + w, y + h), RADIUS, &g.brushes.hover);
        }
        if self.narrow.focus == Some(p) {
            stroke_round(
                dc,
                rect(x + 1.0, y + 1.0, x + w - 1.0, y + h - 1.0),
                RADIUS,
                &g.brushes.terracotta,
            );
        }
        icons::draw(
            dc,
            icon,
            rect(
                x + 12.0,
                y + (h - 20.0) / 2.0,
                x + 32.0,
                y + (h + 20.0) / 2.0,
            ),
            TEXT,
            if self.narrow_enabled(p) { 1.0 } else { 0.4 },
        );
        if let Some(label) = label {
            self.narrow_label(dc, label, (x + 44.0, y, w - 52.0, h), false)?;
        }
        Ok(())
    }

    fn narrow_label(
        &self,
        dc: &ID2D1DeviceContext,
        label: &str,
        (x, y, w, h): Rect,
        muted: bool,
    ) -> Result<()> {
        let l = self.g.layout(
            &gfx::wide(label),
            if muted {
                &self.g.formats.time
            } else {
                &self.g.formats.channel
            },
            w.max(1.0),
        )?;
        unsafe {
            l.SetWordWrapping(windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP)?;
            dc.PushAxisAlignedClip(&rect(x, y, x + w, y + h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        }
        text::draw(
            dc,
            &l,
            x,
            y + (h - gfx::metrics(&l).height) / 2.0,
            if muted {
                &self.g.brushes.muted
            } else {
                &self.g.brushes.text
            },
        );
        unsafe {
            dc.PopAxisAlignedClip();
        }
        Ok(())
    }

    pub fn draw_narrow_header(&self, dc: &ID2D1DeviceContext) -> Result<()> {
        self.narrow_button(dc, Part::Browse, Icon::Menu, None)?;
        if let Some((x, y, w, h)) = self.narrow_rect(Part::Heading) {
            if self.narrow.hover == Some(Part::Heading) {
                fill_round(dc, rect(x, y, x + w, y + h), RADIUS, &self.g.brushes.hover);
            }
            if self.narrow.focus == Some(Part::Heading) {
                stroke_round(
                    dc,
                    rect(x + 1.0, y + 1.0, x + w - 1.0, y + h - 1.0),
                    RADIUS,
                    &self.g.brushes.terracotta,
                );
            }
            self.narrow_label(dc, &self.c.title(), (x + 8.0, y, w - 40.0, 24.0), false)?;
            let subtitle = if let Some((status, _)) = &self.status {
                status.clone()
            } else if self.c.direct.is_none() {
                let voice = self
                    .channel_voice(self.c.channel)
                    .map(|(n, _)| format!(" · {n} in voice"))
                    .unwrap_or_default();
                format!("{} members{voice}", self.c.listed_members)
            } else {
                "Direct message".into()
            };
            self.narrow_label(dc, &subtitle, (x + 8.0, y + 24.0, w - 40.0, 20.0), true)?;
            icons::draw(
                dc,
                Icon::ChevronDown,
                rect(x + w - 28.0, y + 14.0, x + w - 12.0, y + 30.0),
                TEXT_MUTED,
                1.0,
            );
        }
        Ok(())
    }

    pub fn draw_narrow(&mut self) -> Result<()> {
        if !std::mem::take(&mut self.dirty.narrow) {
            return Ok(());
        }
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        self.narrow_p.ensure(&self.g, px(self.w), px(self.h))?;
        self.narrow_p.offset(0.0, 0.0)?;
        self.narrow_p.draw(s, |dc| {
            unsafe {
                dc.Clear(Some(&gfx::color(0, 0.0)));
            }
            if self.browsing() {
                self.narrow_button(dc, Part::CloseBrowse, Icon::Close, None)?;
                self.draw_foot(&self.g, dc, self.h)?;
            }
            if self.strip_shown() {
                unsafe {
                    dc.FillRectangle(
                        &rect(0.0, HEADER_H, self.w, HEADER_H + STRIP_H),
                        &self.g.solid(0x19201B),
                    );
                }
                let (label, detail, icon) = match self.voice_card() {
                    Some(Card::Call(c)) => (
                        format!("#{}", c.channel),
                        format!(
                            "{} {}",
                            c.status.unwrap_or_default(),
                            c.timer.unwrap_or_default()
                        ),
                        if c.muted { Icon::MicOff } else { Icon::Mic },
                    ),
                    _ => (
                        self.channel_voice(self.c.channel)
                            .map(|(n, _)| format!("{n} talking"))
                            .unwrap_or_default(),
                        "Join voice".into(),
                        Icon::Headphones,
                    ),
                };
                icons::draw(
                    dc,
                    Icon::AudioLines,
                    rect(12.0, HEADER_H + 18.0, 32.0, HEADER_H + 38.0),
                    TEXT_MUTED,
                    1.0,
                );
                self.narrow_label(
                    dc,
                    &label,
                    (42.0, HEADER_H + 5.0, self.w - 112.0, 24.0),
                    false,
                )?;
                let calling = matches!(self.voice_card(), Some(Card::Call(_)));
                let channel = self.live.as_ref().and_then(|l| {
                    if calling {
                        l.voice.state.active_channel().map(str::to_owned)
                    } else {
                        self.ids.channels.get(self.c.channel).cloned()
                    }
                });
                let people = channel
                    .as_ref()
                    .zip(self.live.as_ref())
                    .map(|(id, l)| l.voice_people(id))
                    .unwrap_or_default();
                let slots = people.len().min(if calling { 5 } else { 2 });
                let mut left = 42.0;
                for (i, p) in people.iter().take(slots).enumerate() {
                    if i + 1 == slots && people.len() > slots {
                        self.narrow_label(
                            dc,
                            &format!("+{}", people.len() - i),
                            (left, HEADER_H + 30.0, 26.0, 20.0),
                            true,
                        )?;
                    } else {
                        let speaking = self
                            .live
                            .as_ref()
                            .zip(channel.as_deref())
                            .is_some_and(|(l, c)| l.voice_speaking(c, p));
                        crate::call::face(
                            &self.g,
                            dc,
                            p,
                            (left, HEADER_H + 31.0, 18.0, 18.0),
                            speaking,
                        )?;
                    }
                    left += 24.0;
                }
                self.narrow_label(
                    dc,
                    &detail,
                    (
                        left + 4.0,
                        HEADER_H + 29.0,
                        (self.w - 66.0 - left).max(0.0),
                        22.0,
                    ),
                    true,
                )?;
                if calling {
                    self.narrow_button(dc, Part::VoiceAction, icon, None)?;
                } else if let Some((x, y, w, h)) = self.narrow_rect(Part::VoiceAction) {
                    let action = self.g.solid(ACTION);
                    fill_round(
                        dc,
                        rect(x, y, x + w, y + h),
                        RADIUS,
                        if self.narrow_enabled(Part::VoiceAction) {
                            &action
                        } else {
                            &self.g.brushes.hover
                        },
                    );
                    let l = self
                        .g
                        .layout(&gfx::wide("Join"), &self.g.formats.channel, w)?;
                    text::draw(
                        dc,
                        &l,
                        x + (w - gfx::metrics(&l).width) / 2.0,
                        y + (h - gfx::metrics(&l).height) / 2.0,
                        &self.g.brushes.blackout,
                    );
                    if self.narrow.focus == Some(Part::VoiceAction) {
                        stroke_round(
                            dc,
                            rect(x + 1.0, y + 1.0, x + w - 1.0, y + h - 1.0),
                            RADIUS,
                            &self.g.brushes.terracotta,
                        );
                    }
                }
            }
            Ok(())
        })?;
        if !self.narrow.menu && self.narrow_rect(Part::CloseMembers).is_none() {
            return self.narrow_menu_p.offset(-100_000.0, 0.0);
        }
        self.narrow_menu_p.ensure(&self.g, px(self.w), px(self.h))?;
        self.narrow_menu_p.offset(0.0, 0.0)?;
        self.narrow_menu_p.draw(s, |dc| {
            unsafe {
                dc.Clear(Some(&gfx::color(0, 0.0)));
            }
            self.narrow_button(dc, Part::CloseMembers, Icon::Close, None)?;
            if self.narrow.menu {
                let h = if self.narrow_parts().contains(&Part::Members) {
                    104.0
                } else {
                    60.0
                };
                fill_round(
                    dc,
                    rect(4.0, HEADER_H + 4.0, self.w - 4.0, HEADER_H + h),
                    RADIUS,
                    &self.g.brushes.surface,
                );
                stroke_round(
                    dc,
                    rect(4.5, HEADER_H + 4.5, self.w - 4.5, HEADER_H + h - 0.5),
                    RADIUS,
                    &self.g.brushes.border,
                );
                self.narrow_button(dc, Part::Pins, Icon::Pin, Some("Pinned messages"))?;
                self.narrow_button(
                    dc,
                    Part::Members,
                    Icon::Users,
                    Some(&self.narrow_name(Part::Members)),
                )?;
            }
            Ok(())
        })
    }
}
