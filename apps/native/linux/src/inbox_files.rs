//! Inbox attachment drafts use the shared profile/inbox uploader, not the
//! currently selected channel. Every picker, preview and edit stays with its row.

use super::{App, State, editor};
use crate::images::Icon;
use crate::ui::label;
use caper_client::{Rgba, activity::Item, attachment_edit::Edit, profile_dm};
use gtk::{gdk, gio, glib, prelude::*};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, atomic::Ordering};

pub(super) struct File {
    pub key: u64,
    pub file: profile_dm::File,
    pub preview: Option<Rgba>,
}

/// Cancel both an already completed reservation and a completion racing removal.
/// The shared upload cancels earlier failures itself; this also covers a result
/// published just after its final cancellation check.
pub(super) fn abandon(file: &profile_dm::File, api: &caper_client::api::Api, token: &str) {
    let upload = file.upload.clone();
    if let Some(upload) = &upload {
        upload.cancel.store(true, Ordering::Relaxed);
    }
    let (api, token, attached) = (api.clone(), token.to_owned(), file.uploaded_id());
    std::thread::spawn(move || {
        if let Some(id) = &attached {
            api.cancel_asset(&token, id);
        }
        if let Some(upload) = upload {
            while upload.result.get().is_none() {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if let Some(Ok((_, asset, _))) = upload.result.get()
                && attached.as_ref() != Some(&asset.id)
            {
                api.cancel_asset(&token, &asset.id);
            }
        }
    });
}

impl State {
    pub(super) fn inbox_editable(&self, item: &Item) -> bool {
        self.inbox_writable(item)
            && self
                .replies
                .get(&item.draft_key())
                .is_none_or(|reply| reply.editable())
    }
}

impl App {
    pub(super) fn inbox_file_chips(
        self: &Rc<Self>,
        item: &Item,
        files: &[File],
        editable: bool,
    ) -> gtk::Box {
        let chips = gtk::Box::new(gtk::Orientation::Vertical, 6);
        for staged in files {
            let file = &staged.file;
            let chip = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            chip.add_css_class("chip");
            if let Some(texture) = staged.preview.as_ref().and_then(crate::media::texture) {
                let image = gtk::Image::from_paintable(Some(&texture));
                image.set_pixel_size(48);
                chip.append(&image);
            } else {
                chip.append(&self.images.icon_image(Icon::FileText, "#b9bcbe", 24));
            }
            let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
            text.set_hexpand(true);
            let name = label(&file.name(), &["file-name"]);
            name.set_xalign(0.0);
            name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
            name.set_max_width_chars(24);
            text.append(&name);
            let (status, failed) = file.upload_status();
            let size = caper_client::uploads::format_bytes(file.size);
            let status = label(
                if status.is_empty() { &size } else { status },
                &[if failed { "error" } else { "muted" }, "small"],
            );
            status.set_xalign(0.0);
            status.set_wrap(true);
            text.append(&status);
            chip.append(&text);
            let button = |icon, name: &str, action| {
                let button: gtk::Button = super::named("GtkButton", name);
                button.set_tooltip_text(Some(name));
                button.set_child(Some(&self.images.icon_image(icon, "#b9bcbe", 16)));
                button.add_css_class("flat");
                button.set_sensitive(editable);
                let (weak, target, key) = (Rc::downgrade(self), item.clone(), staged.key);
                button.connect_clicked(move |_| {
                    if let Some(app) = weak.upgrade() {
                        let target = target.clone();
                        glib::idle_add_local_once(move || {
                            app.inbox_file_action(&target, key, action)
                        });
                    }
                });
                button
            };
            if file.editable.is_some() {
                chip.append(&button(
                    Icon::Pencil,
                    &format!("Edit {}", file.name()),
                    Action::Edit,
                ));
                chip.append(&button(
                    if file.spoiler {
                        Icon::EyeOff
                    } else {
                        Icon::Eye
                    },
                    if file.spoiler {
                        "Remove spoiler"
                    } else {
                        "Mark as spoiler"
                    },
                    Action::Spoiler,
                ));
            }
            if failed {
                chip.append(&button(Icon::Retry, "Retry upload", Action::Retry));
            }
            chip.append(&button(
                Icon::Close,
                &format!("Remove {}", file.name()),
                Action::Remove,
            ));
            chips.append(&chip);
        }
        chips
    }

    pub(super) fn pick_inbox_files(self: &Rc<Self>, item: &Item) {
        let open = self.inbox_window.borrow();
        let Some(open) = open.as_ref() else { return };
        if open.chooser.borrow().is_some() {
            return;
        }
        let epoch = self.state.borrow().epoch;
        #[allow(deprecated)]
        let dialog = gtk::FileChooserNative::new(
            Some("Attach files"),
            Some(&open.window),
            gtk::FileChooserAction::Open,
            Some("Attach"),
            Some("Cancel"),
        );
        #[allow(deprecated)]
        dialog.set_select_multiple(true);
        let (weak, target, parent) = (Rc::downgrade(self), item.clone(), open.window.downgrade());
        #[allow(deprecated)]
        dialog.connect_response(move |dialog, response| {
            if let Some(app) = weak.upgrade() {
                if response == gtk::ResponseType::Accept
                    && parent.upgrade().is_some_and(|window| window.is_visible())
                {
                    let files = dialog.files();
                    let paths = (0..files.n_items())
                        .filter_map(|index| files.item(index).and_downcast::<gio::File>())
                        .filter_map(|file| file.path())
                        .map(|path| (path, None))
                        .collect();
                    app.add_inbox_files(&target, epoch, paths);
                }
                if let Some(open) = app.inbox_window.borrow().as_ref()
                    && parent.upgrade().as_ref() == Some(&open.window)
                {
                    open.chooser.replace(None);
                }
            }
            dialog.destroy();
        });
        #[allow(deprecated)]
        dialog.show();
        open.chooser.replace(Some(dialog));
    }

    pub(super) fn paste_inbox_files(self: &Rc<Self>, item: &Item) -> bool {
        let (epoch, allowed) = {
            let state = self.state.borrow();
            (
                state.epoch,
                state.inbox_editable(item) && state.uploads.is_some(),
            )
        };
        if !allowed {
            return false;
        }
        let Some(parent) = self
            .inbox_window
            .borrow()
            .as_ref()
            .map(|open| open.window.downgrade())
        else {
            return false;
        };
        let clipboard = self.ui.window.clipboard();
        let formats = clipboard.formats();
        let (weak, target) = (Rc::downgrade(self), item.clone());
        if formats.contains_type(gdk::FileList::static_type()) {
            clipboard.read_value_async(
                gdk::FileList::static_type(),
                glib::Priority::DEFAULT,
                gio::Cancellable::NONE,
                move |value| {
                    if let Some(app) = weak.upgrade()
                        && parent.upgrade().is_some_and(|window| window.is_visible())
                    {
                        let paths = value
                            .ok()
                            .and_then(|value| value.get::<gdk::FileList>().ok())
                            .map(|list| {
                                list.files()
                                    .iter()
                                    .filter_map(|file| file.path())
                                    .map(|path| (path, None))
                                    .collect()
                            })
                            .unwrap_or_default();
                        app.add_inbox_files(&target, epoch, paths);
                    }
                },
            );
            return true;
        }
        if formats.contains_type(gdk::Texture::static_type())
            && !formats.contain_mime_type("text/plain")
        {
            clipboard.read_texture_async(gio::Cancellable::NONE, move |texture| {
                if let (Some(app), Ok(Some(texture))) = (weak.upgrade(), texture)
                    && parent.upgrade().is_some_and(|window| window.is_visible())
                {
                    let scratch = Arc::new(caper_client::uploads::Scratch::new("png"));
                    if texture.save_to_png(scratch.path()).is_ok() {
                        app.add_inbox_files(
                            &target,
                            epoch,
                            vec![(scratch.path().to_owned(), Some(scratch))],
                        );
                    }
                }
            });
            return true;
        }
        false
    }

    fn add_inbox_files(
        self: &Rc<Self>,
        item: &Item,
        epoch: u64,
        paths: Vec<(PathBuf, Option<Arc<caper_client::uploads::Scratch>>)>,
    ) {
        let mut state = self.state.borrow_mut();
        if state.epoch != epoch
            || !state.inbox_editable(item)
            || self.inbox_window.borrow().is_none()
        {
            return;
        }
        let (Some(token), Some(settings)) = (state.token.clone(), state.uploads.clone()) else {
            return;
        };
        let key = item.draft_key();
        let count = state.replies.get(&key).map_or(0, |reply| reply.files.len());
        let (room, error) = crate::uploads::room(count, paths.len());
        state.replies.entry(key.clone()).or_default().error = error;
        for (path, scratch) in paths.into_iter().take(room) {
            state.file_keys += 1;
            let id = state.file_keys;
            let mut file = profile_dm::File::staged(path, scratch);
            file.upload_eagerly(
                &self.api,
                &token,
                (&key, &item.actor_name, Some(item.clone())),
                &settings,
                &self.wakes,
            );
            self.inbox_preview(item, epoch, id, &file);
            state
                .replies
                .entry(key.clone())
                .or_default()
                .files
                .push(File {
                    key: id,
                    file,
                    preview: None,
                });
        }
        drop(state);
        self.render_inbox();
    }

    fn inbox_preview(self: &Rc<Self>, item: &Item, epoch: u64, key: u64, file: &profile_dm::File) {
        let Some(kind) = file.editable else { return };
        let (path, scratch) = (file.path.clone(), file.scratch.clone());
        let (sender, receiver) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _scratch = scratch;
            let image = caper_client::attachment_edit::load_still(&path, kind).map(|still| {
                Rgba::from_image(
                    image::DynamicImage::ImageRgba8(still.image)
                        .thumbnail(96, 96)
                        .to_rgba8(),
                )
            });
            let _ = sender.send_blocking(image);
        });
        let (weak, draft) = (Rc::downgrade(self), item.draft_key());
        glib::spawn_future_local(async move {
            let image = receiver.recv().await.ok().flatten();
            let Some(app) = weak.upgrade() else { return };
            let mut state = app.state.borrow_mut();
            if state.epoch == epoch
                && let Some(file) = state
                    .replies
                    .get_mut(&draft)
                    .and_then(|reply| reply.files.iter_mut().find(|file| file.key == key))
            {
                file.preview = image;
            }
            drop(state);
            app.render_inbox();
        });
    }

    pub(super) fn inbox_file_action(self: &Rc<Self>, item: &Item, key: u64, action: Action) {
        let mut state = self.state.borrow_mut();
        if !state.inbox_editable(item) {
            return;
        }
        let (Some(token), Some(settings)) = (state.token.clone(), state.uploads.clone()) else {
            return;
        };
        let draft = item.draft_key();
        let Some(reply) = state.replies.get_mut(&draft) else {
            return;
        };
        let Some(index) = reply.files.iter().position(|file| file.key == key) else {
            return;
        };
        let staged = &mut reply.files[index];
        match action {
            Action::Edit => {
                let Some(kind) = staged.file.editable else {
                    return;
                };
                let (name, path, edit) = (
                    staged.file.name(),
                    staged.file.path.clone(),
                    staged.file.edit.clone(),
                );
                drop(state);
                self.open_editor(
                    editor::Target::Inbox {
                        item: Box::new(item.clone()),
                        key,
                    },
                    kind,
                    &name,
                    path,
                    edit,
                );
                return;
            }
            Action::Remove => {
                abandon(&reply.files.remove(index).file, &self.api, &token);
                reply.error = None;
            }
            Action::Spoiler => staged.file.spoiler = !staged.file.spoiler,
            Action::Retry => {
                if !staged.file.upload_status().1 {
                    return;
                }
                staged.file.upload = None;
                staged.file.upload_eagerly(
                    &self.api,
                    &token,
                    (&draft, &item.actor_name, Some(item.clone())),
                    &settings,
                    &self.wakes,
                );
            }
        }
        drop(state);
        self.render_inbox();
    }

    pub(super) fn save_inbox_edit(
        self: &Rc<Self>,
        item: &Item,
        key: u64,
        edit: Option<Edit>,
        preview: Option<Rgba>,
    ) {
        let mut state = self.state.borrow_mut();
        if !state.inbox_editable(item) {
            return;
        }
        let (Some(token), Some(settings)) = (state.token.clone(), state.uploads.clone()) else {
            return;
        };
        state.file_keys += 1;
        let next = state.file_keys;
        let draft = item.draft_key();
        let Some(staged) = state
            .replies
            .get_mut(&draft)
            .and_then(|reply| reply.files.iter_mut().find(|file| file.key == key))
        else {
            return;
        };
        if staged.file.edit == edit {
            return;
        }
        abandon(&staged.file, &self.api, &token);
        staged.key = next;
        staged.file.set_edit(edit);
        staged.preview = preview;
        staged.file.upload_eagerly(
            &self.api,
            &token,
            (&draft, &item.actor_name, Some(item.clone())),
            &settings,
            &self.wakes,
        );
        drop(state);
        self.render_inbox();
    }
}

#[derive(Clone, Copy)]
pub(super) enum Action {
    Edit,
    Remove,
    Retry,
    Spoiler,
}
