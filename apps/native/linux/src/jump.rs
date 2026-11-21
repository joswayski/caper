//! "Jump to latest" over the message list and the thread
//! (`caper_client::jump`): shown more than half a screen from the newest
//! message, counting others' messages that land meanwhile; activating it
//! glides back to the live end and follows new messages again. A child of
//! `app`.

use super::App;
use caper_client::jump::{self, Jump};
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

impl App {
    fn jump_list(&self, thread: bool) -> &crate::ui::List {
        if thread {
            &self.ui.thread_list
        } else {
            &self.ui.list
        }
    }

    /// After a list scrolled or changed height. `live`: the main list's
    /// history reaches the live end (not a jump's detached window); `None`
    /// reads it from the state when that is free.
    pub(super) fn sync_jump(&self, thread: bool, live: Option<bool>) {
        let live = match live {
            Some(live) => live,
            None if thread => true,
            None => match self.state.try_borrow() {
                Ok(state) => !state.has_newer,
                Err(_) => return,
            },
        };
        let list = self.jump_list(thread);
        let adjustment = list.scroller.vadjustment();
        let page = adjustment.page_size();
        let distance = (adjustment.upper() - page - adjustment.value()).max(0.0);
        // Following is the list's own "read at the end" (48px of slack).
        let at_end = if thread {
            self.thread_follow.get()
        } else {
            self.follow.get()
        };
        let mut state = list.jump.state();
        if state.scrolled(distance, page, at_end, live) {
            list.jump.show(state);
        }
    }

    /// Someone else's message landed at a list's live end: counted while
    /// the button shows.
    pub(super) fn jump_arrived(&self, thread: bool) {
        let list = self.jump_list(thread);
        let mut state = list.jump.state();
        if state.arrived() {
            list.jump.show(state);
        }
    }

    /// A conversation or thread opened afresh: no button, no count.
    pub(super) fn reset_jump(&self, thread: bool) {
        self.jump_list(thread).jump.show(Jump::default());
    }

    /// The button: back to the newest message, gliding there unless motion
    /// is reduced or it is many screens away, then following again.
    pub(super) fn jump_to_latest(self: &Rc<Self>, thread: bool) {
        let list = self.jump_list(thread);
        let focused = list.jump.button.has_focus();
        list.jump.show(Jump::default());
        // The hidden button can't keep focus: it goes to the composer.
        if focused {
            if thread {
                self.ui.thread_composer.view.grab_focus();
            } else {
                self.focus_composer();
            }
        }
        let adjustment = list.scroller.vadjustment();
        let page = adjustment.page_size();
        let distance = adjustment.upper() - page - adjustment.value();
        let reduced =
            !gtk::Settings::default().is_none_or(|settings| settings.is_gtk_enable_animations());
        let moving = if thread {
            self.gliding.get()
        } else {
            self.settling.get()
        };
        if moving || !jump::glides(distance, page, reduced) {
            self.land(thread);
            return;
        }
        // The glide's own scrolling is not the reader's: it neither stops
        // following nor shows the button again.
        if thread {
            self.gliding.set(true);
        } else {
            self.settling.set(true);
        }
        let from = adjustment.value();
        let start = Cell::new(None);
        let app = self.clone();
        list.view.add_tick_callback(move |_, clock| {
            let now = clock.frame_time();
            let begun = start.get().unwrap_or(now);
            start.set(Some(begun));
            let t = (now - begun) as f64 / jump::GLIDE.as_micros() as f64;
            let adjustment = app.jump_list(thread).scroller.vadjustment();
            let to = adjustment.upper() - adjustment.page_size();
            adjustment.set_value(jump::glide_at(from, to, t));
            if t < 1.0 {
                return glib::ControlFlow::Continue;
            }
            if thread {
                app.gliding.set(false);
            } else {
                app.settling.set(false);
            }
            app.land(thread);
            glib::ControlFlow::Break
        });
    }

    /// At the newest message, following new ones.
    fn land(self: &Rc<Self>, thread: bool) {
        if thread {
            self.thread_follow.set(true);
            let count = self.ui.thread_list.store.n_items();
            if count > 0 {
                let _ = self
                    .ui
                    .thread_list
                    .view
                    .activate_action("list.scroll-to-item", Some(&(count - 1).to_variant()));
            }
        } else {
            self.follow.set(true);
            self.pin_tail();
        }
        self.reset_jump(thread);
    }
}
