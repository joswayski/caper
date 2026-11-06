//! Paging the channel history: the head row above the first message ("Load
//! older messages", "Loading…", the error with Retry, or "Beginning of
//! conversation"), loading the previous page when the list reaches its
//! start, and putting that page above the shown rows without moving them.
use crate::list::{self, HEAD_H};
use crate::live::{self, Older};
use crate::model::Conversation;
use crate::theme::HEADER_H;
use crate::{App, EV_ROWS, side_layouts};
use windows::core::Result;

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
        let y = HEADER_H + (top - self.scroll_top) as f32;
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
}
