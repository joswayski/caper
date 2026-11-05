//! The image viewer: a message's images full size, one at a time, with
//! previous and next, copy and open in the browser. Escape closes it; the
//! arrow keys step through.

use crate::images::{Icon, Images};
use crate::media::Media;
use caper_client::attachments::inline_source;
use caper_client::model::Attachment;
use gtk::{gdk, glib, prelude::*};
use std::cell::Cell;
use std::rc::Rc;

struct Viewer {
    files: Vec<Attachment>,
    index: Cell<usize>,
    picture: gtk::Picture,
    name: gtk::Label,
    count: gtk::Label,
    previous: gtk::Button,
    next: gtk::Button,
    status: gtk::Label,
    images: Rc<Images>,
}

impl Viewer {
    fn show(&self) {
        let index = self.index.get();
        let file = self.images.media.resolve(&self.files[index]);
        self.picture.set_paintable(gdk::Paintable::NONE);
        self.picture.remove_css_class("loaded");
        // GIFs play from their tile's frames; stills load full size.
        if file.content_type == "image/gif" {
            if let Some(url) = inline_source(&file) {
                self.images.media.bind(&self.picture, &file.id, url);
            }
        } else if let Some(url) = &file.url {
            self.images
                .media
                .bind(&self.picture, &Media::full_key(&file.id), url);
        }
        self.name.set_label(&file.name);
        let many = self.files.len() > 1;
        self.count
            .set_label(&format!("{} of {}", index + 1, self.files.len()));
        self.count.set_visible(many);
        self.previous.set_visible(many);
        self.next.set_visible(many);
        self.previous.set_sensitive(index > 0);
        self.next.set_sensitive(index + 1 < self.files.len());
        self.status.set_label("");
    }

    fn step(&self, forward: bool) {
        let index = self.index.get();
        let next = if forward {
            (index + 1).min(self.files.len() - 1)
        } else {
            index.saturating_sub(1)
        };
        if next != index {
            self.index.set(next);
            self.show();
        }
    }

    fn url(&self) -> Option<String> {
        self.images.media.resolve(&self.files[self.index.get()]).url
    }
}

fn icon_button(images: &Images, icon: Icon, name: &str) -> gtk::Button {
    let button: gtk::Button = crate::app::named("GtkButton", name);
    button.set_child(Some(&images.icon_image(icon, "#f3f4f5", 18)));
    button.set_tooltip_text(Some(name));
    button.add_css_class("flat");
    button.set_valign(gtk::Align::Center);
    button
}

pub fn open(
    parent: &gtk::ApplicationWindow,
    images: &Rc<Images>,
    files: Vec<Attachment>,
    index: usize,
) {
    if files.is_empty() {
        return;
    }
    let window = gtk::Window::builder()
        .transient_for(parent)
        .modal(true)
        .title("Image viewer")
        .default_width(parent.width().max(800) - 80)
        .default_height(parent.height().max(600) - 80)
        .build();
    window.add_css_class("caper");
    window.add_css_class("viewer");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    header.add_css_class("header");
    let name = crate::ui::label("", &["author"]);
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    name.set_hexpand(true);
    let count = crate::ui::label("", &["muted"]);
    let status = crate::ui::label("", &["muted", "small"]);
    let copy = icon_button(images, Icon::Copy, "Copy image");
    let link = icon_button(images, Icon::Link, "Copy link");
    let browser = icon_button(images, Icon::ExternalLink, "Open in browser");
    let close = icon_button(images, Icon::Close, "Close viewer");
    header.append(&name);
    header.append(&count);
    header.append(&status);
    header.append(&copy);
    header.append(&link);
    header.append(&browser);
    header.append(&close);
    root.append(&header);
    let stage = gtk::Overlay::new();
    stage.set_vexpand(true);
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.add_css_class("viewer-picture");
    stage.set_child(Some(&picture));
    let previous = icon_button(images, Icon::ChevronLeft, "Previous image");
    previous.set_halign(gtk::Align::Start);
    previous.add_css_class("step");
    let next = icon_button(images, Icon::ChevronRight, "Next image");
    next.set_halign(gtk::Align::End);
    next.add_css_class("step");
    stage.add_overlay(&previous);
    stage.add_overlay(&next);
    root.append(&stage);
    window.set_child(Some(&root));

    let viewer = Rc::new(Viewer {
        files,
        index: Cell::new(index),
        picture,
        name,
        count,
        previous: previous.clone(),
        next: next.clone(),
        status,
        images: images.clone(),
    });
    viewer.index.set(index.min(viewer.files.len() - 1));
    viewer.show();
    let shown = viewer.clone();
    previous.connect_clicked(move |_| shown.step(false));
    let shown = viewer.clone();
    next.connect_clicked(move |_| shown.step(true));
    let shown = viewer.clone();
    browser.connect_clicked(move |_| {
        let file = shown.files[shown.index.get()].clone();
        shown.images.media.open(&file);
    });
    let shown = viewer.clone();
    link.connect_clicked(move |button| {
        if let Some(url) = shown.url() {
            button.clipboard().set_text(&url);
            shown.status.set_label("Link copied");
        }
    });
    let shown = viewer.clone();
    copy.connect_clicked(move |button| {
        match shown
            .picture
            .paintable()
            .and_then(|paintable| paintable.downcast::<gdk::Texture>().ok())
        {
            Some(texture) => {
                button.clipboard().set_texture(&texture);
                shown.status.set_label("Image copied");
            }
            None => shown.status.set_label("Couldn’t copy image yet"),
        }
    });
    let closing = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(window) = closing.upgrade() {
            window.close();
        }
    });
    let keys = gtk::EventControllerKey::new();
    let (shown, closing) = (viewer.clone(), window.downgrade());
    keys.connect_key_pressed(move |_, key, _, _| match key {
        gdk::Key::Escape => {
            if let Some(window) = closing.upgrade() {
                window.close();
            }
            glib::Propagation::Stop
        }
        gdk::Key::Left => {
            shown.step(false);
            glib::Propagation::Stop
        }
        gdk::Key::Right => {
            shown.step(true);
            glib::Propagation::Stop
        }
        _ => glib::Propagation::Proceed,
    });
    window.add_controller(keys);
    window.present();
}
