//! The attachment editor, as on the egui desktop (`attachment_editor.rs`) and
//! web's `AttachmentEditor.tsx`: crop and turn a picture, draw on it and add
//! text. It edits a description of the changes (`attachment_edit::Edit`),
//! never the file. A composer's file opens viewing, as it will be sent, with
//! Spoiler and Remove beside the tools; a profile picture opens on Crop, held
//! square, with Apply crop.
//!
//! The committed marks are painted by `attachment_edit::render` on a
//! screen-sized copy, the export's own geometry; strokes being drawn, the crop
//! frame and moving text are drawn over it with cairo. A child of `app`.

use super::{App, clear};
use crate::images::Icon;
use crate::ui::label;
use caper_client::attachment_edit::{
    self as markup, COLOR_NAMES, COLORS, Crop, Edit, Handle, Kind, MAX_TEXT, Mark, STROKE_SIZES,
    TEXT_SIZES,
};
use caper_client::uploads::Scratch;
use gtk::{cairo, gdk, glib, prelude::*};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

/// Space around the picture: the crop frame's handles need it.
const PADDING: f64 = 28.0;
/// The editor draws at most this large; exports use the original.
const SCREEN_EDGE: u32 = 1600;
const MIN_CROP_POINTS: f64 = 32.0;
const HANDLE_HIT: f64 = 14.0;
/// Freehand samples closer than this add nothing.
const SAMPLE_POINTS: f64 = 1.5;
/// A press on text that moves less than this edits it rather than moving it.
const MOVE_POINTS: f64 = 4.0;
const MIN_BRUSH: f64 = 6.0;
pub(super) const CLOSE: &str = "Close editor";
const OPEN_SENT: &str = "Open in default app";
const EDIT_PENDING: &str = "Available when your edit is ready";
const VIDEO_HINT: &str = "Drawings and text stay on screen for the whole video.";
const VIDEO_CROP_HINT: &str = "Crop and rotation apply to the whole video.";
const VIDEO_MARKS_UNAVAILABLE: &str =
    "Drawing on videos needs a newer FFmpeg than this build includes.";

/// What is being edited.
#[derive(Clone)]
pub(super) enum Target {
    /// A new profile picture or space icon.
    Picture {
        destination: caper_client::pictures::Target,
        path: PathBuf,
        scratch: Option<Arc<Scratch>>,
    },
    /// A composer's file, by its upload key.
    Draft(u64),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Crop,
    Draw,
    Text,
}

/// What the editor asked for when it closed or changed its file.
pub(super) enum Outcome {
    Cancel,
    /// A change; `None` returns a draft to its original.
    Save(Option<Edit>),
    Spoiler,
    Remove,
}

enum Gesture {
    Draw {
        mark: Mark,
        last: (f64, f64),
    },
    Erase {
        erased: Vec<usize>,
        last: (f64, f64),
    },
    Crop {
        handle: Handle,
        start: Crop,
    },
    Text {
        index: Option<usize>,
        from: (f64, f64),
        origin: (f64, f64),
        moved: bool,
    },
}

/// A text field open over the picture: a new mark, or `index` being changed.
struct Field {
    index: Option<usize>,
    x: f64,
    y: f64,
    color: [u8; 3],
    size: f64,
    popover: gtk::Popover,
    entry: gtk::Entry,
}

/// Where the picture is on screen and which part of it shows.
#[derive(Clone, Copy)]
struct Layout {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    region: Crop,
    /// The full rotated picture's longer edge in points.
    edge: f64,
}

impl Layout {
    fn fraction(&self, (px, py): (f64, f64)) -> (f64, f64) {
        (
            self.region.x + (px - self.x) / self.width * self.region.width,
            self.region.y + (py - self.y) / self.height * self.region.height,
        )
    }

    fn screen(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.x + (x - self.region.x) / self.region.width * self.width,
            self.y + (y - self.region.y) / self.region.height * self.height,
        )
    }

    /// Points per fraction of the full picture, across and down.
    fn full(&self) -> (f64, f64) {
        (
            self.width / self.region.width,
            self.height / self.region.height,
        )
    }

    fn crop_rect(&self, crop: Crop) -> (f64, f64, f64, f64) {
        let (x, y) = self.screen(crop.x, crop.y);
        let (right, bottom) = self.screen(crop.x + crop.width, crop.y + crop.height);
        (x, y, right - x, bottom - y)
    }

    fn contains(&self, (x, y): (f64, f64)) -> bool {
        x >= self.x && y >= self.y && x <= self.x + self.width && y <= self.y + self.height
    }
}

struct Session {
    target: Target,
    kind: Kind,
    path: PathBuf,
    /// The upright still at screen size, and the original's size.
    still: Option<(image::RgbaImage, u32, u32)>,
    loading: bool,
    failed: bool,
    marks_allowed: bool,
    initial: Edit,
    states: Vec<Edit>,
    index: usize,
    tool: Tool,
    viewing: bool,
    color: [u8; 3],
    stroke_points: f64,
    text_points: f64,
    erasing: bool,
    gesture: Option<Gesture>,
    live_crop: Option<Crop>,
    live_text: Option<(usize, f64, f64)>,
    field: Option<Field>,
    hover: Option<(f64, f64)>,
    /// The rendered rotation and marks, with what they painted.
    rendered: Option<(u16, Vec<Mark>, cairo::ImageSurface)>,
    /// A spoiler's cover: the rendered picture blurred, by what it shows.
    cover: Option<(u16, Vec<Mark>, cairo::ImageSurface)>,
    spoiler: Option<bool>,
}

impl Session {
    fn picture(&self) -> bool {
        matches!(self.target, Target::Picture { .. })
    }

    fn current(&self) -> &Edit {
        &self.states[self.index]
    }

    fn square(&self, mut edit: Edit) -> Edit {
        if self.picture()
            && let Some((_, width, height)) = self.still
        {
            let (width, height) = markup::rotated_size(width, height, edit.rotation);
            edit.crop = caper_client::pictures::square_frame(edit.crop, width, height);
        }
        edit
    }

    /// Adds a change as one undo step, dropping anything undone.
    fn commit(&mut self, next: Edit) {
        let next = self.square(next);
        if &next == self.current() {
            return;
        }
        self.states.truncate(self.index + 1);
        self.states.push(next);
        self.index += 1;
    }

    fn with_field(&self) -> Edit {
        let edit = self.current().clone();
        let Some(field) = &self.field else {
            return edit;
        };
        let text: String = field.entry.text().trim().chars().take(MAX_TEXT).collect();
        let mut marks = edit.marks.clone();
        match field.index {
            Some(index) => {
                if text.is_empty() {
                    if index < marks.len() {
                        marks.remove(index);
                    }
                } else if let Some(Mark::Text {
                    color,
                    size,
                    text: old,
                    ..
                }) = marks.get_mut(index)
                {
                    (*color, *size, *old) = (field.color, field.size, text);
                }
            }
            None if !text.is_empty() => marks.push(Mark::Text {
                color: field.color,
                size: field.size,
                x: field.x,
                y: field.y,
                text,
            }),
            None => {}
        }
        Edit { marks, ..edit }
    }

    fn commit_field(&mut self) {
        let next = self.with_field();
        if let Some(field) = self.field.take() {
            field.popover.popdown();
            field.popover.unparent();
        }
        self.commit(next);
    }

    fn drop_field(&mut self) {
        if let Some(field) = self.field.take() {
            field.popover.popdown();
            field.popover.unparent();
        }
    }

    fn undo(&mut self) {
        self.drop_field();
        self.index = self.index.saturating_sub(1);
    }

    fn redo(&mut self) {
        self.drop_field();
        self.index = (self.index + 1).min(self.states.len() - 1);
    }

    fn choose(&mut self, tool: Option<Tool>) {
        self.commit_field();
        self.gesture = None;
        self.live_crop = None;
        self.live_text = None;
        self.erasing = false;
        self.viewing = tool.is_none();
        if let Some(tool) = tool {
            self.tool = tool;
        }
    }

    /// What the committed picture shows: the edit without the open field's
    /// mark or the lines being erased.
    fn shown(&self) -> Edit {
        let mut shown = self.current().clone();
        let field = self.field.as_ref().and_then(|field| field.index);
        let erased = match &self.gesture {
            Some(Gesture::Erase { erased, .. }) => erased.as_slice(),
            _ => &[],
        };
        let moving = self.live_text.map(|(index, ..)| index);
        let mut index = 0;
        shown.marks.retain(|_| {
            let keep = Some(index) != field && Some(index) != moving && !erased.contains(&index);
            index += 1;
            keep
        });
        shown
    }

    fn layout(&self, width: f64, height: f64) -> Option<Layout> {
        let (_, original_width, original_height) = self.still.as_ref()?;
        let (rw, rh) =
            markup::rotated_size(*original_width, *original_height, self.current().rotation);
        let (rw, rh) = (f64::from(rw), f64::from(rh));
        let region = if self.tool == Tool::Crop && !self.viewing {
            Crop::FULL
        } else {
            self.current().crop
        };
        let (w, h) = (region.width * rw, region.height * rh);
        let scale = ((width - PADDING * 2.0) / w)
            .min((height - PADDING * 2.0) / h)
            .max(0.0001);
        Some(Layout {
            x: (width - w * scale) / 2.0,
            y: (height - h * scale) / 2.0,
            width: w * scale,
            height: h * scale,
            region,
            edge: scale * rw.max(rh),
        })
    }

    /// The topmost text mark under a screen position.
    fn text_at(&self, layout: &Layout, at: (f64, f64)) -> Option<usize> {
        self.current()
            .marks
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, mark)| {
                let Mark::Text {
                    size, x, y, text, ..
                } = mark
                else {
                    return None;
                };
                let pixels = size * layout.edge;
                let width = markup::text_metrics(text, pixels as f32)
                    .map_or(0.0, |metrics| f64::from(metrics.width));
                let (cx, cy) = layout.screen(*x, *y);
                ((at.0 - cx).abs() <= width / 2.0 + 8.0 && (at.1 - cy).abs() <= pixels * 0.6 + 8.0)
                    .then_some(index)
            })
    }

    fn touched_along(
        &self,
        layout: &Layout,
        from: (f64, f64),
        to: (f64, f64),
        erased: &[usize],
    ) -> Vec<usize> {
        let radius = (self.stroke_points).max(MIN_BRUSH) / 2.0;
        let distance = ((to.0 - from.0).powi(2) + (to.1 - from.1).powi(2)).sqrt();
        let steps = (distance / (radius / 2.0).max(1.0)).ceil().max(1.0) as usize;
        let full = layout.full();
        let mut erased = erased.to_vec();
        for step in 1..=steps {
            let t = step as f64 / steps as f64;
            let at = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
            for index in
                markup::strokes_touched(&self.current().marks, layout.fraction(at), radius, full)
            {
                if !erased.contains(&index) {
                    erased.push(index);
                }
            }
        }
        erased
    }
}

/// Premultiplied ARGB32, as cairo paints it, from straight RGBA.
fn surface(image: &image::RgbaImage) -> Option<cairo::ImageSurface> {
    let (width, height) = (image.width() as i32, image.height() as i32);
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height).ok()?;
    let stride = surface.stride() as usize;
    {
        let mut data = surface.data().ok()?;
        for (y, row) in image.rows().enumerate() {
            for (x, pixel) in row.enumerate() {
                let alpha = u32::from(pixel[3]);
                let at = y * stride + x * 4;
                let premultiply = |channel: u8| ((u32::from(channel) * alpha + 127) / 255) as u8;
                let value = u32::from(pixel[3]) << 24
                    | u32::from(premultiply(pixel[0])) << 16
                    | u32::from(premultiply(pixel[1])) << 8
                    | u32::from(premultiply(pixel[2]));
                data[at..at + 4].copy_from_slice(&value.to_ne_bytes());
            }
        }
    }
    Some(surface)
}

fn rgb(cr: &cairo::Context, [red, green, blue]: [u8; 3], alpha: f64) {
    cr.set_source_rgba(
        f64::from(red) / 255.0,
        f64::from(green) / 255.0,
        f64::from(blue) / 255.0,
        alpha,
    );
}

fn icon_button(app: &App, icon: Icon, name: &str, tooltip: &str) -> gtk::Button {
    let button: gtk::Button = crate::app::named("GtkButton", name);
    let image = app.images.icon_image(icon, "#f3f4f5", 18);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    button.set_child(Some(&image));
    button.set_tooltip_text(Some(tooltip));
    button.add_css_class("flat");
    button.add_css_class("editor-icon");
    button
}

/// The open editor's widgets.
pub(super) struct EditorWindow {
    window: gtk::Window,
    session: Rc<RefCell<Session>>,
    area: gtk::DrawingArea,
    undo: gtk::Button,
    redo: gtk::Button,
    reset: gtk::Button,
    done: gtk::Button,
    open: Option<gtk::Button>,
    hint: gtk::Label,
    options: gtk::Box,
    tools: gtk::Box,
    /// What the options and tools last showed.
    shown: RefCell<String>,
}

impl App {
    /// Opens the editor on `path`; a composer's file starts viewing, a
    /// profile picture on Crop.
    pub(super) fn open_editor(
        self: &Rc<Self>,
        target: Target,
        kind: Kind,
        name: &str,
        path: PathBuf,
        edit: Option<Edit>,
    ) {
        self.close_editor();
        let picture = matches!(target, Target::Picture { .. });
        let initial = edit.unwrap_or_default();
        let session = Rc::new(RefCell::new(Session {
            target,
            kind,
            path: path.clone(),
            still: None,
            loading: true,
            failed: false,
            marks_allowed: kind == Kind::Image,
            states: vec![initial.clone()],
            initial,
            index: 0,
            tool: Tool::Crop,
            viewing: !picture,
            color: COLORS[0],
            stroke_points: f64::from(STROKE_SIZES.2),
            text_points: f64::from(TEXT_SIZES.2),
            erasing: false,
            gesture: None,
            live_crop: None,
            live_text: None,
            field: None,
            hover: None,
            rendered: None,
            cover: None,
            spoiler: None,
        }));
        let parent = self.ui.window.clone();
        let window = gtk::Window::builder()
            .transient_for(&parent)
            .modal(true)
            .title(name)
            .default_width(parent.width().max(720))
            .default_height(parent.height().max(560))
            .build();
        window.add_css_class("caper");
        window.add_css_class("editor");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);

        // Top bar: Close, the name, Open in default app, Undo, Redo, Reset
        // and Done.
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        top.add_css_class("editor-top");
        let close = icon_button(self, Icon::Close, CLOSE, "Close");
        top.append(&close);
        let title = label(name, &["strong"]);
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        top.append(&title);
        let open = (!picture).then(|| {
            let open = icon_button(self, Icon::ExternalLink, OPEN_SENT, OPEN_SENT);
            top.append(&open);
            open
        });
        let undo = icon_button(self, Icon::Undo, "Undo", "Undo (Ctrl+Z)");
        let redo = icon_button(self, Icon::Redo, "Redo", "Redo (Ctrl+Shift+Z)");
        let reset = gtk::Button::with_label("Reset");
        reset.add_css_class("flat");
        let done = gtk::Button::with_label(if picture { "Apply crop" } else { "Done" });
        done.add_css_class("primary");
        for button in [&undo, &redo] {
            top.append(button);
        }
        top.append(&reset);
        top.append(&done);
        root.append(&top);

        let area = gtk::DrawingArea::new();
        area.set_vexpand(true);
        area.set_hexpand(true);
        area.set_focusable(true);
        area.set_can_focus(true);
        root.append(&area);
        let hint = label("", &["muted", "small"]);
        hint.set_visible(false);
        root.append(&hint);
        let options = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        options.add_css_class("editor-options");
        options.set_halign(gtk::Align::Center);
        root.append(&options);
        let tools = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        tools.add_css_class("editor-tools");
        tools.set_halign(gtk::Align::Center);
        root.append(&tools);
        window.set_child(Some(&root));

        let draw_session = session.clone();
        area.set_draw_func(move |_, cr, width, height| {
            if let Ok(mut session) = draw_session.try_borrow_mut() {
                paint(&mut session, cr, f64::from(width), f64::from(height));
            }
        });

        self.connect_editor_input(&window, &area, &session);
        let weak = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.finish_editor(Outcome::Cancel);
            }
        });
        let weak = Rc::downgrade(self);
        done.connect_clicked(move |_| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let outcome = {
                let Some(editor) = app.editor.borrow().as_ref().map(|e| e.session.clone()) else {
                    return;
                };
                let session = editor.borrow();
                let fin = session.with_field();
                let picture = session.picture();
                if fin == session.initial && !picture {
                    Outcome::Cancel
                } else {
                    Outcome::Save((fin.is_edited() || picture).then_some(fin))
                }
            };
            app.finish_editor(outcome);
        });
        for (button, step) in [(&undo, 0), (&redo, 1), (&reset, 2)] {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.editor_step(step);
                }
            });
        }
        if let Some(open) = &open {
            let weak = Rc::downgrade(self);
            open.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.open_sent();
                }
            });
        }
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(app) = weak.upgrade() {
                glib::idle_add_local_once(move || app.finish_editor(Outcome::Cancel));
            }
            glib::Propagation::Stop
        });

        *self.editor.borrow_mut() = Some(EditorWindow {
            window: window.clone(),
            session: session.clone(),
            area: area.clone(),
            undo,
            redo,
            reset,
            done,
            open,
            hint,
            options,
            tools,
            shown: RefCell::default(),
        });

        // Decoded off the main thread.
        let (sender, receiver) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let _ = sender.send_blocking(markup::load_still(&path, kind));
        });
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let still = receiver.recv().await.ok().flatten();
            let Some(app) = weak.upgrade() else {
                return;
            };
            {
                let mut session = session.borrow_mut();
                session.loading = false;
                match still {
                    Some(still) => {
                        session.marks_allowed = still.marks;
                        let image = if still.image.width().max(still.image.height()) > SCREEN_EDGE {
                            image::DynamicImage::ImageRgba8(still.image)
                                .thumbnail(SCREEN_EDGE, SCREEN_EDGE)
                                .to_rgba8()
                        } else {
                            still.image
                        };
                        session.still = Some((image, still.width, still.height));
                        if session.picture() {
                            let initial = session.square(Edit::default());
                            session.states = vec![initial.clone()];
                            session.initial = initial;
                        }
                    }
                    None => session.failed = true,
                }
            }
            app.render_editor();
        });
        self.render_editor();
        window.present();
        area.grab_focus();
    }

    fn connect_editor_input(
        self: &Rc<Self>,
        window: &gtk::Window,
        area: &gtk::DrawingArea,
        session: &Rc<RefCell<Session>>,
    ) {
        let drag = gtk::GestureDrag::new();
        drag.set_button(gtk::gdk::BUTTON_PRIMARY);
        let (weak, held) = (Rc::downgrade(self), session.clone());
        drag.connect_drag_begin(move |gesture, x, y| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Some(area) = gesture.widget() else {
                return;
            };
            let started = {
                let mut session = held.borrow_mut();
                let Some(layout) =
                    session.layout(f64::from(area.width()), f64::from(area.height()))
                else {
                    return;
                };
                begin(&mut session, &layout, (x, y))
            };
            if !started {
                gesture.set_state(gtk::EventSequenceState::Denied);
            }
            area.queue_draw();
            app.render_editor_controls();
        });
        let held = session.clone();
        drag.connect_drag_update(move |gesture, dx, dy| {
            let Some((x, y)) = gesture.start_point() else {
                return;
            };
            let Some(area) = gesture.widget() else {
                return;
            };
            let mut session = held.borrow_mut();
            if let Some(layout) = session.layout(f64::from(area.width()), f64::from(area.height()))
            {
                update(&mut session, &layout, (x, y), (x + dx, y + dy));
            }
            area.queue_draw();
        });
        let (weak, held) = (Rc::downgrade(self), session.clone());
        drag.connect_drag_end(move |gesture, dx, dy| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let Some((x, y)) = gesture.start_point() else {
                return;
            };
            let Some(area) = gesture.widget() else {
                return;
            };
            let open_text = {
                let mut session = held.borrow_mut();
                let Some(layout) =
                    session.layout(f64::from(area.width()), f64::from(area.height()))
                else {
                    return;
                };
                update(&mut session, &layout, (x, y), (x + dx, y + dy));
                end(&mut session, &layout)
            };
            if let Some((index, at)) = open_text {
                app.open_text_field(index, at);
            }
            area.queue_draw();
            app.render_editor_controls();
        });
        area.add_controller(drag);

        let motion = gtk::EventControllerMotion::new();
        let held = session.clone();
        motion.connect_motion(move |controller, x, y| {
            if let Ok(mut session) = held.try_borrow_mut() {
                session.hover = Some((x, y));
            }
            if let Some(widget) = controller.widget() {
                widget.queue_draw();
            }
        });
        let held = session.clone();
        motion.connect_leave(move |controller| {
            if let Ok(mut session) = held.try_borrow_mut() {
                session.hover = None;
            }
            if let Some(widget) = controller.widget() {
                widget.queue_draw();
            }
        });
        area.add_controller(motion);

        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(app) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
            let shift = modifiers.contains(gdk::ModifierType::SHIFT_MASK);
            match key {
                gdk::Key::Escape => {
                    let field = app
                        .editor
                        .borrow()
                        .as_ref()
                        .is_some_and(|editor| editor.session.borrow().field.is_some());
                    if field {
                        if let Some(editor) = app.editor.borrow().as_ref() {
                            editor.session.borrow_mut().drop_field();
                            editor.area.queue_draw();
                        }
                    } else {
                        app.finish_editor(Outcome::Cancel);
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::z | gdk::Key::Z if control => {
                    app.editor_step(if shift { 1 } else { 0 });
                    glib::Propagation::Stop
                }
                gdk::Key::y if control => {
                    app.editor_step(1);
                    glib::Propagation::Stop
                }
                gdk::Key::Left | gdk::Key::Right | gdk::Key::Up | gdk::Key::Down => {
                    let step = 0.01;
                    let (dx, dy) = match key {
                        gdk::Key::Left => (-step, 0.0),
                        gdk::Key::Right => (step, 0.0),
                        gdk::Key::Up => (0.0, -step),
                        _ => (0.0, step),
                    };
                    let moved = app.editor.borrow().as_ref().is_some_and(|editor| {
                        let mut session = editor.session.borrow_mut();
                        if session.viewing || session.tool != Tool::Crop || session.still.is_none()
                        {
                            return false;
                        }
                        let handle = if shift { Handle::SE } else { Handle::Move };
                        let drag = if session.picture() {
                            markup::drag_square_crop
                        } else {
                            markup::drag_crop
                        };
                        let crop = drag(session.current().crop, handle, dx, dy, (0.02, 0.02));
                        let next = Edit {
                            crop,
                            ..session.current().clone()
                        };
                        session.commit(next);
                        editor.area.queue_draw();
                        true
                    });
                    if moved {
                        app.render_editor_controls();
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                _ => glib::Propagation::Proceed,
            }
        });
        window.add_controller(keys);
    }

    /// Undo (0), Redo (1) or Reset (2).
    fn editor_step(self: &Rc<Self>, step: u8) {
        if let Some(editor) = self.editor.borrow().as_ref() {
            let mut session = editor.session.borrow_mut();
            if session.still.is_none() || session.gesture.is_some() {
                return;
            }
            match step {
                0 => session.undo(),
                1 => session.redo(),
                _ => {
                    session.drop_field();
                    session.commit(Edit::default());
                }
            }
            editor.area.queue_draw();
        }
        self.render_editor_controls();
    }

    /// A text field over the picture at `at` (fractions), for a new mark or
    /// `index`.
    fn open_text_field(self: &Rc<Self>, index: Option<usize>, at: (f64, f64)) {
        let open = self.editor.borrow();
        let Some(editor) = open.as_ref() else {
            return;
        };
        let mut session = editor.session.borrow_mut();
        let Some(layout) = session.layout(
            f64::from(editor.area.width()),
            f64::from(editor.area.height()),
        ) else {
            return;
        };
        let (mut color, mut size, mut text) = (
            session.color,
            session.text_points / layout.edge,
            String::new(),
        );
        let (mut x, mut y) = at;
        if let Some(Mark::Text {
            color: c,
            size: s,
            x: mx,
            y: my,
            text: t,
        }) = index
            .and_then(|index| session.current().marks.get(index))
            .cloned()
        {
            (color, size, text, x, y) = (c, s, t, mx, my);
            session.color = c;
            session.text_points = (s * layout.edge)
                .round()
                .clamp(f64::from(TEXT_SIZES.0), f64::from(TEXT_SIZES.1));
        }
        let entry = crate::app::named::<gtk::Entry>(
            "GtkEntry",
            if index.is_none() {
                "New text"
            } else {
                "Edit text"
            },
        );
        entry.set_text(&text);
        entry.set_max_length(MAX_TEXT as i32);
        entry.set_width_chars(16);
        let popover = gtk::Popover::new();
        popover.set_autohide(false);
        popover.set_has_arrow(false);
        popover.add_css_class("editor-text");
        popover.set_child(Some(&entry));
        popover.set_parent(&editor.area);
        let (sx, sy) = layout.screen(x, y);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(sx as i32, sy as i32, 1, 1)));
        popover.set_position(gtk::PositionType::Bottom);
        let weak = Rc::downgrade(self);
        entry.connect_activate(move |_| {
            if let Some(app) = weak.upgrade() {
                glib::idle_add_local_once(move || {
                    if let Some(editor) = app.editor.borrow().as_ref() {
                        editor.session.borrow_mut().commit_field();
                        editor.area.queue_draw();
                    }
                    app.render_editor_controls();
                });
            }
        });
        session.field = Some(Field {
            index,
            x,
            y,
            color,
            size,
            popover: popover.clone(),
            entry: entry.clone(),
        });
        drop(session);
        popover.popup();
        entry.grab_focus();
        editor.area.queue_draw();
        drop(open);
        self.render_editor_controls();
    }

    /// Opens what will be sent in its default app.
    fn open_sent(&self) {
        let editor = self.editor.borrow();
        let Some(editor) = editor.as_ref() else {
            return;
        };
        let session = editor.session.borrow();
        let Target::Draft(key) = session.target else {
            return;
        };
        let state = self.state.borrow();
        let Some(file) = state.files.iter().find(|file| file.key == key) else {
            return;
        };
        let path = match (&file.edit, &file.prepared) {
            (None, _) => session.path.clone(),
            (Some(_), Some(prepared)) => match prepared.on_disk() {
                Ok(scratch) => {
                    let path = scratch.path().to_owned();
                    // Kept until the app quits: the other app may still read it.
                    self.opened.borrow_mut().push(scratch);
                    path
                }
                Err(_) => return,
            },
            (Some(_), None) => return,
        };
        let uri = gtk::gio::File::for_path(&path).uri();
        if let Err(error) =
            gtk::gio::AppInfo::launch_default_for_uri(&uri, None::<&gtk::gio::AppLaunchContext>)
        {
            eprintln!("Couldn’t open the file: {error}");
        }
    }

    pub(super) fn close_editor(&self) {
        if let Some(editor) = self.editor.borrow_mut().take() {
            editor.session.borrow_mut().drop_field();
            editor.window.destroy();
        }
    }

    /// The editor's Done, Close, Spoiler or Remove.
    fn finish_editor(self: &Rc<Self>, outcome: Outcome) {
        let Some((target, preview)) = self.editor.borrow().as_ref().map(|editor| {
            let session = editor.session.borrow();
            let preview = match &outcome {
                Outcome::Save(edit) => session.still.as_ref().and_then(|(image, ..)| {
                    let shown = image::DynamicImage::ImageRgba8(image.clone());
                    let rendered = match edit {
                        Some(edit) => markup::render(&shown, edit)?,
                        None => shown,
                    };
                    Some(caper_client::Rgba::from_image(
                        rendered.thumbnail(96, 96).to_rgba8(),
                    ))
                }),
                _ => None,
            };
            (session.target.clone(), preview)
        }) else {
            return;
        };
        let keep_open = matches!(outcome, Outcome::Spoiler);
        if !keep_open {
            self.close_editor();
        }
        match (target, outcome) {
            (_, Outcome::Cancel) => {}
            (
                Target::Picture {
                    destination,
                    path,
                    scratch,
                },
                Outcome::Save(edit),
            ) => self.apply_picture(destination, path, edit, scratch),
            (Target::Draft(key), Outcome::Save(edit)) => self.save_draft_edit(key, edit, preview),
            (Target::Draft(key), Outcome::Spoiler) => {
                let spoiler = self
                    .state
                    .borrow()
                    .files
                    .iter()
                    .find(|file| file.key == key)
                    .map(|file| !file.spoiler);
                if let Some(spoiler) = spoiler {
                    self.chip_action(crate::uploads::ChipAction::Spoiler(key, spoiler));
                }
                self.render_editor();
            }
            (Target::Draft(key), Outcome::Remove) => {
                self.chip_action(crate::uploads::ChipAction::Remove(key));
            }
            (Target::Picture { .. }, _) => {}
        }
    }

    /// A composer file's edit changed: it prepares and uploads again, and
    /// the upload it replaces is cancelled.
    fn save_draft_edit(
        self: &Rc<Self>,
        key: u64,
        edit: Option<Edit>,
        preview: Option<caper_client::Rgba>,
    ) {
        {
            let mut state = self.state.borrow_mut();
            let (Some(token), Some(settings)) = (state.token.clone(), state.uploads.clone()) else {
                return;
            };
            state.file_keys += 1;
            let next = state.file_keys;
            let Some(file) = state.files.iter_mut().find(|file| file.key == key) else {
                return;
            };
            if file.edit == edit || file.local_path.is_none() {
                return;
            }
            file.stop();
            let old = file.attachment.take();
            file.key = next;
            file.cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
            file.edit = edit;
            file.name = file.source_name.clone();
            file.prepared = None;
            file.thumbnail = preview;
            file.stored_size = None;
            file.compressing = None;
            file.progress = 0.0;
            file.error = None;
            file.uploaded_at = None;
            file.upload_expired = false;
            file.uploading = true;
            self.worker.send(file.command(token.clone(), settings));
            if let Some(old) = old {
                self.worker
                    .send(caper_client::worker::Command::CancelAssets {
                        token,
                        ids: vec![old.id],
                    });
            }
            state.files_revision += 1;
        }
        self.render();
    }

    /// Opens a composer file in the editor, viewing it as it will be sent.
    pub(super) fn view_draft(self: &Rc<Self>, key: u64) {
        let opened = {
            let state = self.state.borrow();
            state
                .files
                .iter()
                .find(|file| file.key == key)
                .and_then(|file| {
                    Some((
                        file.editable?,
                        file.source_name.clone(),
                        file.local_path.clone()?,
                        file.edit.clone(),
                    ))
                })
        };
        if let Some((kind, name, path, edit)) = opened {
            self.open_editor(Target::Draft(key), kind, &name, path, edit);
        }
    }

    /// Redraws the editor and its controls.
    pub(super) fn render_editor(self: &Rc<Self>) {
        if let Some(editor) = self.editor.borrow().as_ref() {
            editor.area.queue_draw();
        }
        self.render_editor_controls();
    }

    /// Undo, Redo, Reset, Done, the hint, the chosen tool's options and the
    /// tool row, rebuilt when what they show changes.
    fn render_editor_controls(self: &Rc<Self>) {
        let editor = self.editor.borrow();
        let Some(editor) = editor.as_ref() else {
            return;
        };
        let Ok(mut session) = editor.session.try_borrow_mut() else {
            return;
        };
        let ready = session.still.is_some();
        let spoiler = match session.target {
            Target::Draft(key) => self.state.try_borrow().ok().and_then(|state| {
                state
                    .files
                    .iter()
                    .find(|file| file.key == key)
                    .map(|f| f.spoiler)
            }),
            Target::Picture { .. } => None,
        };
        session.spoiler = spoiler;
        editor.undo.set_sensitive(ready && session.index > 0);
        editor
            .redo
            .set_sensitive(ready && session.index + 1 < session.states.len());
        editor
            .reset
            .set_sensitive(ready && session.current().is_edited());
        editor.done.set_sensitive(ready || !session.picture());
        if let (Some(open), Target::Draft(key)) = (&editor.open, &session.target) {
            let pending = self.state.try_borrow().ok().and_then(|state| {
                state
                    .files
                    .iter()
                    .find(|file| file.key == *key)
                    .map(|file| file.edit.is_some() && file.prepared.is_none())
            });
            let pending = pending.unwrap_or(true);
            open.set_sensitive(!pending);
            open.set_tooltip_text(Some(if pending { EDIT_PENDING } else { OPEN_SENT }));
        }
        let hint = (session.kind == Kind::Video && ready).then_some(if session.marks_allowed {
            VIDEO_HINT
        } else {
            VIDEO_CROP_HINT
        });
        editor.hint.set_label(hint.unwrap_or_default());
        editor.hint.set_visible(hint.is_some());
        let key = format!(
            "{ready}|{:?}|{}|{}|{:?}|{}|{}|{}|{spoiler:?}|{}",
            session.tool,
            session.viewing,
            session.erasing,
            session.color,
            session.stroke_points,
            session.text_points,
            session.marks_allowed,
            session.field.is_some()
        );
        if *editor.shown.borrow() == key {
            return;
        }
        *editor.shown.borrow_mut() = key;
        let (tool, viewing, erasing, color, marks, picture) = (
            session.tool,
            session.viewing,
            session.erasing,
            session.color,
            session.marks_allowed,
            session.picture(),
        );
        let (stroke, text) = (session.stroke_points, session.text_points);
        drop(session);

        // The chosen tool's options.
        clear(&editor.options);
        if ready && !viewing {
            self.editor_options(&editor.options, tool, erasing, color, stroke, text);
        }

        // The tool row: Spoiler, Crop, Draw, Text, Remove.
        clear(&editor.tools);
        let toggle = |icon: Icon, name: &str, selected: bool, enabled: bool| {
            let button: gtk::Button = crate::app::named("GtkButton", name);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            line.append(&self.images.icon_image(
                icon,
                if selected { "#f3f4f5" } else { "#b9bcbe" },
                16,
            ));
            line.append(&label(name, &[]));
            button.set_child(Some(&line));
            button.add_css_class("flat");
            button.add_css_class("editor-tool");
            if selected {
                button.add_css_class("selected");
            }
            button.set_sensitive(enabled);
            button
        };
        if let Some(marked) = spoiler {
            let button = toggle(Icon::EyeOff, "Spoiler", marked, true);
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    glib::idle_add_local_once(move || app.finish_editor(Outcome::Spoiler));
                }
            });
            editor.tools.append(&button);
            editor
                .tools
                .append(&gtk::Separator::new(gtk::Orientation::Vertical));
        }
        for (choice, icon, name) in [
            (Tool::Crop, Icon::Crop, "Crop"),
            (Tool::Draw, Icon::Brush, "Draw"),
            (Tool::Text, Icon::Type, "Text"),
        ] {
            let enabled = ready && (choice == Tool::Crop || marks);
            let selected = !viewing && tool == choice;
            let button = toggle(icon, name, selected, enabled);
            if ready && !enabled {
                button.set_tooltip_text(Some(VIDEO_MARKS_UNAVAILABLE));
            }
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                glib::idle_add_local_once(move || {
                    if let Some(editor) = app.editor.borrow().as_ref() {
                        let mut session = editor.session.borrow_mut();
                        if !selected {
                            session.choose(Some(choice));
                        } else if !picture {
                            // A profile picture stays on Crop.
                            session.choose(None);
                        }
                        editor.area.queue_draw();
                    }
                    app.render_editor_controls();
                });
            });
            editor.tools.append(&button);
        }
        if spoiler.is_some() {
            editor
                .tools
                .append(&gtk::Separator::new(gtk::Orientation::Vertical));
            let button = toggle(Icon::Trash, "Remove", false, true);
            button.add_css_class("danger-tool");
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    glib::idle_add_local_once(move || app.finish_editor(Outcome::Remove));
                }
            });
            editor.tools.append(&button);
        }
    }

    fn editor_options(
        self: &Rc<Self>,
        options: &gtk::Box,
        tool: Tool,
        erasing: bool,
        color: [u8; 3],
        stroke: f64,
        text: f64,
    ) {
        if tool == Tool::Crop {
            let rotate: gtk::Button = crate::app::named("GtkButton", "Rotate left");
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            line.append(&self.images.icon_image(Icon::RotateCcw, "#f3f4f5", 16));
            line.append(&label("Rotate left", &[]));
            rotate.set_child(Some(&line));
            let weak = Rc::downgrade(self);
            rotate.connect_clicked(move |_| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                if let Some(editor) = app.editor.borrow().as_ref() {
                    let mut session = editor.session.borrow_mut();
                    let next = session.current().rotate_left();
                    session.commit(next);
                    editor.area.queue_draw();
                }
                app.render_editor_controls();
            });
            options.append(&rotate);
            return;
        }
        let draw = tool == Tool::Draw;
        let set_color = |app: &Rc<App>, choice: [u8; 3]| {
            if let Some(editor) = app.editor.borrow().as_ref() {
                let mut session = editor.session.borrow_mut();
                session.color = choice;
                session.erasing = false;
                if let Some(field) = &mut session.field {
                    field.color = choice;
                    field.entry.grab_focus();
                }
                editor.area.queue_draw();
            }
        };
        for (swatch, name) in COLORS.into_iter().zip(COLOR_NAMES) {
            let button: gtk::Button = crate::app::named("GtkButton", name);
            button.set_tooltip_text(Some(name));
            button.add_css_class("swatch");
            button.set_valign(gtk::Align::Center);
            if !erasing && swatch == color {
                button.add_css_class("selected");
            }
            let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            dot.add_css_class("swatch-dot");
            dot.set_size_request(18, 18);
            dot.set_halign(gtk::Align::Center);
            dot.set_valign(gtk::Align::Center);
            let provider = gtk::CssProvider::new();
            provider.load_from_data(&format!(
                "box {{ background: #{:02x}{:02x}{:02x}; }}",
                swatch[0], swatch[1], swatch[2]
            ));
            #[allow(deprecated)]
            dot.style_context()
                .add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            button.set_child(Some(&dot));
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    set_color(&app, swatch);
                    app.render_editor_controls();
                }
            });
            options.append(&button);
        }
        // Any other colour.
        let custom = crate::app::named::<gtk::ColorButton>("GtkColorButton", "Custom colour");
        #[allow(deprecated)]
        custom.set_rgba(&gdk::RGBA::new(
            f32::from(color[0]) / 255.0,
            f32::from(color[1]) / 255.0,
            f32::from(color[2]) / 255.0,
            1.0,
        ));
        custom.set_tooltip_text(Some("Custom colour"));
        let weak = Rc::downgrade(self);
        #[allow(deprecated)]
        custom.connect_color_set(move |button| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let rgba = button.rgba();
            let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
            set_color(
                &app,
                [
                    channel(rgba.red()),
                    channel(rgba.green()),
                    channel(rgba.blue()),
                ],
            );
            app.render_editor_controls();
        });
        options.append(&custom);
        if draw {
            let eraser: gtk::ToggleButton = crate::app::named("GtkToggleButton", "Eraser");
            let image = self.images.icon_image(Icon::Eraser, "#f3f4f5", 16);
            eraser.set_child(Some(&image));
            eraser.set_tooltip_text(Some("Eraser"));
            eraser.add_css_class("flat");
            eraser.set_active(erasing);
            let weak = Rc::downgrade(self);
            eraser.connect_toggled(move |toggle| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                if let Some(editor) = app.editor.borrow().as_ref() {
                    editor.session.borrow_mut().erasing = toggle.is_active();
                }
                glib::idle_add_local_once(move || app.render_editor_controls());
            });
            options.append(&eraser);
        }
        options.append(&label("Size", &["muted", "small"]));
        let (min, max) = if draw {
            (STROKE_SIZES.0, STROKE_SIZES.1)
        } else {
            (TEXT_SIZES.0, TEXT_SIZES.1)
        };
        let size = crate::app::named::<gtk::Scale>(
            "GtkScale",
            match (draw, erasing) {
                (false, _) => "Text size",
                (true, true) => "Eraser size",
                (true, false) => "Line size",
            },
        );
        size.set_orientation(gtk::Orientation::Horizontal);
        size.set_adjustment(&gtk::Adjustment::new(
            if draw { stroke } else { text },
            f64::from(min),
            f64::from(max),
            1.0,
            1.0,
            0.0,
        ));
        size.set_digits(0);
        size.set_draw_value(true);
        size.set_value_pos(gtk::PositionType::Right);
        size.set_size_request(220, -1);
        let weak = Rc::downgrade(self);
        size.connect_value_changed(move |scale| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            let value = scale.value().round();
            if let Some(editor) = app.editor.borrow().as_ref() {
                let mut session = editor.session.borrow_mut();
                let edge = session
                    .layout(
                        f64::from(editor.area.width()),
                        f64::from(editor.area.height()),
                    )
                    .map_or(1.0, |layout| layout.edge);
                if draw {
                    session.stroke_points = value;
                } else {
                    session.text_points = value;
                    if let Some(field) = &mut session.field {
                        field.size = value / edge;
                    }
                }
                editor.area.queue_draw();
            }
        });
        options.append(&size);
    }
}

/// A press on the picture starts a gesture; false declines it.
fn begin(session: &mut Session, layout: &Layout, at: (f64, f64)) -> bool {
    if session.viewing || session.still.is_none() {
        return false;
    }
    if session.field.is_some() {
        // A press outside the open field commits it, like a blur.
        session.commit_field();
        return false;
    }
    match session.tool {
        Tool::Crop => {
            let crop = session.current().crop;
            let (x, y, w, h) = layout.crop_rect(crop);
            let handle = Handle::EDGES
                .into_iter()
                .find(|handle| {
                    let (ax, ay) = handle.anchor();
                    let (hx, hy) = (x + w * f64::from(ax), y + h * f64::from(ay));
                    (at.0 - hx).abs() <= HANDLE_HIT && (at.1 - hy).abs() <= HANDLE_HIT
                })
                .or_else(|| {
                    (at.0 >= x && at.0 <= x + w && at.1 >= y && at.1 <= y + h)
                        .then_some(Handle::Move)
                });
            let Some(handle) = handle else {
                return false;
            };
            session.gesture = Some(Gesture::Crop {
                handle,
                start: crop,
            });
        }
        Tool::Draw => {
            if !layout.contains(at) {
                return false;
            }
            if session.erasing {
                let erased = session.touched_along(layout, at, at, &[]);
                session.gesture = Some(Gesture::Erase { erased, last: at });
            } else {
                let (x, y) = layout.fraction(at);
                session.gesture = Some(Gesture::Draw {
                    mark: Mark::Stroke {
                        color: session.color,
                        width: session.stroke_points / layout.edge,
                        points: vec![x, y],
                    },
                    last: at,
                });
            }
        }
        Tool::Text => {
            if !layout.contains(at) {
                return false;
            }
            let index = session.text_at(layout, at);
            let from = layout.fraction(at);
            let origin = match index.and_then(|index| session.current().marks.get(index)) {
                Some(Mark::Text { x, y, .. }) => (*x, *y),
                _ => from,
            };
            session.gesture = Some(Gesture::Text {
                index,
                from,
                origin,
                moved: false,
            });
        }
    }
    true
}

fn update(session: &mut Session, layout: &Layout, start: (f64, f64), at: (f64, f64)) {
    let picture = session.picture();
    match &mut session.gesture {
        Some(Gesture::Draw {
            mark: Mark::Stroke { points, .. },
            last,
        }) => {
            if ((at.0 - last.0).powi(2) + (at.1 - last.1).powi(2)).sqrt() >= SAMPLE_POINTS {
                let (x, y) = layout.fraction(at);
                points.extend([x, y]);
                *last = at;
            }
        }
        Some(Gesture::Erase { erased, last }) => {
            let (from, previous) = (*last, erased.clone());
            *last = at;
            let next = session.touched_along(layout, from, at, &previous);
            if let Some(Gesture::Erase { erased, .. }) = &mut session.gesture {
                *erased = next;
            }
        }
        Some(Gesture::Crop {
            handle,
            start: crop,
        }) => {
            let (fx, fy) = layout.full();
            let min = (
                (MIN_CROP_POINTS / fx).clamp(0.02, 1.0),
                (MIN_CROP_POINTS / fy).clamp(0.02, 1.0),
            );
            let drag = if picture {
                markup::drag_square_crop
            } else {
                markup::drag_crop
            };
            session.live_crop = Some(drag(
                *crop,
                *handle,
                (at.0 - start.0) / fx,
                (at.1 - start.1) / fy,
                min,
            ));
        }
        Some(Gesture::Text {
            index: Some(index),
            from,
            origin,
            moved,
        }) => {
            *moved |= ((at.0 - start.0).powi(2) + (at.1 - start.1).powi(2)).sqrt() > MOVE_POINTS;
            if *moved {
                let (x, y) = layout.fraction(at);
                session.live_text = Some((
                    *index,
                    (origin.0 + x - from.0).clamp(0.0, 1.0),
                    (origin.1 + y - from.1).clamp(0.0, 1.0),
                ));
            }
        }
        _ => {}
    }
}

/// Ends a gesture, committing it. Returns a text field to open.
fn end(session: &mut Session, _layout: &Layout) -> Option<(Option<usize>, (f64, f64))> {
    match session.gesture.take()? {
        Gesture::Draw { mark, .. } => {
            let mut next = session.current().clone();
            next.marks.push(mark);
            session.commit(next);
        }
        Gesture::Erase { erased, .. } => {
            if !erased.is_empty() {
                let mut next = session.current().clone();
                let mut index = 0;
                next.marks.retain(|_| {
                    let keep = !erased.contains(&index);
                    index += 1;
                    keep
                });
                session.commit(next);
            }
        }
        Gesture::Crop { .. } => {
            if let Some(crop) = session.live_crop.take() {
                let next = Edit {
                    crop: markup::clamp_crop(crop, 0.02),
                    ..session.current().clone()
                };
                session.commit(next);
            }
        }
        Gesture::Text {
            index, from, moved, ..
        } => {
            let live = session.live_text.take();
            match index {
                None => return Some((None, from)),
                Some(_) if moved => {
                    if let Some((index, x, y)) = live {
                        let mut next = session.current().clone();
                        if let Some(Mark::Text {
                            x: at_x, y: at_y, ..
                        }) = next.marks.get_mut(index)
                        {
                            (*at_x, *at_y) = (x, y);
                        }
                        session.commit(next);
                    }
                }
                Some(index) => return Some((Some(index), from)),
            }
        }
    }
    None
}

fn paint(session: &mut Session, cr: &cairo::Context, width: f64, height: f64) {
    cr.set_source_rgb(0.047, 0.051, 0.059);
    let _ = cr.paint();
    let Some(layout) = session.layout(width, height) else {
        let text = if session.failed {
            "This file can’t be edited here."
        } else if session.loading {
            "Loading…"
        } else {
            ""
        };
        cr.set_source_rgb(0.725, 0.737, 0.745);
        cr.select_font_face(
            "Satoshi",
            cairo::FontSlant::Normal,
            cairo::FontWeight::Normal,
        );
        cr.set_font_size(14.0);
        if let Ok(extents) = cr.text_extents(text) {
            cr.move_to((width - extents.width()) / 2.0, height / 2.0);
            let _ = cr.show_text(text);
        }
        return;
    };
    // The committed edit, rendered whole and turned.
    let shown = session.shown();
    let fresh = |cached: &Option<(u16, Vec<Mark>, cairo::ImageSurface)>| {
        cached.as_ref().is_some_and(|(rotation, marks, _)| {
            *rotation == shown.rotation && *marks == shown.marks
        })
    };
    let covered = session.viewing && session.spoiler == Some(true);
    if (!fresh(&session.rendered) || (covered && !fresh(&session.cover)))
        && let Some((image, ..)) = &session.still
    {
        let whole = Edit {
            crop: Crop::FULL,
            ..shown.clone()
        };
        let rendered = markup::render(&image::DynamicImage::ImageRgba8(image.clone()), &whole)
            .map(|rendered| rendered.to_rgba8());
        let key = (shown.rotation, shown.marks.clone());
        if covered && let Some(rendered) = &rendered {
            // Blurred as a spoilered attachment's cover is.
            let blurred =
                caper_client::media::blur(&caper_client::Rgba::from_image(rendered.clone()));
            session.cover = image::RgbaImage::from_raw(
                blurred.size[0] as u32,
                blurred.size[1] as u32,
                blurred.pixels,
            )
            .and_then(|cover| surface(&cover))
            .map(|cover| (key.0, key.1.clone(), cover));
        }
        session.rendered = rendered
            .and_then(|rendered| surface(&rendered))
            .map(|surface| (key.0, key.1, surface));
    }
    let (fx, fy) = layout.full();
    let origin = (
        layout.x - layout.region.x * fx,
        layout.y - layout.region.y * fy,
    );
    // While viewing, a spoiler shows covered, as it will be sent.
    let picture = if covered {
        session.cover.as_ref()
    } else {
        session.rendered.as_ref()
    };
    cr.save().ok();
    cr.rectangle(layout.x, layout.y, layout.width, layout.height);
    cr.clip();
    if let Some((_, _, picture)) = picture {
        cr.translate(origin.0, origin.1);
        cr.scale(
            fx / f64::from(picture.width()),
            fy / f64::from(picture.height()),
        );
        let _ = cr.set_source_surface(picture, 0.0, 0.0);
        cr.source().set_filter(cairo::Filter::Good);
        let _ = cr.paint();
    }
    cr.restore().ok();
    if covered {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
        cr.rectangle(layout.x, layout.y, layout.width, layout.height);
        let _ = cr.fill();
    }
    if session.viewing {
        return;
    }
    // A line being drawn.
    if let Some(Gesture::Draw {
        mark: Mark::Stroke {
            color,
            width,
            points,
        },
        ..
    }) = &session.gesture
    {
        rgb(cr, *color, 1.0);
        cr.set_line_width(width * layout.edge);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        for (index, pair) in points.chunks(2).enumerate() {
            if let [x, y] = pair {
                let (sx, sy) = layout.screen(*x, *y);
                if index == 0 {
                    cr.move_to(sx, sy);
                }
                cr.line_to(sx, sy);
            }
        }
        let _ = cr.stroke();
    }
    // Text being moved: its outline where it will go.
    if let Some((index, x, y)) = session.live_text
        && let Some(Mark::Text { size, text, .. }) = session.current().marks.get(index)
    {
        let pixels = size * layout.edge;
        let w = markup::text_metrics(text, pixels as f32)
            .map_or(pixels, |metrics| f64::from(metrics.width));
        let (sx, sy) = layout.screen(x, y);
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.8);
        cr.set_line_width(1.0);
        cr.set_dash(&[4.0, 3.0], 0.0);
        cr.rectangle(sx - w / 2.0 - 4.0, sy - pixels * 0.6, w + 8.0, pixels * 1.2);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
    }
    match session.tool {
        Tool::Crop => {
            let crop = session.live_crop.unwrap_or(session.current().crop);
            let (x, y, w, h) = layout.crop_rect(crop);
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.55);
            cr.rectangle(layout.x, layout.y, layout.width, layout.height);
            cr.rectangle(x, y, w, h);
            cr.set_fill_rule(cairo::FillRule::EvenOdd);
            let _ = cr.fill();
            cr.set_fill_rule(cairo::FillRule::Winding);
            cr.set_source_rgba(1.0, 1.0, 1.0, 0.9);
            cr.set_line_width(1.5);
            cr.rectangle(x, y, w, h);
            let _ = cr.stroke();
            cr.set_source_rgb(0.953, 0.957, 0.961);
            cr.set_line_width(3.0);
            cr.set_line_cap(cairo::LineCap::Round);
            let length = 18.0;
            for handle in Handle::EDGES {
                let (ax, ay) = handle.anchor();
                let (ax, ay) = (f64::from(ax), f64::from(ay));
                let (hx, hy) = (x + w * ax, y + h * ay);
                if ax != 0.5 && ay != 0.5 {
                    let (ix, iy) = (1.0 - ax * 2.0, 1.0 - ay * 2.0);
                    cr.move_to(hx + ix * length, hy);
                    cr.line_to(hx, hy);
                    cr.line_to(hx, hy + iy * length);
                } else if ax == 0.5 {
                    cr.move_to(hx - length / 2.0, hy);
                    cr.line_to(hx + length / 2.0, hy);
                } else {
                    cr.move_to(hx, hy - length / 2.0);
                    cr.line_to(hx, hy + length / 2.0);
                }
                let _ = cr.stroke();
            }
        }
        Tool::Draw => {
            // The brush: a ring as wide as a line drawn now.
            if let Some(at) = session.hover.filter(|at| layout.contains(*at)) {
                let radius = session.stroke_points.max(MIN_BRUSH) / 2.0;
                let color = if session.erasing {
                    [255, 255, 255]
                } else {
                    session.color
                };
                rgb(cr, color, 0.3);
                cr.arc(
                    at.0,
                    at.1,
                    (radius - 1.0).max(0.5),
                    0.0,
                    std::f64::consts::TAU,
                );
                let _ = cr.fill();
                cr.set_line_width(1.5);
                cr.set_source_rgb(1.0, 1.0, 1.0);
                cr.arc(at.0, at.1, radius - 0.75, 0.0, std::f64::consts::TAU);
                let _ = cr.stroke();
                cr.set_line_width(1.0);
                cr.set_source_rgba(0.047, 0.051, 0.059, 0.7);
                cr.arc(at.0, at.1, radius + 0.5, 0.0, std::f64::consts::TAU);
                let _ = cr.stroke();
            }
        }
        Tool::Text => {}
    }
}
