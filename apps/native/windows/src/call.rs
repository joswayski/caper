//! Voice calls, as in the egui client: a joined channel's row offers Join
//! (Switch, from a call elsewhere) while it is hovered, and your call's
//! card sits at the bottom of the sidebar, above the account footer: where
//! the call is, its state and running time, who is in it, then Mute,
//! Deafen and Leave (Cancel while joining). A failed join or call shows its
//! error there instead, with a dismiss button. The shared
//! `caper_client::voice::Voice`, kept in `Live`, runs the call.
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::live::{CallView, VoiceAction};
use crate::overlay::Rect;
use crate::settings::FOOT_H;
use crate::theme::*;
use crate::{App, EV_ROWS, text};
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::*;

/// Redraws the call's timer every second while a call is under way.
pub const TIMER_CALL: usize = 12;
const CARD_H: f32 = 112.0;
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
}

impl VPart {
    pub fn index(self) -> usize {
        match self {
            Self::Status => 0,
            Self::Mute => 1,
            Self::Deafen => 2,
            Self::Leave => 3,
            Self::Dismiss => 4,
            Self::Join(i) => 8 + i,
        }
    }
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

impl App {
    pub fn voice_card(&self) -> Option<Card> {
        let live = self.live.as_ref().filter(|_| self.c.is_live())?;
        match live.call_view() {
            Some(call) => Some(Card::Call(call)),
            None => live.voice.error.clone().map(Card::Error),
        }
    }

    /// The card (client DIPs), above the account footer.
    pub fn call_rect(&self) -> Option<Rect> {
        let h = match self.voice_card()? {
            Card::Call(_) => CARD_H,
            Card::Error(_) => ERROR_H,
        };
        let foot = if self.foot_shown() { FOOT_H } else { 0.0 };
        Some((RAIL_W + 8.0, self.h - foot - 8.0 - h, SIDEBAR_W - 16.0, h))
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
    pub fn join_rect(i: usize) -> Rect {
        let (x, y, w, h) = App::channel_rect(i);
        (x + w - 58.0, y + (h - 24.0) / 2.0, 24.0, 24.0)
    }

    /// The card's parts, in reading order.
    pub fn voice_parts(&self) -> Vec<VPart> {
        match self.voice_card() {
            None => Vec::new(),
            Some(Card::Call(_)) => vec![VPart::Status, VPart::Mute, VPart::Deafen, VPart::Leave],
            Some(Card::Error(_)) => vec![VPart::Status, VPart::Dismiss],
        }
    }

    pub fn voice_rect(&self, p: VPart) -> Option<Rect> {
        if let VPart::Join(i) = p {
            return self.join_action(i).map(|_| App::join_rect(i));
        }
        let call = matches!(self.voice_card()?, Card::Call(_));
        let (x, y, w, h) = self.call_rect()?;
        let row = y + h - 44.0;
        Some(match p {
            VPart::Status if call => (x + 12.0, y + 10.0, w - 24.0, h - 60.0),
            VPart::Status => (x + 12.0, y + 10.0, w - 56.0, h - 20.0),
            VPart::Mute => (x + 12.0, row, 32.0, 32.0),
            VPart::Deafen => (x + 50.0, row, 32.0, 32.0),
            VPart::Leave => (x + w - 12.0 - 92.0, row, 92.0, 32.0),
            VPart::Dismiss => (x + w - 40.0, y + (h - 28.0) / 2.0, 28.0, 28.0),
            VPart::Join(_) => return None,
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
        (0..self.c.channels.len())
            .find(|&i| contains(App::join_rect(i), x, y) && self.join_action(i).is_some())
            .map(VPart::Join)
    }

    /// Whether the pointer is on the card (which covers what is under it).
    pub fn on_call(&self, x: f32, y: f32) -> bool {
        self.call_rect().is_some_and(|r| contains(r, x, y))
    }

    pub fn voice_enabled(&self, p: VPart) -> bool {
        match p {
            VPart::Join(i) => self.join_action(i).is_some_and(|(_, on)| on),
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
        }
    }

    pub fn voice_activate(&mut self, p: VPart) {
        if !self.voice_enabled(p) {
            return;
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
            VPart::Status => {}
        }
        self.voice_changed();
    }

    /// Voice changed: the sidebar redraws, and the call's timer runs only
    /// while a call is under way.
    pub fn voice_changed(&mut self) {
        self.dirty.sidebar = true;
        self.events |= EV_ROWS;
        self.need = true;
        let calling = self.live.as_ref().is_some_and(|l| l.in_voice());
        unsafe {
            if calling {
                SetTimer(Some(self.hwnd), TIMER_CALL, 1000, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_CALL);
            }
        }
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
        self.need = true;
        if let Some(VPart::Join(i)) = hover
            && let Some(id) = self.ids.channels.get(i).cloned()
            && let Some(live) = &mut self.live
        {
            live.prepare_voice(&id);
        }
    }

    /// Draws the card in the sidebar (sidebar-local DIPs).
    pub fn draw_call(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        let (Some(card), Some((cx, cy, cw, ch))) = (self.voice_card(), self.call_rect()) else {
            return Ok(());
        };
        let x = cx - RAIL_W;
        let r = rect(x, cy, x + cw, cy + ch);
        let local = |p: VPart| {
            self.voice_rect(p)
                .map(|(px, py, pw, ph)| (px - RAIL_W, py, pw, ph))
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
                if !call.names.is_empty() {
                    let ml = g.layout(&gfx::wide(&call.names), &g.formats.direct_note, tw)?;
                    unsafe { ml.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)? };
                    crate::emoji::apply(&ml, &call.names, 12.0, 1.0)?;
                    text::draw(dc, &ml, tx, ty + 38.0, &g.brushes.text);
                }
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
        Ok(())
    }
}
