//! DM message requests, as on the egui desktop: "Message requests" with a
//! count above the DMs, opening to the incoming requests. An open request
//! is read-only: its bar replaces the composer with Accept, Decline and
//! Block, and reading it marks nothing read. A DM with someone you blocked
//! shows "You blocked @user." with Unblock instead, and your own request
//! notes that it waits. A child of `app`.

use super::{App, State, clear_list, peer_avatar};
use crate::images::AvatarView;
use crate::ui::label;
use caper_client::model::{BlockedAccount, DirectConversation, DirectPeer, DirectStatus};
use caper_client::worker::{AccountOperation, AccountResult};
use gtk::{glib, prelude::*};
use std::rc::Rc;

/// The request in flight: Accept or Decline. Only the clicked one says so.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Answer {
    Accepting,
    Declining,
}

#[derive(Default)]
pub(super) struct Requests {
    /// "Message requests" is open (an open request keeps it open too).
    pub(super) open: bool,
    pub(super) busy: Option<Answer>,
    /// An answer or block that failed, shown in the bar.
    pub(super) error: Option<String>,
}

fn account_of(peer: &DirectPeer) -> BlockedAccount {
    BlockedAccount {
        id: peer.id.clone(),
        username: peer.username.clone(),
        display_name: peer.display_name.clone(),
        avatar_id: peer.avatar_id,
        avatar_asset: peer.avatar_asset.clone(),
        avatar_url: peer.avatar_url.clone(),
    }
}

impl State {
    /// The open DM, unless it is your own notes.
    pub(super) fn open_peer_direct(&self) -> Option<&DirectConversation> {
        let me = self.account.as_ref().map(|account| account.id.as_str());
        let id = self.direct.as_deref()?;
        self.directs
            .iter()
            .find(|direct| direct.id == id && Some(direct.peer.id.as_str()) != me)
    }

    /// The open conversation when it is an incoming message request.
    pub(super) fn open_request(&self) -> Option<&DirectConversation> {
        self.open_peer_direct()
            .filter(|direct| direct.status == DirectStatus::Incoming)
    }

    /// The open DM is with someone you blocked.
    pub(super) fn open_blocked(&self) -> bool {
        self.open_peer_direct()
            .is_some_and(|direct| direct.blocked || self.blocked(&direct.peer.id))
    }

    pub(super) fn incoming_requests(&self) -> impl Iterator<Item = &DirectConversation> {
        self.directs
            .iter()
            .filter(|direct| direct.status == DirectStatus::Incoming)
    }
}

impl App {
    pub(super) fn connect_requests(self: &Rc<Self>) {
        let ui = &self.ui;
        let app = self.clone();
        ui.requests.connect_row_activated(move |_, row| {
            let index = row.index();
            if index == 0 {
                let mut state = app.state.borrow_mut();
                let open = state.requests.open || state.open_request().is_some();
                state.requests.open = !open;
                state.layout += 1;
                drop(state);
                app.render();
                return;
            }
            let id = usize::try_from(index - 1)
                .ok()
                .and_then(|index| app.request_ids.borrow().get(index).cloned());
            if let Some(id) = id {
                app.open_direct(id);
            }
        });
        let app = self.clone();
        ui.request
            .accept
            .connect_clicked(move |_| app.answer_request(Answer::Accepting));
        let app = self.clone();
        ui.request
            .decline
            .connect_clicked(move |_| app.answer_request(Answer::Declining));
        let app = self.clone();
        ui.request.block.connect_clicked(move |_| {
            let target = app
                .state
                .borrow()
                .open_request()
                .map(|direct| (account_of(&direct.peer), direct.id.clone()));
            if let Some((account, id)) = target {
                app.confirm_block(account, Some(id));
            }
        });
        let app = self.clone();
        ui.request.unblock.connect_clicked(move |_| {
            let account = app
                .state
                .borrow()
                .open_peer_direct()
                .map(|direct| account_of(&direct.peer));
            if let Some(account) = account {
                app.state.borrow_mut().requests.error = None;
                app.set_block(account, false, None);
            }
        });
    }

    fn answer_request(self: &Rc<Self>, answer: Answer) {
        {
            let mut state = self.state.borrow_mut();
            let Some(id) = state.open_request().map(|direct| direct.id.clone()) else {
                return;
            };
            if state.requests.busy.is_some() {
                return;
            }
            state.requests.busy = Some(answer);
            state.requests.error = None;
            self.account_op(
                &state,
                match answer {
                    Answer::Accepting => AccountOperation::Accept(id),
                    Answer::Declining => AccountOperation::Decline(id),
                },
            );
        }
        self.render();
    }

    pub(super) fn request_result(&self, state: &mut State, result: AccountResult) {
        match result {
            AccountResult::Accepted { id, result } => {
                state.requests.busy = None;
                match result {
                    Ok(direct) => {
                        state.requests.error = None;
                        match state.directs.iter_mut().find(|item| item.id == id) {
                            Some(existing) => *existing = direct,
                            None => state.directs.push(direct),
                        }
                        if state.incoming_requests().next().is_none() {
                            state.requests.open = false;
                        }
                        // Accepted, it reads like any DM.
                        if state.direct.as_deref() == Some(&id) {
                            self.mark_read(state);
                        }
                    }
                    Err(error) => state.requests.error = Some(error),
                }
            }
            AccountResult::Declined { id, result } => {
                state.requests.busy = None;
                match result {
                    Ok(()) => {
                        state.requests.error = None;
                        self.leave_request(state, &id);
                    }
                    Err(error) => state.requests.error = Some(error),
                }
            }
            _ => {}
        }
        state.layout += 1;
    }

    /// A declined (or blocked) request goes at once; if it was open, the
    /// next request opens, else the first space.
    pub(super) fn leave_request(&self, state: &mut State, id: &str) {
        state.directs.retain(|direct| direct.id != id);
        state.drafts.remove(id);
        state.layout += 1;
        if state.direct.as_deref() != Some(id) {
            return;
        }
        let next = state
            .incoming_requests()
            .next()
            .map(|direct| direct.id.clone());
        if let Some(next) = next {
            state.requests.open = true;
            self.enter_direct(state, next);
        } else if let Some(space) = state.spaces.first().map(|space| space.id.clone()) {
            self.navigate(state, Some(space), None);
        } else {
            self.leave_conversation(state);
            state.channel = None;
        }
    }

    /// The sidebar's "Message requests": hidden without any.
    pub(super) fn render_request_navigation(&self, state: &State) {
        let ui = &self.ui;
        clear_list(&ui.requests);
        let requests: Vec<&DirectConversation> = state.incoming_requests().collect();
        ui.requests.set_visible(!requests.is_empty());
        let mut ids = self.request_ids.borrow_mut();
        ids.clear();
        if requests.is_empty() {
            return;
        }
        let open = state.requests.open || state.open_request().is_some();
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("request-header");
        let chevron = self
            .images
            .icon_image(crate::images::Icon::ChevronDown, "#b9bcbe", 14);
        if !open {
            header.add_css_class("collapsed");
        }
        header.append(&chevron);
        let title = label("Message requests", &[]);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        header.append(&title);
        header.append(&label(
            &requests.len().to_string(),
            &["muted", "request-count"],
        ));
        let row: gtk::ListBoxRow = super::named(
            "GtkListBoxRow",
            &format!(
                "Message requests, {} {}",
                requests.len(),
                if open { "expanded" } else { "collapsed" }
            ),
        );
        row.set_child(Some(&header));
        ui.requests.append(&row);
        if !open {
            return;
        }
        for request in requests {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.add_css_class("request-row");
            let avatar = AvatarView::new(22, false);
            avatar.set(&self.images, &peer_avatar(request));
            line.append(&avatar.root);
            let name = label(&request.peer.display_name, &[]);
            name.set_ellipsize(gtk::pango::EllipsizeMode::End);
            line.append(&name);
            let user = label(&format!("@{}", request.peer.username), &["muted"]);
            user.set_ellipsize(gtk::pango::EllipsizeMode::End);
            user.set_hexpand(true);
            user.set_xalign(0.0);
            line.append(&user);
            let row: gtk::ListBoxRow = super::named(
                "GtkListBoxRow",
                &format!(
                    "Message request from {} @{}",
                    request.peer.display_name, request.peer.username
                ),
            );
            row.set_child(Some(&line));
            ui.requests.append(&row);
            if state.direct.as_deref() == Some(&request.id) {
                ui.requests.select_row(Some(&row));
            }
            ids.push(request.id.clone());
        }
        if state.open_request().is_none() {
            ui.requests.unselect_all();
        }
    }

    /// The request or blocked bar in place of the composer, and the
    /// waiting note over it. Returns whether the composer is replaced.
    pub(super) fn render_request_bar(&self, state: &State) -> bool {
        let bar = &self.ui.request;
        let request = state.open_request();
        let blocked = request.is_none() && state.open_blocked();
        let direct = state.open_peer_direct();
        bar.root.set_visible(request.is_some() || blocked);
        let waiting = direct.filter(|direct| {
            !blocked && request.is_none() && direct.status == DirectStatus::Outgoing
        });
        bar.waiting.set_visible(waiting.is_some());
        if let Some(direct) = waiting {
            bar.waiting.set_label(&format!(
                "Waiting for @{} to accept. They'll see your messages when they do.",
                direct.peer.username
            ));
        }
        let error = state.requests.error.as_deref().unwrap_or_default();
        bar.error.set_label(error);
        bar.error.set_visible(!error.is_empty());
        let busy = state.requests.busy;
        bar.accept.set_visible(request.is_some());
        bar.decline.set_visible(request.is_some());
        bar.block
            .set_visible(request.is_some_and(|direct| !state.protected(&direct.peer.id)));
        bar.unblock.set_visible(blocked);
        if let Some(direct) = request {
            bar.text.set_markup(&format!(
                "<b>{}</b> (@{}) wants to message you. You don't share a space.",
                glib::markup_escape_text(&direct.peer.display_name),
                glib::markup_escape_text(&direct.peer.username),
            ));
            bar.accept.set_label(if busy == Some(Answer::Accepting) {
                "Accepting…"
            } else {
                "Accept"
            });
            bar.decline.set_label(if busy == Some(Answer::Declining) {
                "Declining…"
            } else {
                "Decline"
            });
            for button in [&bar.accept, &bar.decline, &bar.block] {
                button.set_sensitive(busy.is_none() && !state.block_busy);
            }
        } else if let Some(direct) = direct.filter(|_| blocked) {
            bar.text
                .set_label(&format!("You blocked @{}.", direct.peer.username));
            bar.unblock.set_label(if state.block_busy {
                "Unblocking…"
            } else {
                "Unblock"
            });
            bar.unblock.set_sensitive(!state.block_busy);
        }
        request.is_some() || blocked
    }
}
