//! Owner management and membership, as on the egui desktop: create, manage,
//! leave and delete spaces and channels; invite and remove members and
//! private channel grants; invite links (the primary link, custom links,
//! revoke and regenerate); joining with a pasted invite link; and space and
//! private channel invitations. Spaces and invitations changed elsewhere
//! reconcile every 15 seconds and when the window regains focus, as on web.
//! A child of `app`.
//!
//! `State::admin` decides which windows are open: `dialog` is the form
//! window and `confirm` the confirmation over it. `render_admin` opens,
//! closes and updates them to match.

use super::{App, State, clear, popup};
use crate::images::{Avatar, AvatarView};
use crate::people::PeopleField;
use crate::ui::label;
use caper_client::effects::Effect;
use caper_client::invites::{
    self, ExpiryUnit, INVITATION_SENT, InviteLink, InvitePreview, admin_error, invite_error,
};
use caper_client::mentions;
use caper_client::model::{Member, Space, SpaceLimits, Spaces};
use caper_client::names::{channel_name_error, normalize_channel, space_name_error};
use caper_client::notifications::Scope;
use caper_client::worker::{AccountOperation, AccountResult, AdminOperation, AdminResult, Command};
use gtk::{gio, glib, prelude::*};
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// "Copied" shows this long on a link's Copy button.
const COPIED_FOR: Duration = Duration::from_secs(2);
/// Web's membership reconcile interval.
const REFRESH: Duration = Duration::from_secs(15);

/// The form window.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum Dialog {
    CreateSpace,
    ManageSpace,
    InvitePeople,
    JoinInvite,
    CreateChannel,
    ManageChannel(String),
}

/// A confirmation, over the form window or alone.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum Confirm {
    Invitation(Space),
    LeaveSpace {
        id: String,
        name: String,
    },
    LeaveChannel {
        space: String,
        channel: String,
        name: String,
    },
    Delete {
        space: String,
        channel: Option<String>,
        name: String,
    },
    RemoveMember {
        space: String,
        channel: Option<String>,
        member: String,
        name: String,
        place: String,
    },
    /// Regenerate (or deactivate) an invite link.
    Link {
        code: String,
        regenerate: bool,
    },
}

/// The open space's invite links.
#[derive(Default)]
struct Links {
    /// Panel lifetime and the latest list or mutation.
    request: u64,
    /// The primary link's ensure or regenerate.
    default_request: u64,
    space: Option<String>,
    default_link: Option<InviteLink>,
    regenerating: bool,
    settings_open: bool,
    links: Vec<InviteLink>,
    loading: bool,
    creating: bool,
    revoking: BTreeSet<String>,
    /// The channel the primary link opens in, when invited from a channel.
    initial_channel: Option<String>,
    error: Option<String>,
    copied: Option<(String, Instant)>,
}

/// "Join with invite link": the pasted link's preview, then the join.
#[derive(Default)]
struct Join {
    request: u64,
    busy: bool,
    joining: bool,
    preview: Option<InvitePreview>,
    error: Option<String>,
}

#[derive(Default)]
pub(super) struct Admin {
    /// Spaces you're invited to, and the account's limits.
    pub(super) invitations: Vec<Space>,
    limits: Option<SpaceLimits>,
    pub(super) dialog: Option<Dialog>,
    pub(super) confirm: Option<Confirm>,
    /// A change is in flight.
    pub(super) busy: bool,
    declining: bool,
    error: Option<String>,
    /// The managed private channel, or `None` for the space.
    channel: Option<String>,
    members: Vec<Member>,
    invited: Vec<Member>,
    member_error: Option<&'static str>,
    member_status: Option<&'static str>,
    /// Empty the member field on the next render.
    clear_member: bool,
    links: Links,
    join: Join,
    /// Advanced when lists change, so they redraw.
    revision: u64,
    refresh_request: u64,
    refreshing: bool,
    /// Advanced by each change made here: an older refresh is dropped.
    changes: u64,
    ticking: bool,
    /// The open space's icon change.
    pub(super) icon: super::picture::IconStatus,
}

impl State {
    pub(super) fn set_spaces(&mut self, spaces: Spaces) {
        self.spaces = spaces.spaces;
        self.admin.invitations = spaces.invitations;
        self.admin.limits = spaces.limits;
        self.layout += 1;
    }

    /// You own the open space.
    pub(super) fn owner(&self) -> bool {
        self.account
            .as_ref()
            .zip(self.detail.as_ref())
            .is_some_and(|(account, detail)| {
                account.id == detail.space.owner_id && !detail.space.demo
            })
    }

    fn can_leave_space(&self) -> bool {
        self.account
            .as_ref()
            .zip(self.detail.as_ref())
            .is_some_and(|(account, detail)| {
                !detail.space.demo && account.id != detail.space.owner_id
            })
    }

    /// Web's first-space page: a signed-in account whose loaded space list
    /// is empty names its first space instead of seeing an empty workspace.
    pub(super) fn needs_first_space(&self) -> bool {
        self.account.is_some()
            && self.admin.limits.is_some()
            && self.admin.dialog.is_none()
            && self.direct.is_none()
            && self.admin.invitations.is_empty()
            && !self.spaces.iter().any(|space| !space.demo)
    }

    pub(super) fn can_create_space(&self) -> bool {
        let (Some(limits), Some(account)) = (&self.admin.limits, &self.account) else {
            return false;
        };
        let owned = self
            .spaces
            .iter()
            .filter(|space| space.owner_id == account.id && !space.demo)
            .count();
        owned < limits.owned_spaces
            && self.spaces.iter().filter(|space| !space.demo).count() < limits.total_spaces
    }

    pub(super) fn create_space_tooltip(&self) -> String {
        if self.can_create_space() {
            "Create space".into()
        } else {
            let (owned, total) = self.admin.limits.as_ref().map_or((20, 100), |limits| {
                (limits.owned_spaces, limits.total_spaces)
            });
            format!("Space limit reached ({owned} owned, {total} total)")
        }
    }

    pub(super) fn can_create_channel(&self) -> bool {
        self.admin
            .limits
            .as_ref()
            .zip(self.detail.as_ref())
            .is_some_and(|(limits, detail)| detail.channels.len() < limits.channels_per_space)
    }

    pub(super) fn create_channel_tooltip(&self) -> String {
        if self.can_create_channel() {
            "Create channel".into()
        } else {
            format!(
                "Channel limit reached ({})",
                self.admin
                    .limits
                    .as_ref()
                    .map_or(100, |limits| limits.channels_per_space)
            )
        }
    }

    fn space_name(&self) -> String {
        self.detail
            .as_ref()
            .map_or("this space", |detail| detail.space.name.as_str())
            .to_owned()
    }

    /// A channel's saved name and privacy.
    fn saved_channel(&self, channel: &str) -> Option<(String, bool)> {
        self.detail
            .as_ref()?
            .channels
            .iter()
            .find(|entry| entry.id == channel)
            .map(|entry| (entry.name.clone(), entry.private))
    }

    /// Public channels an invite link can open in.
    fn link_channels(&self) -> Vec<(Option<String>, String)> {
        let mut channels: Vec<_> = self
            .detail
            .iter()
            .flat_map(|detail| {
                detail
                    .channels
                    .iter()
                    .filter(|channel| !channel.private)
                    .map(|channel| (Some(channel.id.clone()), format!("#{}", channel.name)))
            })
            .collect();
        if channels.is_empty() {
            channels.push((None, "No public channels".to_owned()));
        }
        channels
    }

    /// Who the member field suggests: for a space, known people outside it
    /// and not invited; for a private channel, space members without access.
    fn member_suggestions(&self) -> Vec<mentions::Person> {
        let admin = &self.admin;
        if admin.channel.is_none() {
            return mentions::invitees(
                self.people.as_deref().unwrap_or_default(),
                &admin.members,
                &admin.invited,
            );
        }
        let space = self
            .detail
            .as_ref()
            .filter(|detail| self.space.as_ref() == Some(&detail.space.id))
            .map_or(&[][..], |detail| detail.members.as_slice());
        mentions::grantees(
            space,
            self.account.as_ref().map(|account| account.id.as_str()),
            &admin.members,
            &admin.invited,
        )
    }

    /// The confirmation's form stays open until its change finishes.
    fn holds(&self) -> bool {
        self.admin.busy
            && matches!(
                self.admin.dialog,
                Some(Dialog::CreateSpace | Dialog::CreateChannel)
            )
    }
}

/// The form window's widgets.
pub(super) struct AdminWindow {
    pub(super) window: gtk::Window,
    dialog: Dialog,
    rendered: Option<u64>,
    title: gtk::Label,
    name: Option<gtk::Entry>,
    private: Option<gtk::CheckButton>,
    private_hint: Option<gtk::Label>,
    submit: Option<gtk::Button>,
    cancel: Option<gtk::Button>,
    error: gtk::Label,
    member: Option<Rc<PeopleField>>,
    member_add: Option<gtk::Button>,
    member_note: Option<gtk::Label>,
    /// The primary invite link (Invite people).
    primary: Option<gtk::Box>,
    custom: Option<CustomLink>,
    links: Option<gtk::Box>,
    invited: Option<gtk::Box>,
    members: Option<gtk::Box>,
    members_count: Option<gtk::Label>,
    /// A private channel's member section, shown once it is saved private.
    grants: Option<gtk::Box>,
    join_input: Option<gtk::Box>,
    join_entry: Option<gtk::Entry>,
    preview: Option<gtk::Box>,
    save_bar: Option<gtk::Box>,
    save: Option<gtk::Button>,
    reset: Option<gtk::Button>,
    /// Manage space's icon, for its owner.
    icon: Option<super::picture::IconRow>,
}

/// "Custom links": the next link's choices.
struct CustomLink {
    root: gtk::Box,
    amount: gtk::Entry,
    unit: gtk::DropDown,
    uses: gtk::Entry,
    channel: gtk::DropDown,
    channels: Vec<Option<String>>,
    create: gtk::Button,
}

pub(super) struct ConfirmWindow {
    pub(super) window: gtk::Window,
    confirm: Confirm,
    button: gtk::Button,
    /// The action's label while nothing is in flight.
    action: &'static str,
    cancel: gtk::Button,
    error: gtk::Label,
}

fn button(text: &str, class: &str) -> gtk::Button {
    let button = gtk::Button::with_label(text);
    if !class.is_empty() {
        button.add_css_class(class);
    }
    button
}

fn actions(left: &gtk::Button, right: &gtk::Button) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::End);
    row.set_margin_top(8);
    row.append(left);
    row.append(right);
    row
}

fn heading(text: &str) -> gtk::Label {
    label(text, &["admin-heading"])
}

fn hint(text: &str) -> gtk::Label {
    let hint = label(text, &["muted", "small"]);
    hint.set_wrap(true);
    hint
}

fn separator() -> gtk::Separator {
    let line = gtk::Separator::new(gtk::Orientation::Horizontal);
    line.set_margin_top(10);
    line.set_margin_bottom(6);
    line
}

fn set_error(label: &gtk::Label, error: Option<&str>) {
    label.set_label(error.unwrap_or_default());
    label.set_visible(error.is_some());
}

fn number_entry(text: &str, placeholder: &str) -> gtk::Entry {
    let entry = gtk::Entry::new();
    entry.set_text(text);
    entry.set_placeholder_text(Some(placeholder));
    entry.set_input_purpose(gtk::InputPurpose::Digits);
    entry.set_width_chars(5);
    entry
}

impl App {
    /// Installs the `admin.*` actions the space and channel menus use.
    pub(super) fn install_admin_actions(self: &Rc<Self>) {
        let group = gio::SimpleActionGroup::new();
        type Run = fn(&Rc<App>, String);
        let actions: [(&str, Run); 9] = [
            ("create-space", |app, _| app.open_create_space()),
            ("join", |app, _| app.open_join_invite()),
            ("manage-space", |app, _| app.open_manage_space()),
            ("invite", |app, channel| {
                app.open_invite_people(Some(channel).filter(|id| !id.is_empty()))
            }),
            ("create-channel", |app, _| app.open_create_channel()),
            ("browse", |app, _| app.open_browse()),
            ("manage-channel", |app, id| app.open_manage_channel(&id)),
            ("leave-space", |app, _| {
                let confirm = {
                    let state = app.state.borrow();
                    state.detail.as_ref().map(|detail| Confirm::LeaveSpace {
                        id: detail.space.id.clone(),
                        name: detail.space.name.clone(),
                    })
                };
                app.open_confirm(confirm);
            }),
            ("leave-channel", |app, id| {
                let confirm = {
                    let state = app.state.borrow();
                    state.detail.as_ref().and_then(|detail| {
                        detail
                            .channels
                            .iter()
                            .find(|channel| channel.id == id)
                            .map(|channel| Confirm::LeaveChannel {
                                space: detail.space.id.clone(),
                                channel: id.clone(),
                                name: channel.name.clone(),
                            })
                    })
                };
                app.open_confirm(confirm);
            }),
        ];
        for (name, run) in actions {
            let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
            let app = Rc::downgrade(self);
            action.connect_activate(move |_, parameter| {
                if let (Some(app), Some(value)) = (
                    app.upgrade(),
                    parameter.and_then(|value| value.get::<String>()),
                ) {
                    run(&app, value);
                }
            });
            group.add_action(&action);
        }
        self.ui.window.insert_action_group("admin", Some(&group));

        let weak = Rc::downgrade(self);
        self.ui.space_header.connect_clicked(move |button| {
            if let Some(app) = weak.upgrade() {
                app.space_menu(button.upcast_ref());
            }
        });
        let weak = Rc::downgrade(self);
        self.ui.new_channel.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.open_create_channel();
            }
        });
        // Web reconciles membership when the window regains focus.
        let weak = Rc::downgrade(self);
        self.ui.window.connect_is_active_notify(move |window| {
            if window.is_active()
                && let Some(app) = weak.upgrade()
            {
                app.refresh_membership();
            }
        });
    }

    /// `scope`'s management items for its right-click menu.
    pub(super) fn admin_menu(state: &State, scope: &Scope) -> Option<gio::Menu> {
        let item = |text: &str, action: &str, target: &str| {
            let item = gio::MenuItem::new(Some(text), None);
            item.set_action_and_target_value(Some(action), Some(&target.to_variant()));
            item
        };
        let detail = state.detail.as_ref()?;
        let owner = state.owner();
        let menu = gio::Menu::new();
        match scope {
            Scope::Space(space) if *space == detail.space.id => {
                if owner {
                    menu.append_item(&item("Invite people…", "admin.invite", ""));
                    menu.append_item(&item("Space settings", "admin.manage-space", ""));
                } else if state.can_leave_space() {
                    menu.append_item(&item("Leave space…", "admin.leave-space", ""));
                }
            }
            Scope::Channel { space, channel } if *space == detail.space.id => {
                let private = detail
                    .channels
                    .iter()
                    .find(|item| item.id == *channel)?
                    .private;
                if owner && !private {
                    menu.append_item(&item("Invite people…", "admin.invite", channel));
                }
                if owner {
                    menu.append_item(&item("Channel settings", "admin.manage-channel", channel));
                }
                menu.append_item(&item("Leave channel", "admin.leave-channel", channel));
            }
            _ => return None,
        }
        (menu.n_items() > 0).then_some(menu)
    }

    /// The space name's menu: invite, settings, create channel, leave.
    fn space_menu(self: &Rc<Self>, anchor: &gtk::Widget) {
        let menu = {
            let state = self.state.borrow();
            let Some(detail) = state.detail.as_ref() else {
                return;
            };
            let menu = gio::Menu::new();
            let browse = gio::MenuItem::new(Some("Browse channels"), None);
            browse.set_action_and_target_value(Some("admin.browse"), Some(&"".to_variant()));
            menu.append_item(&browse);
            if let Some(admin) = Self::admin_menu(&state, &Scope::Space(detail.space.id.clone())) {
                menu.append_section(None, &admin);
            }
            if state.owner() && state.can_create_channel() {
                let item = gio::MenuItem::new(Some("Create channel"), None);
                item.set_action_and_target_value(
                    Some("admin.create-channel"),
                    Some(&"".to_variant()),
                );
                menu.append_item(&item);
            }
            menu
        };
        if menu.n_items() > 0 {
            popup(&gtk::PopoverMenu::from_model(Some(&menu)), anchor);
        }
    }

    // ---------------------------------------------------------------------
    // Opening.

    fn admin_reset(state: &mut State, dialog: Dialog) {
        let admin = &mut state.admin;
        admin.dialog = Some(dialog);
        admin.confirm = None;
        admin.busy = false;
        admin.error = None;
        admin.member_error = None;
        admin.member_status = None;
        admin.icon.error = None;
        admin.revision += 1;
    }

    /// The first-space page's Create space, as the dialog's.
    pub(super) fn create_first_space(self: &Rc<Self>) {
        let name = self.ui.first_name.text().to_string();
        {
            let mut state = self.state.borrow_mut();
            if state.admin.busy || !state.can_create_space() || name.trim().is_empty() {
                return;
            }
            // Web validates on submit and keeps the name on failure.
            match space_name_error(&name) {
                Some(error) => state.error = Some(error.into()),
                None => self.admin(&mut state, AdminOperation::CreateSpace { name }),
            }
        }
        self.render();
    }

    pub(super) fn render_first_space(&self, state: &State) {
        let ui = &self.ui;
        let busy = state.admin.busy;
        let allowed = state.can_create_space();
        ui.first_limit.set_visible(!allowed);
        ui.first_name.set_sensitive(!busy);
        ui.first_create
            .set_label(if busy { "Creating…" } else { "Create space" });
        ui.first_create
            .set_sensitive(!busy && allowed && !ui.first_name.text().trim().is_empty());
        let error = state.error.as_deref().unwrap_or_default();
        ui.first_error.set_label(error);
        ui.first_error.set_visible(!error.is_empty());
    }

    pub(super) fn open_create_space(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if state.token.is_none() || !state.can_create_space() {
                return;
            }
            Self::admin_reset(&mut state, Dialog::CreateSpace);
        }
        self.render_admin();
    }

    pub(super) fn open_create_channel(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if !state.owner() || !state.can_create_channel() {
                return;
            }
            Self::admin_reset(&mut state, Dialog::CreateChannel);
        }
        self.render_admin();
    }

    pub(super) fn open_join_invite(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if state.token.is_none() {
                return;
            }
            Self::admin_reset(&mut state, Dialog::JoinInvite);
            let request = state.admin.join.request + 1;
            state.admin.join = Join {
                request,
                ..Join::default()
            };
        }
        self.render_admin();
    }

    /// Opens the members, invitations and invite links of the open space.
    fn load_space_admin(&self, state: &mut State) {
        let (Some(token), Some(space)) = (state.token.clone(), state.space.clone()) else {
            return;
        };
        state.admin.channel = None;
        state.admin.members = state
            .detail
            .as_ref()
            .map_or_else(Vec::new, |detail| detail.members.clone());
        state.admin.invited.clear();
        let request = state.admin.links.request.wrapping_add(1);
        state.admin.links = Links {
            request,
            default_request: state.admin.links.default_request,
            space: Some(space.clone()),
            loading: true,
            ..Links::default()
        };
        self.account_op(
            state,
            AccountOperation::LoadInviteLinks {
                space: space.clone(),
                request,
            },
        );
        for operation in [
            AdminOperation::LoadMembers {
                space: space.clone(),
                channel: None,
            },
            AdminOperation::LoadInvitations { space },
        ] {
            self.worker.send(Command::Admin {
                generation: state.generation,
                token: token.clone(),
                operation,
            });
        }
        self.load_people(state);
    }

    pub(super) fn open_manage_space(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if !state.owner() {
                return;
            }
            Self::admin_reset(&mut state, Dialog::ManageSpace);
            self.load_space_admin(&mut state);
        }
        self.render_admin();
    }

    pub(super) fn open_invite_people(self: &Rc<Self>, channel: Option<String>) {
        {
            let mut state = self.state.borrow_mut();
            if !state.owner() {
                return;
            }
            Self::admin_reset(&mut state, Dialog::InvitePeople);
            self.load_space_admin(&mut state);
            let channel = channel.filter(|id| {
                state.detail.as_ref().is_some_and(|detail| {
                    detail
                        .channels
                        .iter()
                        .any(|item| item.id == *id && !item.private)
                })
            });
            state.admin.links.initial_channel = channel;
            self.ensure_default_link(&mut state);
        }
        self.render_admin();
    }

    fn ensure_default_link(&self, state: &mut State) {
        let links = &mut state.admin.links;
        if links.creating || !links.revoking.is_empty() {
            return;
        }
        let Some(space) = state.space.clone() else {
            return;
        };
        links.default_request = links.default_request.wrapping_add(1);
        links.request = links.request.wrapping_add(1);
        links.loading = false;
        links.creating = true;
        links.error = None;
        let operation = AccountOperation::EnsureDefaultInviteLink {
            space,
            channel: links.initial_channel.clone(),
            request: links.default_request,
        };
        self.account_op(state, operation);
    }

    pub(super) fn open_manage_channel(self: &Rc<Self>, id: &str) {
        {
            let mut state = self.state.borrow_mut();
            let Some((_, private)) = state.saved_channel(id) else {
                return;
            };
            if !state.owner() {
                return;
            }
            Self::admin_reset(&mut state, Dialog::ManageChannel(id.into()));
            state.admin.channel = Some(id.into());
            state.admin.members.clear();
            state.admin.invited.clear();
            if private
                && let (Some(token), Some(space)) = (state.token.clone(), state.space.clone())
            {
                self.worker.send(Command::Admin {
                    generation: state.generation,
                    token,
                    operation: AdminOperation::LoadMembers {
                        space,
                        channel: Some(id.into()),
                    },
                });
            }
        }
        self.render_admin();
    }

    /// `OpenInvitation` answered: a space invitation asks to join; a private
    /// channel invitation opens its space, whose sidebar lists it.
    pub(super) fn invitation_loaded(
        self: &Rc<Self>,
        space: String,
        channel: Option<String>,
        result: Result<(Spaces, Option<caper_client::model::SpaceDetail>), String>,
    ) {
        let confirm = {
            let mut state = self.state.borrow_mut();
            let (spaces, detail) = match result {
                Ok(loaded) => loaded,
                Err(error) => {
                    state.error = Some(error);
                    drop(state);
                    self.render();
                    return;
                }
            };
            state.set_spaces(spaces);
            const GONE: &str = "This invitation is no longer available.";
            match channel {
                Some(channel) => {
                    match detail.filter(|detail| {
                        detail
                            .channel_invitations
                            .iter()
                            .any(|invite| invite.channel.id == channel)
                    }) {
                        Some(detail)
                            if state.space.as_deref() == Some(&space) && state.direct.is_none() =>
                        {
                            state.detail = Some(detail);
                            state.layout += 1;
                        }
                        Some(_) => self.navigate(&mut state, Some(space), None),
                        None => state.error = Some(GONE.into()),
                    }
                    None
                }
                None => {
                    let invite = state
                        .admin
                        .invitations
                        .iter()
                        .find(|invite| invite.id == space)
                        .cloned();
                    if invite.is_none() {
                        state.error = Some(GONE.into());
                    }
                    invite.map(Confirm::Invitation)
                }
            }
        };
        if confirm.is_some() {
            self.open_confirm(confirm);
        }
        self.render();
    }

    fn open_confirm(self: &Rc<Self>, confirm: Option<Confirm>) {
        let Some(confirm) = confirm else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            if state.admin.busy {
                return;
            }
            if matches!(confirm, Confirm::Delete { .. }) {
                self.play(Effect::Warning);
            }
            state.admin.confirm = Some(confirm);
            state.admin.error = None;
        }
        self.render_admin();
    }

    /// Sends a change; its window shows it is in progress until it answers.
    fn admin(&self, state: &mut State, operation: AdminOperation) {
        let Some(token) = state.token.clone() else {
            return;
        };
        state.admin.busy = true;
        state.admin.error = None;
        state.admin.changes += 1;
        self.worker.send(Command::Admin {
            generation: state.generation,
            token,
            operation,
        });
    }

    // ---------------------------------------------------------------------
    // Results.

    /// A finished `Command::Admin`. Joining a previewed channel uses it too.
    pub(super) fn admin_result(&self, state: &mut State, result: Result<AdminResult, String>) {
        state.joining = false;
        let dialog_open = state.admin.dialog.is_some() || state.admin.confirm.is_some();
        let busy = std::mem::take(&mut state.admin.busy);
        state.admin.revision += 1;
        state.layout += 1;
        let result = match result {
            Ok(result) => result,
            Err(error) => {
                let error = admin_error(error);
                if dialog_open && busy {
                    state.admin.error = Some(error);
                } else if !dialog_open {
                    state.error = Some(error);
                }
                return;
            }
        };
        if matches!(
            result,
            AdminResult::SpaceDeleted(_) | AdminResult::ChannelDeleted(_)
        ) {
            self.play(Effect::Delete);
        }
        let admin = &mut state.admin;
        match result {
            AdminResult::SpaceCreated(space) => {
                state.spaces.push(space.clone());
                admin.dialog = None;
                self.navigate(state, Some(space.id), None);
            }
            AdminResult::SpaceUpdated(space) => {
                self.voice.borrow_mut().rename_space(&space.id, &space.name);
                if let Some(item) = state.spaces.iter_mut().find(|item| item.id == space.id) {
                    *item = space.clone();
                }
                if let Some(detail) = &mut state.detail {
                    detail.space = space;
                }
                admin.dialog = None;
            }
            AdminResult::SpaceDeleted(id) | AdminResult::SpaceLeft(id) => {
                self.forget_space(state, &id);
            }
            AdminResult::ChannelCreated(channel) => {
                if let Some(detail) = &mut state.detail {
                    detail.channels.push(channel.clone());
                }
                admin.dialog = None;
                state.channel_name.clone_from(&channel.name);
                let space = state.space.clone();
                self.navigate(state, space, Some(channel.id.clone()));
                // Web opens a new private channel's settings to add members.
                if channel.private {
                    state.admin.dialog = Some(Dialog::ManageChannel(channel.id.clone()));
                    state.admin.channel = Some(channel.id.clone());
                    state.admin.members.clear();
                    state.admin.invited.clear();
                    if let (Some(token), Some(space)) = (state.token.clone(), state.space.clone()) {
                        self.worker.send(Command::Admin {
                            generation: state.generation,
                            token,
                            operation: AdminOperation::LoadMembers {
                                space,
                                channel: Some(channel.id),
                            },
                        });
                    }
                }
            }
            AdminResult::ChannelUpdated(channel) => {
                if let Some(item) = state.detail.as_mut().and_then(|detail| {
                    detail
                        .channels
                        .iter_mut()
                        .find(|item| item.id == channel.id)
                }) {
                    *item = channel.clone();
                }
                if state.channel.as_deref() == Some(&channel.id) {
                    state.channel_name = channel.name;
                }
                admin.dialog = None;
            }
            AdminResult::ChannelDeleted(id) => {
                self.voice.borrow_mut().revoke_channel(&id);
                if let Some(detail) = &mut state.detail {
                    detail.channels.retain(|channel| channel.id != id);
                }
                admin.dialog = None;
                admin.confirm = None;
                if state.channel.as_deref() == Some(&id) && state.direct.is_none() {
                    let next = state
                        .detail
                        .as_ref()
                        .and_then(|detail| detail.channels.iter().find(|item| item.joined))
                        .map(|item| item.id.clone());
                    let space = state.space.clone();
                    self.navigate(state, space, next);
                }
            }
            AdminResult::Members {
                channel,
                members,
                invitations,
            } => {
                if channel == admin.channel {
                    admin.members = members;
                    admin.invited = invitations;
                }
            }
            AdminResult::Invitations(members) => {
                if admin.channel.is_none() {
                    admin.invited = members;
                }
            }
            AdminResult::InvitationCreated(member) => {
                admin.invited.retain(|item| item.id != member.id);
                admin.invited.push(member);
                admin.clear_member = true;
                admin.member_status = Some(INVITATION_SENT);
            }
            AdminResult::InvitationCancelled(user) => {
                admin.invited.retain(|item| item.id != user);
            }
            AdminResult::InvitationAccepted(space) => {
                admin.invitations.retain(|item| item.id != space.id);
                admin.confirm = None;
                if !state.spaces.iter().any(|item| item.id == space.id) {
                    state.spaces.push(space.clone());
                }
                self.navigate(state, Some(space.id), None);
            }
            AdminResult::InvitationDeclined(space) => {
                admin.invitations.retain(|item| item.id != space);
                admin.confirm = None;
            }
            AdminResult::MemberAdded { channel, member } => {
                if channel == admin.channel {
                    admin.invited.retain(|item| item.id != member.id);
                    admin.invited.push(member);
                    admin.clear_member = true;
                    admin.member_status = Some(INVITATION_SENT);
                }
            }
            AdminResult::MemberRemoved { channel, member } => {
                if channel == admin.channel {
                    admin.members.retain(|item| item.id != member);
                    admin.invited.retain(|item| item.id != member);
                }
                // The confirmation closes; its settings stay open.
                if matches!(admin.confirm, Some(Confirm::RemoveMember { .. })) {
                    admin.confirm = None;
                }
                if channel.is_none()
                    && let Some(detail) = &mut state.detail
                {
                    detail.members.retain(|item| item.id != member);
                }
            }
            AdminResult::ChannelLeft(channel) => {
                self.voice.borrow_mut().revoke_channel(&channel);
                admin.confirm = None;
                if let Some(mut detail) = state.detail.clone() {
                    let owner = state.owner();
                    detail
                        .channels
                        .retain(|item| item.id != channel || !item.private || owner);
                    for item in &mut detail.channels {
                        if item.id == channel {
                            item.joined = false;
                        }
                    }
                    self.membership(state, detail, channel);
                }
            }
            AdminResult::ChannelMembership {
                detail, channel, ..
            } => self.membership(state, detail, channel),
            AdminResult::ChannelInvitationDeclined(channel) => {
                if let Some(detail) = &mut state.detail {
                    detail
                        .channel_invitations
                        .retain(|invite| invite.channel.id != channel);
                }
            }
        }
    }

    /// Joined or left a channel: its space's new detail, then that channel
    /// (or the first joined one once it is gone).
    fn membership(
        &self,
        state: &mut State,
        detail: caper_client::model::SpaceDetail,
        channel: String,
    ) {
        let space = Some(detail.space.id.clone());
        let next = if detail.channels.iter().any(|item| item.id == channel) {
            Some(channel.clone())
        } else {
            detail
                .channels
                .iter()
                .find(|item| item.joined)
                .map(|item| item.id.clone())
        };
        state.detail = Some(detail);
        // As on the desktop, the channel opens (as a preview once left).
        self.navigate(state, space, next);
    }

    /// A space you can no longer open: it leaves the rail, and the first
    /// remaining space opens in its place.
    fn forget_space(&self, state: &mut State, id: &str) {
        self.voice.borrow_mut().revoke_space(id);
        state.spaces.retain(|space| space.id != id);
        state.unread.remove(id);
        if state.space.as_deref() != Some(id) {
            return;
        }
        state.admin.dialog = None;
        state.admin.confirm = None;
        state.admin.members.clear();
        state.admin.invited.clear();
        if state.direct.is_some() {
            state.space = None;
            state.detail = None;
            return;
        }
        match state.spaces.iter().find(|space| !space.demo) {
            Some(next) => {
                let next = next.id.clone();
                self.navigate(state, Some(next), None);
            }
            None => {
                self.leave_conversation(state);
                state.space = None;
                state.detail = None;
                state.channel = None;
                state.channel_name.clear();
                state.timeline = Default::default();
                state.status.clear();
            }
        }
    }

    /// Invite link and join answers from the account worker.
    pub(super) fn invite_result(&self, state: &mut State, result: AccountResult) {
        state.admin.revision += 1;
        let current = |state: &State, space: &str| {
            state.space.as_deref() == Some(space)
                && state.admin.links.space.as_deref() == Some(space)
                && matches!(
                    state.admin.dialog,
                    Some(Dialog::ManageSpace | Dialog::InvitePeople)
                )
        };
        match result {
            AccountResult::DefaultInviteLink {
                space,
                request,
                result,
            } if current(state, &space) && request == state.admin.links.default_request => {
                let links = &mut state.admin.links;
                links.creating = false;
                match result {
                    Ok(link) => {
                        if let Some(previous) = links.default_link.replace(link.clone())
                            && links.regenerating
                        {
                            links.links.retain(|item| item.code != previous.code);
                        }
                        links.links.retain(|item| item.code != link.code);
                        links.links.insert(0, link);
                        links.error = None;
                        links.copied = None;
                    }
                    Err(error) => links.error = Some(error),
                }
                links.regenerating = false;
                links.loading = true;
                let request = links.request;
                self.account_op(state, AccountOperation::LoadInviteLinks { space, request });
            }
            AccountResult::InviteLinks {
                space,
                request,
                result,
            } if current(state, &space) && request == state.admin.links.request => {
                let links = &mut state.admin.links;
                links.loading = false;
                match result {
                    Ok(list) => {
                        if links.default_link.as_ref().is_some_and(|default| {
                            !list.iter().any(|link| link.code == default.code)
                        }) {
                            links.default_link = None;
                        }
                        links.links = list;
                    }
                    Err(error) => links.error = Some(error),
                }
            }
            AccountResult::InviteLinkCreated {
                space,
                request,
                result,
            } if current(state, &space) && request == state.admin.links.request => {
                let base = self.api.base().clone();
                let links = &mut state.admin.links;
                links.creating = false;
                match result {
                    Ok(link) => {
                        // The new link leads the list and is copied at once.
                        links.error = None;
                        links.links.retain(|item| item.code != link.code);
                        self.ui
                            .window
                            .clipboard()
                            .set_text(&invites::invite_url(&base, &link.code));
                        links.copied = Some((link.code.clone(), Instant::now()));
                        links.links.insert(0, link);
                    }
                    Err(error) => links.error = Some(error),
                }
            }
            AccountResult::InviteLinkRevoked {
                space,
                request,
                code,
                result,
            } if current(state, &space) && request == state.admin.links.request => {
                let links = &mut state.admin.links;
                links.revoking.remove(&code);
                match result {
                    Ok(()) => {
                        links.links.retain(|item| item.code != code);
                        if links
                            .default_link
                            .as_ref()
                            .is_some_and(|link| link.code == code)
                        {
                            links.default_link = None;
                        }
                    }
                    Err(error) => {
                        // It may have expired or been revoked elsewhere.
                        links.error = Some(error);
                        links.loading = true;
                        self.account_op(
                            state,
                            AccountOperation::LoadInviteLinks { space, request },
                        );
                    }
                }
            }
            AccountResult::InvitePreview { request, result }
                if request == state.admin.join.request =>
            {
                let join = &mut state.admin.join;
                join.busy = false;
                match result {
                    Ok(preview) => {
                        join.preview = Some(preview);
                        join.error = None;
                    }
                    Err(error) => join.error = Some(error),
                }
            }
            AccountResult::InviteJoined { request, result }
                if request == state.admin.join.request =>
            {
                let join = &mut state.admin.join;
                join.busy = false;
                join.joining = false;
                match result {
                    Ok(joined) => {
                        let space = joined.space;
                        state.admin.changes += 1;
                        state.admin.invitations.retain(|item| item.id != space.id);
                        match state.spaces.iter_mut().find(|item| item.id == space.id) {
                            Some(existing) => *existing = space.clone(),
                            None => state.spaces.push(space.clone()),
                        }
                        state.admin.join.request += 1;
                        state.admin.dialog = None;
                        if state.space.as_ref() == Some(&space.id) {
                            state.detail = None;
                            state.space = None;
                        }
                        self.navigate(state, Some(space.id), joined.channel_id);
                    }
                    Err(error) => join.error = Some(error),
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------------
    // Membership refresh.

    /// Starts the 15-second reconcile once signed in.
    pub(super) fn start_membership_refresh(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            if state.token.is_none() || std::mem::replace(&mut state.admin.ticking, true) {
                return;
            }
        }
        let weak = Rc::downgrade(self);
        self.load_voice_activity();
        glib::timeout_add_local(REFRESH, move || {
            let Some(app) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            app.load_voice_activity();
            if app
                .state
                .try_borrow()
                .is_ok_and(|state| state.token.is_none())
            {
                if let Ok(mut state) = app.state.try_borrow_mut() {
                    state.admin.ticking = false;
                }
                return glib::ControlFlow::Break;
            }
            app.refresh_membership();
            glib::ControlFlow::Continue
        });
    }

    /// Which spaces have someone in voice, for the rail's badges (desktop
    /// polls every 15 seconds too).
    fn load_voice_activity(&self) {
        let Ok(mut state) = self.state.try_borrow_mut() else {
            return;
        };
        let Some(token) = state.token.clone() else {
            return;
        };
        if std::mem::replace(&mut state.voice_activity_loading, true) {
            return;
        }
        self.worker.send(Command::LoadVoiceActivity {
            epoch: state.epoch,
            token,
        });
    }

    fn refresh_membership(&self) {
        let Ok(mut state) = self.state.try_borrow_mut() else {
            return;
        };
        let Some(token) = state.token.clone() else {
            return;
        };
        if state.account.is_none() || state.admin.refreshing || state.admin.busy {
            return;
        }
        state.admin.refresh_request += 1;
        state.admin.refreshing = true;
        self.worker.send(Command::RefreshMembership {
            epoch: state.epoch,
            request: state.admin.refresh_request,
            revision: state.admin.changes,
            navigation: state.navigation,
            token,
            space: state.space.clone().filter(|_| state.direct.is_none()),
        });
    }

    /// `GET /api/spaces` (and the open space) answered. A change made here
    /// meanwhile wins, and a failed read keeps what is shown.
    pub(super) fn membership_refreshed(
        &self,
        state: &mut State,
        request: u64,
        revision: u64,
        navigation: u64,
        result: Result<(Spaces, Option<caper_client::model::SpaceDetail>), String>,
    ) {
        if request != state.admin.refresh_request {
            return;
        }
        state.admin.refreshing = false;
        let Ok((spaces, detail)) = result else {
            return;
        };
        if revision != state.admin.changes || state.admin.busy {
            return;
        }
        let available: BTreeSet<String> =
            spaces.spaces.iter().map(|space| space.id.clone()).collect();
        state.set_spaces(spaces);
        let active = self.voice.borrow().active_space.clone();
        if let Some(active) = active.filter(|active| !available.contains(active)) {
            self.voice.borrow_mut().revoke_space(&active);
        }
        if let Some(Confirm::Invitation(space)) = &state.admin.confirm
            && !state
                .admin
                .invitations
                .iter()
                .any(|item| item.id == space.id)
        {
            state.admin.confirm = None;
        }
        let Some(open) = state.space.clone() else {
            return;
        };
        if !available.contains(&open) {
            self.forget_space(state, &open);
            state.error = Some("This space is no longer available.".into());
            return;
        }
        let Some(detail) = detail.filter(|detail| {
            navigation == state.navigation
                && detail.space.id == open
                && state.direct.is_none()
                && state.detail.as_ref() != Some(detail)
        }) else {
            return;
        };
        if let Some(space) = state.spaces.iter_mut().find(|space| space.id == open) {
            *space = detail.space.clone();
        }
        let joined = |detail: Option<&caper_client::model::SpaceDetail>, channel: &str| {
            detail.and_then(|detail| {
                detail
                    .channels
                    .iter()
                    .find(|item| item.id == channel)
                    .map(|item| item.joined)
            })
        };
        let previous = state.detail.replace(detail);
        let first = state.detail.as_ref().and_then(|detail| {
            detail
                .channels
                .iter()
                .find(|item| item.joined)
                .map(|item| item.id.clone())
        });
        let Some(channel) = state.channel.clone() else {
            if first.is_some() {
                self.navigate(state, Some(open), first);
            }
            return;
        };
        match joined(state.detail.as_ref(), &channel) {
            // Only its name or privacy changed.
            Some(now) if joined(previous.as_ref(), &channel) == Some(now) => {
                if let Some((name, _)) = state.saved_channel(&channel) {
                    state.channel_name = name;
                }
            }
            // Joined or left elsewhere: reopen with or without the composer.
            Some(_) => self.navigate(state, Some(open), Some(channel)),
            // Deleted, or private access removed, elsewhere.
            None => self.navigate(state, Some(open), first),
        }
    }

    // ---------------------------------------------------------------------
    // Windows.

    /// Opens, closes and updates the admin windows to match the state.
    pub(super) fn render_admin(self: &Rc<Self>) {
        let (dialog, confirm) = {
            let Ok(state) = self.state.try_borrow() else {
                return;
            };
            (state.admin.dialog.clone(), state.admin.confirm.clone())
        };
        let stale = self
            .admin_window
            .borrow()
            .as_ref()
            .is_some_and(|window| Some(&window.dialog) != dialog.as_ref());
        if stale && let Some(window) = self.admin_window.borrow_mut().take() {
            window.window.destroy();
        }
        if let Some(dialog) = dialog
            && self.admin_window.borrow().is_none()
        {
            let window = self.build_admin(dialog);
            window.window.present();
            *self.admin_window.borrow_mut() = Some(window);
        }
        let stale = self
            .confirm_window
            .borrow()
            .as_ref()
            .is_some_and(|window| Some(&window.confirm) != confirm.as_ref());
        if stale && let Some(window) = self.confirm_window.borrow_mut().take() {
            window.window.destroy();
        }
        if let Some(confirm) = confirm
            && self.confirm_window.borrow().is_none()
        {
            let window = self.build_confirm(confirm);
            window.window.present();
            window.cancel.grab_focus();
            *self.confirm_window.borrow_mut() = Some(window);
        }
        let clear_member = std::mem::take(&mut self.state.borrow_mut().admin.clear_member);
        if clear_member
            && let Some(field) = self
                .admin_window
                .borrow()
                .as_ref()
                .and_then(|window| window.member.clone())
        {
            field.entry.set_text("");
        }
        self.update_admin();
        self.update_confirm();
    }

    fn admin_shell(
        self: &Rc<Self>,
        title: &str,
        width: i32,
    ) -> (gtk::Window, gtk::Box, gtk::Label) {
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title(title)
            .default_width(width)
            .build();
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("dialog-body");
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let heading = label(title, &["dialog-title"]);
        heading.set_hexpand(true);
        header.append(&heading);
        let close = crate::app::named::<gtk::Button>("GtkButton", &format!("Close {title}"));
        close.set_child(Some(&self.images.icon_image(
            crate::images::Icon::Close,
            "#b9bcbe",
            16,
        )));
        close.add_css_class("flat");
        close.add_css_class("tool");
        let weak = window.downgrade();
        close.connect_clicked(move |_| {
            if let Some(window) = weak.upgrade() {
                window.close();
            }
        });
        header.append(&close);
        root.append(&header);
        let keys = gtk::EventControllerKey::new();
        let weak = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape
                && let Some(window) = weak.upgrade()
            {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
        (window, root, heading)
    }

    fn build_admin(self: &Rc<Self>, dialog: Dialog) -> AdminWindow {
        let state = self.state.borrow();
        let wide = matches!(
            dialog,
            Dialog::ManageSpace | Dialog::InvitePeople | Dialog::ManageChannel(_)
        );
        let title = match &dialog {
            Dialog::CreateSpace => "Create a space",
            Dialog::ManageSpace => "Manage space",
            Dialog::InvitePeople => "Invite people",
            Dialog::JoinInvite => "Join with invite link",
            Dialog::CreateChannel => "Create a channel",
            Dialog::ManageChannel(_) => "Channel settings",
        };
        let (window, root, title_label) = self.admin_shell(title, if wide { 600 } else { 440 });
        let error = label("", &["error"]);
        error.set_wrap(true);
        let mut widgets = AdminWindow {
            window: window.clone(),
            dialog: dialog.clone(),
            rendered: None,
            title: title_label,
            name: None,
            private: None,
            private_hint: None,
            submit: None,
            cancel: None,
            error: error.clone(),
            member: None,
            member_add: None,
            member_note: None,
            primary: None,
            custom: None,
            links: None,
            invited: None,
            members: None,
            members_count: None,
            grants: None,
            join_input: None,
            join_entry: None,
            preview: None,
            save_bar: None,
            save: None,
            reset: None,
            icon: None,
        };
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        match &dialog {
            Dialog::CreateSpace | Dialog::ManageSpace => {
                let manage = dialog == Dialog::ManageSpace;
                if manage {
                    body.append(&hint(
                        "Only the owner can change this space and its membership.",
                    ));
                    if state.owner()
                        && let Some(space) = state.space.clone()
                    {
                        widgets.icon = Some(self.build_icon_row(&body, &space));
                    }
                }
                body.append(&label("Space name", &["field"]));
                let name = gtk::Entry::new();
                name.set_placeholder_text(Some("Gaming nights"));
                name.set_max_length(80);
                if manage {
                    name.set_text(&state.space_name());
                }
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                name.set_hexpand(true);
                row.append(&name);
                body.append(&row);
                body.append(&error);
                if manage {
                    let save = button("Save name", "");
                    row.append(&save);
                    widgets.submit = Some(save);
                    body.append(&separator());
                    self.build_custom_links(&state, &body, &mut widgets);
                    body.append(&separator());
                    let invited = gtk::Box::new(gtk::Orientation::Vertical, 6);
                    body.append(&invited);
                    widgets.invited = Some(invited);
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    line.set_margin_top(10);
                    line.append(&heading("Members"));
                    let count = label("", &["muted", "small"]);
                    line.append(&count);
                    widgets.members_count = Some(count);
                    body.append(&line);
                    self.build_member_row(&body, &mut widgets, "Invite");
                    let members = gtk::Box::new(gtk::Orientation::Vertical, 4);
                    body.append(&members);
                    widgets.members = Some(members);
                    body.append(&separator());
                    self.build_delete(
                        &body,
                        "Delete space",
                        "Delete this space and all its channels for every member.",
                        None,
                    );
                } else {
                    let cancel = button("Cancel", "");
                    let create = button("Create space", "primary");
                    body.append(&actions(&cancel, &create));
                    widgets.cancel = Some(cancel);
                    widgets.submit = Some(create);
                }
                widgets.name = Some(name);
            }
            Dialog::InvitePeople => {
                body.append(&hint(
                    "Anyone with this link can join. New links allow 100 joins for 30 days.",
                ));
                let primary = gtk::Box::new(gtk::Orientation::Vertical, 6);
                body.append(&primary);
                widgets.primary = Some(primary);
                body.append(&error);
                self.build_custom_links(&state, &body, &mut widgets);
                body.append(&separator());
                body.append(&heading("Invite by username"));
                self.build_member_row(&body, &mut widgets, "Invite");
            }
            Dialog::JoinInvite => {
                let input = gtk::Box::new(gtk::Orientation::Vertical, 8);
                input.append(&hint("Paste it below to join!"));
                let entry: gtk::Entry = crate::app::named("GtkEntry", "Invite link");
                entry.set_placeholder_text(Some("it looks like → caper.chat/invite/AbCdE12345"));
                input.append(&entry);
                body.append(&input);
                let preview = gtk::Box::new(gtk::Orientation::Vertical, 4);
                preview.add_css_class("invite-preview");
                body.append(&preview);
                body.append(&error);
                let cancel = button("Cancel", "");
                let next = button("Continue", "primary");
                body.append(&actions(&cancel, &next));
                widgets.join_input = Some(input);
                widgets.join_entry = Some(entry);
                widgets.preview = Some(preview);
                widgets.cancel = Some(cancel);
                widgets.submit = Some(next);
            }
            Dialog::CreateChannel | Dialog::ManageChannel(_) => {
                let channel = match &dialog {
                    Dialog::ManageChannel(id) => Some(id.clone()),
                    _ => None,
                };
                let saved = channel.as_deref().and_then(|id| state.saved_channel(id));
                body.append(&label("Channel name", &["field"]));
                let name = gtk::Entry::new();
                name.set_placeholder_text(Some("project-updates"));
                if let Some((saved, _)) = &saved {
                    name.set_text(saved);
                }
                body.append(&name);
                if channel.is_none() {
                    body.append(&hint(
                        "Channels are where conversations happen around a topic. Use a name that is easy to find and understand.",
                    ));
                }
                let private = gtk::CheckButton::with_label("Private channel");
                private.set_active(saved.as_ref().is_some_and(|(_, private)| *private));
                private.set_margin_top(6);
                body.append(&private);
                let private_hint = hint("");
                body.append(&private_hint);
                body.append(&error);
                if let Some(channel) = &channel {
                    let grants = gtk::Box::new(gtk::Orientation::Vertical, 8);
                    grants.append(&separator());
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    line.append(&heading("Members"));
                    let count = label("", &["muted", "small"]);
                    line.append(&count);
                    grants.append(&line);
                    widgets.members_count = Some(count);
                    self.build_member_row(&grants, &mut widgets, "Add");
                    let members = gtk::Box::new(gtk::Orientation::Vertical, 4);
                    grants.append(&members);
                    let invited = gtk::Box::new(gtk::Orientation::Vertical, 6);
                    grants.append(&invited);
                    body.append(&grants);
                    widgets.members = Some(members);
                    widgets.invited = Some(invited);
                    widgets.grants = Some(grants);
                    body.append(&separator());
                    self.build_delete(
                        &body,
                        "Delete channel",
                        "Delete this channel for everyone in the space.",
                        Some(channel.clone()),
                    );
                } else {
                    let cancel = button("Cancel", "");
                    let create = button("Create channel", "primary");
                    body.append(&actions(&cancel, &create));
                    widgets.cancel = Some(cancel);
                    widgets.submit = Some(create);
                }
                widgets.name = Some(name);
                widgets.private = Some(private);
                widgets.private_hint = Some(private_hint);
            }
        }
        drop(state);
        if wide {
            // A fixed shell, as on the desktop: lists scroll inside it.
            window.set_default_height(if dialog == Dialog::ManageSpace {
                744
            } else {
                618
            });
            let scroll = gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .vexpand(true)
                .child(&body)
                .build();
            root.append(&scroll);
        } else {
            root.append(&body);
        }
        if let Dialog::ManageChannel(_) = dialog {
            // Web's sticky bar while the channel has unsaved changes.
            let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            bar.add_css_class("save-bar");
            let note = label("You have unsaved changes.", &["small"]);
            note.set_hexpand(true);
            bar.append(&note);
            let reset = button("Reset", "");
            let save = button("Save changes", "primary");
            bar.append(&reset);
            bar.append(&save);
            root.append(&bar);
            widgets.save_bar = Some(bar);
            widgets.save = Some(save);
            widgets.reset = Some(reset);
        }
        window.set_child(Some(&root));
        self.connect_admin(&widgets);
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            let Some(app) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let Ok(mut state) = app.state.try_borrow_mut() else {
                return glib::Propagation::Proceed;
            };
            if state.holds() {
                return glib::Propagation::Stop;
            }
            state.admin.dialog = None;
            state.admin.confirm = None;
            state.admin.join.request += 1;
            state.admin.links.request = state.admin.links.request.wrapping_add(1);
            drop(state);
            app.admin_window.borrow_mut().take();
            if let Some(confirm) = app.confirm_window.borrow_mut().take() {
                confirm.window.destroy();
            }
            glib::Propagation::Proceed
        });
        if let Some(entry) = widgets.name.as_ref().or(widgets.join_entry.as_ref())
            && !matches!(dialog, Dialog::ManageSpace | Dialog::ManageChannel(_))
        {
            entry.grab_focus();
        }
        widgets
    }

    fn build_member_row(self: &Rc<Self>, body: &gtk::Box, widgets: &mut AdminWindow, verb: &str) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let field = PeopleField::new(self.images.clone(), "Exact username");
        field.root.set_hexpand(true);
        row.append(&field.root);
        let add = button(verb, "");
        add.set_valign(gtk::Align::Start);
        row.append(&add);
        body.append(&row);
        let note = label("", &["small"]);
        note.set_wrap(true);
        body.append(&note);
        widgets.member = Some(field);
        widgets.member_add = Some(add);
        widgets.member_note = Some(note);
    }

    fn build_delete(
        self: &Rc<Self>,
        body: &gtk::Box,
        title: &str,
        text: &str,
        channel: Option<String>,
    ) {
        body.append(&heading(title));
        body.append(&hint(text));
        let delete = button(title, "destructive-action");
        delete.set_halign(gtk::Align::Start);
        let weak = Rc::downgrade(self);
        delete.connect_clicked(move |_| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let confirm = {
                let state = app.state.borrow();
                state.space.clone().map(|space| Confirm::Delete {
                    name: match &channel {
                        Some(id) => state
                            .saved_channel(id)
                            .map_or_else(|| "this channel".into(), |(name, _)| name),
                        None => state.space_name(),
                    },
                    space,
                    channel: channel.clone(),
                })
            };
            app.open_confirm(confirm);
        });
        body.append(&delete);
    }

    fn build_custom_links(
        self: &Rc<Self>,
        state: &State,
        body: &gtk::Box,
        widgets: &mut AdminWindow,
    ) {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.append(&heading("Custom links"));
        root.append(&hint(
            "Create a separate invite link. This does not change the primary link or other shared links.",
        ));
        let grid = gtk::Grid::new();
        grid.set_column_spacing(12);
        grid.set_row_spacing(6);
        grid.attach(&label("Expire after", &["muted", "small"]), 0, 0, 2, 1);
        grid.attach(&label("Max uses", &["muted", "small"]), 2, 0, 1, 1);
        grid.attach(&label("Opens in", &["muted", "small"]), 3, 0, 1, 1);
        let amount = number_entry(invites::DEFAULT_EXPIRY.0, "");
        let units: Vec<&str> = invites::EXPIRY_UNITS
            .iter()
            .map(|(_, name)| *name)
            .collect();
        let unit = gtk::DropDown::from_strings(&units);
        unit.set_selected(
            invites::EXPIRY_UNITS
                .iter()
                .position(|(value, _)| *value == invites::DEFAULT_EXPIRY.1)
                .unwrap_or(0) as u32,
        );
        let uses = number_entry("100", "No limit");
        let choices = state.link_channels();
        let names: Vec<&str> = choices.iter().map(|(_, name)| name.as_str()).collect();
        let channel = gtk::DropDown::from_strings(&names);
        channel.set_hexpand(true);
        grid.attach(&amount, 0, 1, 1, 1);
        grid.attach(&unit, 1, 1, 1, 1);
        grid.attach(&uses, 2, 1, 1, 1);
        grid.attach(&channel, 3, 1, 1, 1);
        root.append(&grid);
        let create = button("Create separate link", "primary");
        create.set_halign(gtk::Align::End);
        root.append(&create);
        // A never-expiring link has no amount.
        let weak = amount.downgrade();
        unit.connect_selected_notify(move |unit| {
            if let Some(amount) = weak.upgrade() {
                amount.set_visible(
                    invites::EXPIRY_UNITS
                        .get(unit.selected() as usize)
                        .is_none_or(|(value, _)| *value != ExpiryUnit::Never),
                );
            }
        });
        body.append(&root);
        let links = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.append(&links);
        widgets.links = Some(links);
        widgets.custom = Some(CustomLink {
            root,
            amount,
            unit,
            uses,
            channel,
            channels: choices.into_iter().map(|(id, _)| id).collect(),
            create,
        });
    }

    fn connect_admin(self: &Rc<Self>, widgets: &AdminWindow) {
        let dialog = widgets.dialog.clone();
        if let Some(name) = &widgets.name {
            let weak = Rc::downgrade(self);
            let channel = matches!(dialog, Dialog::CreateChannel | Dialog::ManageChannel(_));
            name.connect_changed(move |entry| {
                if channel {
                    let text = entry.text();
                    let normalized = normalize_channel(&text);
                    if normalized != text {
                        entry.set_text(&normalized);
                        entry.set_position(-1);
                        return;
                    }
                }
                if let Some(app) = weak.upgrade() {
                    if let Ok(mut state) = app.state.try_borrow_mut() {
                        state.admin.error = None;
                    }
                    app.update_admin();
                }
            });
            if !matches!(dialog, Dialog::ManageChannel(_)) {
                let weak = Rc::downgrade(self);
                name.connect_activate(move |_| {
                    if let Some(app) = weak.upgrade() {
                        app.submit_admin();
                    }
                });
            }
        }
        if let Some(private) = &widgets.private {
            let weak = Rc::downgrade(self);
            private.connect_toggled(move |check| {
                if let Some(app) = weak.upgrade() {
                    if app.state.try_borrow().is_ok() {
                        app.play(if check.is_active() {
                            Effect::ToggleOn
                        } else {
                            Effect::ToggleOff
                        });
                    }
                    app.update_admin();
                }
            });
        }
        if let Some(entry) = &widgets.join_entry {
            let weak = Rc::downgrade(self);
            entry.connect_changed(move |_| {
                if let Some(app) = weak.upgrade() {
                    if let Ok(mut state) = app.state.try_borrow_mut() {
                        state.admin.join.error = None;
                    }
                    app.update_admin();
                }
            });
            let weak = Rc::downgrade(self);
            entry.connect_activate(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.submit_admin();
                }
            });
        }
        if let Some(submit) = &widgets.submit {
            let weak = Rc::downgrade(self);
            submit.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.submit_admin();
                }
            });
        }
        if let Some(cancel) = &widgets.cancel {
            let weak = widgets.window.downgrade();
            cancel.connect_clicked(move |_| {
                if let Some(window) = weak.upgrade() {
                    window.close();
                }
            });
        }
        if let Some(save) = &widgets.save {
            let weak = Rc::downgrade(self);
            save.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.submit_admin();
                }
            });
        }
        if let Some(reset) = &widgets.reset {
            let weak = Rc::downgrade(self);
            reset.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.reset_channel();
                }
            });
        }
        if let Some(field) = &widgets.member {
            let weak = Rc::downgrade(self);
            field.connect_changed(move |text| {
                if let Some(app) = weak.upgrade() {
                    // Emptied after an invite: its confirmation stays.
                    if let Ok(mut state) = app.state.try_borrow_mut()
                        && !text.is_empty()
                    {
                        state.admin.member_error = None;
                        state.admin.member_status = None;
                    }
                    app.update_admin();
                }
            });
            let weak = Rc::downgrade(self);
            field.entry.connect_activate(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.add_member();
                }
            });
        }
        if let Some(add) = &widgets.member_add {
            let weak = Rc::downgrade(self);
            add.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.add_member();
                }
            });
        }
        if let Some(custom) = &widgets.custom {
            let weak = Rc::downgrade(self);
            custom.create.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.create_link();
                }
            });
        }
    }

    fn reset_channel(self: &Rc<Self>) {
        let saved = {
            let state = self.state.borrow();
            match &state.admin.dialog {
                Some(Dialog::ManageChannel(id)) => state.saved_channel(id),
                _ => None,
            }
        };
        let widgets = self.admin_window.borrow();
        if let (Some((name, private)), Some(widgets)) = (saved, widgets.as_ref()) {
            if let Some(entry) = &widgets.name {
                entry.set_text(&name);
            }
            if let Some(check) = &widgets.private {
                check.set_active(private);
            }
        }
        drop(widgets);
        self.state.borrow_mut().admin.error = None;
        self.update_admin();
    }

    /// The window's main action: create, save, continue or join.
    fn submit_admin(self: &Rc<Self>) {
        let (name, private, input) = {
            let widgets = self.admin_window.borrow();
            let Some(widgets) = widgets.as_ref() else {
                return;
            };
            (
                widgets.name.as_ref().map(|entry| entry.text().to_string()),
                widgets
                    .private
                    .as_ref()
                    .is_some_and(|check| check.is_active()),
                widgets
                    .join_entry
                    .as_ref()
                    .map(|entry| entry.text().to_string()),
            )
        };
        {
            let mut state = self.state.borrow_mut();
            if state.admin.busy {
                return;
            }
            let Some(dialog) = state.admin.dialog.clone() else {
                return;
            };
            let name = name.unwrap_or_default();
            match dialog {
                Dialog::CreateSpace => match space_name_error(&name) {
                    Some(error) => state.admin.error = Some(error.into()),
                    None => self.admin(&mut state, AdminOperation::CreateSpace { name }),
                },
                Dialog::ManageSpace => {
                    if let Some(error) = space_name_error(&name) {
                        state.admin.error = Some(error.into());
                    } else if let Some(space) = state.space.clone()
                        && name.trim() != state.space_name()
                    {
                        self.admin(&mut state, AdminOperation::UpdateSpace { space, name });
                    }
                }
                Dialog::CreateChannel | Dialog::ManageChannel(_) => {
                    let name = name.trim_end_matches('-').to_owned();
                    if let Some(error) = channel_name_error(&name) {
                        state.admin.error = Some(error.into());
                    } else if let Some(space) = state.space.clone() {
                        let operation = match dialog {
                            Dialog::ManageChannel(channel) => AdminOperation::UpdateChannel {
                                space,
                                channel,
                                name,
                                private,
                            },
                            _ => AdminOperation::CreateChannel {
                                space,
                                name,
                                private,
                            },
                        };
                        self.admin(&mut state, operation);
                    }
                }
                Dialog::JoinInvite => {
                    let join = &mut state.admin.join;
                    if join.busy {
                        return;
                    }
                    let operation = match &join.preview {
                        Some(preview) => {
                            join.joining = true;
                            AccountOperation::JoinInvite {
                                request: join.request + 1,
                                code: preview.code.clone(),
                            }
                        }
                        None => match invites::invite_code(&input.unwrap_or_default()) {
                            Some(code) => AccountOperation::PreviewInvite {
                                request: join.request + 1,
                                code,
                            },
                            None => {
                                join.error = Some(invites::INVALID_LINK.into());
                                drop(state);
                                self.update_admin();
                                return;
                            }
                        },
                    };
                    join.request += 1;
                    join.busy = true;
                    join.error = None;
                    self.account_op(&state, operation);
                }
                Dialog::InvitePeople => {}
            }
        }
        self.update_admin();
    }

    fn add_member(self: &Rc<Self>) {
        let Some(username) = self
            .admin_window
            .borrow()
            .as_ref()
            .and_then(|widgets| widgets.member.as_ref().map(|field| field.text()))
        else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            if state.admin.busy {
                return;
            }
            state.admin.member_status = None;
            let channel = state.admin.channel.clone();
            if username.len() < 3 {
                state.admin.member_error =
                    Some("Use 3–32 lowercase letters, numbers, or underscores.");
            } else if let Some(error) = invite_error(
                &username,
                &state.admin.members,
                &state.admin.invited,
                channel.is_some(),
            ) {
                state.admin.member_error = Some(error);
            } else if let Some(space) = state.space.clone() {
                state.admin.member_error = None;
                self.admin(
                    &mut state,
                    AdminOperation::AddMember {
                        space,
                        channel,
                        username,
                    },
                );
            }
        }
        self.update_admin();
    }

    fn create_link(self: &Rc<Self>) {
        let choices = {
            let widgets = self.admin_window.borrow();
            let Some(custom) = widgets.as_ref().and_then(|widgets| widgets.custom.as_ref()) else {
                return;
            };
            let unit = invites::EXPIRY_UNITS
                .get(custom.unit.selected() as usize)
                .map_or(ExpiryUnit::Days, |(unit, _)| *unit);
            (
                invites::expiry_seconds(&custom.amount.text(), unit),
                invites::max_uses(&custom.uses.text()),
                custom
                    .channels
                    .get(custom.channel.selected() as usize)
                    .cloned()
                    .flatten(),
            )
        };
        {
            let mut state = self.state.borrow_mut();
            let Some(space) = state.space.clone() else {
                return;
            };
            let links = &mut state.admin.links;
            if links.creating || links.loading || !links.revoking.is_empty() {
                return;
            }
            match choices {
                (Err(error), _, _) | (_, Err(error), _) => links.error = Some(error.into()),
                (Ok(max_age), Ok(max_uses), channel) => {
                    links.request = links.request.wrapping_add(1);
                    links.creating = true;
                    links.error = None;
                    let operation = AccountOperation::CreateInviteLink {
                        space,
                        request: links.request,
                        channel,
                        max_age,
                        max_uses,
                    };
                    self.account_op(&state, operation);
                }
            }
            state.admin.revision += 1;
        }
        self.update_admin();
    }

    fn copy_link(self: &Rc<Self>, code: &str) {
        self.ui
            .window
            .clipboard()
            .set_text(&invites::invite_url(self.api.base(), code));
        {
            let mut state = self.state.borrow_mut();
            state.admin.links.copied = Some((code.to_owned(), Instant::now()));
            state.admin.revision += 1;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(COPIED_FOR, move || {
            if let Some(app) = weak.upgrade() {
                app.state.borrow_mut().admin.revision += 1;
                app.update_admin();
            }
        });
        self.update_admin();
    }

    /// Updates the form window's labels, buttons and lists.
    fn update_admin(self: &Rc<Self>) {
        let Ok(mut widgets) = self.admin_window.try_borrow_mut() else {
            return;
        };
        let Some(widgets) = widgets.as_mut() else {
            return;
        };
        let Ok(state) = self.state.try_borrow() else {
            return;
        };
        let admin = &state.admin;
        let busy = admin.busy;
        let name = widgets
            .name
            .as_ref()
            .map(|entry| entry.text().to_string())
            .unwrap_or_default();
        let private = widgets
            .private
            .as_ref()
            .is_some_and(|check| check.is_active());
        set_error(&widgets.error, admin.error.as_deref());
        if let Some(icon) = &widgets.icon {
            self.update_icon_row(icon, &state);
        }
        if let Some(entry) = &widgets.name {
            entry.set_sensitive(!busy);
        }
        match &widgets.dialog {
            Dialog::CreateSpace => {
                if let Some(submit) = &widgets.submit {
                    submit.set_label(if busy { "Creating…" } else { "Create space" });
                    submit.set_sensitive(!busy && !name.trim().is_empty());
                }
                if let Some(cancel) = &widgets.cancel {
                    cancel.set_sensitive(!busy);
                }
            }
            Dialog::ManageSpace => {
                if let Some(submit) = &widgets.submit {
                    submit.set_label(
                        if busy
                            && admin.member_status.is_none()
                            && admin.dialog == Some(Dialog::ManageSpace)
                            && name.trim() != state.space_name()
                        {
                            "Saving…"
                        } else {
                            "Save name"
                        },
                    );
                    submit.set_sensitive(!busy && name.trim() != state.space_name());
                }
            }
            Dialog::CreateChannel | Dialog::ManageChannel(_) => {
                if let Some(hint) = &widgets.private_hint {
                    hint.set_label(&if private {
                        "Only you and the people you add can view or join.".to_owned()
                    } else {
                        format!(
                            "Anyone in {} can view or join this channel.",
                            state.space_name()
                        )
                    });
                }
                if let Some(submit) = &widgets.submit {
                    submit.set_label(if busy {
                        "Creating…"
                    } else {
                        "Create channel"
                    });
                    submit.set_sensitive(!busy && !name.trim_end_matches('-').is_empty());
                }
                if let Some(cancel) = &widgets.cancel {
                    cancel.set_sensitive(!busy);
                }
                if let Dialog::ManageChannel(id) = &widgets.dialog {
                    let saved = state.saved_channel(id);
                    let dirty = saved
                        .as_ref()
                        .is_some_and(|(saved, was)| *saved != name || *was != private);
                    if let Some(bar) = &widgets.save_bar {
                        bar.set_visible(dirty);
                    }
                    if let Some(save) = &widgets.save {
                        save.set_label(if busy { "Saving…" } else { "Save changes" });
                        save.set_sensitive(!busy);
                    }
                    if let Some(grants) = &widgets.grants {
                        // Members follow the saved privacy, as on web.
                        grants.set_visible(saved.is_some_and(|(_, private)| private));
                    }
                }
            }
            Dialog::JoinInvite => {
                let join = &admin.join;
                if let Some(input) = &widgets.join_input {
                    input.set_visible(join.preview.is_none());
                }
                let text = widgets
                    .join_entry
                    .as_ref()
                    .map(|entry| entry.text().to_string())
                    .unwrap_or_default();
                set_error(&widgets.error, join.error.as_deref());
                if let Some(submit) = &widgets.submit {
                    submit.set_label(match (&join.preview, join.joining, join.busy) {
                        (Some(preview), true, _) if preview.already_member => "Opening…",
                        (Some(preview), false, _) if preview.already_member => "Open space",
                        (Some(_), true, _) => "Joining…",
                        (Some(_), false, _) => "Join space",
                        (None, _, true) => "Checking…",
                        (None, _, false) => "Continue",
                    });
                    submit.set_sensitive(
                        !join.busy && (join.preview.is_some() || !text.trim().is_empty()),
                    );
                }
                if let Some(cancel) = &widgets.cancel {
                    cancel.set_sensitive(!join.joining);
                }
                let title = if join
                    .preview
                    .as_ref()
                    .is_some_and(|preview| preview.already_member)
                {
                    "Open space"
                } else {
                    "Join with invite link"
                };
                widgets.title.set_label(title);
            }
            Dialog::InvitePeople => {}
        }
        if let Some(add) = &widgets.member_add {
            add.set_sensitive(!busy);
        }
        if let Some(field) = &widgets.member {
            field.set_people(state.member_suggestions());
        }
        if let Some(note) = &widgets.member_note {
            let (text, error) = match (admin.member_error, admin.member_status) {
                (Some(error), _) => (error, true),
                (None, Some(status)) => (status, false),
                (None, None) => ("", false),
            };
            note.set_label(text);
            note.set_visible(!text.is_empty());
            if error {
                note.add_css_class("error");
                note.remove_css_class("muted");
            } else {
                note.remove_css_class("error");
                note.add_css_class("muted");
            }
        }
        if let Some(count) = &widgets.members_count {
            count.set_label(&admin.members.len().to_string());
        }
        if let Some(custom) = &widgets.custom {
            let links = &admin.links;
            let enabled = !links.loading && !links.creating && links.revoking.is_empty();
            custom.create.set_sensitive(enabled);
            custom.create.set_label(
                if links.creating && !links.regenerating && widgets.dialog == Dialog::ManageSpace {
                    "Creating…"
                } else {
                    "Create separate link"
                },
            );
            custom
                .root
                .set_visible(widgets.dialog == Dialog::ManageSpace || links.settings_open);
        }
        if widgets.rendered == Some(admin.revision) {
            return;
        }
        widgets.rendered = Some(admin.revision);
        // Rebuilt once the click that changed them has finished: GTK must
        // not lose a button while it handles that button's click.
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(app) = weak.upgrade() {
                app.rebuild_admin_lists();
            }
        });
    }

    fn rebuild_admin_lists(self: &Rc<Self>) {
        let Ok(widgets) = self.admin_window.try_borrow() else {
            return;
        };
        let Some(widgets) = widgets.as_ref() else {
            return;
        };
        let Ok(state) = self.state.try_borrow() else {
            return;
        };
        if let Some(primary) = &widgets.primary {
            self.render_primary_link(&state, primary);
        }
        if let Some(links) = &widgets.links {
            self.render_links(&state, links, widgets.dialog == Dialog::InvitePeople);
        }
        if let Some(invited) = &widgets.invited {
            self.render_invited(&state, invited);
        }
        if let Some(members) = &widgets.members {
            self.render_managed_members(&state, members);
        }
        if let Some(preview) = &widgets.preview {
            self.render_preview(&state, preview);
        }
    }

    fn render_primary_link(self: &Rc<Self>, state: &State, root: &gtk::Box) {
        clear(root);
        let links = &state.admin.links;
        let pending = links.creating || !links.revoking.is_empty();
        let custom_button = |text: &str| {
            let button = button(text, "flat");
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    {
                        let mut state = app.state.borrow_mut();
                        let links = &mut state.admin.links;
                        links.settings_open = !links.settings_open;
                        state.admin.revision += 1;
                    }
                    app.update_admin();
                }
            });
            button
        };
        if let Some(link) = &links.default_link {
            let url = invites::invite_url(self.api.base(), &link.code);
            let text = label(&url, &["monospace", "invite-url"]);
            root.append(&text);
            root.append(&label(
                &invites::link_meta(link, chrono::Utc::now()),
                &["muted", "small"],
            ));
            if links.creating {
                root.append(&hint("Preparing invite link…"));
            }
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let copy = button("Copy link", "primary");
            copy.set_sensitive(!pending);
            let weak = Rc::downgrade(self);
            let code = link.code.clone();
            copy.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.copy_link(&code);
                }
            });
            row.append(&copy);
            for (text, regenerate) in [("Regenerate link", true), ("Deactivate link", false)] {
                let action = button(text, "flat");
                action.set_sensitive(!pending);
                let weak = Rc::downgrade(self);
                let code = link.code.clone();
                action.connect_clicked(move |_| {
                    if let Some(app) = weak.upgrade() {
                        app.open_confirm(Some(Confirm::Link {
                            code: code.clone(),
                            regenerate,
                        }));
                    }
                });
                row.append(&action);
            }
            let custom = custom_button("Custom links");
            custom.set_sensitive(!pending);
            row.append(&custom);
            root.append(&row);
        } else if links.creating {
            root.append(&hint("Preparing invite link…"));
        } else {
            root.append(&hint("Opening this dialog again prepares a new link."));
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let again = button("Create another link", "primary");
            let weak = Rc::downgrade(self);
            again.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    {
                        let mut state = app.state.borrow_mut();
                        app.ensure_default_link(&mut state);
                        state.admin.revision += 1;
                    }
                    app.update_admin();
                }
            });
            row.append(&again);
            row.append(&custom_button("Custom links"));
            root.append(&row);
        }
        if let Some(error) = &links.error {
            let error = label(error, &["error", "small"]);
            error.set_wrap(true);
            root.append(&error);
        }
        if links.settings_open {
            root.append(&hint(
                "These settings create a separate link; they do not edit the existing link.",
            ));
        }
    }

    fn render_links(self: &Rc<Self>, state: &State, root: &gtk::Box, invite_people: bool) {
        clear(root);
        let links = &state.admin.links;
        if invite_people && !links.settings_open {
            return;
        }
        if !invite_people && let Some(error) = &links.error {
            let error = label(error, &["error", "small"]);
            error.set_wrap(true);
            root.append(&error);
        }
        if links.loading && links.links.is_empty() {
            root.append(&hint("Loading invite links…"));
        }
        let enabled = !links.loading && !links.creating && links.revoking.is_empty();
        let now = chrono::Utc::now();
        for link in &links.links {
            let card = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            card.add_css_class("invite-link");
            let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
            text.set_hexpand(true);
            let url = label(
                &invites::invite_url(self.api.base(), &link.code),
                &["monospace", "small"],
            );
            url.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.append(&url);
            text.append(&label(&invites::link_meta(link, now), &["muted", "small"]));
            card.append(&text);
            let copied = links
                .copied
                .as_ref()
                .is_some_and(|(code, at)| *code == link.code && at.elapsed() < COPIED_FOR);
            let copy = button(if copied { "Copied" } else { "Copy" }, "");
            copy.set_valign(gtk::Align::Center);
            let weak = Rc::downgrade(self);
            let code = link.code.clone();
            copy.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.copy_link(&code);
                }
            });
            let revoking = links.revoking.contains(&link.code);
            let revoke = button(
                if revoking { "Revoking…" } else { "Revoke" },
                "destructive-action",
            );
            revoke.set_valign(gtk::Align::Center);
            revoke.set_sensitive(enabled);
            let weak = Rc::downgrade(self);
            let code = link.code.clone();
            revoke.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.open_confirm(Some(Confirm::Link {
                        code: code.clone(),
                        regenerate: false,
                    }));
                }
            });
            card.append(&copy);
            card.append(&revoke);
            root.append(&card);
        }
    }

    fn person_row(&self, member: &Member, size: i32) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let avatar = AvatarView::new(size, false);
        avatar.set(
            &self.images,
            &Avatar::new(
                &member.display_name,
                member.avatar_id,
                &member.avatar_asset,
                &member.avatar_url,
            ),
        );
        row.append(&avatar.root);
        row
    }

    fn render_invited(self: &Rc<Self>, state: &State, root: &gtk::Box) {
        clear(root);
        let admin = &state.admin;
        let channel = admin.channel.clone();
        let title = if channel.is_some() {
            label(
                &format!("Pending invitations · {}", admin.invited.len()),
                &["muted", "small"],
            )
        } else {
            heading(&format!("Pending invitations  {}", admin.invited.len()))
        };
        title.set_margin_top(6);
        root.append(&title);
        for invitation in &admin.invited {
            let row = self.person_row(invitation, 24);
            let name = label(&format!("@{}", invitation.username), &[]);
            name.set_hexpand(true);
            row.append(&name);
            let cancel = button("Cancel", "");
            cancel.set_sensitive(!admin.busy);
            let weak = Rc::downgrade(self);
            let (user, channel) = (invitation.id.clone(), channel.clone());
            cancel.connect_clicked(move |_| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                {
                    let mut state = app.state.borrow_mut();
                    let Some(space) = state.space.clone() else {
                        return;
                    };
                    let operation = match channel.clone() {
                        Some(channel) => AdminOperation::RemoveMember {
                            space,
                            channel: Some(channel),
                            member: user.clone(),
                        },
                        None => AdminOperation::CancelInvitation {
                            space,
                            user: user.clone(),
                        },
                    };
                    app.admin(&mut state, operation);
                    state.admin.revision += 1;
                }
                app.update_admin();
            });
            row.append(&cancel);
            root.append(&row);
        }
    }

    fn render_managed_members(self: &Rc<Self>, state: &State, root: &gtk::Box) {
        clear(root);
        let admin = &state.admin;
        let channel = admin.channel.clone();
        for member in &admin.members {
            let row = self.person_row(member, 30);
            row.add_css_class("managed-member");
            let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
            text.set_hexpand(true);
            text.append(&label(&member.display_name, &["strong"]));
            text.append(&label(
                &format!(
                    "@{}{}",
                    member.username,
                    if member.owner { " · Owner" } else { "" }
                ),
                &["muted", "small"],
            ));
            row.append(&text);
            if !member.owner {
                let remove = button("Remove", "destructive-action");
                remove.set_valign(gtk::Align::Center);
                let weak = Rc::downgrade(self);
                let (member, channel) = (member.clone(), channel.clone());
                remove.connect_clicked(move |_| {
                    let Some(app) = weak.upgrade() else {
                        return;
                    };
                    let confirm = {
                        let state = app.state.borrow();
                        state.space.clone().map(|space| Confirm::RemoveMember {
                            place: match &channel {
                                Some(id) => state
                                    .saved_channel(id)
                                    .map_or_else(|| "this channel".into(), |(name, _)| name),
                                None => state.space_name(),
                            },
                            space,
                            channel: channel.clone(),
                            member: member.id.clone(),
                            name: member.display_name.clone(),
                        })
                    };
                    app.open_confirm(confirm);
                });
                row.append(&remove);
            }
            root.append(&row);
        }
    }

    fn render_preview(&self, state: &State, root: &gtk::Box) {
        clear(root);
        let Some(preview) = &state.admin.join.preview else {
            root.set_visible(false);
            return;
        };
        root.set_visible(true);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let tile = label(&caper_client::space_initial(&preview.space.name), &["space-tile"]);
        tile.set_xalign(0.5);
        line.append(&tile);
        line.append(&label(&preview.space.name, &["admin-heading"]));
        root.append(&line);
        let inviter = &preview.inviter;
        let by = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let avatar = AvatarView::new(18, false);
        avatar.set(
            &self.images,
            &Avatar::new(
                &inviter.display_name,
                inviter.avatar_id,
                &inviter.avatar_asset,
                &inviter.avatar_url,
            ),
        );
        by.append(&avatar.root);
        by.append(&label(
            &format!("Invited by {}", inviter.display_name),
            &["muted", "small"],
        ));
        root.append(&by);
        for line in [
            preview
                .already_member
                .then(|| "You’re already a member".to_owned()),
            Some(invites::plural(preview.member_count, "member")),
            preview
                .channel
                .as_ref()
                .map(|channel| format!("Opens in #{}", channel.name)),
        ]
        .into_iter()
        .flatten()
        {
            root.append(&label(&line, &["muted", "small"]));
        }
    }

    // ---------------------------------------------------------------------
    // Confirmations.

    fn build_confirm(self: &Rc<Self>, confirm: Confirm) -> ConfirmWindow {
        let state = self.state.borrow();
        let title = match &confirm {
            Confirm::Invitation(_) => "You’re invited!".to_owned(),
            Confirm::LeaveSpace { name, .. } => format!("Leave {name}?"),
            Confirm::LeaveChannel { name, .. } => format!("Leave #{name}?"),
            Confirm::Delete { channel, .. } => if channel.is_some() {
                "Delete channel"
            } else {
                "Delete space"
            }
            .to_owned(),
            Confirm::RemoveMember {
                channel,
                name,
                place,
                ..
            } => {
                if channel.is_some() {
                    format!("Remove {name} from #{place}?")
                } else {
                    format!("Remove {name}?")
                }
            }
            Confirm::Link { regenerate, .. } => if *regenerate {
                "Regenerate link?"
            } else {
                "Deactivate link?"
            }
            .to_owned(),
        };
        let (window, root, _) = self.admin_shell(&title, 440);
        if let Some(parent) = self.admin_window.borrow().as_ref() {
            window.set_transient_for(Some(&parent.window));
        }
        let mut lines: Vec<String> = Vec::new();
        let (cancel, action, danger) = match &confirm {
            Confirm::Invitation(space) => {
                lines.push(format!("Join {}?", space.name));
                if let Some(inviter) = &space.inviter {
                    lines.push(format!(
                        "{} (@{}) invited you.",
                        inviter.display_name, inviter.username
                    ));
                }
                ("Decline", "Accept invitation", false)
            }
            Confirm::LeaveSpace { .. } => {
                lines.push("You will lose access to its channels and conversations. An owner can add you again later.".into());
                ("Cancel", "Leave space", true)
            }
            Confirm::LeaveChannel { channel, .. } => {
                let private_loss = !state.owner()
                    && state
                        .saved_channel(channel)
                        .is_some_and(|(_, private)| private);
                lines.push(if private_loss {
                    "You’ll lose access and need another invitation to return.".into()
                } else {
                    "It will leave your sidebar. You can preview and rejoin it from the channel list.".into()
                });
                ("Cancel", "Leave channel", false)
            }
            Confirm::Delete { channel, name, .. } => {
                let display = if channel.is_some() {
                    format!("#{name}")
                } else {
                    name.clone()
                };
                lines.push(format!(
                    "Delete {display} for everyone? {} This cannot be undone.",
                    if channel.is_some() {
                        "This channel and its messages will disappear from the space."
                    } else {
                        "All its channels and their messages will disappear from the space."
                    }
                ));
                (
                    "Cancel",
                    if channel.is_some() {
                        "Delete channel"
                    } else {
                        "Delete space"
                    },
                    true,
                )
            }
            Confirm::RemoveMember { channel, place, .. } => {
                lines.push(if channel.is_some() {
                    "They’ll lose access to this private channel. You can add them again later.".into()
                } else {
                    format!("They’ll lose access to {place} and its channels. You can invite them again later.")
                });
                ("Cancel", "Remove", true)
            }
            Confirm::Link { regenerate, .. } => {
                lines.push("The shared old URL will stop accepting new joins.".into());
                if *regenerate {
                    lines.push("The replacement link resets to 30 days and 100 joins.".into());
                }
                (
                    "Cancel",
                    if *regenerate {
                        "Regenerate link"
                    } else {
                        "Deactivate link"
                    },
                    true,
                )
            }
        };
        drop(state);
        for line in lines {
            let text = label(&line, &[]);
            text.set_wrap(true);
            text.set_max_width_chars(48);
            root.append(&text);
        }
        let error = label("", &["error"]);
        error.set_wrap(true);
        root.append(&error);
        let action_label = action;
        let cancel = button(cancel, "");
        let action = button(
            action,
            if danger {
                "destructive-action"
            } else {
                "primary"
            },
        );
        root.append(&actions(&cancel, &action));
        window.set_child(Some(&root));
        let weak = Rc::downgrade(self);
        let invitation = matches!(confirm, Confirm::Invitation(_));
        cancel.connect_clicked(move |button| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            if invitation {
                app.confirm_action(true);
            } else if let Some(window) = button.root().and_downcast::<gtk::Window>() {
                window.close();
            }
        });
        let weak = Rc::downgrade(self);
        action.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.confirm_action(false);
            }
        });
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            let Some(app) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let Ok(mut state) = app.state.try_borrow_mut() else {
                return glib::Propagation::Proceed;
            };
            // A change in flight can't be cancelled.
            if state.admin.busy {
                return glib::Propagation::Stop;
            }
            state.admin.confirm = None;
            state.admin.error = None;
            drop(state);
            app.confirm_window.borrow_mut().take();
            glib::Propagation::Proceed
        });
        ConfirmWindow {
            window,
            confirm,
            button: action,
            action: action_label,
            cancel,
            error,
        }
    }

    /// The confirmation's action, or an invitation's Decline.
    fn confirm_action(self: &Rc<Self>, decline: bool) {
        {
            let mut state = self.state.borrow_mut();
            if state.admin.busy {
                return;
            }
            let Some(confirm) = state.admin.confirm.clone() else {
                return;
            };
            state.admin.declining = decline;
            let operation = match confirm {
                Confirm::Invitation(space) if decline => {
                    AdminOperation::DeclineInvitation { space: space.id }
                }
                Confirm::Invitation(space) => AdminOperation::AcceptInvitation { space: space.id },
                Confirm::LeaveSpace { id, .. } => {
                    let Some(member) = state.account.as_ref().map(|account| account.id.clone())
                    else {
                        return;
                    };
                    AdminOperation::LeaveSpace { space: id, member }
                }
                Confirm::LeaveChannel { space, channel, .. } => {
                    self.play(Effect::Leave);
                    AdminOperation::LeaveChannel { space, channel }
                }
                Confirm::Delete { space, channel, .. } => match channel {
                    Some(channel) => AdminOperation::DeleteChannel { space, channel },
                    None => AdminOperation::DeleteSpace { space },
                },
                Confirm::RemoveMember {
                    space,
                    channel,
                    member,
                    ..
                } => AdminOperation::RemoveMember {
                    space,
                    channel,
                    member,
                },
                Confirm::Link { code, regenerate } => {
                    let Some(space) = state.space.clone() else {
                        return;
                    };
                    state.admin.confirm = None;
                    let links = &mut state.admin.links;
                    links.request = links.request.wrapping_add(1);
                    links.loading = false;
                    links.error = None;
                    let operation = if regenerate {
                        links.default_request = links.default_request.wrapping_add(1);
                        links.creating = true;
                        links.regenerating = true;
                        AccountOperation::RegenerateInviteLink {
                            space,
                            code,
                            request: links.default_request,
                        }
                    } else {
                        links.revoking.insert(code.clone());
                        AccountOperation::RevokeInviteLink {
                            space,
                            code,
                            request: links.request,
                        }
                    };
                    state.admin.revision += 1;
                    self.account_op(&state, operation);
                    drop(state);
                    self.render_admin();
                    return;
                }
            };
            self.admin(&mut state, operation);
        }
        self.update_confirm();
    }

    fn update_confirm(&self) {
        let Ok(widgets) = self.confirm_window.try_borrow() else {
            return;
        };
        let Some(widgets) = widgets.as_ref() else {
            return;
        };
        let Ok(state) = self.state.try_borrow() else {
            return;
        };
        let busy = state.admin.busy;
        set_error(&widgets.error, state.admin.error.as_deref());
        widgets.button.set_sensitive(!busy);
        widgets.cancel.set_sensitive(!busy);
        let progress = match &widgets.confirm {
            Confirm::Invitation(_) if state.admin.declining => {
                widgets
                    .cancel
                    .set_label(if busy { "Declining…" } else { "Decline" });
                None
            }
            Confirm::Invitation(_) => Some("Accepting…"),
            Confirm::LeaveSpace { .. } | Confirm::LeaveChannel { .. } => Some("Leaving…"),
            Confirm::Delete { .. } => Some("Deleting…"),
            Confirm::RemoveMember { .. } => Some("Removing…"),
            Confirm::Link { .. } => None,
        };
        match progress {
            Some(progress) if busy => widgets.button.set_label(progress),
            _ => widgets.button.set_label(widgets.action),
        }
    }

    // ---------------------------------------------------------------------
    // The sidebar: the rail's create and join buttons, space invitations
    // and private channel invitations.

    pub(super) fn render_admin_sidebar(self: &Rc<Self>, state: &State) {
        let ui = &self.ui;
        for space in &state.admin.invitations {
            let tile: gtk::Button =
                crate::app::named("GtkButton", &format!("Invitation to {}", space.name));
            tile.set_label(&caper_client::space_initial(&space.name));
            tile.set_tooltip_text(Some(&format!("Invitation to {}", space.name)));
            tile.add_css_class("invited");
            let weak = Rc::downgrade(self);
            let space = space.clone();
            tile.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.open_confirm(Some(Confirm::Invitation(space.clone())));
                }
            });
            ui.rail.append(&tile);
        }
        let create: gtk::Button = crate::app::named("GtkButton", "Create space");
        create.set_child(Some(&{
            let image = self
                .images
                .icon_image(crate::images::Icon::Plus, "#d8775c", 18);
            image.set_halign(gtk::Align::Center);
            image.set_valign(gtk::Align::Center);
            image
        }));
        create.add_css_class("rail-action");
        create.add_css_class("dashed");
        create.set_tooltip_text(Some(&state.create_space_tooltip()));
        create.set_sensitive(state.can_create_space());
        let weak = Rc::downgrade(self);
        create.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.open_create_space();
            }
        });
        ui.rail.append(&create);
        // Joining by link never counts against owned spaces.
        let join: gtk::Button = crate::app::named("GtkButton", "Join with invite link");
        join.set_child(Some(&{
            let image = self
                .images
                .icon_image(crate::images::Icon::Link, "#b9bcbe", 18);
            image.set_halign(gtk::Align::Center);
            image.set_valign(gtk::Align::Center);
            image
        }));
        join.add_css_class("rail-action");
        join.set_tooltip_text(Some("Join with invite link"));
        let weak = Rc::downgrade(self);
        join.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.open_join_invite();
            }
        });
        ui.rail.append(&join);

        let owner = state.owner();
        ui.space_header.set_visible(state.detail.is_some());
        ui.space_header
            .set_sensitive(owner || state.can_leave_space());
        ui.new_channel.set_visible(owner);
        ui.new_channel.set_sensitive(state.can_create_channel());
        ui.new_channel
            .set_tooltip_text(Some(&state.create_channel_tooltip()));
        clear(&ui.channel_invites);
        let invitations = state
            .detail
            .as_ref()
            .map_or(&[][..], |detail| detail.channel_invitations.as_slice());
        if !invitations.is_empty() {
            ui.channel_invites
                .append(&label("Invitations", &["section"]));
        }
        for invitation in invitations {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
            card.add_css_class("channel-invite");
            card.append(&label(
                &format!("Private invitation · #{}", invitation.channel.name),
                &["strong", "small"],
            ));
            let text = label(
                &format!(
                    "{} (@{}) invited you to private channel #{}.",
                    invitation.inviter.display_name,
                    invitation.inviter.username,
                    invitation.channel.name
                ),
                &["muted", "small"],
            );
            text.set_wrap(true);
            card.append(&text);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            for (text, accept) in [("Decline", false), ("Accept", true)] {
                let action = button(text, if accept { "primary" } else { "" });
                action.set_sensitive(!state.admin.busy);
                let weak = Rc::downgrade(self);
                let channel = invitation.channel.id.clone();
                action.connect_clicked(move |_| {
                    let Some(app) = weak.upgrade() else {
                        return;
                    };
                    {
                        let mut state = app.state.borrow_mut();
                        let Some(space) = state.space.clone() else {
                            return;
                        };
                        let channel = channel.clone();
                        app.admin(
                            &mut state,
                            if accept {
                                AdminOperation::AcceptChannelInvitation { space, channel }
                            } else {
                                AdminOperation::DeclineChannelInvitation { space, channel }
                            },
                        );
                        state.layout += 1;
                    }
                    app.render();
                });
                row.append(&action);
            }
            card.append(&row);
            ui.channel_invites.append(&card);
        }
    }
}
