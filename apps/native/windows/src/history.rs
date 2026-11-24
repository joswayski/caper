//! Paging the channel history: the head row above the first message ("Load
//! older messages", "Loading…", the error with Retry, or "Beginning of
//! conversation"), loading the previous page when the list reaches its
//! start, and putting that page above the shown rows without moving them.
//! After View original jumped into the past, the foot below the last row
//! offers "Load newer messages", padded clear of the "You're viewing older
//! messages" toast floating over it (`jump`), whose Jump to latest reloads
//! the latest page; the jumped-to message is centred and highlighted.
use crate::list::{self, HEAD_H, Head};
use crate::live::{self, Older};
use crate::model::Conversation;
use crate::{App, EV_ROWS, side_layouts};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows::core::Result;

/// Steps the jumped-to message's highlight while it holds and fades.
pub const TIMER_FOCUS: usize = 50;
/// The highlight holds at full strength, then fades to nothing.
const FOCUS_HOLD: f64 = 1.0;
const FOCUS_FADE: f64 = 4.0;

/// The highlight's strength `elapsed` seconds after a jump: 1 for the first
/// second, then the quadratic ease-out (1 - t)² over four seconds, as web's
/// cubic-bezier(0.33, 0.67, 0.67, 1). With reduced motion it holds the five
/// seconds and then goes at once.
pub fn focus_strength(elapsed: f64, motion: bool) -> f32 {
    if elapsed >= FOCUS_HOLD + FOCUS_FADE {
        0.0
    } else if elapsed < FOCUS_HOLD || !motion {
        1.0
    } else {
        let t = (elapsed - FOCUS_HOLD) / FOCUS_FADE;
        ((1.0 - t) * (1.0 - t)) as f32
    }
}

impl App {
    /// The head's height: shown above the first message of a live channel
    /// once that message is measured.
    pub fn head_h(&self) -> f32 {
        if self.live.is_some() && self.rows.lo == 0 && !self.c.messages.is_empty() {
            HEAD_H
        } else {
            0.0
        }
    }

    /// The head's top in list coordinates, while it exists.
    pub fn head_top(&self) -> Option<f64> {
        (self.head_h() > 0.0).then(|| self.rows.top[0] - HEAD_H as f64)
    }

    /// The head's button (or label) in client DIPs.
    pub fn head_rect(&self) -> Option<(f32, f32, f32, f32)> {
        let top = self.head_top()?;
        let state = self.live.as_ref()?.older();
        let r = list::head_rect(&self.g, state, self.list_w()).ok()?;
        let y = self.history_y() + (top - self.scroll_top) as f32;
        Some((
            self.list_x() + r.left,
            y + r.top,
            r.right - r.left,
            r.bottom - r.top,
        ))
    }

    /// Whether (x, y) is on the head's button.
    pub fn on_head_button(&self, x: f32, y: f32) -> bool {
        let clickable = self
            .live
            .as_ref()
            .is_some_and(|l| matches!(l.older(), Older::More | Older::Failed));
        clickable
            && self.in_history(x, y)
            && self
                .head_rect()
                .is_some_and(|(l, t, w, h)| x >= l && x < l + w && y >= t && y < t + h)
    }

    /// Clicks and UIA Invoke on the head's button.
    pub fn load_older(&mut self) {
        if self.live.as_mut().is_some_and(live::Live::load_older) {
            self.redraw_head = true;
            self.need = true;
        }
        if let Err(e) = self.drain_live() {
            eprintln!("older messages failed: {e:?}");
        }
    }

    pub fn track_head_hover(&mut self) {
        let hover = self.pointer.is_some_and(|(x, y)| self.on_head_button(x, y));
        if hover != self.head_hover {
            self.head_hover = hover;
            self.redraw_head = true;
            self.need = true;
        }
    }

    /// An older page arrived as the whole history again. When the shown
    /// messages are its newest ones, the new rows go above them and the
    /// reader's place stays; otherwise it is shown afresh.
    pub fn prepend_history(&mut self, mut c: Conversation, ids: live::Ids) -> Result<()> {
        let k = c.messages.len().saturating_sub(self.c.messages.len());
        let kept = c.messages[k..]
            .iter()
            .map(|m| &m.id)
            .eq(self.c.messages.iter().map(|m| &m.id));
        // Presence and reaction errors live on the UI's copy.
        c.presence = self.c.presence;
        for (new, old) in c.members.iter_mut().zip(&self.c.members) {
            new.online = old.online;
        }
        for (new, old) in c.messages[k..].iter_mut().zip(&self.c.messages) {
            new.reaction_error = old.reaction_error.clone();
        }
        if !kept {
            self.ids = ids;
            return self.set_conversation(c);
        }
        if c.members.len() != self.c.members.len() {
            let (channels, members) = side_layouts(&self.g, &c)?;
            self.channel_layouts = channels;
            self.member_layouts = members;
            self.dirty.members = true;
        }
        self.c = c;
        self.ids = ids;
        self.rows.prepend(k);
        self.sel = self
            .sel
            .map(|((ai, ap), (bi, bp))| ((ai + k, ap), (bi + k, bp)));
        self.hover_row = None;
        self.menu = None;
        self.picker = None;
        self.drawn = None;
        self.events |= EV_ROWS;
        self.need = true;
        Ok(())
    }

    /// The foot's height: a history row and the room the toast floats in,
    /// while newer messages exist past the shown window.
    pub fn foot_h(&self) -> f32 {
        if self.live.as_ref().is_some_and(live::Live::has_newer) {
            HEAD_H + FOOT_PAD
        } else {
            0.0
        }
    }

    /// The foot's Load newer messages button.
    pub fn foot_head(&self) -> Head<'static> {
        let loading = self.live.as_ref().is_some_and(|l| l.context_loading);
        Head {
            label: FOOT,
            button: true,
            enabled: !loading,
            note: None,
        }
    }

    /// Load newer messages in client DIPs, while the foot exists.
    pub fn foot_rect(&self) -> Option<(f32, f32, f32, f32)> {
        if self.foot_h() == 0.0 || self.rows.lo >= self.rows.len() {
            return None;
        }
        let r = list::head_button_rect(&self.g, &self.foot_head(), self.list_w()).ok()?;
        let y = self.history_y() + (self.rows.bottom() - self.scroll_top) as f32;
        Some((
            self.list_x() + r.left,
            y + r.top,
            r.right - r.left,
            r.bottom - r.top,
        ))
    }

    /// Whether (x, y) is on Load newer messages (not under the toast).
    pub fn foot_at(&self, x: f32, y: f32) -> bool {
        self.in_history(x, y)
            && !self.jump_at(x, y)
            && self
                .foot_rect()
                .is_some_and(|(l, t, w, h)| x >= l && x < l + w && y >= t && y < t + h)
    }

    /// Clicks and UIA Invoke on Load newer messages.
    pub fn foot_activate(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        live.load_newer();
        self.redraw_foot = true;
        self.need = true;
        if let Err(e) = self.drain_live() {
            eprintln!("newer messages failed: {e:?}");
        }
    }

    /// Leaves a detached window for the latest page, landing at its end:
    /// the toast's Jump to latest there, and End.
    pub fn load_latest(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        live.back_to_latest();
        self.redraw_foot = true;
        self.need = true;
        if let Err(e) = self.drain_live() {
            eprintln!("latest messages failed: {e:?}");
        }
    }

    pub fn track_foot_hover(&mut self) {
        let hover = self.pointer.is_some_and(|(x, y)| self.foot_at(x, y));
        if hover != self.foot_hover {
            self.foot_hover = hover;
            self.redraw_foot = true;
            self.need = true;
        }
    }

    /// Scrolls row `id` to the middle of the history and highlights it, as
    /// egui does for View original.
    pub fn focus_message(&mut self, id: &str) -> Result<()> {
        let Some(&i) = self.ids.rows.get(id) else {
            return Ok(());
        };
        if i >= self.rows.len() {
            return Ok(());
        }
        self.set_focused(Some(id.to_owned()));
        // Every jump, to the same message too, restarts the highlight.
        self.focus_at = Some(std::time::Instant::now());
        self.focus_tick();
        // A fresh history has no width yet; measure at the drawn one so the
        // frame's rewrap does not move the rows under the new scroll.
        self.rows.set_width(list::Rows::body_width(self.list_w()));
        if self.rows.lo == self.rows.len() {
            self.rows.measure_up_to(&self.g, &self.c, f64::INFINITY)?;
        }
        while self.rows.lo > i {
            let y = self.rows.top[self.rows.lo] - 1.0;
            self.rows.measure_up_to(&self.g, &self.c, y)?;
        }
        let h = self.rows.height(i) as f64;
        self.scroll_top = self.rows.top[i] + h / 2.0 - self.vh() as f64 / 2.0;
        self.at_end = false;
        self.need = true;
        Ok(())
    }

    /// The highlighted message (View original's target), redrawing the
    /// rows that gain or lose it.
    pub fn set_focused(&mut self, id: Option<String>) {
        if id == self.focused {
            return;
        }
        for old in [self.focused.take(), id.clone()].into_iter().flatten() {
            if let Some(&i) = self.ids.rows.get(&old) {
                self.redraw.push(i);
            }
        }
        self.focused = id;
        self.need = true;
    }

    /// The highlight's current strength. It fades on its own clock and
    /// leaves `focused`, which the foot and the scroll position follow, as is.
    pub fn focus_strength(&self) -> f32 {
        self.focus_at.map_or(0.0, |at| {
            focus_strength(at.elapsed().as_secs_f64(), crate::files::motion())
        })
    }

    /// Redraws the highlighted row and waits for its next change: the end
    /// of the hold, the next fade frame, or (reduced motion) its removal.
    pub fn focus_tick(&mut self) {
        let elapsed = self
            .focus_at
            .map_or(f64::INFINITY, |at| at.elapsed().as_secs_f64());
        if let Some(&i) = self.focused.as_ref().and_then(|id| self.ids.rows.get(id)) {
            self.redraw.push(i);
            self.need = true;
        }
        let end = FOCUS_HOLD + FOCUS_FADE;
        let wait = if elapsed >= end {
            None
        } else if elapsed < FOCUS_HOLD {
            Some(FOCUS_HOLD - elapsed)
        } else if crate::files::motion() {
            Some(0.016)
        } else {
            Some(end - elapsed)
        };
        unsafe {
            match wait {
                Some(s) => {
                    let ms = (s * 1000.0).ceil().max(1.0) as u32;
                    SetTimer(Some(self.hwnd), TIMER_FOCUS, ms, None);
                }
                None => {
                    let _ = KillTimer(Some(self.hwnd), TIMER_FOCUS);
                }
            }
        }
    }

    /// Newer messages arrived as the whole window again. When the shown
    /// messages are its oldest ones, the new rows go after them and the
    /// reader's place stays; otherwise it is shown afresh.
    pub fn append_history(&mut self, mut c: Conversation, ids: live::Ids) -> Result<()> {
        let n = self.c.messages.len();
        let kept = c.messages.len() >= n
            && c.messages[..n]
                .iter()
                .map(|m| &m.id)
                .eq(self.c.messages.iter().map(|m| &m.id));
        c.presence = self.c.presence;
        for (new, old) in c.members.iter_mut().zip(&self.c.members) {
            new.online = old.online;
        }
        if !kept {
            self.ids = ids;
            return self.set_conversation(c);
        }
        for (new, old) in c.messages.iter_mut().zip(&self.c.messages) {
            new.reaction_error = old.reaction_error.clone();
        }
        if c.members.len() != self.c.members.len() {
            let (channels, members) = side_layouts(&self.g, &c)?;
            self.channel_layouts = channels;
            self.member_layouts = members;
            self.dirty.members = true;
        }
        let k = c.messages.len() - n;
        self.c = c;
        self.ids = ids;
        for _ in 0..k {
            self.rows.append(&self.g, &self.c)?;
        }
        self.at_end = false;
        self.drawn = None;
        self.events |= EV_ROWS;
        self.need = true;
        Ok(())
    }
}

impl App {
    /// The channel row `i`'s quote panel under client point (x, y): the
    /// whole card opens the original, as web, except its author's avatar and
    /// name, which open their profile.
    pub fn quote_link_at(&self, x: f32, y: f32) -> Option<usize> {
        if !self.in_history(x, y) || self.live.is_none() {
            return None;
        }
        let ay = self.scroll_top + (y - self.history_y()) as f64;
        let i = self.rows.row_at(ay)?;
        let ry = (ay - self.rows.top[i]) as f32;
        let r = self.rows.quote_panel(i)?;
        let lx = x - self.list_x();
        let grouped = self.c.messages.get(i)?.grouped;
        (lx >= r.left
            && lx < r.right
            && ry >= r.top
            && ry < r.bottom
            && self.rows.who_at(i, grouped, lx, ry) != Some(list::Who::Quote))
        .then_some(i)
    }

    /// Lifts the quote panel under the pointer and settles the one it left.
    pub fn track_quote_hover(&mut self) {
        let hover = self.pointer.and_then(|(x, y)| self.quote_link_at(x, y));
        if hover != self.rows.quote_hover {
            self.redraw.extend(self.rows.quote_hover);
            self.redraw.extend(hover);
            self.rows.quote_hover = hover;
            self.need = true;
        }
    }

    /// Row `i`'s View original in client DIPs, while the row is laid out.
    pub fn quote_link_rect(&self, i: usize) -> Option<(f32, f32, f32, f32)> {
        if self.live.is_none() || i < self.rows.lo || i >= self.rows.len() {
            return None;
        }
        let r = self.rows.quote_link(i)?;
        let y = self.history_y() + (self.rows.top[i] - self.scroll_top) as f32;
        Some((
            self.list_x() + r.left,
            y + r.top,
            r.right - r.left,
            r.bottom - r.top,
        ))
    }

    /// View original on row `i`'s quote: its thread, or the original
    /// centred and highlighted in the history.
    pub fn view_original_of(&mut self, i: usize) {
        let Some(r) = self.c.messages.get(i).and_then(|m| m.reply.as_ref()) else {
            return;
        };
        let (id, root) = (r.id.clone(), r.thread_root.clone());
        if let Some(live) = &mut self.live {
            live.view_original(&id, root.as_deref());
        }
        if let Err(e) = self.drain_live() {
            eprintln!("view original failed: {e:?}");
        }
    }
}

impl App {
    /// The read state arrived: the "New messages" divider goes above row
    /// `id`, which then starts its own group, as in the egui client.
    pub fn mark_unread(&mut self, id: &str) -> Result<()> {
        let Some(&i) = self.ids.rows.get(id) else {
            return Ok(());
        };
        let m = &mut self.c.messages[i];
        if m.unread {
            return Ok(());
        }
        m.unread = true;
        if m.grouped {
            m.grouped = false;
            // An ungrouped row carries "(edited)" in its time instead.
            if std::mem::take(&mut m.edited_inline) {
                m.time.push_str(" (edited)");
            }
        }
        if self.rows.replace(&self.g, &self.c, i)? {
            self.drawn = None;
        } else {
            self.redraw.push(i);
        }
        self.events |= EV_ROWS;
        self.need = true;
        Ok(())
    }
}

const FOOT: &str = "Load newer messages";
/// Below Load newer messages, as web's 68 px: the toast never covers it.
const FOOT_PAD: f32 = 68.0;
