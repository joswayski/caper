//! Voice calls, as in the egui client: a joined channel's row offers Join
//! (Switch, from a call elsewhere) while it is hovered, and your call's
//! card sits at the bottom of the sidebar, above the account footer: where
//! the call is, its state and running time, who is in it, then Mute,
//! Deafen and Leave (Cancel while joining). A failed join or call shows its
//! error there instead, with a dismiss button. The shared
//! `caper_client::voice::Voice`, kept in `Live`, runs the call.
use crate::avatars::{self, Face};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::live::{CallView, VoiceAction};
use crate::overlay::Rect;
use crate::settings::FOOT_H;
use crate::theme::*;
use crate::{App, EV_FOCUS, EV_ROWS, text};
use caper_client::model::VoiceOccupant;
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

/// Redraws levels and their 180ms speaking release while in a call.
pub const TIMER_CALL: usize = 12;
const CARD_H: f32 = 136.0;
const PERSON_H: f32 = 42.0;
const AUDIO_H: f32 = 104.0;
const PAGE_H: f32 = 26.0;
const ERROR_H: f32 = 60.0;
/// egui's live call card and its colours.
const LIVE_CARD: u32 = 0x22382C;
const LIVE_CARD_BORDER: u32 = 0x4B8A60;
const VOICE_AMBER: u32 = 0xD9AB5C;
/// egui's Leave tone.
const LEAVE: u32 = 0x8C2F25;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VPart {
    /// Joined channel `i`'s Join or Switch.
    Join(usize),
    /// The card's text: the call, or the error.
    Status,
    Mute,
    Deafen,
    Leave,
    Dismiss,
    People,
    Person(usize),
    Volume,
    LocalMute,
    Previous,
    Next,
}

impl VPart {
    pub fn index(self) -> usize {
        match self {
            Self::Status => 0,
            Self::Mute => 1,
            Self::Deafen => 2,
            Self::Leave => 3,
            Self::Dismiss => 4,
            Self::People => 5,
            Self::Volume => 6,
            Self::LocalMute => 7,
            Self::Previous => 8,
            Self::Next => 9,
            Self::Join(i) => 32 + i,
            Self::Person(i) => i, // Separate UIA runtime-id group.
        }
    }
}

/// Expanded roster and a local listener's controls. The selected identity
/// survives roster reordering; leaving or switching calls resets it.
#[derive(Default)]
pub struct People {
    open: bool,
    start: usize,
    audio: Option<String>,
    generation: Option<u64>,
    pub focus: Option<VPart>,
    dragging: bool,
}

/// What the card shows.
#[derive(Clone, PartialEq, Debug)]
pub enum Card {
    Call(CallView),
    Error(String),
}

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

fn label(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    value: &str,
    (x, y, w, h): Rect,
    color: u32,
) -> Result<()> {
    let l = g.layout(&gfx::wide(value), &g.formats.direct_note, w.max(1.0))?;
    crate::emoji::apply(&l, value, 12.0, 1.0)?;
    unsafe {
        l.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        dc.PushAxisAlignedClip(&rect(x, y, x + w, y + h), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    }
    text::draw(
        dc,
        &l,
        x,
        y + (h - gfx::metrics(&l).height) / 2.0,
        &g.solid(color),
    );
    unsafe {
        dc.PopAxisAlignedClip();
    }
    Ok(())
}

pub fn face(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    p: &VoiceOccupant,
    (x, y, size, _): Rect,
    speaking: bool,
) -> Result<()> {
    let person = crate::live::person(&p.id, &p.name, p.avatar());
    let r = rect(x, y, x + size, y + size);
    let picture = avatars::draw(dc, &person.avatar, r, size * 0.3);
    if !picture {
        avatars::circle(
            g,
            dc,
            &person.avatar,
            (x + size / 2.0, y + size / 2.0, size / 2.0),
            person.color,
            &person.initials,
        )?;
    }
    if speaking {
        stroke_round(
            dc,
            rect(x - 2.0, y - 2.0, x + size + 2.0, y + size + 2.0),
            if picture {
                size * 0.3 + 2.0
            } else {
                size / 2.0 + 2.0
            },
            &g.solid(VOICE_GREEN),
        );
    }
    Ok(())
}

impl App {
    pub fn focus_voice(&mut self, p: VPart) {
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        self.set_member_search_focus(false);
        self.set_kb(None);
        self.call_ui.focus = Some(p);
        self.events |= EV_FOCUS;
        self.dirty.sidebar = true;
        self.dirty.narrow |= self.inline_voice_error();
        self.need = true;
    }

    pub fn set_call_volume(&mut self, gain: u16) {
        if let (Some(id), Some(playback)) =
            (self.call_audio().map(str::to_owned), self.call_playback())
        {
            self.live
                .as_mut()
                .unwrap()
                .participant_playback(&id, gain.min(200), playback.muted);
            self.voice_changed();
        }
    }

    fn call_scrub(&mut self, px: f32) {
        if let Some((x, _, w, _)) = self.voice_rect(VPart::Volume) {
            self.set_call_volume(
                (((px - x - 8.0) / (w - 16.0)).clamp(0.0, 1.0) * 200.0).round() as u16,
            );
        }
    }

    /// The card owns input only inside itself (or during a captured slider
    /// drag). It does not steal wheel/keyboard input from other surfaces.
    pub fn call_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        shift: bool,
    ) -> Option<LRESULT> {
        match msg {
            WM_LBUTTONDOWN | WM_RBUTTONDOWN => {
                if !self.on_call(x, y) {
                    self.call_ui.focus = None;
                    return None;
                }
                if let Some(p) = self.voice_at(x, y) {
                    self.focus_voice(p);
                    if p == VPart::Volume && msg == WM_LBUTTONDOWN {
                        self.call_ui.dragging = true;
                        unsafe {
                            SetCapture(self.hwnd);
                        }
                        self.call_scrub(x);
                    } else if msg == WM_LBUTTONDOWN || matches!(p, VPart::Person(_)) {
                        self.voice_activate(p);
                    }
                }
            }
            WM_MOUSEMOVE if self.call_ui.dragging => self.call_scrub(x),
            WM_LBUTTONUP if self.call_ui.dragging => {
                self.call_scrub(x);
                self.call_ui.dragging = false;
                unsafe {
                    let _ = ReleaseCapture();
                }
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                self.call_ui.dragging = false;
                return None;
            }
            WM_CONTEXTMENU if self.on_call(x, y) => {}
            WM_MOUSEWHEEL
                if self.call_ui.open && self.pointer.is_some_and(|(x, y)| self.on_call(x, y)) =>
            {
                let up = ((wp.0 >> 16) & 0xffff) as i16 > 0;
                self.voice_activate(if up { VPart::Previous } else { VPart::Next });
            }
            WM_KEYDOWN
                if self.call_ui.focus.is_some()
                    && !self.composer.focused
                    && !self.member_search.as_ref().is_some_and(|s| s.field.focused)
                    && !self.thread.as_ref().is_some_and(|t| t.composer.focused) =>
            {
                let p = self.call_ui.focus.unwrap();
                let key = VIRTUAL_KEY(wp.0 as u16);
                match key {
                    VK_ESCAPE => {
                        self.call_ui.open = false;
                        self.call_ui.audio = None;
                        self.call_ui.focus = None;
                        self.voice_changed();
                        self.set_composer_focus(true);
                    }
                    VK_TAB => {
                        let order: Vec<_> = self
                            .voice_parts()
                            .into_iter()
                            .filter(|p| *p != VPart::Status && self.voice_enabled(*p))
                            .collect();
                        let at = order.iter().position(|part| *part == p);
                        let next = at
                            .and_then(|k| if shift { k.checked_sub(1) } else { Some(k + 1) })
                            .and_then(|k| order.get(k));
                        if let Some(next) = next {
                            self.focus_voice(*next);
                        } else {
                            self.call_ui.focus = None;
                            self.set_composer_focus(true);
                        }
                    }
                    VK_LEFT | VK_RIGHT | VK_DOWN | VK_UP | VK_HOME | VK_END
                        if p == VPart::Volume =>
                    {
                        let v = self.call_playback()?.gain_percent;
                        let next = match key {
                            VK_HOME => 0,
                            VK_END => 200,
                            VK_LEFT | VK_DOWN => v.saturating_sub(if shift { 10 } else { 1 }),
                            _ => v.saturating_add(if shift { 10 } else { 1 }).min(200),
                        };
                        self.set_call_volume(next);
                    }
                    VK_RETURN | VK_SPACE => self.voice_activate(p),
                    _ => return None,
                }
            }
            _ => return None,
        }
        Some(LRESULT(0))
    }

    /// Compact faces leave at least half the row for its channel name.
    pub fn draw_voice_row(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        i: usize,
        right: f32,
        cy: f32,
    ) -> Result<f32> {
        let Some((live, id)) = self.live.as_ref().zip(self.ids.channels.get(i)) else {
            return Ok(right);
        };
        let people = live.voice_people(id);
        if people.is_empty() {
            return Ok(right);
        }
        let count = people.len().min(2);
        let extra = if people.len() > count { 26.0 } else { 0.0 };
        let left = right - count as f32 * 20.0 - extra;
        for (k, p) in people.iter().take(count).enumerate() {
            face(
                g,
                dc,
                p,
                (left + k as f32 * 20.0, cy - 9.0, 18.0, 18.0),
                live.voice_speaking(id, p),
            )?;
        }
        if people.len() > count {
            label(
                g,
                dc,
                &format!("+{}", people.len() - count),
                (right - extra + 2.0, cy - 10.0, extra - 2.0, 20.0),
                TEXT_MUTED,
            )?;
        }
        Ok(left - 8.0)
    }

    fn draw_call_people(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        let people = self.call_people();
        let Some(live) = &self.live else {
            return Ok(());
        };
        let local = |p| self.voice_rect(p).map(|(x, y, w, h)| (x - RAIL_W, y, w, h));
        let Some((x, y, w, h)) = local(VPart::People) else {
            return Ok(());
        };
        if self.voice_hover == Some(VPart::People) {
            fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
        }
        for (k, p) in people.iter().take(4).enumerate() {
            face(
                g,
                dc,
                p,
                (x + 4.0 + k as f32 * 24.0, y + 5.0, 24.0, 24.0),
                live.voice
                    .speaking(&p.id, p.muted, std::time::Instant::now()),
            )?;
        }
        let fx = x + 8.0 + people.len().min(4) as f32 * 24.0;
        label(
            g,
            dc,
            if self.call_ui.open { "Hide" } else { "People" },
            (fx, y, w - (fx - x), h),
            TEXT,
        )?;
        if !self.call_ui.open {
            return Ok(());
        }
        for k in self.call_range() {
            let p = &people[k];
            let Some((x, y, w, h)) = local(VPart::Person(k)) else {
                continue;
            };
            let own = p.id == live.voice.self_id;
            let selected = self.call_audio() == Some(&p.id);
            if selected || self.voice_hover == Some(VPart::Person(k)) {
                fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
            }
            let speaking = live
                .voice
                .speaking(&p.id, p.muted, std::time::Instant::now());
            face(g, dc, p, (x + 4.0, y + 7.0, 28.0, 28.0), speaking)?;
            let name = if own {
                format!("{} (you)", p.name)
            } else {
                p.name.clone()
            };
            label(
                g,
                dc,
                &name,
                (x + 40.0, y + 2.0, w - 78.0, 20.0),
                if speaking { TEXT } else { TEXT_MUTED },
            )?;
            let playback = live.voice.playback(&p.id);
            let status = if !own && playback.muted {
                "Muted for me"
            } else if p.deafened {
                "Deafened"
            } else if p.muted {
                "Mic muted"
            } else if speaking {
                "Speaking"
            } else {
                ""
            };
            label(
                g,
                dc,
                status,
                (x + 40.0, y + 22.0, w - 78.0, 16.0),
                TEXT_MUTED,
            )?;
            if !own {
                icons::draw(
                    dc,
                    Icon::Volume,
                    rect(x + w - 26.0, y + 13.0, x + w - 10.0, y + 29.0),
                    if selected { TEXT } else { TEXT_MUTED },
                    1.0,
                );
            }
        }
        for (p, title) in [(VPart::Previous, "Previous"), (VPart::Next, "Next")] {
            if let Some((x, y, w, h)) = local(p) {
                label(
                    g,
                    dc,
                    title,
                    (x + 8.0, y, w - 16.0, h),
                    if self.voice_enabled(p) { TEXT } else { BORDER },
                )?;
            }
        }
        if let Some(playback) = self.call_playback() {
            if let Some((x, y, w, _)) = local(VPart::Volume) {
                let name = people
                    .iter()
                    .find(|p| Some(p.id.as_str()) == self.call_audio())
                    .map_or("", |p| p.name.as_str());
                label(g, dc, name, (x, y, w - 44.0, 20.0), TEXT)?;
                label(
                    g,
                    dc,
                    &format!("{}%", playback.gain_percent),
                    (x + w - 40.0, y, 40.0, 20.0),
                    TEXT,
                )?;
                let left = x + 8.0;
                let knob = left + (w - 16.0) * f32::from(playback.gain_percent) / 200.0;
                fill_round(
                    dc,
                    rect(left, y + 33.0, x + w - 8.0, y + 37.0),
                    2.0,
                    &g.brushes.border,
                );
                fill_round(
                    dc,
                    rect(left, y + 33.0, knob, y + 37.0),
                    2.0,
                    &g.brushes.text,
                );
                fill_round(
                    dc,
                    rect(knob - 6.0, y + 29.0, knob + 6.0, y + 41.0),
                    6.0,
                    &g.brushes.text,
                );
            }
            if let Some((x, y, w, h)) = local(VPart::LocalMute) {
                let check = rect(x, y + 5.0, x + 16.0, y + 21.0);
                stroke_round(dc, check, 4.0, &g.brushes.text);
                if playback.muted {
                    fill_round(
                        dc,
                        rect(x + 4.0, y + 9.0, x + 12.0, y + 17.0),
                        2.0,
                        &g.brushes.text,
                    );
                }
                label(g, dc, "Mute for me", (x + 24.0, y, w - 24.0, h), TEXT)?;
                label(
                    g,
                    dc,
                    "Only changes what you hear.",
                    (x, y + h, w, 18.0),
                    TEXT_MUTED,
                )?;
            }
        }
        Ok(())
    }

    pub fn space_voice(&self, i: usize) -> Option<bool> {
        self.live
            .as_ref()
            .filter(|_| self.c.is_live())?
            .voice_badge(self.ids.spaces.get(i)?)
    }

    pub fn call_people(&self) -> Vec<VoiceOccupant> {
        let Some(live) = self.live.as_ref().filter(|_| self.c.is_live()) else {
            return vec![];
        };
        live.voice
            .state
            .active_channel()
            .map_or_else(Vec::new, |id| live.voice_people(id))
    }

    /// The open controls' identity must still belong to this call, and
    /// must never be your own microphone.
    fn call_audio(&self) -> Option<&str> {
        let live = self.live.as_ref()?;
        let id = self.call_ui.audio.as_deref()?;
        (live.in_voice()
            && id != live.voice.self_id
            && live.voice.participants.iter().any(|p| p.id == id))
        .then_some(id)
    }

    pub fn call_playback(&self) -> Option<caper_client::voice_media::TrackPlayback> {
        Some(self.live.as_ref()?.voice.playback(self.call_audio()?))
    }

    fn call_slots(&self) -> usize {
        if !self.call_ui.open {
            return 0;
        }
        let audio = if self.call_audio().is_some() {
            AUDIO_H
        } else {
            0.0
        };
        let available =
            self.h - FOOT_H - self.update_dock_h() - HEADER_H - 24.0 - CARD_H - PAGE_H - audio;
        ((available / PERSON_H).floor().clamp(1.0, 4.0) as usize).min(self.call_people().len())
    }

    fn call_range(&self) -> std::ops::Range<usize> {
        let n = self.call_people().len();
        let slots = self.call_slots();
        let start = self.call_ui.start.min(n.saturating_sub(slots));
        start..start + slots
    }

    fn call_extra(&self) -> f32 {
        if !self.call_ui.open {
            return 0.0;
        }
        self.call_slots() as f32 * PERSON_H
            + PAGE_H
            + if self.call_audio().is_some() {
                AUDIO_H
            } else {
                0.0
            }
    }

    pub fn voice_card(&self) -> Option<Card> {
        let live = self.live.as_ref().filter(|_| self.c.is_live())?;
        match live.call_view() {
            Some(call) => Some(Card::Call(call)),
            None => live.voice.error.clone().map(Card::Error),
        }
    }

    /// Whether the narrow conversation, rather than Browse's account area,
    /// shows the voice error, as egui, web, Android and Apple do on phones.
    pub fn inline_voice_error(&self) -> bool {
        self.is_narrow()
            && self.chat_shown()
            && self.c.has_conversation()
            && matches!(self.voice_card(), Some(Card::Error(_)))
    }

    /// The inline voice error's height and the gap under it; 0 without one.
    pub fn voice_error_h(&self) -> f32 {
        if self.inline_voice_error() {
            ERROR_H + 6.0
        } else {
            0.0
        }
    }

    /// The card (client DIPs), above the account footer, or a narrow
    /// conversation's voice error above the composer.
    pub fn call_rect(&self) -> Option<Rect> {
        if self.inline_voice_error() {
            let (bx, by, bw, _) = self.box_rect();
            let y = by - self.reply_h() - self.note_h() - self.voice_error_h();
            return Some((bx, y, bw, ERROR_H));
        }
        if !self.navigation_shown() {
            return None;
        }
        let h = match self.voice_card()? {
            Card::Call(_) => CARD_H + self.call_extra(),
            Card::Error(_) => ERROR_H,
        };
        let foot = (if self.foot_shown() { FOOT_H } else { 0.0 }) + self.update_dock_h();
        Some((
            RAIL_W + 8.0,
            self.h - foot - 8.0 - h,
            self.sidebar_w() - 16.0,
            h,
        ))
    }

    /// Joined channel `i`'s Join or Switch, and whether it can join now;
    /// None where you are in voice or joining.
    pub fn join_action(&self, i: usize) -> Option<(bool, bool)> {
        let id = self.ids.channels.get(i)?;
        let live = self.live.as_ref().filter(|_| self.c.is_live())?;
        match live.voice_action(id) {
            VoiceAction::Join { switching, enabled } => Some((switching, enabled)),
            _ => None,
        }
    }

    /// Channel `i`'s Join button (client DIPs), left of its options button.
    pub fn join_rect(&self, i: usize) -> Rect {
        let (x, y, w, h) = self.channel_rect(i);
        (x + w - 58.0, y + (h - 24.0) / 2.0, 24.0, 24.0)
    }

    /// The card's parts, in reading order.
    pub fn voice_parts(&self) -> Vec<VPart> {
        match self.voice_card() {
            None => Vec::new(),
            Some(Card::Call(_)) => {
                let mut parts = vec![VPart::Status, VPart::People];
                if self.call_ui.open {
                    parts.extend(self.call_range().map(VPart::Person));
                    parts.extend([VPart::Previous, VPart::Next]);
                    if self.call_audio().is_some() {
                        parts.extend([VPart::Volume, VPart::LocalMute]);
                    }
                }
                parts.extend([VPart::Mute, VPart::Deafen, VPart::Leave]);
                parts
            }
            Some(Card::Error(_)) => vec![VPart::Status, VPart::Dismiss],
        }
    }

    pub fn voice_rect(&self, p: VPart) -> Option<Rect> {
        if let VPart::Join(i) = p {
            return self
                .join_action(i)
                .filter(|_| self.navigation_shown())
                .map(|_| self.join_rect(i));
        }
        let call = matches!(self.voice_card()?, Card::Call(_));
        let (x, y, w, h) = self.call_rect()?;
        let row = y + h - 44.0;
        Some(match p {
            VPart::Status if call => (x + 12.0, y + 10.0, w - 24.0, 40.0),
            VPart::Status => (x + 12.0, y + 10.0, w - 56.0, h - 20.0),
            VPart::People => (x + 12.0, y + 50.0, w - 24.0, 34.0),
            VPart::Person(k) if self.call_ui.open && self.call_range().contains(&k) => (
                x + 12.0,
                y + 86.0 + (k - self.call_range().start) as f32 * PERSON_H,
                w - 24.0,
                PERSON_H,
            ),
            VPart::Previous | VPart::Next if self.call_ui.open => (
                x + 12.0
                    + if p == VPart::Next {
                        (w - 24.0) / 2.0
                    } else {
                        0.0
                    },
                y + 86.0 + self.call_slots() as f32 * PERSON_H,
                (w - 24.0) / 2.0,
                PAGE_H,
            ),
            VPart::Volume if self.call_audio().is_some() => {
                (x + 16.0, row - AUDIO_H + 4.0, w - 32.0, 48.0)
            }
            VPart::LocalMute if self.call_audio().is_some() => {
                (x + 16.0, row - 48.0, w - 32.0, 26.0)
            }
            VPart::Mute => (x + 12.0, row, 32.0, 32.0),
            VPart::Deafen => (x + 50.0, row, 32.0, 32.0),
            VPart::Leave => (x + w - 12.0 - 92.0, row, 92.0, 32.0),
            VPart::Dismiss => (x + w - 40.0, y + (h - 28.0) / 2.0, 28.0, 28.0),
            _ => return None,
        })
    }

    /// The control under (x, y): a hovered row's Join, or a card button.
    pub fn voice_at(&self, x: f32, y: f32) -> Option<VPart> {
        if let Some(card) = self.call_rect()
            && contains(card, x, y)
        {
            return self.voice_parts().into_iter().find(|&p| {
                p != VPart::Status && self.voice_rect(p).is_some_and(|r| contains(r, x, y))
            });
        }
        if !self.navigation_shown() {
            return None;
        }
        (0..self.c.channels.len())
            .find(|&i| contains(self.join_rect(i), x, y) && self.join_action(i).is_some())
            .map(VPart::Join)
    }

    /// Whether the pointer is on the card (which covers what is under it).
    pub fn on_call(&self, x: f32, y: f32) -> bool {
        self.call_rect().is_some_and(|r| contains(r, x, y))
    }

    pub fn voice_enabled(&self, p: VPart) -> bool {
        match p {
            VPart::Join(i) => self.join_action(i).is_some_and(|(_, on)| on),
            VPart::Person(k) => self
                .call_people()
                .get(k)
                .is_some_and(|p| self.live.as_ref().is_some_and(|l| p.id != l.voice.self_id)),
            VPart::Volume | VPart::LocalMute => self.call_audio().is_some(),
            VPart::Previous => self.call_range().start > 0,
            VPart::Next => self.call_range().end < self.call_people().len(),
            _ => true,
        }
    }

    /// A part's accessible name, as egui's labels word it.
    pub fn voice_name(&self, p: VPart) -> String {
        let card = self.voice_card();
        let call = match &card {
            Some(Card::Call(call)) => Some(call),
            _ => None,
        };
        match p {
            VPart::Join(i) => {
                let name = self.c.channels.get(i).map_or("", |c| c.name.as_str());
                match self.join_action(i) {
                    Some((true, _)) => format!("Switch voice to #{name}"),
                    _ => format!("Join voice in #{name}"),
                }
            }
            VPart::Status => match &card {
                Some(Card::Call(call)) => {
                    let mut s = format!("Your call in #{} \u{b7} {}", call.channel, call.space);
                    if let Some(status) = call.status {
                        s += &format!(", {status}");
                    }
                    if let Some(timer) = &call.timer {
                        s += &format!(", {timer}");
                    }
                    if !call.names.is_empty() {
                        s += &format!(", {}", call.names);
                    }
                    s
                }
                Some(Card::Error(e)) => e.clone(),
                None => String::new(),
            },
            VPart::Mute if call.is_some_and(|c| c.muted) => "Unmute microphone".into(),
            VPart::Mute => "Mute microphone".into(),
            VPart::Deafen if call.is_some_and(|c| c.deafened) => "Undeafen audio".into(),
            VPart::Deafen => "Deafen audio".into(),
            VPart::Leave if call.is_some_and(|c| c.joining) => "Cancel joining voice".into(),
            VPart::Leave => "Leave voice".into(),
            VPart::Dismiss => "Dismiss voice error".into(),
            VPart::People => format!(
                "{} call participants",
                if self.call_ui.open { "Hide" } else { "Show" }
            ),
            VPart::Person(k) => self.call_people().get(k).map_or_else(String::new, |p| {
                let live = self.live.as_ref().unwrap();
                let speaking = live
                    .voice
                    .speaking(&p.id, p.muted, std::time::Instant::now());
                let local = live.voice.playback(&p.id).muted;
                format!(
                    "{}{}, {}{}{}{}",
                    p.name,
                    if p.id == live.voice.self_id {
                        " (you)"
                    } else {
                        " audio controls"
                    },
                    if speaking { "speaking" } else { "not speaking" },
                    if p.muted { ", microphone muted" } else { "" },
                    if p.deafened { ", deafened" } else { "" },
                    if local { ", muted for me" } else { "" }
                )
            }),
            VPart::Volume | VPart::LocalMute => {
                let name = self
                    .call_people()
                    .into_iter()
                    .find(|p| Some(p.id.as_str()) == self.call_audio())
                    .map_or_else(String::new, |p| p.name);
                format!(
                    "{}: {name}",
                    if p == VPart::Volume {
                        "User volume"
                    } else {
                        "Mute for me"
                    }
                )
            }
            VPart::Previous => "Previous call participants".into(),
            VPart::Next => "Next call participants".into(),
        }
    }

    pub fn voice_activate(&mut self, p: VPart) {
        if !self.voice_enabled(p) {
            return;
        }
        match p {
            VPart::People => {
                self.call_ui.open = !self.call_ui.open;
                if !self.call_ui.open {
                    self.call_ui.audio = None;
                }
            }
            VPart::Person(k) => {
                if let Some(person) = self.call_people().get(k) {
                    self.call_ui.audio = (self.call_ui.audio.as_deref() != Some(&person.id))
                        .then(|| person.id.clone());
                }
            }
            VPart::Previous => {
                self.call_ui.start = self.call_range().start.saturating_sub(self.call_slots())
            }
            VPart::Next => self.call_ui.start = self.call_range().end,
            VPart::LocalMute => {
                if let (Some(id), Some(playback)) =
                    (self.call_audio().map(str::to_owned), self.call_playback())
                {
                    self.live.as_mut().unwrap().participant_playback(
                        &id,
                        playback.gain_percent,
                        !playback.muted,
                    );
                }
            }
            _ => {}
        }
        let id = match p {
            VPart::Join(i) => self.ids.channels.get(i).cloned(),
            _ => None,
        };
        let Some(live) = &mut self.live else { return };
        match p {
            VPart::Join(_) => {
                if let Some(id) = id {
                    live.join_voice(&id);
                }
            }
            VPart::Mute => live.toggle_mute(),
            VPart::Deafen => live.toggle_deafen(),
            VPart::Leave => live.leave_voice(),
            VPart::Dismiss => live.dismiss_voice_error(),
            _ => {}
        }
        self.voice_changed();
    }

    /// Voice changed: the sidebar redraws, and the call's timer runs only
    /// while a call is under way.
    pub fn voice_changed(&mut self) {
        let dragging = self.call_ui.dragging;
        let generation = self
            .live
            .as_ref()
            .filter(|l| l.in_voice())
            .map(|l| l.voice.state.generation);
        if generation != self.call_ui.generation {
            self.call_ui = People {
                generation,
                ..Default::default()
            };
        }
        if self.call_audio().is_none() {
            self.call_ui.audio = None;
            self.call_ui.dragging = false;
        }
        if dragging && !self.call_ui.dragging {
            unsafe {
                let _ = ReleaseCapture();
            }
        }
        if self
            .call_ui
            .focus
            .is_some_and(|p| !self.voice_parts().contains(&p))
        {
            self.call_ui.focus = match self.voice_card() {
                Some(Card::Call(_)) => Some(VPart::People),
                Some(Card::Error(_)) => Some(VPart::Dismiss),
                None => None,
            };
            self.events |= EV_FOCUS;
        }
        self.dirty.sidebar = true;
        self.dirty.rail = true;
        self.dirty.narrow = true;
        self.dirty.header = true;
        self.events |= EV_ROWS;
        self.need = true;
        let calling = self.live.as_ref().is_some_and(|l| l.in_voice());
        unsafe {
            if calling {
                SetTimer(Some(self.hwnd), TIMER_CALL, 80, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_CALL);
            }
        }
        self.audio_changed();
    }

    /// Tracks the pointer over voice's controls; nearing a Join prepares
    /// that join, as web does.
    pub fn track_voice_hover(&mut self) {
        let hover = self.pointer.and_then(|(x, y)| self.voice_at(x, y));
        if hover == self.voice_hover {
            return;
        }
        self.voice_hover = hover;
        self.dirty.sidebar = true;
        self.dirty.narrow |= self.inline_voice_error();
        self.need = true;
        if let Some(VPart::Join(i)) = hover
            && let Some(id) = self.ids.channels.get(i).cloned()
            && let Some(live) = &mut self.live
        {
            live.prepare_voice(&id);
        }
    }

    /// Draws the card on a surface whose left edge is `left` client DIPs:
    /// the sidebar, or the narrow layer for an inline error.
    pub fn draw_call(&self, g: &Gfx, dc: &ID2D1DeviceContext, left: f32) -> Result<()> {
        let (Some(card), Some((cx, cy, cw, ch))) = (self.voice_card(), self.call_rect()) else {
            return Ok(());
        };
        let x = cx - left;
        let r = rect(x, cy, x + cw, cy + ch);
        let local = |p: VPart| {
            self.voice_rect(p)
                .map(|(px, py, pw, ph)| (px - left, py, pw, ph))
        };
        match card {
            Card::Error(e) => {
                fill_round(dc, r, 10.0, &g.brushes.surface);
                stroke_round(dc, r, 10.0, &g.brushes.border);
                if let Some((tx, ty, tw, _)) = local(VPart::Status) {
                    let l = g.layout(&gfx::wide(&e), &g.formats.direct_note, tw)?;
                    let m = gfx::metrics(&l);
                    text::draw(
                        dc,
                        &l,
                        tx,
                        ty.max(cy + (ch - m.height) / 2.0),
                        &g.brushes.error,
                    );
                }
                if let Some((bx, by, bw, bh)) = local(VPart::Dismiss) {
                    let on = self.voice_hover == Some(VPart::Dismiss);
                    if on {
                        fill_round(dc, rect(bx, by, bx + bw, by + bh), 6.0, &g.brushes.hover);
                    }
                    icons::draw(
                        dc,
                        Icon::Close,
                        rect(bx + 6.0, by + 6.0, bx + bw - 6.0, by + bh - 6.0),
                        if on { TEXT } else { TEXT_MUTED },
                        1.0,
                    );
                }
            }
            Card::Call(call) => {
                fill_round(dc, r, 10.0, &g.solid(LIVE_CARD));
                stroke_round(dc, r, 10.0, &g.solid(LIVE_CARD_BORDER));
                let Some((tx, ty, tw, _)) = local(VPart::Status) else {
                    return Ok(());
                };
                // The waveform, #channel, and the timer or connection state.
                icons::draw(
                    dc,
                    Icon::AudioLines,
                    rect(tx, ty + 2.0, tx + 16.0, ty + 18.0),
                    VOICE_GREEN,
                    1.0,
                );
                let (state, tone) = match (call.status, &call.timer) {
                    (Some(s), _) => (s.to_owned(), VOICE_AMBER),
                    (None, Some(t)) => (t.clone(), VOICE_GREEN),
                    (None, None) => (String::new(), VOICE_GREEN),
                };
                let sl = g.layout(&gfx::wide(&state), &g.formats.direct_note, 1e6)?;
                let sw = gfx::metrics(&sl).widthIncludingTrailingWhitespace;
                text::draw(dc, &sl, tx + tw - sw, ty + 2.0, &g.solid(tone));
                let name = format!("#{}", call.channel);
                let nl = g.layout(
                    &gfx::wide(&name),
                    &g.formats.direct_unread,
                    (tw - 24.0 - sw - 8.0).max(1.0),
                )?;
                unsafe { nl.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
                crate::emoji::apply(&nl, &name, 13.0, 1.0)?;
                text::draw(dc, &nl, tx + 24.0, ty + 1.0, &g.brushes.text);
                // Where it is, then who is in it.
                let people = match call.count {
                    0 => call.space.clone(),
                    1 => format!("{} \u{b7} 1 person", call.space),
                    n => format!("{} \u{b7} {n} people", call.space),
                };
                let pl = g.layout(&gfx::wide(&people), &g.formats.direct_note, tw)?;
                unsafe { pl.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
                text::draw(dc, &pl, tx, ty + 22.0, &g.brushes.muted);
                self.draw_call_people(g, dc)?;
                for (p, icon, on) in [
                    (
                        VPart::Mute,
                        if call.muted { Icon::MicOff } else { Icon::Mic },
                        call.muted,
                    ),
                    (
                        VPart::Deafen,
                        if call.deafened {
                            Icon::VolumeX
                        } else {
                            Icon::Headphones
                        },
                        call.deafened,
                    ),
                ] {
                    let Some((bx, by, bw, bh)) = local(p) else {
                        continue;
                    };
                    let b = rect(bx, by, bx + bw, by + bh);
                    let hover = self.voice_hover == Some(p);
                    // A toggle that is on shows terracotta, as egui's.
                    let fill = if on {
                        g.solid(TERRACOTTA)
                    } else if hover {
                        g.solid(0x2E4A3A)
                    } else {
                        g.solid(0x1B2E24)
                    };
                    fill_round(dc, b, 8.0, &fill);
                    icons::draw(
                        dc,
                        icon,
                        rect(bx + 8.0, by + 8.0, bx + bw - 8.0, by + bh - 8.0),
                        TEXT,
                        1.0,
                    );
                }
                if let Some((bx, by, bw, bh)) = local(VPart::Leave) {
                    let b = rect(bx, by, bx + bw, by + bh);
                    let hover = self.voice_hover == Some(VPart::Leave);
                    fill_round(dc, b, 8.0, &g.solid(if hover { TERRACOTTA } else { LEAVE }));
                    icons::draw(
                        dc,
                        Icon::PhoneOff,
                        rect(bx + 10.0, by + 8.0, bx + 26.0, by + 24.0),
                        TEXT,
                        1.0,
                    );
                    let label = if call.joining { "Cancel" } else { "Leave" };
                    let l = g.layout(&gfx::wide(label), &g.formats.direct_unread, 1e6)?;
                    let m = gfx::metrics(&l);
                    text::draw(
                        dc,
                        &l,
                        bx + 32.0,
                        by + (bh - m.height) / 2.0,
                        &g.brushes.text,
                    );
                }
            }
        }
        if let Some(p) = self.call_ui.focus
            && !self.composer.focused
            && !self.thread.as_ref().is_some_and(|t| t.composer.focused)
            && let Some((x, y, w, h)) = local(p)
        {
            stroke_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.terracotta);
        }
        Ok(())
    }
}
