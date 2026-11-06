//! Choosing and editing a profile picture or a space icon, as on the egui
//! desktop: a picked file opens the editor on a square crop (Apply crop);
//! an animated GIF, APNG or WebP first asks whether to keep its animation,
//! as an edit saves a still. "Edit …" reopens the current one, downloading
//! it when needed. A profile picture becomes the profile draft's; a space
//! icon uploads at once. A child of `app`.

use super::App;
use super::editor::Target as EditorTarget;
use crate::ui::label;
use caper_client::attachment_edit::{Edit, Kind};
use caper_client::compress;
use caper_client::pictures::{self, Applied, Changed, Failure, Target};
use caper_client::uploads::Scratch;
use caper_client::worker::Command;
use gtk::{glib, prelude::*};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// A space icon's change in flight and its outcome, shown in Manage space.
#[derive(Default)]
pub(super) struct IconStatus {
    /// Uploading (`false`) or removing (`true`).
    pub(super) busy: Option<bool>,
    pub(super) error: Option<String>,
    /// The server stores no pictures: choosing steps back.
    pub(super) unavailable: bool,
}

impl App {
    /// The window a picture's dialogs belong to.
    fn picture_parent(&self, target: &Target) -> Option<gtk::Window> {
        match target {
            Target::Avatar => self
                .profile_dialog
                .borrow()
                .as_ref()
                .map(|dialog| dialog.window.clone()),
            Target::SpaceIcon(_) => self
                .admin_window
                .borrow()
                .as_ref()
                .map(|window| window.window.clone()),
        }
    }

    /// Shows why a picture could not be used.
    fn picture_error(self: &Rc<Self>, target: &Target, error: String) {
        match target {
            Target::Avatar => self.set_profile_picture_error(Some(error)),
            Target::SpaceIcon(_) => {
                {
                    let mut state = self.state.borrow_mut();
                    state.admin.icon.error = Some(error);
                }
                self.render_admin();
            }
        }
    }

    /// The file dialog for a new picture.
    pub(super) fn pick_picture(self: &Rc<Self>, target: Target) {
        if self.chooser.borrow().is_some() {
            return;
        }
        let parent = self.picture_parent(&target);
        #[allow(deprecated)]
        let dialog = gtk::FileChooserNative::new(
            Some(match target {
                Target::Avatar => "Choose a profile picture",
                Target::SpaceIcon(_) => "Choose a space icon",
            }),
            parent.as_ref(),
            gtk::FileChooserAction::Open,
            Some("Choose"),
            Some("Cancel"),
        );
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Pictures"));
        for extension in pictures::PICKER_EXTENSIONS {
            filter.add_suffix(extension);
        }
        #[allow(deprecated)]
        dialog.add_filter(&filter);
        let app = self.clone();
        #[allow(deprecated)]
        dialog.connect_response(move |dialog, response| {
            if response == gtk::ResponseType::Accept
                && let Some(path) = dialog.file().and_then(|file| file.path())
            {
                app.stage_picture(target.clone(), path, None);
            }
            dialog.destroy();
            app.chooser.replace(None);
        });
        #[allow(deprecated)]
        dialog.show();
        self.chooser.replace(Some(dialog));
    }

    /// A picked or downloaded picture opens in the editor on Crop; an
    /// animated one first asks whether to keep its animation.
    pub(super) fn stage_picture(
        self: &Rc<Self>,
        target: Target,
        path: PathBuf,
        scratch: Option<Arc<Scratch>>,
    ) {
        use std::io::Read;
        let head = std::fs::File::open(&path).and_then(|file| {
            if file.metadata()?.len() > pictures::MAX_SOURCE_BYTES {
                return Err(std::io::Error::other("too large"));
            }
            let mut head = Vec::new();
            file.take(64 * 1024).read_to_end(&mut head)?;
            Ok(head)
        });
        let Ok(head) = head else {
            self.picture_error(
                &target,
                "Could not read that picture, or it is too large.".into(),
            );
            return;
        };
        let gif = pictures::is_gif(&head);
        let animated = gif
            || (compress::sniff("image/png", &head) && compress::png_is_animated(&head))
            || (compress::sniff("image/webp", &head)
                && head.get(12..16) == Some(b"VP8X")
                && head.get(20).is_some_and(|flags| flags & 0x02 != 0));
        if !animated {
            self.open_picture_editor(target, path, scratch);
            return;
        }
        let Some(parent) = self.picture_parent(&target) else {
            return;
        };
        let window = gtk::Window::builder()
            .transient_for(&parent)
            .modal(true)
            .title("Animated picture")
            .resizable(false)
            .build();
        window.add_css_class("caper");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 12);
        root.add_css_class("dialog-body");
        root.set_size_request(420, -1);
        root.append(&label("Animated picture", &["dialog-title"]));
        let text = label(
            "Editing saves a still frame and removes animation. Original animation upload is available for GIFs only.",
            &["muted"],
        );
        text.set_wrap(true);
        text.set_max_width_chars(48);
        root.append(&text);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::End);
        let cancel = gtk::Button::with_label("Cancel");
        let still = gtk::Button::with_label("Edit still image");
        let keep = gtk::Button::with_label("Keep original animation");
        keep.add_css_class("primary");
        keep.set_sensitive(gif);
        if !gif {
            keep.set_tooltip_text(Some("Available for GIFs only"));
        }
        buttons.append(&cancel);
        buttons.append(&still);
        buttons.append(&keep);
        root.append(&buttons);
        window.set_child(Some(&root));
        let choice = Rc::new(RefCell::new(Some((target, path, scratch))));
        for (button, action) in [(&keep, 1), (&still, 2), (&cancel, 0)] {
            let (weak, window, choice) = (Rc::downgrade(self), window.clone(), choice.clone());
            button.connect_clicked(move |_| {
                window.destroy();
                let (Some(app), Some((target, path, scratch))) = (weak.upgrade(), choice.take())
                else {
                    return;
                };
                match action {
                    1 => app.apply_picture(target, path, None, scratch),
                    2 => app.open_picture_editor(target, path, scratch),
                    _ => {}
                }
            });
        }
        let keys = gtk::EventControllerKey::new();
        let weak_window = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape
                && let Some(window) = weak_window.upgrade()
            {
                window.destroy();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
        window.present();
        still.grab_focus();
    }

    fn open_picture_editor(
        self: &Rc<Self>,
        destination: Target,
        path: PathBuf,
        scratch: Option<Arc<Scratch>>,
    ) {
        self.open_editor(
            EditorTarget::Picture {
                destination,
                path: path.clone(),
                scratch,
            },
            Kind::Image,
            "Picture",
            path,
            None,
        );
    }

    /// "Edit picture…" or "Edit icon…": the current one opened in the
    /// editor, downloaded unless the profile draft already holds it.
    pub(super) fn edit_picture(self: &Rc<Self>, target: Target) {
        let url = match &target {
            Target::Avatar => {
                let (prepared, url) = {
                    let dialog = self.profile_dialog.borrow();
                    let Some(dialog) = dialog.as_ref() else {
                        return;
                    };
                    (
                        dialog.draft.picture.clone(),
                        dialog.draft.preview.avatar_url.clone(),
                    )
                };
                if let Some(picture) = prepared {
                    let scratch = Arc::new(Scratch::new("profile-draft"));
                    if std::fs::write(scratch.path(), &picture.bytes).is_ok() {
                        self.stage_picture(target, scratch.path().to_owned(), Some(scratch));
                    } else {
                        self.picture_error(&target, "Could not open that picture.".into());
                    }
                    return;
                }
                self.set_profile_picture_error(None);
                url
            }
            Target::SpaceIcon(id) => {
                let mut state = self.state.borrow_mut();
                state.admin.icon.error = None;
                state
                    .spaces
                    .iter()
                    .find(|space| space.id == *id)
                    .and_then(|space| space.icon_url.clone())
            }
        };
        let Some(url) = url else {
            return;
        };
        let api = self.api.clone();
        let (sender, receiver) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = api
                .fetch_media(&url)
                .map_err(|_| "Could not download that picture. Try again.".to_string())
                .and_then(|bytes| {
                    let scratch = Arc::new(Scratch::new("picture"));
                    std::fs::write(scratch.path(), bytes)
                        .map_err(|_| "Could not save that picture.".to_string())?;
                    Ok(scratch)
                });
            let _ = sender.send_blocking(result);
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let Ok(result) = receiver.recv().await else {
                return;
            };
            let Some(app) = weak.upgrade() else {
                return;
            };
            // Only while the window it was asked from is still open.
            if app.picture_parent(&target).is_none() {
                return;
            }
            match result {
                Ok(scratch) => app.stage_picture(target, scratch.path().to_owned(), Some(scratch)),
                Err(error) => app.picture_error(&target, error),
            }
        });
    }

    /// The editor's Apply crop, or Keep original animation: a profile
    /// picture becomes the draft's, a space icon uploads now.
    pub(super) fn apply_picture(
        self: &Rc<Self>,
        target: Target,
        path: PathBuf,
        edit: Option<Edit>,
        scratch: Option<Arc<Scratch>>,
    ) {
        if target == Target::Avatar {
            self.prepare_profile_picture(path, edit, scratch);
            return;
        }
        self.send_icon_change(target, Some(path), edit, scratch);
    }

    /// Manage space's Remove icon.
    pub(super) fn remove_space_icon(self: &Rc<Self>, space: String) {
        self.send_icon_change(Target::SpaceIcon(space), None, None, None);
    }

    fn send_icon_change(
        self: &Rc<Self>,
        target: Target,
        path: Option<PathBuf>,
        edit: Option<Edit>,
        scratch: Option<Arc<Scratch>>,
    ) {
        {
            let mut state = self.state.borrow_mut();
            let Some(token) = state.token.clone() else {
                return;
            };
            if state.admin.icon.busy.is_some() {
                return;
            }
            state.admin.icon.busy = Some(path.is_none());
            state.admin.icon.error = None;
            self.worker.send(Command::Picture {
                epoch: state.epoch,
                token,
                target,
                path,
                animate: crate::images::motion(),
                edit,
                scratch,
            });
        }
        self.render_admin();
    }

    /// A space icon upload or removal finished.
    pub(super) fn picture_changed(
        self: &Rc<Self>,
        target: Target,
        result: Result<Changed, Failure>,
    ) {
        {
            let mut state = self.state.borrow_mut();
            state.admin.icon.busy = None;
            match result {
                Ok(changed) => {
                    // The uploaded frames show at once, before any fetch.
                    if let Some((asset, frames)) = changed.local {
                        self.images.loaded(asset, String::new(), Some(frames));
                    }
                    match changed.applied {
                        Applied::Space(space) => {
                            if let Some(item) =
                                state.spaces.iter_mut().find(|item| item.id == space.id)
                            {
                                *item = space.clone();
                            }
                            if let Some(detail) = state
                                .detail
                                .as_mut()
                                .filter(|detail| detail.space.id == space.id)
                            {
                                detail.space = space;
                            }
                            state.layout += 1;
                        }
                        Applied::Account(_) => {}
                    }
                }
                Err(failure) => {
                    state.admin.icon.unavailable |= failure.unavailable;
                    if matches!(target, Target::SpaceIcon(_)) {
                        state.admin.icon.error = Some(failure.message);
                    }
                }
            }
        }
        self.render();
        self.render_admin();
    }
}

/// Manage space's icon: the space's tile with the edit badge, and its
/// progress or error beneath.
pub(super) struct IconRow {
    space: String,
    frame: crate::images::AvatarView,
    button: gtk::Button,
    spinner: gtk::Spinner,
    status: gtk::Label,
}

impl App {
    /// Adds the icon row for `space` to Manage space.
    pub(super) fn build_icon_row(self: &Rc<Self>, body: &gtk::Box, space: &str) -> IconRow {
        let frame = crate::images::AvatarView::new(96, false);
        frame.root.add_css_class("space-icon");
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        badge.add_css_class("picture-badge");
        badge.append(
            &self
                .images
                .icon_image(crate::images::Icon::Pencil, "#0c0d0f", 15),
        );
        badge.set_halign(gtk::Align::End);
        badge.set_valign(gtk::Align::End);
        badge.set_can_target(false);
        let face = gtk::Overlay::new();
        face.set_child(Some(&frame.root));
        face.add_overlay(&badge);
        let button: gtk::Button = crate::app::named("GtkButton", "Change space icon");
        button.set_child(Some(&face));
        button.add_css_class("picture-frame");
        button.set_tooltip_text(Some("Change space icon"));
        button.set_halign(gtk::Align::Start);
        let weak = Rc::downgrade(self);
        let id = space.to_owned();
        button.connect_clicked(move |button| {
            if let Some(app) = weak.upgrade() {
                app.icon_menu(button, &id);
            }
        });
        body.append(&button);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let spinner = gtk::Spinner::new();
        let status = label("", &["muted", "small"]);
        status.set_wrap(true);
        status.set_xalign(0.0);
        line.append(&spinner);
        line.append(&status);
        body.append(&line);
        IconRow {
            space: space.to_owned(),
            frame,
            button,
            spinner,
            status,
        }
    }

    /// With a custom icon: Edit icon…, Choose a new icon… and Remove icon.
    /// Without one, the frame picks at once.
    fn icon_menu(self: &Rc<Self>, button: &gtk::Button, space: &str) {
        let (custom, unavailable) = {
            let state = self.state.borrow();
            (
                state
                    .spaces
                    .iter()
                    .find(|item| item.id == space)
                    .is_some_and(|item| item.icon().is_some()),
                state.admin.icon.unavailable,
            )
        };
        let target = Target::SpaceIcon(space.to_owned());
        if !custom {
            if !unavailable {
                self.pick_picture(target);
            }
            return;
        }
        let Some(window) = button.root().and_downcast::<gtk::Window>() else {
            return;
        };
        let Some(bounds) = button.compute_bounds(&window) else {
            return;
        };
        let popover = gtk::Popover::new();
        popover.set_has_arrow(false);
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 2);
        menu.add_css_class("attach-menu");
        type Run = fn(&Rc<App>, String);
        let entries: [(&str, bool, Run); 3] = [
            ("Edit icon…", !unavailable, |app, id| {
                app.edit_picture(Target::SpaceIcon(id))
            }),
            ("Choose a new icon…", !unavailable, |app, id| {
                app.pick_picture(Target::SpaceIcon(id))
            }),
            ("Remove icon", true, |app, id| app.remove_space_icon(id)),
        ];
        for (name, enabled, run) in entries {
            let item = gtk::Button::with_label(name);
            item.add_css_class("flat");
            if let Some(text) = item.child().and_downcast::<gtk::Label>() {
                text.set_xalign(0.0);
            }
            item.set_sensitive(enabled);
            let (weak, popover, id) = (Rc::downgrade(self), popover.downgrade(), space.to_owned());
            item.connect_clicked(move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.popdown();
                }
                let (Some(app), id) = (weak.upgrade(), id.clone()) else {
                    return;
                };
                glib::idle_add_local_once(move || run(&app, id));
            });
            menu.append(&item);
        }
        popover.set_child(Some(&menu));
        // Parented to the dialog window, which sizes itself by its child.
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
    }

    /// The icon row shows the space's current icon and its change.
    pub(super) fn update_icon_row(&self, row: &IconRow, state: &super::State) {
        let space = state.spaces.iter().find(|item| item.id == row.space);
        let avatar = crate::images::Avatar {
            name: space.map_or_else(String::new, |space| space.name.clone()),
            mascot: None,
            picture: space.and_then(|space| {
                space.icon().map(|icon| {
                    (
                        icon.asset.to_owned(),
                        icon.url.unwrap_or_default().to_owned(),
                    )
                })
            }),
        };
        row.frame.set(&self.images, &avatar);
        let icon = &state.admin.icon;
        let custom = avatar.picture.is_some();
        row.button
            .set_sensitive(icon.busy.is_none() && (custom || !icon.unavailable));
        row.spinner.set_spinning(icon.busy.is_some());
        row.spinner.set_visible(icon.busy.is_some());
        let (text, error) = match (icon.busy, &icon.error) {
            (Some(true), _) => ("Removing…".to_owned(), false),
            (Some(false), _) => ("Uploading…".to_owned(), false),
            (None, Some(error)) => (error.clone(), true),
            (None, None) if icon.unavailable => (pictures::UNAVAILABLE.to_owned(), false),
            (None, None) => (String::new(), false),
        };
        row.status.set_label(&text);
        row.status.set_visible(!text.is_empty());
        if error {
            row.status.add_css_class("error");
        } else {
            row.status.remove_css_class("error");
        }
    }
}
