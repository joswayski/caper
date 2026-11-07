//! The media viewer: a message's images and videos full size, one at a
//! time, with previous and next, copy, save and open in the browser. As on
//! the desktop, double-click zooms to 2.5×, Ctrl+scroll zooms up to 8× about
//! the pointer, and dragging pans while zoomed; videos play with their
//! controls (`crate::video`), Space playing or pausing. Escape closes it;
//! the arrow keys step through.

use crate::images::{Icon, Images};
use crate::media::Media;
use caper_client::attachments::inline_source;
use caper_client::model::{Attachment, AttachmentKind};
use gtk::{gdk, glib, prelude::*};
use std::cell::{Cell, RefCell};
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
    scroller: gtk::ScrolledWindow,
    /// How far the picture is zoomed past its fitted size.
    scale: Cell<f64>,
    /// The pointer over the stage, which zooming keeps still.
    pointer: Cell<Option<(f64, f64)>>,
    save: gtk::Button,
    chooser: RefCell<Option<gtk::FileChooserNative>>,
    copy: gtk::Button,
    /// The stage, and the video playing on it in place of the picture.
    stage: gtk::Overlay,
    slot: gtk::Box,
    video: RefCell<Option<crate::video::Video>>,
}

/// Room left either side of a video for previous and next.
const SIDE: i32 = 64;
/// The header's height, for sizing a video before the stage is laid out.
const HEADER: i32 = 56;

const MAX_ZOOM: f64 = 8.0;
const DOUBLE_CLICK_ZOOM: f64 = 2.5;

impl Viewer {
    fn show(&self) {
        self.zoom_to(1.0);
        let index = self.index.get();
        let file = self.images.media.resolve(&self.files[index]);
        self.picture.set_paintable(gdk::Paintable::NONE);
        self.picture.remove_css_class("loaded");
        // Stepping off a video stops it.
        if let Some(video) = self.video.take() {
            self.slot.remove(&video.root);
        }
        let video = file.kind == AttachmentKind::Video;
        self.scroller.set_visible(!video);
        self.slot.set_visible(video);
        self.copy.set_visible(!video);
        if video {
            let window = self.stage.root().and_downcast::<gtk::Window>();
            let (width, height) = match (self.stage.width(), self.stage.height()) {
                (width, height) if width > 0 && height > 0 => (width, height),
                _ => window.map_or((800, 600), |window| {
                    let (width, height) = window.default_size();
                    (width, height - HEADER)
                }),
            };
            let video = crate::video::stage(
                &self.images,
                &file,
                (width - 2 * SIDE).max(160),
                (height - 16).max(120),
            );
            self.slot.append(&video.root);
            self.video.replace(Some(video));
        } else if file.content_type == "image/gif" {
            // GIFs play from their tile's frames; stills load full size.
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

    /// The picture's size fitted to the stage, before zooming.
    fn fitted(&self) -> Option<(f64, f64)> {
        let paintable = self.picture.paintable()?;
        let (width, height) = (
            f64::from(paintable.intrinsic_width()),
            f64::from(paintable.intrinsic_height()),
        );
        let (stage_width, stage_height) = (
            f64::from(self.scroller.width()),
            f64::from(self.scroller.height()),
        );
        if width <= 0.0 || height <= 0.0 || stage_width <= 0.0 || stage_height <= 0.0 {
            return None;
        }
        let fit = (stage_width / width).min(stage_height / height);
        Some((width * fit, height * fit))
    }

    /// Zooms to `scale` (1 is fitted), keeping the point under the pointer
    /// where it is.
    fn zoom_to(&self, scale: f64) {
        let scale = scale.clamp(1.0, MAX_ZOOM);
        let old = self.scale.replace(scale);
        let Some((width, height)) = self.fitted() else {
            self.scale.set(1.0);
            self.picture.set_size_request(-1, -1);
            return;
        };
        if scale <= 1.0 {
            self.picture.set_size_request(-1, -1);
        } else {
            self.picture
                .set_size_request((width * scale) as i32, (height * scale) as i32);
        }
        let (stage_width, stage_height) = (
            f64::from(self.scroller.width()),
            f64::from(self.scroller.height()),
        );
        let (px, py) = self
            .pointer
            .get()
            .unwrap_or((stage_width / 2.0, stage_height / 2.0));
        let (hadjustment, vadjustment) = (self.scroller.hadjustment(), self.scroller.vadjustment());
        // Where the pointer is on the picture, as a fraction of it.
        let fraction = |value: f64, pointer: f64, fitted: f64, stage: f64, scale: f64| {
            let shown = fitted * scale;
            let offset = ((stage - shown) / 2.0).max(0.0);
            (value + pointer - offset) / shown
        };
        let fx = fraction(hadjustment.value(), px, width, stage_width, old);
        let fy = fraction(vadjustment.value(), py, height, stage_height, old);
        // After the resize lays out (idle runs after GTK's layout).
        glib::idle_add_local_once(move || {
            let place =
                |adjustment: &gtk::Adjustment, f: f64, pointer: f64, fitted: f64, stage: f64| {
                    let shown = fitted * scale;
                    let offset = ((stage - shown) / 2.0).max(0.0);
                    adjustment.set_value(offset + f * shown - pointer);
                };
            place(&hadjustment, fx, px, width, stage_width);
            place(&vadjustment, fy, py, height, stage_height);
        });
    }

    fn save(self: &Rc<Self>, parent: &gtk::Window) {
        if self.chooser.borrow().is_some() {
            return;
        }
        let file = self.images.media.resolve(&self.files[self.index.get()]);
        let Some(url) = file.url.clone() else {
            return;
        };
        #[allow(deprecated)]
        let dialog = gtk::FileChooserNative::new(
            Some("Save"),
            Some(parent),
            gtk::FileChooserAction::Save,
            Some("Save"),
            Some("Cancel"),
        );
        #[allow(deprecated)]
        dialog.set_current_name(&file.name);
        let viewer = self.clone();
        #[allow(deprecated)]
        dialog.connect_response(move |dialog, response| {
            #[allow(deprecated)]
            let path = (response == gtk::ResponseType::Accept)
                .then(|| dialog.file().and_then(|file| file.path()))
                .flatten();
            dialog.destroy();
            viewer.chooser.replace(None);
            let Some(path) = path else {
                return;
            };
            viewer.save.set_sensitive(false);
            viewer.status.set_label("Saving…");
            let api = viewer.images.media.api().clone();
            let (url, name) = (url.clone(), file.name.clone());
            let (sender, receiver) = async_channel::bounded(1);
            std::thread::spawn(move || {
                let result = api
                    .download_media(&url, &path)
                    .map(|()| {
                        format!(
                            "Saved {}",
                            path.file_name()
                                .map_or(name, |file| file.to_string_lossy().into_owned())
                        )
                    })
                    // A refused or stalled download ends here; Save comes back.
                    .unwrap_or_else(|_| "Couldn’t save the file. Please try again.".to_owned());
                let _ = sender.send_blocking(result);
            });
            let viewer = viewer.clone();
            glib::spawn_future_local(async move {
                if let Ok(message) = receiver.recv().await {
                    viewer.status.set_label(&message);
                }
                viewer.save.set_sensitive(true);
            });
        });
        #[allow(deprecated)]
        dialog.show();
        self.chooser.replace(Some(dialog));
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
        .title("Media viewer")
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
    let save = icon_button(images, Icon::Download, "Save");
    let browser = icon_button(images, Icon::ExternalLink, "Open in browser");
    let close = icon_button(images, Icon::Close, "Close viewer");
    header.append(&name);
    header.append(&count);
    header.append(&status);
    header.append(&copy);
    header.append(&link);
    header.append(&save);
    header.append(&browser);
    header.append(&close);
    root.append(&header);
    let stage = gtk::Overlay::new();
    stage.set_vexpand(true);
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.add_css_class("viewer-picture");
    // Zoomed, the picture outgrows the stage and scrolls (without bars:
    // dragging pans).
    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::External)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&picture)
        .build();
    let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    slot.set_halign(gtk::Align::Center);
    slot.set_valign(gtk::Align::Center);
    slot.set_margin_bottom(16);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scroller.set_vexpand(true);
    slot.set_vexpand(true);
    body.append(&scroller);
    body.append(&slot);
    stage.set_child(Some(&body));
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
        scroller: scroller.clone(),
        scale: Cell::new(1.0),
        pointer: Cell::new(None),
        save: save.clone(),
        chooser: RefCell::default(),
        copy: copy.clone(),
        stage: stage.clone(),
        slot,
        video: RefCell::default(),
    });
    viewer.index.set(index.min(viewer.files.len() - 1));
    viewer.show();
    let shown = viewer.clone();
    previous.connect_clicked(move |_| shown.step(false));
    let shown = viewer.clone();
    next.connect_clicked(move |_| shown.step(true));
    let (shown, parent) = (viewer.clone(), window.downgrade());
    save.connect_clicked(move |_| {
        if let Some(parent) = parent.upgrade() {
            shown.save(&parent);
        }
    });
    let motion = gtk::EventControllerMotion::new();
    let shown = viewer.clone();
    motion.connect_motion(move |_, x, y| shown.pointer.set(Some((x, y))));
    let shown = viewer.clone();
    motion.connect_leave(move |_| shown.pointer.set(None));
    scroller.add_controller(motion);
    let clicks = gtk::GestureClick::new();
    let shown = viewer.clone();
    clicks.connect_pressed(move |_, presses, x, y| {
        if presses == 2 {
            shown.pointer.set(Some((x, y)));
            let zoomed = shown.scale.get() > 1.0;
            shown.zoom_to(if zoomed { 1.0 } else { DOUBLE_CLICK_ZOOM });
        }
    });
    scroller.add_controller(clicks);
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    let shown = viewer.clone();
    scroll.connect_scroll(move |controller, _, dy| {
        if !controller
            .current_event_state()
            .contains(gdk::ModifierType::CONTROL_MASK)
        {
            return glib::Propagation::Proceed;
        }
        shown.zoom_to(shown.scale.get() * 1.2_f64.powf(-dy));
        glib::Propagation::Stop
    });
    scroller.add_controller(scroll);
    let drag = gtk::GestureDrag::new();
    let start: Rc<Cell<(f64, f64)>> = Rc::default();
    let (shown, from) = (viewer.clone(), start.clone());
    drag.connect_drag_begin(move |_, _, _| {
        from.set((
            shown.scroller.hadjustment().value(),
            shown.scroller.vadjustment().value(),
        ));
    });
    let shown = viewer.clone();
    drag.connect_drag_update(move |_, dx, dy| {
        if shown.scale.get() > 1.0 {
            let (x, y) = start.get();
            shown.scroller.hadjustment().set_value(x - dx);
            shown.scroller.vadjustment().set_value(y - dy);
        }
    });
    scroller.add_controller(drag);
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
        gdk::Key::space => match shown.video.borrow().as_ref() {
            Some(video) => {
                video.toggle();
                glib::Propagation::Stop
            }
            None => glib::Propagation::Proceed,
        },
        _ => glib::Propagation::Proceed,
    });
    window.add_controller(keys);
    window.present();
}
