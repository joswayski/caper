//! The who-reacted card, as in the egui client: hovering (or keyboard-
//! focusing) a reaction chip shows its emoji large beside the shared
//! summary ("You, Alex and Maya reacted with :fire:"). Names load on demand
//! through the shared reactor cache; until they arrive the summary counts.
use crate::gfx::{self, fill_round, rect, stroke_round};
use crate::theme::{HEADER_H, SURFACE_RAISED};
use crate::{App, TIMER_CARD, emoji, list};
use windows::Win32::Graphics::DirectWrite::IDWriteTextLayout;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows::core::Result;

const CARD_W: f32 = 280.0;
const EMOJI: f32 = 36.0;
const PAD: f32 = 10.0;
const GAP: f32 = 10.0;
/// How long the pointer rests on a chip before its card shows.
const DELAY_MS: u32 = 450;

pub struct Card {
    /// The chip: its message and emoji.
    pub id: String,
    pub emoji: String,
    pub text: String,
    layout: IDWriteTextLayout,
    pub w: f32,
    pub h: f32,
    /// Top-left in client DIPs, beside the chip.
    pub x: f32,
    pub y: f32,
}

impl App {
    /// The chip the card belongs to: the one under the pointer, else the
    /// keyboard-focused one (as row and reaction index).
    fn card_chip(&self) -> Option<(usize, usize, bool)> {
        let hovered = self
            .pointer
            .and_then(|(x, y)| self.action_at(x, y))
            .and_then(|(i, a)| match a {
                list::Action::Chip(k) => Some((i, k, false)),
                _ => None,
            });
        hovered.or_else(|| match self.kb_control()? {
            (i, list::Action::Chip(k)) => Some((i, k, true)),
            _ => None,
        })
    }

    /// Follows the pointer and keyboard focus: a new chip shows its card
    /// after a short rest (at once for keyboard focus, or when a card is
    /// already up), leaving every chip hides it.
    pub fn track_card(&mut self) {
        if self.live.is_none() || self.dialog.is_some() || self.menu.is_some() {
            return self.hide_card();
        }
        let target = self.card_chip().map(|(i, k, focused)| {
            let m = &self.c.messages[i];
            ((m.id.clone(), m.reactions[k].emoji.clone()), focused)
        });
        let key = target.as_ref().map(|(t, _)| t.clone());
        if key == self.card_target {
            // Names may have arrived or the reactions changed (refreshing a
            // failed read after a while, as egui does while it shows), and
            // the chip may have moved with the list.
            if self.card.is_some() {
                self.show_card();
                if self.place_card() {
                    self.dirty.card = true;
                    self.need = true;
                }
            }
            return;
        }
        self.card_target = key;
        match target {
            None => self.hide_card(),
            Some((_, focused)) if focused || self.card.is_some() => self.show_card(),
            Some(_) => unsafe {
                SetTimer(Some(self.hwnd), TIMER_CARD, DELAY_MS, None);
            },
        }
    }

    pub fn hide_card(&mut self) {
        self.card_target = None;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_CARD);
        }
        if self.card.take().is_some() {
            self.dirty.card = true;
            self.need = true;
        }
    }

    /// Shows (or refreshes) the card for the chip being followed, asking
    /// for its names when they are not known at this reaction revision.
    pub fn show_card(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_CARD);
        }
        let Some((id, emoji)) = self.card_target.clone() else {
            return;
        };
        let Some(live) = &mut self.live else {
            return;
        };
        live.load_reactors(&id);
        let Some(text) = live.reactor_summary(&id, &emoji) else {
            return self.hide_card();
        };
        if self
            .card
            .as_ref()
            .is_some_and(|c| c.id == id && c.emoji == emoji && c.text == text)
        {
            return;
        }
        let Ok(layout) = self.g.layout(
            &gfx::wide(&text),
            &self.g.formats.card,
            CARD_W - 3.0 * PAD - EMOJI,
        ) else {
            return;
        };
        let th = gfx::metrics(&layout).height;
        self.card = Some(Card {
            id,
            emoji,
            text,
            layout,
            w: CARD_W,
            h: 2.0 * PAD + th.max(EMOJI),
            x: 0.0,
            y: 0.0,
        });
        self.place_card();
        self.dirty.card = true;
        self.need = true;
    }

    /// The names (or the reactions) of message `id` changed: its card, if
    /// shown, says so.
    pub fn refresh_card(&mut self, id: &str) {
        if self.card.as_ref().is_some_and(|c| c.id == id) {
            self.show_card();
        }
    }

    /// Puts the card above its chip (below when there is no room), inside
    /// the window, as egui's tooltip aligns; true when it moved.
    fn place_card(&mut self) -> bool {
        let Some((i, k, _)) = self.card_chip() else {
            return false;
        };
        let Some((_, r)) = self
            .row_controls(i)
            .into_iter()
            .find(|(a, _)| *a == list::Action::Chip(k))
        else {
            return false;
        };
        let top = HEADER_H + (self.rows.top[i] - self.scroll_top) as f32;
        let (cx, ct, cb) = (self.list_x() + r.left, top + r.top, top + r.bottom);
        let (w, wh) = (self.w, self.h);
        let Some(card) = &mut self.card else {
            return false;
        };
        let x = cx.min(w - card.w - 8.0).max(8.0);
        let y = if ct - GAP - card.h >= HEADER_H {
            ct - GAP - card.h
        } else {
            (cb + GAP).min(wh - card.h - 8.0)
        };
        let moved = (x, y) != (card.x, card.y);
        card.x = x;
        card.y = y;
        moved
    }

    /// Redraws the card surface when it changed.
    pub fn draw_card(&mut self) -> Result<()> {
        if !std::mem::take(&mut self.dirty.card) {
            return Ok(());
        }
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let Some(card) = &self.card else {
            return self.card_p.offset(-10_000.0, 0.0);
        };
        let g = &self.g;
        self.card_p.ensure(g, px(card.w), px(card.h))?;
        self.card_p
            .offset((card.x * s).round(), (card.y * s).round())?;
        self.card_p.draw(s, |dc| unsafe {
            dc.Clear(None);
            let r = rect(0.5, 0.5, card.w - 0.5, card.h - 0.5);
            fill_round(dc, r, 8.0, &g.solid(SURFACE_RAISED));
            stroke_round(dc, r, 8.0, &g.brushes.border);
            let e = rect(PAD, PAD, PAD + EMOJI, PAD + EMOJI);
            match emoji::find(&card.emoji) {
                Some(entry) => emoji::draw(dc, entry, e),
                None => {
                    let l = g.layout(&gfx::wide(&card.emoji), &g.formats.title, 100.0)?;
                    crate::text::draw(dc, &l, e.left, e.top - 6.0, &g.brushes.text);
                }
            }
            let th = gfx::metrics(&card.layout).height;
            let ty = PAD + (EMOJI - th).max(0.0) / 2.0;
            crate::text::draw(dc, &card.layout, 2.0 * PAD + EMOJI, ty, &g.brushes.text);
            Ok(())
        })
    }
}
