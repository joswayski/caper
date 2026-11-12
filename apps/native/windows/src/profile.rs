//! A person's profile card, as on the egui desktop: their avatar (an
//! uploaded picture opens the viewer), name and @username, then icon
//! buttons for Open conversation, More profile actions (Invite to space,
//! Block or Unblock) and Close. Below them: your own card says "You";
//! another person's lists your spaces to invite them to while choosing,
//! the latest status or error, and under a divider a composer that sends
//! them a DM ("Message @username"). Clicking a message author's avatar or
//! name, a quoted author, a mention pill (in the history or the thread) or
//! a member row opens it beside the click. A click outside, Escape or Close
//! closes it, except while it sends; each person's draft is kept until it
//! is sent or you sign out.
use crate::avatars::{self, Avatar};
use crate::composer::{Composer, LINE};
use crate::gfx::{fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::overlay::{Rect, height, layout};
use crate::theme::*;
use crate::{App, EV_FOCUS, EV_SCREEN, list};
use caper_client::mentions::Profile;
use caper_client::model as core;
use windows::Win32::Foundation::{LRESULT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

/// egui's card: 320 DIPs wide with 16 DIPs of padding inside its border.
pub const CARD_W: f32 = 320.0;
const PAD: f32 = 17.0;
const AVATAR: f32 = 48.0;
const ICON: f32 = 28.0;
const GAP: f32 = 12.0;
const SPACE_H: f32 = 36.0;
const NOTE_H: f32 = 18.0;
const FIELD_PAD_X: f32 = 12.0;
const FIELD_PAD_Y: f32 = 10.0;
const SEND: f32 = 32.0;
const FIELD_LINES: f32 = 5.0;
const MENU_W: f32 = 144.0;
const MENU_ITEM: f32 = 32.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PPart {
    /// The avatar, while it is an uploaded picture: opens the viewer.
    Picture,
    Close,
    Open,
    More,
    /// The More menu's items.
    Invite,
    Block,
    /// Your space `k` to invite them to, while choosing.
    Space(usize),
    Field,
    Send,
}

impl PPart {
    pub fn index(self) -> usize {
        match self {
            Self::Picture => 0,
            Self::Close => 1,
            Self::Open => 2,
            Self::More => 3,
            Self::Invite => 4,
            Self::Block => 5,
            Self::Field => 6,
            Self::Send => 7,
            Self::Space(k) => 8 + k,
        }
    }
}

pub struct ProfileCard {
    pub id: String,
    /// The pill's entry (or one made for an author or member) and the
    /// author it came from, which `mentions::profile` resolves.
    entry: core::Mention,
    author: Option<core::Author>,
    /// Where it was opened, in client DIPs.
    anchor: (f32, f32),
    pub hover: Option<PPart>,
    /// The More menu is open.
    pub more: bool,
    pub field: Composer,
    dragging: bool,
}

/// What the card shows now, from the account's loaded state.
pub struct View {
    pub profile: Profile,
    pub blocked: bool,
    pub can_message: bool,
    pub can_block: bool,
    pub can_invite: bool,
    pub busy: bool,
    pub choosing: bool,
    pub spaces: Vec<(core::Space, String, bool)>,
    pub status: Option<String>,
    pub error: Option<String>,
    pub picture: bool,
}

impl View {
    pub fn username(&self) -> Option<&str> {
        self.profile.username.as_deref()
    }

    /// What its part is called (for UI Automation).
    pub fn part_name(&self, part: PPart) -> String {
        match part {
            PPart::Picture => "Open profile picture".into(),
            PPart::Close => "Close profile".into(),
            PPart::Open => "Open conversation".into(),
            PPart::More => "More profile actions".into(),
            PPart::Invite => "Invite to space".into(),
            PPart::Block if self.blocked => "Unblock".into(),
            PPart::Block => "Block".into(),
            PPart::Space(k) => self.spaces.get(k).map_or_else(String::new, |s| s.1.clone()),
            PPart::Field => format!("Message @{}", self.username().unwrap_or_default()),
            PPart::Send => "Send".into(),
        }
    }
}

/// Where the card and its parts sit, in client DIPs.
pub struct Geometry {
    pub card: Rect,
    pub parts: Vec<(PPart, Rect)>,
    pub menu: Option<Rect>,
    /// The "You" line, status and error tops, and the divider's y.
    you: Option<f32>,
    status: Option<f32>,
    error: Option<(f32, f32)>,
    divider: Option<f32>,
}

fn contains((x, y, w, h): Rect, px: f32, py: f32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

fn field_h(field: &Composer) -> f32 {
    field.text_h.clamp(LINE, LINE * FIELD_LINES) + 2.0 * FIELD_PAD_Y
}

impl App {
    /// What the open card shows.
    pub fn profile_view(&self) -> Option<View> {
        let card = self.profile.as_ref()?;
        let live = self.live.as_ref()?;
        let profile = live.profile_of(&card.entry, card.author.as_ref())?;
        let state = live.card.as_ref().filter(|c| c.person == card.id);
        let blocked = live.blocked_person(&card.id);
        let protected = live.protected(&card.id);
        let known = card.entry.id.is_some();
        let owned = if profile.me {
            Vec::new()
        } else {
            live.invitable_spaces(&card.id)
        };
        let can_message = !profile.me && !blocked && profile.username.is_some() && known;
        let can_invite = can_message && !owned.is_empty();
        let can_block =
            !profile.me && known && live.me.is_some() && (blocked || !protected || can_invite);
        let sending = state.is_some_and(|s| s.sending);
        let inviting = state.and_then(|s| s.inviting.clone());
        let busy = sending || inviting.is_some() || live.block_busy();
        let spaces = owned
            .into_iter()
            .map(|s| {
                let invited = state.and_then(|c| c.invited.get(&s.id).copied());
                let label = if inviting.as_ref().is_some_and(|(_, id)| id == &s.id) {
                    "Sending\u{2026}".to_owned()
                } else if let Some(already) = invited {
                    format!("{} \u{b7} {already}", s.name)
                } else {
                    s.name.clone()
                };
                (s, label, invited.is_some())
            })
            .collect();
        Some(View {
            picture: profile.picture().is_some(),
            blocked,
            can_message,
            can_block,
            can_invite,
            busy,
            choosing: can_invite && state.is_some_and(|s| s.choosing_space),
            spaces,
            status: state.and_then(|s| s.status.clone()),
            error: state.and_then(|s| s.error.clone()),
            profile,
        })
    }

    fn profile_enabled(&self, v: &View, part: PPart) -> bool {
        let sending = self
            .live
            .as_ref()
            .and_then(|l| l.card.as_ref())
            .is_some_and(|c| c.sending);
        match part {
            PPart::Picture => !sending,
            PPart::Close | PPart::More | PPart::Block | PPart::Invite => !v.busy,
            PPart::Open => !v.busy && self.live.as_ref().is_some_and(|l| l.me.is_some()),
            PPart::Space(k) => !v.busy && v.spaces.get(k).is_some_and(|s| !s.2),
            PPart::Field => !v.busy,
            PPart::Send => {
                !v.busy
                    && self
                        .profile
                        .as_ref()
                        .is_some_and(|c| !String::from_utf16_lossy(&c.field.text).trim().is_empty())
            }
        }
    }

    pub fn profile_part_enabled(&self, part: PPart) -> bool {
        self.profile_view()
            .is_some_and(|v| self.profile_enabled(&v, part))
    }

    /// Lays the card out beside where it opened, inside the window.
    pub fn profile_geometry(&self, v: &View) -> Option<Geometry> {
        let card = self.profile.as_ref()?;
        let g = &self.g;
        let inner = CARD_W - 2.0 * PAD;
        let note_h = |text: &str, format| -> f32 {
            layout(g, text, format, inner).map_or(NOTE_H, |l| height(&l))
        };
        let mut parts = Vec::new();
        let mut y = PAD;
        if v.picture {
            parts.push((PPart::Picture, (PAD, y, AVATAR, AVATAR)));
        }
        let mut right = CARD_W - PAD;
        let mut icon = |part: PPart, parts: &mut Vec<(PPart, Rect)>| {
            right -= ICON;
            parts.push((part, (right, y, ICON, ICON)));
            right -= 2.0;
        };
        icon(PPart::Close, &mut parts);
        if v.can_message {
            icon(PPart::Open, &mut parts);
        }
        if v.can_block {
            icon(PPart::More, &mut parts);
        }
        y += AVATAR + GAP;
        let mut geo = Geometry {
            card: (0.0, 0.0, CARD_W, 0.0),
            parts,
            menu: None,
            you: None,
            status: None,
            error: None,
            divider: None,
        };
        if v.profile.me {
            geo.you = Some(y);
            y += NOTE_H + GAP;
        } else {
            if v.choosing {
                for k in 0..v.spaces.len() {
                    geo.parts.push((PPart::Space(k), (PAD, y, inner, SPACE_H)));
                    y += SPACE_H + 6.0;
                }
                y += GAP - 6.0;
            }
            if let Some(s) = &v.status {
                geo.status = Some(y);
                y += note_h(s, &g.formats.direct_note) + GAP;
            }
            if let Some(e) = &v.error {
                let h = note_h(e, &g.formats.error);
                geo.error = Some((y, h));
                y += h + GAP;
            }
            if v.can_message {
                geo.divider = Some(y);
                y += GAP;
                let fh = field_h(&card.field);
                geo.parts.push((PPart::Field, (PAD, y, inner, fh)));
                geo.parts.push((
                    PPart::Send,
                    (PAD + inner - SEND - 4.0, y + fh - SEND - 4.0, SEND, SEND),
                ));
                y += fh + GAP;
            }
        }
        let h = y - GAP + PAD;
        let (ax, ay) = card.anchor;
        let x = (ax + 12.0).min(self.w - CARD_W - 8.0).max(8.0);
        let top = (ay - 12.0).min(self.h - h - 8.0).max(8.0);
        geo.card = (x, top, CARD_W, h);
        for (_, r) in &mut geo.parts {
            r.0 += x;
            r.1 += top;
        }
        for y in [&mut geo.you, &mut geo.status, &mut geo.divider]
            .into_iter()
            .flatten()
        {
            *y += top;
        }
        if let Some((y, _)) = &mut geo.error {
            *y += top;
        }
        if card.more
            && let Some(&(_, (mx, my, mw, mh))) = geo.parts.iter().find(|(p, _)| *p == PPart::More)
        {
            let items = 1 + usize::from(v.can_invite);
            let menu = (
                (mx + mw - MENU_W).max(4.0),
                my + mh + 4.0,
                MENU_W,
                8.0 + MENU_ITEM * items as f32,
            );
            let mut iy = menu.1 + 4.0;
            if v.can_invite {
                geo.parts
                    .push((PPart::Invite, (menu.0 + 4.0, iy, MENU_W - 8.0, MENU_ITEM)));
                iy += MENU_ITEM;
            }
            geo.parts
                .push((PPart::Block, (menu.0 + 4.0, iy, MENU_W - 8.0, MENU_ITEM)));
            geo.menu = Some(menu);
        }
        Some(geo)
    }

    /// The parts shown now, in reading order, with their rectangles.
    pub fn profile_parts(&self) -> Vec<(PPart, Rect)> {
        self.profile_view()
            .and_then(|v| self.profile_geometry(&v))
            .map_or_else(Vec::new, |g| g.parts)
    }

    pub fn profile_rect(&self) -> Option<Rect> {
        let v = self.profile_view()?;
        Some(self.profile_geometry(&v)?.card)
    }

    /// The part under (`x`, `y`): the More menu first, then the card.
    fn profile_hit(&self, x: f32, y: f32) -> Option<PPart> {
        let parts = self.profile_parts();
        let menu = |p: &&(PPart, Rect)| matches!(p.0, PPart::Invite | PPart::Block);
        parts
            .iter()
            .filter(menu)
            .chain(parts.iter().filter(|p| !menu(p)))
            .find(|(_, r)| contains(*r, x, y))
            .map(|(p, _)| *p)
    }

    /// The composer's text origin in client DIPs, as drawn.
    pub fn profile_text_origin(&self) -> (f32, f32) {
        let r = self
            .profile_parts()
            .into_iter()
            .find(|(p, _)| *p == PPart::Field)
            .map_or((0.0, 0.0, 0.0, 0.0), |(_, r)| r);
        let scroll = self.profile.as_ref().map_or(0.0, |c| c.field.scroll());
        (r.0 + FIELD_PAD_X, r.1 + FIELD_PAD_Y - scroll)
    }

    /// Opens `id`'s card beside (`x`, `y`), replacing any other (not while
    /// one sends).
    pub fn open_profile(
        &mut self,
        id: String,
        entry: core::Mention,
        author: Option<core::Author>,
        x: f32,
        y: f32,
    ) {
        if self
            .live
            .as_ref()
            .and_then(|l| l.card.as_ref())
            .is_some_and(|c| c.sending)
        {
            return;
        }
        self.close_profile();
        self.close_menu();
        self.hide_card();
        let mut field = Composer {
            caret_on: true,
            ..Default::default()
        };
        if let Some(draft) = self.profile_drafts.remove(&id) {
            field.set_text(draft);
        }
        self.profile = Some(ProfileCard {
            id: id.clone(),
            entry,
            author,
            anchor: (x, y),
            hover: None,
            more: false,
            field,
            dragging: false,
        });
        let Some(v) = self.profile_view() else {
            self.profile = None;
            return;
        };
        if let Some(live) = &mut self.live {
            live.card = Some(crate::live::CardState {
                person: id,
                ..Default::default()
            });
        }
        // As in egui, the composer takes focus so you can just type.
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        if v.can_message {
            self.set_profile_focus(true);
        }
        self.profile_changed();
    }

    /// Closes the card (not while it sends), keeping its draft.
    pub fn close_profile(&mut self) {
        if self
            .live
            .as_ref()
            .and_then(|l| l.card.as_ref())
            .is_some_and(|c| c.sending)
        {
            return;
        }
        if let Some(card) = self.profile.take() {
            if !card.field.text.is_empty() {
                self.profile_drafts.insert(card.id, card.field.text);
            }
            if let Some(live) = &mut self.live {
                live.card = None;
            }
            if card.field.focused {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), crate::TIMER_CARET);
                }
            }
            self.profile_changed();
            self.events |= EV_FOCUS;
        }
    }

    /// The card's message was sent (or its DM opened): it closed.
    pub fn profile_finished(&mut self, person: Option<String>) {
        if let Some(person) = person {
            self.profile_drafts.remove(&person);
            if self.profile.as_ref().is_some_and(|c| c.id == person) {
                if self.profile.as_ref().is_some_and(|c| c.field.focused) {
                    unsafe {
                        let _ = KillTimer(Some(self.hwnd), crate::TIMER_CARET);
                    }
                }
                self.profile = None;
                self.events |= EV_FOCUS;
            }
        }
        self.profile_changed();
    }

    pub fn set_profile_focus(&mut self, focused: bool) {
        let Some(card) = &mut self.profile else {
            return;
        };
        if card.field.focused == focused {
            return;
        }
        card.field.focused = focused;
        card.field.caret_on = true;
        if !focused {
            card.field.end();
        }
        unsafe {
            if focused {
                SetTimer(
                    Some(self.hwnd),
                    crate::TIMER_CARET,
                    GetCaretBlinkTime(),
                    None,
                );
            } else {
                let _ = KillTimer(Some(self.hwnd), crate::TIMER_CARET);
            }
        }
        self.events |= EV_FOCUS;
        self.profile_changed();
    }

    /// After the card's text, state or what it shows changed.
    pub fn profile_changed(&mut self) {
        let g = &self.g;
        if let Some(card) = &mut self.profile {
            let width = CARD_W - 2.0 * PAD - 2.0 * FIELD_PAD_X - SEND;
            let _ = card
                .field
                .relayout_with(g, &g.formats.composer, 15.0, width);
        }
        self.dirty.profile = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// The person a point in the history names (an author's avatar or
    /// name, a quoted author, a mention pill): their id, the entry
    /// `mentions::profile` resolves, and the author it came from.
    pub fn person_at(
        &self,
        x: f32,
        y: f32,
    ) -> Option<(String, core::Mention, Option<core::Author>)> {
        if !self.in_history(x, y) || self.live.is_none() {
            return None;
        }
        let ay = self.scroll_top + (y - HEADER_H) as f64;
        let i = self.rows.row_at(ay)?;
        let grouped = self.c.messages.get(i)?.grouped;
        let who = self.rows.who_at(
            i,
            grouped,
            x - self.list_x(),
            (ay - self.rows.top[i]) as f32,
        )?;
        self.person_of(&self.c.messages[i].id, self.rows.row(i)?, who)
    }

    /// Resolves what `Rows::who_at` found on message `id`'s row.
    pub fn person_of(
        &self,
        id: &str,
        row: &list::Row,
        who: list::Who,
    ) -> Option<(String, core::Mention, Option<core::Author>)> {
        let m = self.live.as_ref()?.message(id)?;
        let author = |a: core::Author| {
            let entry = core::Mention {
                kind: "user".into(),
                id: Some(a.id.clone()),
                username: None,
            };
            (a.id.clone(), entry, Some(a))
        };
        match who {
            list::Who::Author => Some(author(m.author)),
            list::Who::Quote => m.reply_to.map(|r| author(r.author)),
            list::Who::Mention(s, e) => {
                let text = String::from_utf16_lossy(row.plain16.get(s..e)?);
                let (_, entry) = caper_client::mentions::highlights(&text, &m.content.mentions)
                    .into_iter()
                    .next()?;
                // Only people have cards (not @everyone or channels).
                (entry.kind == "user")
                    .then(|| entry.id.clone())
                    .flatten()
                    .map(|id| (id, entry.clone(), None))
            }
        }
    }

    /// Opens member row `i`'s card.
    pub fn open_member_profile(&mut self, i: usize, x: f32, y: f32) {
        let Some(id) = self.ids.members.get(i).cloned() else {
            return;
        };
        let entry = core::Mention {
            kind: "user".into(),
            id: Some(id.clone()),
            username: None,
        };
        self.open_profile(id, entry, None, x, y);
    }

    fn profile_send(&mut self, open_only: bool) {
        let (Some(card), Some(v)) = (self.profile.as_ref(), self.profile_view()) else {
            return;
        };
        let (Some(username), true) = (v.profile.username.clone(), v.can_message) else {
            return;
        };
        let (id, text) = (card.id.clone(), String::from_utf16_lossy(&card.field.text));
        if let Some(live) = &mut self.live {
            live.profile_send(&id, username, text, open_only);
        }
        self.profile_changed();
        if let Err(e) = self.drain_live() {
            eprintln!("profile send failed: {e:?}");
        }
    }

    pub fn profile_activate(&mut self, part: PPart) {
        let (Some(v), Some(card)) = (self.profile_view(), self.profile.as_ref()) else {
            return;
        };
        if !self.profile_enabled(&v, part) {
            return;
        }
        let id = card.id.clone();
        if !matches!(part, PPart::More | PPart::Field)
            && let Some(card) = &mut self.profile
        {
            card.more = false;
        }
        match part {
            PPart::Close => self.close_profile(),
            PPart::Picture => {
                let title = v.profile.title();
                if let Some(a) = v
                    .profile
                    .picture()
                    .and_then(|s| caper_client::attachments::profile_picture(s, &title))
                {
                    self.open_viewer(vec![a], 0);
                }
            }
            PPart::Open => self.profile_send(true),
            PPart::Send => self.profile_send(false),
            PPart::Field => self.set_profile_focus(true),
            PPart::More => {
                if let Some(card) = &mut self.profile {
                    card.more = !card.more;
                }
                self.profile_changed();
            }
            PPart::Invite => {
                if let Some(c) = self.live.as_mut().and_then(|l| l.card.as_mut()) {
                    c.choosing_space = !c.choosing_space;
                }
                self.profile_changed();
            }
            PPart::Space(k) => {
                let (Some((space, _, _)), Some(username)) =
                    (v.spaces.get(k).cloned(), v.profile.username.clone())
                else {
                    return;
                };
                if let Some(live) = &mut self.live {
                    live.profile_invite(space, username, id);
                }
                self.profile_changed();
            }
            PPart::Block => {
                let account = core::BlockedAccount {
                    id: id.clone(),
                    username: v.profile.username.clone().unwrap_or_default(),
                    display_name: v.profile.title(),
                    avatar_id: v.profile.avatar_id,
                    avatar_asset: v.profile.avatar_asset.clone(),
                    avatar_url: v.profile.avatar_url.clone(),
                };
                if v.blocked {
                    if let Some(live) = &mut self.live {
                        if let Some(c) = live.card.as_mut() {
                            c.error = None;
                        }
                        live.unblock_account(account);
                    }
                    self.profile_changed();
                    if let Err(e) = self.drain_live() {
                        eprintln!("unblocking failed: {e:?}");
                    }
                } else {
                    // Blocking asks first, as everywhere else.
                    self.close_profile();
                    self.open_dialog(crate::overlay::Dialog::block(account, None));
                }
            }
        }
    }

    /// The card's input while it is open. None leaves it to the window.
    pub fn profile_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        let done = Some(Some(LRESULT(0)));
        let v = self.profile_view()?;
        let geo = self.profile_geometry(&v)?;
        let on_card = contains(geo.card, x, y) || geo.menu.is_some_and(|m| contains(m, x, y));
        let hit = self.profile_hit(x, y);
        let hwnd = self.hwnd;
        let (ox, oy) = self.profile_text_origin();
        match msg {
            WM_MOUSEMOVE => {
                self.pointer = Some((x, y));
                let card = self.profile.as_mut()?;
                if card.dragging {
                    let p = card.field.hit(x - ox, y - oy);
                    card.field.move_to(p, true);
                    self.profile_changed();
                    return done;
                }
                let hover = hit.filter(|p| *p != PPart::Field);
                if card.hover != hover {
                    card.hover = hover;
                    self.profile_changed();
                }
                on_card.then_some(Some(LRESULT(0)))
            }
            WM_LBUTTONDOWN => {
                let more = self.profile.as_ref()?.more;
                match hit {
                    Some(PPart::Field) if self.profile_enabled(&v, PPart::Field) => {
                        self.set_profile_focus(true);
                        let card = self.profile.as_mut()?;
                        let p = card.field.hit(x - ox, y - oy);
                        card.field.move_to(p, shift);
                        card.dragging = true;
                        card.more = false;
                        unsafe {
                            SetCapture(hwnd);
                        }
                        self.profile_changed();
                    }
                    Some(part) => self.profile_activate(part),
                    // A click off the open More menu only closes it.
                    None if more => {
                        self.profile.as_mut()?.more = false;
                        self.profile_changed();
                    }
                    None if on_card => {}
                    None => {
                        // A click outside closes it and carries on (it may
                        // open another person's card).
                        if !v.busy {
                            self.close_profile();
                        }
                        return if v.busy { done } else { None };
                    }
                }
                done
            }
            WM_LBUTTONUP if self.profile.as_ref()?.dragging => {
                self.profile.as_mut()?.dragging = false;
                unsafe {
                    let _ = ReleaseCapture();
                }
                done
            }
            WM_LBUTTONDBLCLK if on_card => done,
            WM_RBUTTONDOWN if !on_card && !v.busy => {
                self.close_profile();
                None
            }
            WM_MOUSEWHEEL if on_card => done,
            WM_MOUSEWHEEL if !v.busy => {
                self.close_profile();
                None
            }
            WM_SETCURSOR if on_card => {
                let cursor = match hit {
                    Some(PPart::Field) => IDC_IBEAM,
                    Some(p) if self.profile_enabled(&v, p) => IDC_HAND,
                    _ => IDC_ARROW,
                };
                unsafe { SetCursor(LoadCursorW(None, cursor).ok()) };
                Some(Some(LRESULT(1)))
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                let card = self.profile.as_mut()?;
                let focused = card.field.focused;
                if card.field.composing || !card.field.comp.is_empty() {
                    return Some(None);
                }
                match vk {
                    VK_ESCAPE if card.more => {
                        card.more = false;
                        self.profile_changed();
                    }
                    VK_ESCAPE => self.close_profile(),
                    VK_RETURN if focused && shift && !v.busy => {
                        card.field.insert(&['\n' as u16]);
                        self.profile_changed();
                    }
                    VK_RETURN if focused => self.profile_send(false),
                    _ if focused && ctrl && vk == VIRTUAL_KEY('C' as u16) => {
                        let copied = card.field.copied();
                        if !copied.is_empty() {
                            crate::set_clipboard(hwnd, &copied);
                        }
                    }
                    _ if focused && !v.busy && card.field.edit_key(hwnd, vk, ctrl, shift, true) => {
                        self.profile_changed();
                    }
                    // Keys for the window (Ctrl+, and the like) pass on
                    // while the composer doesn't have them.
                    _ if !focused => return None,
                    _ => {}
                }
                done
            }
            WM_CHAR => {
                let card = self.profile.as_mut()?;
                if !card.field.focused {
                    return None;
                }
                let c = wp.0 as u16;
                if !ctrl && c >= 0x20 && c != 0x7f && !v.busy {
                    card.field.insert(&[c]);
                    self.profile_changed();
                }
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
        let (Some(v), Some(card)) = (self.profile_view(), self.profile.as_ref()) else {
            return self.profile_p.offset(-100_000.0, 0.0);
        };
        let Some(geo) = self.profile_geometry(&v) else {
            return self.profile_p.offset(-100_000.0, 0.0);
        };
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let title = v.profile.title();
        let avatar = Avatar::new(
            v.profile.avatar_id,
            &v.profile.avatar_asset,
            &v.profile.avatar_url,
        );
        let face = crate::live::person(&card.id, &title, avatar);
        let enabled: Vec<(PPart, Rect, bool)> = geo
            .parts
            .iter()
            .map(|(p, r)| (*p, *r, self.profile_enabled(&v, *p)))
            .collect();
        let (w, h) = (self.w, self.h);
        let g = &self.g;
        self.profile_p.ensure(g, px(w), px(h))?;
        self.profile_p.offset(0.0, 0.0)?;
        self.profile_p.draw(s, |dc| unsafe {
            dc.Clear(Some(&D2D1_COLOR_F::default()));
            let (cx, cy, cw, ch) = geo.card;
            let body = rect(cx + 0.5, cy + 0.5, cx + cw - 0.5, cy + ch - 0.5);
            fill_round(dc, body, RADIUS, &g.brushes.surface);
            stroke_round(dc, body, RADIUS, &g.brushes.border);
            let r = AVATAR / 2.0;
            avatars::circle(
                g,
                dc,
                &face.avatar,
                (cx + PAD + r, cy + PAD + r, r),
                face.color,
                &face.initials,
            )?;
            // The name over the @username, beside the avatar and before
            // the icon buttons.
            let icons_w =
                (ICON + 2.0) * (1 + usize::from(v.can_message) + usize::from(v.can_block)) as f32;
            let tx = cx + PAD + AVATAR + 12.0;
            let tw = (cx + cw - PAD - icons_w - 4.0 - tx).max(1.0);
            let name = layout(g, &title, &g.formats.name, tw)?;
            name.SetWordWrapping(
                windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP,
            )?;
            crate::text::draw(dc, &name, tx, cy + PAD + 6.0, &g.brushes.text);
            if v.profile.display_name.is_some()
                && let Some(u) = v.username()
            {
                let l = layout(g, &format!("@{u}"), &g.formats.direct_note, tw)?;
                l.SetWordWrapping(
                    windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP,
                )?;
                crate::text::draw(dc, &l, tx, cy + PAD + 8.0 + height(&name), &g.brushes.muted);
            }
            let inner = CARD_W - 2.0 * PAD;
            if let Some(y) = geo.you {
                let l = layout(g, "You", &g.formats.direct_note, inner)?;
                crate::text::draw(dc, &l, cx + PAD, y, &g.brushes.muted);
            }
            if let (Some(y), Some(s)) = (geo.status, &v.status) {
                let l = layout(g, s, &g.formats.direct_note, inner)?;
                crate::text::draw(dc, &l, cx + PAD, y, &g.brushes.muted);
            }
            if let (Some((y, _)), Some(e)) = (geo.error, &v.error) {
                let l = layout(g, e, &g.formats.error, inner)?;
                crate::text::draw(dc, &l, cx + PAD, y, &g.solid(ERROR));
            }
            if let Some(y) = geo.divider {
                dc.FillRectangle(
                    &rect(cx + 1.0, y, cx + cw - 1.0, y + 1.0),
                    &g.brushes.border,
                );
            }
            for &(part, (x, y, pw, ph), on) in &enabled {
                let hover = card.hover == Some(part) && on;
                let r = rect(x, y, x + pw, y + ph);
                let tint = if on { TEXT } else { TEXT_MUTED };
                let alpha = if on { 1.0 } else { 0.5 };
                match part {
                    PPart::Picture | PPart::Invite | PPart::Block => {}
                    PPart::Close | PPart::Open | PPart::More => {
                        if hover || (part == PPart::More && card.more) {
                            fill_round(dc, r, 6.0, &g.brushes.hover);
                        }
                        let i = match part {
                            PPart::Close => Icon::Close,
                            PPart::Open => Icon::MessageCircle,
                            _ => Icon::More,
                        };
                        icons::draw(
                            dc,
                            i,
                            rect(x + 6.0, y + 6.0, x + pw - 6.0, y + ph - 6.0),
                            if hover { TEXT } else { TEXT_MUTED },
                            alpha,
                        );
                    }
                    PPart::Space(k) => {
                        let label = v.spaces.get(k).map_or("", |s| s.1.as_str());
                        fill_round(
                            dc,
                            r,
                            7.0,
                            if hover {
                                &g.brushes.border
                            } else {
                                &g.brushes.raised
                            },
                        );
                        stroke_round(dc, r, 7.0, &g.brushes.border);
                        let l = layout(g, label, &g.formats.button, pw - 24.0)?;
                        let brush = g.solid(tint);
                        brush.SetOpacity(alpha);
                        crate::text::draw(dc, &l, x + 12.0, y + (ph - height(&l)) / 2.0, &brush);
                    }
                    PPart::Field => {
                        fill_round(dc, r, RADIUS, &g.brushes.composer);
                        stroke_round(
                            dc,
                            r,
                            RADIUS,
                            if card.field.focused {
                                &g.brushes.terracotta
                            } else {
                                &g.brushes.border
                            },
                        );
                        let hint = format!("Message @{}", v.username().unwrap_or_default());
                        let hint =
                            layout(g, &hint, &g.formats.composer, pw - 2.0 * FIELD_PAD_X - SEND)?;
                        let (ox, oy) = (x + FIELD_PAD_X, y + FIELD_PAD_Y);
                        dc.PushAxisAlignedClip(
                            &rect(x + 1.0, y + 1.0, x + pw - SEND - 4.0, y + ph - 1.0),
                            D2D1_ANTIALIAS_MODE_ALIASED,
                        );
                        card.field.draw_text(
                            g,
                            dc,
                            ox,
                            oy - card.field.scroll(),
                            Some((&hint, &g.brushes.hint)),
                        );
                        dc.PopAxisAlignedClip();
                    }
                    PPart::Send => {
                        if on {
                            fill_round(
                                dc,
                                r,
                                7.0,
                                &g.solid(if hover { ACTION_HOVER } else { ACTION }),
                            );
                        }
                        icons::draw(
                            dc,
                            Icon::Send,
                            rect(x + 8.0, y + 8.0, x + pw - 8.0, y + ph - 8.0),
                            if on { BLACKOUT } else { TEXT_MUTED },
                            if on { 1.0 } else { 0.6 },
                        );
                    }
                }
            }
            // The More menu over everything.
            if let Some((mx, my, mw, mh)) = geo.menu {
                let m = rect(mx, my, mx + mw, my + mh);
                fill_round(dc, m, RADIUS, &g.brushes.surface);
                stroke_round(dc, m, RADIUS, &g.brushes.border);
                for &(part, (x, y, pw, ph), on) in &enabled {
                    if !matches!(part, PPart::Invite | PPart::Block) {
                        continue;
                    }
                    if card.hover == Some(part) && on {
                        fill_round(dc, rect(x, y, x + pw, y + ph), 4.0, &g.brushes.border);
                    }
                    let l = layout(g, &v.part_name(part), &g.formats.menu_item, pw - 16.0)?;
                    let brush = g.solid(TEXT);
                    brush.SetOpacity(if on { 1.0 } else { 0.5 });
                    crate::text::draw(dc, &l, x + 8.0, y + (ph - height(&l)) / 2.0, &brush);
                }
            }
            Ok(())
        })
    }
}
