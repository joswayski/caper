//! Jumping to a message with its context, as on the egui desktop: a reply
//! quote or Pins' "Go to message" loads the history around a message the
//! list doesn't hold (`LoadContext`), and the list then shows only that
//! window. "Load newer messages" extends it; "Back to latest messages"
//! returns to the live end. A quoted thread reply opens its thread at that
//! reply instead. A child of `app`.

use super::{App, State};
use caper_client::model::{Message, MessageContext};
use caper_client::worker::{Command, LoadError};
use gtk::prelude::*;
use std::rc::Rc;

/// The sequences the main list shows while it is away from the live end:
/// from `start`, through `end` when newer messages exist past it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Window {
    pub(super) start: u64,
    pub(super) end: Option<u64>,
}

impl Window {
    pub(super) fn contains(self, message: &Message) -> bool {
        message
            .seq
            .parse::<u64>()
            .is_ok_and(|seq| seq >= self.start && self.end.is_none_or(|end| seq <= end))
    }
}

impl App {
    /// The quote or "Go to message": in view already, it scrolls there;
    /// in a thread, the thread opens at it; otherwise its context loads.
    pub(super) fn jump_to_message(self: &Rc<Self>, id: String, root: Option<String>) {
        if let Some(pins) = self.pins.borrow_mut().take() {
            pins.popover.popdown();
        }
        if let Some(root) = root {
            self.open_thread_at(root, Some(id));
            return;
        }
        let shown = self.state.borrow().rows.iter().any(|row| row.id == id);
        if shown {
            {
                let mut state = self.state.borrow_mut();
                state.focused = Some(id.clone());
                state.jump_to = Some(id);
            }
            self.render_rows();
            self.continue_jump();
            return;
        }
        {
            let mut state = self.state.borrow_mut();
            self.load_context(&mut state, id, false);
        }
        self.render();
    }

    fn load_context(&self, state: &mut State, anchor: String, newer: bool) {
        let Some(channel) = state.channel.clone() else {
            return;
        };
        state.request += 1;
        state.loading_older = false;
        state.context_loading = true;
        state.error = None;
        self.worker.send(Command::LoadContext {
            generation: state.generation,
            request: state.request,
            token: state.token.clone(),
            channel,
            root: None,
            anchor,
            newer,
        });
    }

    pub(super) fn load_newer(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if state.context_loading {
                return;
            }
            let Some(end) = state.context_window.and_then(|window| window.end) else {
                return;
            };
            self.load_context(&mut state, end.to_string(), true);
        }
        self.render();
    }

    pub(super) fn back_to_latest(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            self.reload(&mut state);
        }
        self.follow.set(true);
        self.render();
    }

    pub(super) fn accept_context(
        &self,
        state: &mut State,
        anchor: &str,
        newer: bool,
        result: Result<MessageContext, LoadError>,
    ) {
        state.context_loading = false;
        let page = match result {
            Ok(page) => page,
            Err(error) => {
                state.error = Some(error.message);
                return;
            }
        };
        let channel = state.channel.clone();
        if !page.messages.iter().all(|message| {
            Some(&message.channel_id) == channel.as_ref() && message.is_channel_message()
        }) || (!newer && !page.messages.iter().any(|message| message.id == anchor))
        {
            state.error = Some("Caper returned invalid message context.".into());
            return;
        }
        let seq = |message: Option<&Message>| message.and_then(|message| message.seq.parse().ok());
        let start = seq(page.messages.first()).unwrap_or(0);
        let end = seq(page.messages.last()).filter(|_| page.has_newer);
        if let Err(error) = state.timeline.prepend(page.messages) {
            state.error = Some(error);
            return;
        }
        state.has_newer = page.has_newer;
        if newer {
            if let Some(window) = &mut state.context_window {
                window.end = end;
            }
        } else {
            state.thread = None;
            state.context_window = Some(Window { start, end });
            state.has_more = page.has_more;
            state.focused = Some(anchor.to_owned());
            state.jump_to = Some(anchor.to_owned());
            self.follow.set(false);
        }
        // Back at the live end: nothing hides newer messages any more.
        if !state.has_newer && newer {
            state.context_window = state.context_window.map(|window| Window {
                end: None,
                ..window
            });
        }
    }

    /// The bar under the list while it is away from the live end.
    pub(super) fn render_context_bar(&self, state: &State) {
        let ui = &self.ui;
        ui.newer_bar.set_visible(state.has_newer);
        ui.load_newer.set_sensitive(!state.context_loading);
        ui.load_newer.set_label(if state.context_loading {
            "Loading…"
        } else {
            "Load newer messages"
        });
    }
}
