//! A person's profile card, as on the egui desktop: their avatar (a picture
//! opens the viewer), name and @username, Open conversation, a More menu
//! (Invite to space, Block or Unblock) and a composer that sends them a DM.
//! Your own card says "You". Opened from member rows, authors and mention
//! pills. A child of `app`, so it reads and changes the app's state.

use super::{App, popup};
use crate::images::{Avatar, AvatarView, Icon};
use crate::ui::label;
use caper_client::mentions::{self, Profile};
use caper_client::model::{Author, BlockedAccount, Space};
use caper_client::worker::{AccountOperation, Command};
use gtk::{gdk, glib, prelude::*};
use std::collections::HashMap;
use std::rc::Rc;

/// What the open card is doing, kept in the app's state.
#[derive(Default)]
pub(super) struct CardState {
    pub person: String,
    pub author: Option<Author>,
    pub sending: bool,
    pub status: Option<String>,
    pub error: Option<String>,
    pub choosing_space: bool,
    /// The invitation on its way: its request and space.
    pub inviting: Option<(u64, String)>,
    /// Spaces already answered: "Invited" or "Member".
    pub invited: HashMap<String, &'static str>,
    /// An uncertain failed send retries with the same message id.
    pub retry_id: Option<String>,
}

/// The open card's widgets.
pub(super) struct Card {
    pub popover: gtk::Popover,
    pub person: String,
    identity: gtk::Box,
    /// Built once and updated in place: replacing a widget that owns an
    /// open (or closing) popover leaves its pointer grab behind on GTK 4.8.
    more: gtk::ToggleButton,
    invite: gtk::Button,
    block: gtk::Button,
    open: gtk::Button,
    close: gtk::Button,
    details: gtk::Box,
    composer: Option<(gtk::TextView, gtk::Button)>,
    /// The identity row was built for this name and picture.
    built: std::cell::RefCell<Option<Profile>>,
}

impl App {
    /// Opens `id`'s card at `anchor`, replacing any other.
    pub(super) fn open_profile(
        self: &Rc<Self>,
        id: String,
        author: Option<Author>,
        anchor: &gtk::Widget,
    ) {
        if self
            .state
            .borrow()
            .card
            .as_ref()
            .is_some_and(|card| card.sending)
        {
            return;
        }
        self.close_profile();
        let Some(profile) = self.state.borrow().profile_of(&id, author.as_ref()) else {
            return;
        };
        self.state.borrow_mut().card = Some(CardState {
            person: id.clone(),
            author,
            ..CardState::default()
        });
        let popover = gtk::Popover::new();
        popover.add_css_class("profile-card");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.set_size_request(288, -1);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let identity = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        identity.set_hexpand(true);
        top.append(&identity);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        actions.set_valign(gtk::Align::Start);
        top.append(&actions);
        // More reveals its actions inside the card: a popover nested in
        // this one loses the pointer grab on GTK 4.8 once an item is clicked.
        let more: gtk::ToggleButton = crate::app::named("GtkToggleButton", "More profile actions");
        more.set_child(Some(&self.images.icon_image(Icon::More, "#b9bcbe", 18)));
        more.add_css_class("flat");
        let items = gtk::Box::new(gtk::Orientation::Vertical, 2);
        items.add_css_class("profile-actions");
        let invite = gtk::Button::with_label("Invite to space");
        let block = gtk::Button::with_label("Block");
        for item in [&invite, &block] {
            item.add_css_class("flat");
            if let Some(text) = item.child().and_downcast::<gtk::Label>() {
                text.set_xalign(0.0);
            }
            items.append(item);
        }
        more.bind_property("active", &items, "visible")
            .sync_create()
            .build();
        let (app, toggle) = (self.clone(), more.clone());
        invite.connect_clicked(move |_| {
            toggle.set_active(false);
            if let Some(card) = app.state.borrow_mut().card.as_mut() {
                card.choosing_space = !card.choosing_space;
            }
            app.render_profile();
        });
        let (app, toggle) = (self.clone(), more.clone());
        block.connect_clicked(move |_| {
            toggle.set_active(false);
            let app = app.clone();
            glib::idle_add_local_once(move || app.profile_block());
        });
        let open: gtk::Button = crate::app::named("GtkButton", "Open conversation");
        open.set_child(Some(&self.images.icon_image(
            Icon::MessageCircle,
            "#b9bcbe",
            18,
        )));
        open.set_tooltip_text(Some("Open conversation"));
        open.add_css_class("flat");
        let app = self.clone();
        open.connect_clicked(move |_| app.profile_send(true));
        let close: gtk::Button = crate::app::named("GtkButton", "Close profile");
        close.set_child(Some(&self.images.icon_image(Icon::Close, "#b9bcbe", 18)));
        close.add_css_class("flat");
        let app = self.clone();
        close.connect_clicked(move |_| app.close_profile());
        for button in [
            more.upcast_ref::<gtk::Widget>(),
            open.upcast_ref(),
            close.upcast_ref(),
        ] {
            actions.append(button);
        }
        root.append(&top);
        root.append(&items);
        let details = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.append(&details);
        // Built for anyone you could message; hidden while they're blocked.
        let composer = (!profile.me && profile.username.is_some()).then(|| {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.add_css_class("composer");
            line.add_css_class("profile-composer");
            let view: gtk::TextView = crate::app::named("GtkTextView", "Message");
            view.set_wrap_mode(gtk::WrapMode::WordChar);
            view.set_accepts_tab(false);
            view.set_hexpand(true);
            let rich = crate::composing::attach(&view, &line);
            let placeholder = label(
                &format!(
                    "Message @{}",
                    profile.username.as_deref().unwrap_or_default()
                ),
                &["placeholder"],
            );
            placeholder.set_can_target(false);
            placeholder.set_valign(gtk::Align::Start);
            placeholder.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(
                &gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .propagate_natural_height(true)
                    .max_content_height(120)
                    .child(&view)
                    .hexpand(true)
                    .build(),
            ));
            overlay.add_overlay(&placeholder);
            overlay.set_hexpand(true);
            line.append(&overlay);
            let empty = placeholder.clone();
            view.buffer()
                .connect_changed(move |buffer| empty.set_visible(buffer.char_count() == 0));
            let send: gtk::Button = crate::app::named("GtkButton", "Send message");
            send.set_child(Some(&self.images.icon_image(Icon::Send, "#b9bcbe", 18)));
            send.set_valign(gtk::Align::End);
            send.add_css_class("flat");
            send.add_css_class("send");
            line.append(&send);
            root.append(&line);
            let keys = gtk::EventControllerKey::new();
            keys.set_propagation_phase(gtk::PropagationPhase::Capture);
            let app = self.clone();
            keys.connect_key_pressed(move |_, key, _, modifiers| {
                if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                    && !modifiers.contains(gdk::ModifierType::SHIFT_MASK)
                    && !rich.composing()
                {
                    app.profile_send(false);
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
            view.add_controller(keys);
            let app = self.clone();
            send.connect_clicked(move |_| app.profile_send(false));
            (view, send)
        });
        popover.set_child(Some(&root));
        let app = Rc::downgrade(self);
        let person = id.clone();
        popover.connect_closed(move |_| {
            let Some(app) = app.upgrade() else {
                return;
            };
            // A send keeps its state so the answer still lands.
            let ours = app
                .profile
                .try_borrow()
                .is_ok_and(|card| card.as_ref().is_some_and(|card| card.person == person));
            if ours {
                let app = app.clone();
                glib::idle_add_local_once(move || {
                    app.profile.borrow_mut().take();
                    let mut state = app.state.borrow_mut();
                    if state.card.as_ref().is_some_and(|card| !card.sending) {
                        state.card = None;
                    }
                });
            }
        });
        *self.profile.borrow_mut() = Some(Card {
            popover: popover.clone(),
            person: id,
            identity,
            more,
            invite,
            block,
            open,
            close,
            details,
            composer: composer.clone(),
            built: std::cell::RefCell::default(),
        });
        self.render_profile();
        popup(&popover, anchor);
        if let Some((view, _)) = composer {
            view.grab_focus();
        }
    }

    pub(super) fn close_profile(&self) {
        let card = self.profile.borrow_mut().take();
        if let Some(card) = card {
            card.popover.popdown();
        }
        let mut state = self.state.borrow_mut();
        if state.card.as_ref().is_some_and(|card| !card.sending) {
            state.card = None;
        }
    }

    /// Redraws the open card from the state.
    pub(super) fn render_profile(self: &Rc<Self>) {
        let Ok(card) = self.profile.try_borrow() else {
            return;
        };
        let Some(card) = card.as_ref() else {
            return;
        };
        let state = self.state.borrow();
        let Some(open) = state
            .card
            .as_ref()
            .filter(|open| open.person == card.person)
        else {
            return;
        };
        let Some(profile) = state.profile_of(&open.person, open.author.as_ref()) else {
            return;
        };
        let person = open.person.clone();
        let blocked = state.blocked(&person);
        let protected =
            open.author.as_ref().is_some_and(|author| author.admin) || state.protected(&person);
        let spaces = state.invitable_spaces(&person);
        let can_message = state.can_message(&person, &profile);
        let can_invite = can_message && !spaces.is_empty();
        let can_block =
            !profile.me && state.account.is_some() && (blocked || !protected || can_invite);
        let busy = open.sending || open.inviting.is_some() || state.block_busy;

        let name = profile.title();
        while let Some(child) = card.details.first_child() {
            card.details.remove(&child);
        }
        if card.built.borrow().as_ref() != Some(&profile) {
            *card.built.borrow_mut() = Some(profile.clone());
            while let Some(child) = card.identity.first_child() {
                card.identity.remove(&child);
            }
            self.profile_identity(&card.identity, &profile, &name);
        }
        card.more.set_visible(can_block);
        card.more.set_sensitive(!busy);
        card.invite.set_visible(can_invite);
        card.block.set_visible(blocked || !protected);
        card.block
            .set_label(if blocked { "Unblock" } else { "Block" });
        card.open.set_visible(can_message);
        card.open.set_sensitive(!busy && state.token.is_some());
        card.close.set_sensitive(!busy);
        if profile.me {
            card.details.append(&label("You", &["muted", "small"]));
            // As on the desktop, your own card edits your profile.
            if state.account.is_some() {
                let edit = gtk::Button::with_label("Edit profile");
                edit.add_css_class("primary");
                let weak = Rc::downgrade(self);
                edit.connect_clicked(move |_| {
                    if let Some(app) = weak.upgrade() {
                        app.open_profile_editor();
                    }
                });
                card.details.append(&edit);
            }
        }
        if can_invite && open.choosing_space {
            for space in &spaces {
                let text = if open
                    .inviting
                    .as_ref()
                    .is_some_and(|(_, id)| id == &space.id)
                {
                    "Sending…".to_owned()
                } else if let Some(already) = open.invited.get(&space.id) {
                    format!("{} · {already}", space.name)
                } else {
                    space.name.clone()
                };
                let button = gtk::Button::with_label(&text);
                button.set_sensitive(!busy && !open.invited.contains_key(&space.id));
                let (app, space, username) = (
                    self.clone(),
                    space.clone(),
                    profile.username.clone().unwrap_or_default(),
                );
                let user = person.clone();
                button
                    .connect_clicked(move |_| app.invite_to_space(space.clone(), &username, &user));
                card.details.append(&button);
            }
        }
        if let Some(status) = &open.status {
            let status = label(status, &["muted", "small"]);
            status.set_wrap(true);
            card.details.append(&status);
        }
        if let Some(error) = &open.error {
            let error = label(error, &["error", "small"]);
            error.set_wrap(true);
            card.details.append(&error);
        }
        card.details
            .set_visible(card.details.first_child().is_some());
        if let Some((view, send)) = &card.composer {
            view.set_editable(!open.sending);
            send.set_sensitive(!busy);
            if let Some(line) = send.parent() {
                line.set_visible(can_message);
            }
        }
    }

    /// The card's avatar (a picture opens the viewer), name and @username.
    fn profile_identity(self: &Rc<Self>, identity: &gtk::Box, profile: &Profile, name: &str) {
        let avatar = AvatarView::new(48, false);
        avatar.set(
            &self.images,
            &Avatar::new(
                profile
                    .display_name
                    .as_deref()
                    .or(profile.username.as_deref())
                    .unwrap_or("Profile"),
                profile.avatar_id,
                &profile.avatar_asset,
                &profile.avatar_url,
            ),
        );
        if let Some(picture) = profile
            .picture()
            .and_then(|source| caper_client::attachments::profile_picture(source, name))
        {
            let click = gtk::GestureClick::new();
            let app = self.clone();
            click.connect_released(move |_, _, _, _| {
                crate::viewer::open(&app.ui.window, &app.images, vec![picture.clone()], 0);
            });
            avatar.root.add_controller(click);
            avatar.root.set_cursor_from_name(Some("pointer"));
            avatar.root.set_tooltip_text(Some("Open profile picture"));
        }
        identity.append(&avatar.root);
        let names = gtk::Box::new(gtk::Orientation::Vertical, 2);
        names.set_valign(gtk::Align::Center);
        let title = label(name, &["profile-name"]);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        names.append(&title);
        if profile.display_name.is_some()
            && let Some(username) = &profile.username
        {
            let handle = label(&format!("@{username}"), &["muted", "small"]);
            handle.set_ellipsize(gtk::pango::EllipsizeMode::End);
            names.append(&handle);
        }
        identity.append(&names);
    }

    /// The card's Block (asks first) or Unblock.
    fn profile_block(self: &Rc<Self>) {
        let account = {
            let state = self.state.borrow();
            let Some(open) = state.card.as_ref() else {
                return;
            };
            let Some(profile) = state.profile_of(&open.person, open.author.as_ref()) else {
                return;
            };
            (
                state.blocked(&open.person),
                BlockedAccount {
                    id: open.person.clone(),
                    username: profile.username.clone().unwrap_or_default(),
                    display_name: profile.title(),
                    avatar_id: profile.avatar_id,
                    avatar_asset: profile.avatar_asset.clone(),
                    avatar_url: profile.avatar_url.clone(),
                },
            )
        };
        match account {
            (true, account) => self.set_block(account, false, None),
            (false, account) => {
                self.close_profile();
                self.confirm_block(account, None);
            }
        }
    }

    /// Sends the card's draft to the person's DM (`open_only`: only opens
    /// it), creating the conversation when needed.
    fn profile_send(self: &Rc<Self>, open_only: bool) {
        let text = self
            .profile
            .borrow()
            .as_ref()
            .and_then(|card| card.composer.as_ref())
            .map(|(view, _)| {
                let buffer = view.buffer();
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), true)
                    .to_string()
            })
            .unwrap_or_default();
        let mut state = self.state.borrow_mut();
        let Some(token) = state.token.clone() else {
            return;
        };
        let name = state.name();
        let uploads = state.uploads.clone();
        let epoch = state.epoch;
        let Some(open) = state.card.as_ref() else {
            return;
        };
        if open.sending || open.inviting.is_some() {
            return;
        }
        let Some(profile) = state.profile_of(&open.person, open.author.as_ref()) else {
            return;
        };
        let (person, retry) = (open.person.clone(), open.retry_id.clone());
        if !state.can_message(&person, &profile) {
            return;
        }
        let Some(username) = profile.username else {
            return;
        };
        if !open_only && text.trim().is_empty() {
            return;
        }
        if text.chars().count() > 4_000 {
            if let Some(card) = state.card.as_mut() {
                card.error = Some("Messages can be at most 4,000 characters.".into());
            }
            drop(state);
            self.render_profile();
            return;
        }
        let card = state.card.as_mut().expect("checked above");
        card.sending = true;
        card.error = None;
        card.status = Some(
            if open_only {
                "Opening…"
            } else {
                "Sending…"
            }
            .into(),
        );
        let client_id = retry.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        self.worker.send(Command::SendProfile {
            epoch,
            person: person.clone(),
            token,
            send: Box::new(caper_client::profile_dm::Send {
                person,
                username,
                name,
                client_id,
                text: if open_only { String::new() } else { text },
                files: Vec::new(),
                settings: uploads,
                cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                retrying: false,
                open_only,
                voice_only: false,
                direct: None,
            }),
        });
        drop(state);
        self.render_profile();
    }

    fn invite_to_space(self: &Rc<Self>, space: Space, username: &str, user: &str) {
        {
            let mut state = self.state.borrow_mut();
            state.membership_request += 1;
            let request = state.membership_request;
            if let Some(card) = state.card.as_mut() {
                card.inviting = Some((request, space.id.clone()));
                card.error = None;
                card.status = None;
            }
            self.account_op(
                &state,
                AccountOperation::InviteToSpace {
                    space,
                    username: username.to_owned(),
                    user: user.to_owned(),
                    request,
                },
            );
        }
        self.render_profile();
    }

    /// Block asks first; it hides their messages and stops their DMs.
    /// `request` is their message request, declined with the block.
    pub(super) fn confirm_block(self: &Rc<Self>, account: BlockedAccount, request: Option<String>) {
        if self.state.borrow().protected(&account.id) {
            return;
        }
        #[allow(deprecated)]
        let dialog = gtk::MessageDialog::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .message_type(gtk::MessageType::Question)
            .text(format!("Block {}?", account.display_name))
            .secondary_text(
                "You won't see their messages unless you choose to, and they can't send you DMs or requests.",
            )
            .build();
        dialog.add_css_class("caper");
        #[allow(deprecated)]
        {
            dialog.add_button("Cancel", gtk::ResponseType::Cancel);
            dialog.add_button("Block", gtk::ResponseType::Accept);
            if let Some(button) = dialog.widget_for_response(gtk::ResponseType::Accept) {
                button.add_css_class("destructive-action");
            }
        }
        let app = self.clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept {
                app.set_block(account.clone(), true, request.clone());
            }
            dialog.close();
        });
        dialog.present();
    }

    pub(super) fn set_block(
        self: &Rc<Self>,
        account: BlockedAccount,
        blocked: bool,
        request: Option<String>,
    ) {
        {
            let mut state = self.state.borrow_mut();
            state.block_busy = true;
            if let Some(card) = state.card.as_mut() {
                card.error = None;
            }
            self.account_op(
                &state,
                AccountOperation::SetBlock {
                    account,
                    blocked,
                    request,
                },
            );
        }
        self.render_profile();
        self.render();
    }
}

impl super::State {
    /// The card for `id` from what is loaded: an author's own identity when
    /// nothing else knows them.
    pub(super) fn profile_of(&self, id: &str, author: Option<&Author>) -> Option<Profile> {
        let me = self.account.as_ref().map(|account| account.id.as_str());
        let members = self
            .detail
            .as_ref()
            .map_or(&[][..], |detail| detail.members.as_slice());
        let people = self.people.as_deref().unwrap_or_default();
        let mut profile = match author {
            Some(author) => Some(mentions::author_profile(
                author,
                me,
                members,
                people,
                &self.directs,
            )),
            None => mentions::profile(
                &caper_client::model::Mention {
                    kind: "user".into(),
                    id: Some(id.to_owned()),
                    username: None,
                },
                me,
                members,
                people,
                &self.directs,
            ),
        }?;
        if profile.me
            && let Some(account) = &self.account
        {
            profile.username.clone_from(&account.username);
            profile.display_name.clone_from(&account.display_name);
        }
        Some(profile)
    }

    /// Everyone you blocked, for collapsing their messages.
    pub(super) fn blocked_ids(&self) -> std::collections::BTreeSet<String> {
        self.blocks
            .iter()
            .flatten()
            .map(|blocked| blocked.id.clone())
            .chain(
                self.directs
                    .iter()
                    .filter(|direct| direct.blocked)
                    .map(|direct| direct.peer.id.clone()),
            )
            .collect()
    }

    pub(super) fn blocked(&self, id: &str) -> bool {
        self.blocks
            .as_ref()
            .is_some_and(|blocks| blocks.iter().any(|blocked| blocked.id == id))
            || self
                .directs
                .iter()
                .any(|direct| direct.peer.id == id && direct.blocked)
    }

    /// Caper team accounts can't be blocked.
    /// A message author you may block or unblock from its menu: not you,
    /// a guest, or (unless already blocked) Caper staff.
    pub(super) fn blockable_author(
        &self,
        author: &caper_client::model::Author,
    ) -> Option<BlockedAccount> {
        let me = self.account.as_ref().map(|account| account.id.as_str());
        if author.is_guest
            || Some(author.id.as_str()) == me
            || (!self.blocked(&author.id) && (author.admin || self.protected(&author.id)))
        {
            return None;
        }
        let username = self
            .detail
            .iter()
            .flat_map(|detail| &detail.members)
            .find(|member| member.id == author.id)
            .map(|member| member.username.clone())
            .or_else(|| {
                self.directs
                    .iter()
                    .find(|direct| direct.peer.id == author.id)
                    .map(|direct| direct.peer.username.clone())
            })
            .unwrap_or_default();
        Some(BlockedAccount {
            id: author.id.clone(),
            username,
            display_name: author.name.clone(),
            avatar_id: author.avatar_id,
            avatar_asset: author.avatar_asset.clone(),
            avatar_url: author.avatar_url.clone(),
        })
    }

    pub(super) fn protected(&self, id: &str) -> bool {
        self.directs
            .iter()
            .any(|direct| direct.peer.id == id && (direct.peer.admin || direct.peer.team))
            || self
                .people
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|person| person.id == id && person.admin)
            || self
                .detail
                .iter()
                .flat_map(|detail| &detail.members)
                .any(|member| member.id == id && member.admin)
    }

    fn can_message(&self, id: &str, profile: &Profile) -> bool {
        !profile.me && !self.blocked(id) && profile.username.is_some()
    }

    /// Spaces you own that `id` is not in (the open one only if they aren't).
    fn invitable_spaces(&self, id: &str) -> Vec<Space> {
        let member_of_open = self
            .detail
            .as_ref()
            .filter(|detail| detail.members.iter().any(|member| member.id == id))
            .map(|detail| detail.space.id.as_str());
        self.spaces
            .iter()
            .filter(|space| {
                !space.demo
                    && self
                        .account
                        .as_ref()
                        .is_some_and(|account| account.id == space.owner_id)
                    && member_of_open != Some(space.id.as_str())
            })
            .cloned()
            .collect()
    }
}
