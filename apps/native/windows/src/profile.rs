//! A person's profile card, as on the egui and GTK desktops: their avatar,
//! name and @username ("You" on your own), Open conversation, and Block or
//! Unblock. Clicking a message author's avatar or name, or a member row,
//! opens it beside the click; a click outside, Escape or the wheel closes it.
use crate::avatars::{self, Avatar};
use crate::gfx::{self, rect, stroke_round};
use crate::overlay::{Rect, Tone, draw_button, height, layout};
use crate::theme::*;
use crate::{App, EV_SCREEN, list};
use caper_client::mentions::Profile;
use caper_client::model as core;
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

const CARD_W: f32 = 300.0;
const PAD: f32 = 16.0;
const AVATAR: f32 = 56.0;
const BUTTON_H: f32 = 32.0;
const CARD_H: f32 = PAD + AVATAR + 14.0 + BUTTON_H + PAD;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PPart {
    Close,
    Open,
    Block,
}

pub struct ProfileCard {
    pub id: String,
    author: Option<core::Author>,
    /// Top-left in client DIPs.
    pub x: f32,
    pub y: f32,
    pub hover: Option<PPart>,
}

impl ProfileCard {
    pub fn rect(&self) -> Rect {
        (self.x, self.y, CARD_W, CARD_H)
    }
}

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

impl App {
    fn card_profile(&self) -> Option<(&ProfileCard, Profile)> {
        let card = self.profile.as_ref()?;
        let profile = self
            .live
            .as_ref()?
            .profile_of(&card.id, card.author.as_ref())?;
        Some((card, profile))
    }

    /// Its buttons, each with whether it can be used now.
    fn profile_parts(&self) -> Vec<(PPart, bool)> {
        let (Some((card, profile)), Some(live)) = (self.card_profile(), self.live.as_ref()) else {
            return Vec::new();
        };
        let blocked = live.blocked_person(&card.id);
        let mut parts = vec![(PPart::Close, true)];
        parts.push((
            PPart::Open,
            profile.me || (!blocked && profile.username.is_some()),
        ));
        if !profile.me && (blocked || !live.protected(&card.id)) {
            parts.push((PPart::Block, true));
        }
        parts
    }

    fn profile_part_rect(&self, part: PPart) -> Option<Rect> {
        let card = self.profile.as_ref()?;
        let by = card.y + PAD + AVATAR + 14.0;
        let block = self.profile_parts().iter().any(|(p, _)| *p == PPart::Block);
        let inner = CARD_W - 2.0 * PAD;
        Some(match part {
            PPart::Close => (card.x + CARD_W - 8.0 - 24.0, card.y + 8.0, 24.0, 24.0),
            PPart::Open if block => (card.x + PAD, by, inner - 96.0 - 8.0, BUTTON_H),
            PPart::Open => (card.x + PAD, by, inner, BUTTON_H),
            PPart::Block => (card.x + CARD_W - PAD - 96.0, by, 96.0, BUTTON_H),
        })
    }

    fn profile_hit(&self, x: f32, y: f32) -> Option<PPart> {
        self.profile_parts()
            .into_iter()
            .filter(|(_, enabled)| *enabled)
            .map(|(p, _)| p)
            .find(|p| {
                self.profile_part_rect(*p)
                    .is_some_and(|r| contains(r, x, y))
            })
    }

    /// Opens `id`'s card beside (`x`, `y`), replacing any other.
    pub fn open_profile(&mut self, id: String, author: Option<core::Author>, x: f32, y: f32) {
        self.close_menu();
        self.hide_card();
        let cx = (x + 12.0).min(self.w - CARD_W - 8.0).max(8.0);
        let cy = (y - 12.0).min(self.h - CARD_H - 8.0).max(8.0);
        self.profile = Some(ProfileCard {
            id,
            author,
            x: cx,
            y: cy,
            hover: None,
        });
        if self.card_profile().is_none() {
            self.profile = None;
            return;
        }
        self.profile_changed();
    }

    pub fn close_profile(&mut self) {
        if self.profile.take().is_some() {
            self.profile_changed();
        }
    }

    fn profile_changed(&mut self) {
        self.dirty.profile = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// The message row whose author's avatar or name is at (`x`, `y`).
    pub fn author_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.in_history(x, y) || self.live.is_none() {
            return None;
        }
        let ay = self.scroll_top + (y - HEADER_H) as f64;
        let i = self.rows.row_at(ay)?;
        let row = self.rows.row(i)?;
        let h = row.header.as_ref()?;
        let top = (self.rows.top[i] - self.scroll_top) as f32
            + HEADER_H
            + list::content_top(self.c.messages[i].grouped)
            + row.header_y;
        let lx = x - self.list_x();
        let avatar = (list::PAD_X..list::PAD_X + list::AVATAR).contains(&lx)
            && (top..top + list::AVATAR).contains(&y);
        let name = (list::BODY_X..list::BODY_X + h.name_w).contains(&lx)
            && (top..top + gfx::metrics(&h.name).height).contains(&y);
        (avatar || name).then_some(i)
    }

    /// Opens the author's card of message row `i`.
    pub fn open_author_profile(&mut self, i: usize, x: f32, y: f32) {
        let Some(m) = self
            .c
            .messages
            .get(i)
            .and_then(|r| self.live.as_ref()?.message(&r.id))
        else {
            return;
        };
        self.open_profile(m.author.id.clone(), Some(m.author), x, y);
    }

    /// Opens member row `i`'s card.
    pub fn open_member_profile(&mut self, i: usize, x: f32, y: f32) {
        if let Some(id) = self.ids.members.get(i).cloned() {
            self.open_profile(id, None, x, y);
        }
    }

    fn profile_activate(&mut self, part: PPart) {
        let Some((card, profile)) = self.card_profile() else {
            return;
        };
        let id = card.id.clone();
        match part {
            PPart::Close => self.close_profile(),
            PPart::Open => {
                self.close_profile();
                let Some(live) = &mut self.live else { return };
                if profile.me {
                    live.open_self_direct();
                } else if let Some(username) = profile.username {
                    live.open_conversation_with(&id, username);
                }
                if let Err(e) = self.drain_live() {
                    eprintln!("opening a conversation failed: {e:?}");
                }
            }
            PPart::Block => {
                let account = core::BlockedAccount {
                    id: id.clone(),
                    username: profile.username.clone().unwrap_or_default(),
                    display_name: profile.title(),
                    avatar_id: profile.avatar_id,
                    avatar_asset: profile.avatar_asset.clone(),
                    avatar_url: profile.avatar_url.clone(),
                };
                self.close_profile();
                let Some(live) = &mut self.live else { return };
                if live.blocked_person(&id) {
                    live.unblock_account(account);
                    if let Err(e) = self.drain_live() {
                        eprintln!("unblocking failed: {e:?}");
                    }
                } else {
                    // Blocking asks first, as everywhere else.
                    self.open_dialog(crate::overlay::Dialog::block(account, None));
                }
            }
        }
    }

    /// The card's input while it is open.
    pub fn profile_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let card_rect = self.profile.as_ref()?.rect();
        let hit = self.profile_hit(x, y);
        match msg {
            WM_MOUSEMOVE => {
                self.pointer = Some((x, y));
                let card = self.profile.as_mut()?;
                if card.hover != hit {
                    card.hover = hit;
                    self.profile_changed();
                }
                contains(card_rect, x, y).then_some(Some(LRESULT(0)))
            }
            WM_LBUTTONDOWN => {
                if let Some(part) = hit {
                    self.profile_activate(part);
                    return done;
                }
                if contains(card_rect, x, y) {
                    return done;
                }
                // A click outside closes it and carries on (it may open
                // another person's card).
                self.close_profile();
                None
            }
            WM_RBUTTONDOWN | WM_MOUSEWHEEL => {
                self.close_profile();
                None
            }
            WM_KEYDOWN if VIRTUAL_KEY(wp.0 as u16) == VK_ESCAPE => {
                self.close_profile();
                done
            }
            WM_SETCURSOR if contains(card_rect, x, y) => {
                let cursor = if hit.is_some() { IDC_HAND } else { IDC_ARROW };
                unsafe { SetCursor(LoadCursorW(None, cursor).ok()) };
                done
            }
            _ => None,
        }
    }

    /// Redraws the card's surface when it changed.
    pub fn draw_profile(&mut self) -> Result<()> {
        if !std::mem::take(&mut self.dirty.profile) {
            return Ok(());
        }
        let Some((card, profile)) = self.card_profile() else {
            return self.profile_p.offset(-10_000.0, 0.0);
        };
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let (cx, cy) = (card.x, card.y);
        let hover = card.hover;
        let title = profile.title();
        let avatar = Avatar::new(
            profile.avatar_id,
            &profile.avatar_asset,
            &profile.avatar_url,
        );
        let face = crate::live::person(&card.id, &title, avatar);
        let sub = if profile.me {
            "You".to_owned()
        } else {
            profile
                .username
                .as_ref()
                .map(|u| format!("@{u}"))
                .unwrap_or_default()
        };
        let blocked = self
            .live
            .as_ref()
            .is_some_and(|l| l.blocked_person(&card.id));
        let parts: Vec<(PPart, Rect, bool)> = self
            .profile_parts()
            .into_iter()
            .filter_map(|(p, enabled)| {
                let (x, y, w, h) = self.profile_part_rect(p)?;
                Some((p, (x - cx, y - cy, w, h), enabled))
            })
            .collect();
        let g = &self.g;
        self.profile_p.ensure(g, px(CARD_W), px(CARD_H))?;
        self.profile_p.offset((cx * s).round(), (cy * s).round())?;
        self.profile_p.draw(s, |dc| unsafe {
            dc.Clear(None);
            let body = rect(0.5, 0.5, CARD_W - 0.5, CARD_H - 0.5);
            gfx::fill_round(dc, body, RADIUS, &g.solid(SURFACE_RAISED));
            stroke_round(dc, body, RADIUS, &g.brushes.border);
            let r = AVATAR / 2.0;
            avatars::circle(
                g,
                dc,
                &face.avatar,
                (PAD + r, PAD + r, r),
                face.color,
                &face.initials,
            )?;
            let tx = PAD + AVATAR + 12.0;
            let tw = CARD_W - tx - 40.0;
            let name = layout(g, &title, &g.formats.dialog_title, tw)?;
            let note = layout(g, &sub, &g.formats.direct_note, tw)?;
            let block_h = height(&name) + 2.0 + height(&note);
            let ny = PAD + (AVATAR - block_h) / 2.0;
            crate::text::draw(dc, &name, tx, ny, &g.brushes.text);
            crate::text::draw(dc, &note, tx, ny + height(&name) + 2.0, &g.brushes.muted);
            for (part, (x, y, w, h), enabled) in parts {
                let state = match (enabled, hover == Some(part)) {
                    (false, _) => 0,
                    (true, true) => 2,
                    (true, false) => 1,
                };
                match part {
                    PPart::Close => {
                        if state == 2 {
                            gfx::fill_round(dc, rect(x, y, x + w, y + h), 6.0, &g.brushes.hover);
                        }
                        crate::icons::draw(
                            dc,
                            crate::icons::Icon::Close,
                            rect(x + 4.0, y + 4.0, x + w - 4.0, y + h - 4.0),
                            if state == 2 { TEXT } else { TEXT_MUTED },
                            1.0,
                        );
                    }
                    PPart::Open => draw_button(
                        g,
                        dc,
                        "Open conversation",
                        (x, y, w, h),
                        Tone::Primary,
                        state,
                    )?,
                    PPart::Block if blocked => {
                        draw_button(g, dc, "Unblock", (x, y, w, h), Tone::Secondary, state)?
                    }
                    PPart::Block => {
                        draw_button(g, dc, "Block", (x, y, w, h), Tone::DangerOutline, state)?
                    }
                }
            }
            Ok::<(), windows::core::Error>(())
        })
    }
}
