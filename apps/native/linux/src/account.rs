//! Your profile, as on the egui desktop: the setup dialog an account without
//! a username or display name must finish, and Edit profile from your own
//! card. The picture (a mascot, a picked picture or a previous one) stays a
//! private draft until Save profile. A child of `app`.

use super::{App, State};
use crate::images::{Avatar, AvatarView, Icon};
use crate::ui::label;
use caper_client::model::Account;
use caper_client::names;
use caper_client::pictures::{self, PreviousPicture};
use caper_client::profile_edit::Draft;
use caper_client::uploads::Scratch;
use caper_client::worker::Command;
use gtk::{gio, glib, prelude::*};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

pub(super) struct ProfileDialog {
    pub(super) window: gtk::Window,
    /// An account without a username or display name: it can't be closed
    /// until saved (Log out leaves instead).
    setup: bool,
    pub(super) draft: Draft,
    /// The save on its way, by its generation.
    saving: Option<u64>,
    error: Option<String>,
    picture_error: Option<String>,
    /// The picked picture's first frame, shown until it is saved.
    local: Option<caper_client::pictures::Frames>,
    /// Previous pictures: the request on its way, what loaded and the next
    /// page's cursor.
    history: Option<History>,
    frame: AvatarView,
    picture_menu: gtk::Button,
    regenerate: gtk::Button,
    picture_status: gtk::Label,
    gallery: gtk::FlowBox,
    gallery_box: gtk::Box,
    gallery_more: gtk::Button,
    username: gtk::Entry,
    name: gtk::Entry,
    name_hint: gtk::Label,
    error_label: gtk::Label,
    save: gtk::Button,
}

#[derive(Default)]
struct History {
    request: Option<Instant>,
    pictures: Vec<PreviousPicture>,
    cursor: Option<String>,
    error: Option<String>,
    /// How many pictures the gallery shows, so it rebuilds only on change.
    shown: usize,
}

impl State {
    /// An account must choose a username and display name first.
    pub(super) fn needs_profile(&self) -> bool {
        self.account
            .as_ref()
            .is_some_and(|account| account.username.is_none() || account.display_name.is_none())
    }
}

fn text(entry: &gtk::Entry) -> String {
    entry.text().to_string()
}

impl App {
    pub(super) fn open_profile_editor(self: &Rc<Self>) {
        if self.profile_dialog.borrow().is_some() {
            return;
        }
        let Some(account) = self.state.borrow().account.clone() else {
            return;
        };
        self.close_profile();
        let setup = account.username.is_none() || account.display_name.is_none();
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title("Edit profile")
            .default_width(420)
            .resizable(false)
            .deletable(!setup)
            .build();
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("dialog-body");
        root.append(&label("Edit profile", &["dialog-title"]));

        // The picture: a frame with its menu, and Regenerate for a mascot.
        // The frame opens the picture menu; its pencil badge sits on the
        // corner, and Regenerate on the other corner for a mascot.
        let frame = AvatarView::new(96, false);
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        badge.add_css_class("picture-badge");
        badge.append(&self.images.icon_image(Icon::Pencil, "#0c0d0f", 15));
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::End);
        badge.set_can_target(false);
        let face = gtk::Overlay::new();
        face.set_child(Some(&frame.root));
        face.add_overlay(&badge);
        let picture_menu: gtk::Button = crate::app::named("GtkButton", "Change profile picture");
        picture_menu.set_child(Some(&face));
        picture_menu.add_css_class("picture-frame");
        picture_menu.set_tooltip_text(Some("Change profile picture"));
        let regenerate: gtk::Button = crate::app::named("GtkButton", "Regenerate avatar");
        regenerate.set_child(Some(&self.images.icon_image(
            Icon::RotateCcw,
            "#0c0d0f",
            15,
        )));
        regenerate.set_tooltip_text(Some("Regenerate avatar"));
        regenerate.add_css_class("picture-badge");
        regenerate.set_halign(gtk::Align::Start);
        regenerate.set_valign(gtk::Align::End);
        let pictures_row = gtk::Overlay::new();
        pictures_row.set_child(Some(&picture_menu));
        pictures_row.add_overlay(&regenerate);
        pictures_row.set_halign(gtk::Align::Start);
        pictures_row.set_margin_top(6);
        root.append(&pictures_row);
        let picture_status = label("", &["muted", "small"]);
        picture_status.set_wrap(true);
        root.append(&picture_status);
        let gallery_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        gallery_box.append(&label("Previous pictures", &["field"]));
        let gallery = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(6)
            .column_spacing(6)
            .row_spacing(6)
            .build();
        gallery_box.append(&gallery);
        let gallery_more = gtk::Button::with_label("Show more");
        gallery_more.set_halign(gtk::Align::Start);
        gallery_box.append(&gallery_more);
        gallery_box.set_visible(false);
        root.append(&gallery_box);

        root.append(&label("Username", &["field"]));
        let username = gtk::Entry::builder().max_length(32).build();
        username.set_text(account.username.as_deref().unwrap_or_default());
        root.append(&username);
        let hint = label(
            "Must be unique across Caper. 3–32 lowercase letters, numbers, or underscores.",
            &["muted", "small"],
        );
        hint.set_wrap(true);
        root.append(&hint);
        root.append(&label("Display name", &["field"]));
        let name = gtk::Entry::builder().max_length(64).build();
        name.set_text(account.display_name.as_deref().unwrap_or_default());
        root.append(&name);
        let name_hint = label("", &["muted", "small"]);
        name_hint.set_wrap(true);
        root.append(&name_hint);
        let error_label = label("", &["error"]);
        error_label.set_wrap(true);
        root.append(&error_label);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_margin_top(12);
        let log_out = gtk::Button::with_label("Log out");
        log_out.set_hexpand(true);
        log_out.set_halign(gtk::Align::Start);
        let save = gtk::Button::with_label("Save profile");
        save.add_css_class("primary");
        save.set_size_request(112, -1);
        buttons.append(&log_out);
        if !setup {
            let cancel = gtk::Button::with_label("Cancel");
            let weak = Rc::downgrade(self);
            cancel.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.close_profile_editor();
                }
            });
            buttons.append(&cancel);
        }
        buttons.append(&save);
        root.append(&buttons);
        window.set_child(Some(&root));

        // The picture menu.
        let actions = gio::SimpleActionGroup::new();
        type Run = fn(&Rc<App>);
        let entries: [(&str, Run); 4] = [
            ("edit", |app| app.edit_picture(pictures::Target::Avatar)),
            ("choose", |app| app.pick_picture(pictures::Target::Avatar)),
            ("previous", |app| app.load_previous_pictures(None)),
            ("remove", |app| {
                if let Some(dialog) = app.profile_dialog.borrow_mut().as_mut() {
                    dialog.draft.select(None, None);
                    dialog.local = None;
                    dialog.picture_error = None;
                }
                app.render_profile_editor();
            }),
        ];
        for (name, run) in entries {
            let action = gio::SimpleAction::new(name, None);
            let weak = Rc::downgrade(self);
            action.connect_activate(move |_, _| {
                if let Some(app) = weak.upgrade() {
                    run(&app);
                }
            });
            actions.add_action(&action);
        }
        window.insert_action_group("picture", Some(&actions));

        let weak = Rc::downgrade(self);
        picture_menu.connect_clicked(move |button| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let custom = app.profile_dialog.borrow().as_ref().is_some_and(|dialog| {
                dialog.local.is_some()
                    || pictures::Source::new(
                        &dialog.draft.preview.avatar_asset,
                        &dialog.draft.preview.avatar_url,
                    )
                    .is_some()
            });
            let menu = gio::Menu::new();
            if custom {
                menu.append(Some("Edit picture…"), Some("picture.edit"));
            }
            menu.append(Some("Choose a new picture…"), Some("picture.choose"));
            menu.append(Some("Previous pictures"), Some("picture.previous"));
            if custom {
                menu.append(Some("Remove picture"), Some("picture.remove"));
            }
            // Parented to the dialog window, which sizes itself by its
            // child alone: under a box or button the open menu would be
            // measured into the layout and stretch the dialog.
            let popover = gtk::PopoverMenu::from_model(Some(&menu));
            popover.set_has_arrow(false);
            let Some(window) = button.root().and_downcast::<gtk::Window>() else {
                return;
            };
            let Some(bounds) = button.compute_bounds(&window) else {
                return;
            };
            popover.set_parent(&window);
            popover.set_position(gtk::PositionType::Bottom);
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                bounds.x() as i32,
                bounds.y() as i32,
                bounds.width() as i32,
                bounds.height() as i32,
            )));
            popover.connect_closed(|popover| {
                let popover = popover.clone();
                glib::idle_add_local_once(move || popover.unparent());
            });
            popover.popup();
        });
        let weak = Rc::downgrade(self);
        regenerate.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                if let Some(dialog) = app.profile_dialog.borrow_mut().as_mut() {
                    dialog.draft.regenerate();
                    dialog.local = None;
                    dialog.picture_error = None;
                }
                app.render_profile_editor();
            }
        });
        let weak = Rc::downgrade(self);
        gallery_more.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                let cursor = app
                    .profile_dialog
                    .borrow()
                    .as_ref()
                    .and_then(|dialog| dialog.history.as_ref()?.cursor.clone());
                app.load_previous_pictures(cursor);
            }
        });
        for entry in [&username, &name] {
            let weak = Rc::downgrade(self);
            entry.connect_changed(move |entry| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                if entry.widget_name() == "username" {
                    let normalized = names::normalize_username(&entry.text());
                    if normalized != entry.text() {
                        let position = entry.position();
                        entry.set_text(&normalized);
                        entry.set_position(position.min(normalized.chars().count() as i32));
                    }
                }
                if let Some(dialog) = app.profile_dialog.borrow_mut().as_mut() {
                    dialog.error = None;
                }
                app.render_profile_editor();
            });
            let weak = Rc::downgrade(self);
            entry.connect_activate(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.save_profile();
                }
            });
        }
        username.set_widget_name("username");
        let weak = Rc::downgrade(self);
        save.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.save_profile();
            }
        });
        let weak = Rc::downgrade(self);
        log_out.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.close_profile_editor();
                app.sign_out();
            }
        });
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            let Some(app) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            // Setup can't be dismissed; an edit closes without saving.
            if app
                .profile_dialog
                .borrow()
                .as_ref()
                .is_some_and(|dialog| dialog.setup)
            {
                return glib::Propagation::Stop;
            }
            app.profile_dialog.borrow_mut().take();
            glib::Propagation::Proceed
        });
        *self.profile_dialog.borrow_mut() = Some(ProfileDialog {
            window: window.clone(),
            setup,
            draft: Draft::new(&account),
            saving: None,
            error: None,
            picture_error: None,
            local: None,
            history: None,
            frame,
            picture_menu,
            regenerate,
            picture_status,
            gallery,
            gallery_box,
            gallery_more,
            username: username.clone(),
            name,
            name_hint,
            error_label,
            save,
        });
        self.render_profile_editor();
        window.present();
        username.grab_focus();
    }

    pub(super) fn close_profile_editor(&self) {
        let dialog = self.profile_dialog.borrow_mut().take();
        if let Some(dialog) = dialog {
            dialog.window.destroy();
        }
    }

    /// Prepares `path` as the draft's picture (square, center-cropped) off
    /// the main thread.
    pub(super) fn prepare_profile_picture(
        self: &Rc<Self>,
        path: std::path::PathBuf,
        edit: Option<caper_client::attachment_edit::Edit>,
        scratch: Option<Arc<Scratch>>,
    ) {
        let wakes = self.wakes.clone();
        if let Some(dialog) = self.profile_dialog.borrow_mut().as_mut() {
            dialog.picture_error = None;
            dialog
                .draft
                .prepare(path, edit, scratch, crate::images::motion(), move || {
                    caper_client::Host::wake(&wakes)
                });
        }
        self.render_profile_editor();
    }

    pub(super) fn set_profile_picture_error(self: &Rc<Self>, error: Option<String>) {
        if let Some(dialog) = self.profile_dialog.borrow_mut().as_mut() {
            dialog.picture_error = error;
        }
        self.render_profile_editor();
    }

    /// A prepared picture became the draft's.
    pub(super) fn poll_profile_picture(self: &Rc<Self>) {
        let result = {
            let mut dialog = self.profile_dialog.borrow_mut();
            let Some(dialog) = dialog.as_mut() else {
                return;
            };
            let Some(result) = dialog.draft.poll() else {
                return;
            };
            match result {
                Ok((key, frames)) => {
                    dialog.local = Some(frames.clone());
                    Some((key, frames))
                }
                Err(error) => {
                    dialog.picture_error = Some(error);
                    None
                }
            }
        };
        if let Some((key, image)) = result {
            self.images.loaded(key, String::new(), Some(image));
        }
        self.render_profile_editor();
    }

    fn load_previous_pictures(self: &Rc<Self>, cursor: Option<String>) {
        let (token, epoch) = {
            let state = self.state.borrow();
            let Some(token) = state.token.clone() else {
                return;
            };
            (token, state.epoch)
        };
        let request = Instant::now();
        if let Some(dialog) = self.profile_dialog.borrow_mut().as_mut() {
            let history = dialog.history.get_or_insert_with(History::default);
            history.request = Some(request);
            history.error = None;
            if cursor.is_none() {
                history.pictures.clear();
            }
        }
        self.worker.send(Command::AvatarHistory {
            epoch,
            request,
            token,
            cursor,
        });
        self.render_profile_editor();
    }

    pub(super) fn previous_pictures(
        self: &Rc<Self>,
        request: Instant,
        result: Result<pictures::History, String>,
    ) {
        {
            let mut dialog = self.profile_dialog.borrow_mut();
            let Some(history) = dialog
                .as_mut()
                .and_then(|dialog| dialog.history.as_mut())
                .filter(|history| history.request == Some(request))
            else {
                return;
            };
            history.request = None;
            match result {
                Ok(page) => {
                    history.pictures.extend(page.pictures);
                    history.cursor = page.next_cursor;
                }
                Err(error) => history.error = Some(error),
            }
        }
        self.render_profile_editor();
    }

    fn save_profile(self: &Rc<Self>) {
        let (generation, token) = {
            let state = self.state.borrow();
            (state.generation, state.token.clone().unwrap_or_default())
        };
        let command = {
            let mut dialog = self.profile_dialog.borrow_mut();
            let Some(dialog) = dialog.as_mut() else {
                return;
            };
            if !ready(dialog, &self.state.borrow()) {
                return;
            }
            dialog.saving = Some(generation);
            dialog.error = None;
            Command::Profile {
                generation,
                token,
                username: text(&dialog.username).trim().to_ascii_lowercase(),
                display_name: text(&dialog.name).trim().into(),
                avatar: dialog.draft.save(),
            }
        };
        self.worker.send(command);
        self.render_profile_editor();
    }

    /// The save finished: the account changed, or the dialog shows why not.
    pub(super) fn profiled(
        self: &Rc<Self>,
        generation: u64,
        result: Result<(Account, caper_client::model::Spaces), String>,
    ) {
        let local = {
            let mut dialog = self.profile_dialog.borrow_mut();
            let Some(dialog) = dialog
                .as_mut()
                .filter(|dialog| dialog.saving == Some(generation))
            else {
                return;
            };
            dialog.saving = None;
            match &result {
                Ok(_) => dialog.draft.picture.as_ref().and(dialog.local.clone()),
                Err(error) => {
                    dialog.error = Some(error.clone());
                    None
                }
            }
        };
        let Ok((account, spaces)) = result else {
            self.render_profile_editor();
            return;
        };
        // The uploaded picture shows at once rather than loading again.
        if let (Some(image), Some(asset)) = (local, account.avatar_asset.clone()) {
            self.images.loaded(
                asset,
                account.avatar_url.clone().unwrap_or_default(),
                Some(image),
            );
        }
        let setup = self
            .profile_dialog
            .borrow()
            .as_ref()
            .is_some_and(|dialog| dialog.setup);
        {
            let mut state = self.state.borrow_mut();
            // Your row in the open space follows at once.
            if let Some(member) = state.detail.as_mut().and_then(|detail| {
                detail
                    .members
                    .iter_mut()
                    .find(|member| member.id == account.id)
            }) {
                member.avatar_id = account.avatar_id;
                member.avatar_asset.clone_from(&account.avatar_asset);
                member.avatar_url.clone_from(&account.avatar_url);
                if let Some(name) = &account.display_name {
                    member.display_name.clone_from(name);
                }
                if let Some(username) = &account.username {
                    member.username.clone_from(username);
                }
            }
            state.account = Some(account);
            state.spaces = spaces.spaces;
            state.layout += 1;
            if setup
                && state.space.is_none()
                && let Some(space) = state.spaces.first().map(|space| space.id.clone())
            {
                self.navigate(&mut state, Some(space), None);
            }
        }
        self.close_profile_editor();
        self.render();
    }

    pub(super) fn render_profile_editor(self: &Rc<Self>) {
        let Ok(mut dialog) = self.profile_dialog.try_borrow_mut() else {
            return;
        };
        let Some(dialog) = dialog.as_mut() else {
            return;
        };
        let Ok(state) = self.state.try_borrow() else {
            return;
        };
        let preview = &dialog.draft.preview;
        let name = text(&dialog.name);
        let shown_name = if name.trim().is_empty() {
            preview.username.clone().unwrap_or_default()
        } else {
            name.trim().to_owned()
        };
        let mut avatar = Avatar::new(
            &shown_name,
            preview.avatar_id,
            &preview.avatar_asset,
            &preview.avatar_url,
        );
        if dialog.local.is_some()
            && let Some(asset) = &preview.avatar_asset
        {
            avatar.picture = Some((asset.clone(), String::new()));
        }
        let custom = avatar.picture.is_some();
        dialog.frame.set(&self.images, &avatar);
        let preparing = dialog.draft.preparing.is_some();
        let saving = dialog.saving.is_some();
        let idle = !preparing && !saving;
        // Setup asks for a name first, as on the desktop.
        let named = preview.username.is_some() && preview.display_name.is_some();
        dialog.picture_menu.set_visible(named);
        dialog.regenerate.set_visible(named && !custom);
        dialog.regenerate.set_sensitive(idle);
        dialog.picture_menu.set_sensitive(idle);
        let status = if preparing {
            Some("Preparing picture…".to_owned())
        } else {
            dialog.picture_error.clone()
        };
        dialog
            .picture_status
            .set_label(status.as_deref().unwrap_or_default());
        dialog.picture_status.set_visible(status.is_some());
        if dialog.picture_error.is_some() && !preparing {
            dialog.picture_status.add_css_class("error");
        } else {
            dialog.picture_status.remove_css_class("error");
        }
        // Previous pictures.
        if let Some(history) = &mut dialog.history {
            dialog.gallery_box.set_visible(true);
            if history.shown != history.pictures.len() || history.pictures.is_empty() {
                while let Some(child) = dialog.gallery.first_child() {
                    dialog.gallery.remove(&child);
                }
                for picture in &history.pictures {
                    let view = AvatarView::new(48, false);
                    view.set(
                        &self.images,
                        &Avatar {
                            name: shown_name.clone(),
                            mascot: None,
                            picture: Some((
                                picture.avatar_asset.clone(),
                                picture.avatar_url.clone(),
                            )),
                        },
                    );
                    let button: gtk::Button = crate::app::named("GtkButton", "Use this picture");
                    button.set_child(Some(&view.root));
                    button.add_css_class("flat");
                    button.add_css_class("previous-picture");
                    let (weak, asset, url) = (
                        Rc::downgrade(self),
                        picture.avatar_asset.clone(),
                        picture.avatar_url.clone(),
                    );
                    button.connect_clicked(move |_| {
                        if let Some(app) = weak.upgrade() {
                            if let Some(dialog) = app.profile_dialog.borrow_mut().as_mut() {
                                dialog.draft.select(Some(asset.clone()), Some(url.clone()));
                                dialog.local = None;
                                dialog.picture_error = None;
                            }
                            app.render_profile_editor();
                        }
                    });
                    dialog.gallery.insert(&button, -1);
                }
                if history.pictures.is_empty() {
                    let empty = if history.request.is_some() {
                        "Loading…"
                    } else if history.error.is_some() {
                        "Previous pictures couldn’t load."
                    } else {
                        "No previous pictures yet."
                    };
                    dialog
                        .gallery
                        .insert(&label(empty, &["muted", "small"]), -1);
                }
                history.shown = history.pictures.len();
            }
            dialog.gallery_more.set_visible(history.cursor.is_some());
            dialog.gallery_more.set_sensitive(history.request.is_none());
        }
        let name_error = names::display_name_error(
            &name,
            state
                .account
                .as_ref()
                .and_then(|account| account.display_name.as_deref()),
        );
        dialog
            .name_hint
            .set_label(name_error.unwrap_or("This is what people see in conversations."));
        if name_error.is_some() {
            dialog.name_hint.add_css_class("error");
        } else {
            dialog.name_hint.remove_css_class("error");
        }
        dialog
            .error_label
            .set_label(dialog.error.as_deref().unwrap_or_default());
        dialog.error_label.set_visible(dialog.error.is_some());
        dialog.username.set_sensitive(!saving);
        dialog.name.set_sensitive(!saving);
        dialog
            .save
            .set_label(if saving { "Saving…" } else { "Save profile" });
        dialog.save.set_sensitive(ready(dialog, &state));
    }
}

/// Whether Save profile can run: something changed, nothing is busy, and
/// the username and display name are valid.
fn ready(dialog: &ProfileDialog, state: &State) -> bool {
    let username = text(&dialog.username);
    let name = text(&dialog.name);
    dialog.saving.is_none()
        && dialog.draft.preparing.is_none()
        && dialog.draft.dirty(&username, &name)
        && username.trim().len() >= 3
        && !name.trim().is_empty()
        && names::display_name_error(
            &name,
            state
                .account
                .as_ref()
                .and_then(|account| account.display_name.as_deref()),
        )
        .is_none()
}
