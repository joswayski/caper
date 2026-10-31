//! The attachment editor, like web's `AttachmentEditor.tsx`: crop and turn a
//! file waiting to be sent, draw on it and add text (docs/media.md, "Editing
//! attachments before sending"). It edits a description of the changes,
//! never the file; the draft prepares its original again with them
//! (`uploads::prepare_edited_path`). Videos show a still from just after the
//! start; crops and turns apply to the whole video and marks stay on screen
//! throughout. Caper's trimmed FFmpeg cannot lay marks over video frames, so
//! Draw and Text step back for videos when the FFmpeg in use lacks that.
//!
//! The picture is decoded off the UI thread, downscaled for the screen, and
//! drawn turned and cropped by texture coordinates; marks are painted with
//! the same geometry the export uses (`attachment_edit`).
//!
//! A staged file opens viewing, with no tool chosen: the file as it will be
//! sent, zoomable, covered while marked as a spoiler. Choosing a tool edits
//! in place on the same screen, and Open in default app opens what will be
//! sent.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};

use crate::attachment_edit::{
    self as markup, COLOR_NAMES, COLORS, Crop, Edit, Handle, Kind, MAX_TEXT, Mark, STROKE_SIZES,
    TEXT_SIZES,
};
use crate::compress::Compression;
use crate::uploads;
use crate::viewer::Zoom;
use crate::{ACTION, BLACKOUT, BORDER, ERROR, MUTED, NavIcon, RAISED, SURFACE, TERRACOTTA, TEXT};

const TOP_BAR: f32 = 56.0;
const TOOLS: f32 = 56.0;
const OPTIONS: f32 = 52.0;
const HINT: f32 = 24.0;
/// Below this width the colours and the size take a row each.
const NARROW: f32 = 560.0;
/// The picture is shown at most this large; exports use the original.
const DISPLAY_EDGE: u32 = 2048;
/// Space around the picture; the crop frame's handles need more.
const PADDING: f32 = 16.0;
const CROP_PADDING: f32 = 28.0;
const MIN_CROP_POINTS: f32 = 32.0;
/// Freehand samples closer than this add nothing.
const SAMPLE_POINTS: f32 = 1.5;
/// A press on text that moves less than this edits it rather than moving it.
const MOVE_POINTS: f32 = 4.0;
const HANDLE_HIT: f32 = 28.0;
/// Text marks are laid out at most this large and scaled up from there, so a
/// heavily cropped picture never asks the font atlas for giant glyphs.
const MAX_GLYPH_POINTS: f32 = 160.0;
const SHADE: Color32 = Color32::from_black_alpha(140);
/// The brush ring is never smaller than this across.
const MIN_BRUSH: f32 = 6.0;
/// The tools, in order, as the editor and the preview show them.
const TOOL_BUTTONS: [(Tool, NavIcon, &str); 3] = [
    (Tool::Crop, NavIcon::Crop, "Crop"),
    (Tool::Draw, NavIcon::Brush, "Draw"),
    (Tool::Text, NavIcon::Type, "Text"),
];
const TOOL_WIDTH: f32 = 96.0;
/// The tool row's buttons sit 4 points apart in a bordered group with 5 points
/// of padding; a divider with 4 points either side separates Spoiler and
/// Remove from the editor's tools.
const PREVIEW_TOOL_GAP: f32 = 4.0;
const PREVIEW_GROUP_GAP: f32 = 9.0;
const PREVIEW_TOOLS_HEIGHT: f32 = 50.0;
/// Narrower preview tools show only their icons.
const MIN_PREVIEW_TOOL: f32 = 36.0;
/// The editor's button for what will be sent, and why it waits.
const OPEN_SENT: &str = "Open in default app";
/// The editor's Close, which discards any unsaved change.
pub const CLOSE: &str = "Close editor";
const EDIT_PENDING: &str = "Available when your edit is ready";
pub const VIDEO_MARKS_UNAVAILABLE: &str =
    "Drawing on videos needs a newer FFmpeg than this build includes.";
const VIDEO_HINT: &str = "Drawings and text stay on screen for the whole video.";
const VIDEO_CROP_HINT: &str = "Crop and rotation apply to the whole video.";
/// Added to the preview picture's name while it is marked as a spoiler.
pub const SPOILER_LABEL: &str = ", marked as spoiler";

/// The draft being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Picture {
        destination: crate::pictures::Target,
        epoch: u64,
        path: PathBuf,
        scratch: Option<Arc<uploads::Scratch>>,
    },
    /// A conversation or thread composer's file, by its upload key.
    Draft(u64),
    /// A profile card's staged file, by its place and path.
    Profile {
        person: String,
        index: usize,
        path: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Crop,
    Draw,
    Text,
}

pub enum Outcome {
    Open,
    /// Close, discarding any unsaved change.
    Cancel,
    /// A change; `None` returns the draft to its original.
    Save(Option<Edit>),
    /// Mark the staged file as a spoiler, or not; the editor stays open.
    Spoiler,
    /// Remove the staged file and close, discarding any unsaved change.
    Remove,
}

struct Picture {
    texture: egui::TextureHandle,
    /// The shown pixels, for the composer's thumbnail of the result.
    pixels: image::RgbaImage,
    /// The upright original's size in pixels.
    width: u32,
    height: u32,
}

struct Loaded {
    image: image::RgbaImage,
    width: u32,
    height: u32,
    /// Whether marks can be applied: always for stills; for videos, when
    /// this FFmpeg can overlay them.
    marks: bool,
}

/// A text field open over the picture: a new mark, or `index` being changed.
struct Field {
    index: Option<usize>,
    x: f64,
    y: f64,
    text: String,
    color: [u8; 3],
    /// Font size as a fraction of the rotated picture's longer edge.
    size: f64,
    focus: bool,
}

enum Gesture {
    Draw {
        mark: Mark,
        last: Pos2,
    },
    /// Draw's eraser; `erased` indexes the committed marks.
    Erase {
        erased: Vec<usize>,
        last: Pos2,
    },
    Crop {
        handle: Handle,
        start: Crop,
        from: Pos2,
    },
    Text {
        index: Option<usize>,
        from: (f64, f64),
        origin: (f64, f64),
        press: Pos2,
        moved: bool,
    },
}

/// Where the picture is on screen and which part of it is shown.
#[derive(Clone, Copy)]
struct Layout {
    rect: Rect,
    /// The shown part, in fractions of the rotated picture.
    region: Crop,
    /// The full rotated picture's longer edge in points: sizes are fractions of it.
    edge: f32,
}

impl Layout {
    fn new(stage: Rect, rotated: (f32, f32), region: Crop, padding: f32) -> Self {
        let width = region.width as f32 * rotated.0;
        let height = region.height as f32 * rotated.1;
        let scale = ((stage.width() - padding * 2.0) / width)
            .min((stage.height() - padding * 2.0) / height)
            .max(0.0001);
        Self {
            rect: Rect::from_center_size(stage.center(), egui::vec2(width, height) * scale),
            region,
            edge: scale * rotated.0.max(rotated.1),
        }
    }

    /// A screen position in fractions of the rotated, uncropped picture.
    fn fraction(&self, pos: Pos2) -> (f64, f64) {
        let region = self.region;
        (
            region.x + f64::from((pos.x - self.rect.left()) / self.rect.width()) * region.width,
            region.y + f64::from((pos.y - self.rect.top()) / self.rect.height()) * region.height,
        )
    }

    fn screen(&self, x: f64, y: f64) -> Pos2 {
        let region = self.region;
        egui::pos2(
            self.rect.left() + ((x - region.x) / region.width) as f32 * self.rect.width(),
            self.rect.top() + ((y - region.y) / region.height) as f32 * self.rect.height(),
        )
    }

    fn crop_rect(&self, crop: Crop) -> Rect {
        Rect::from_min_max(
            self.screen(crop.x, crop.y),
            self.screen(crop.x + crop.width, crop.y + crop.height),
        )
    }

    /// Points per fraction of the full picture, across and down.
    fn full(&self) -> Vec2 {
        egui::vec2(
            self.rect.width() / self.region.width as f32,
            self.rect.height() / self.region.height as f32,
        )
    }
}

pub struct Editor {
    pub target: Target,
    kind: Kind,
    name: String,
    /// The original, which Open in default app opens while unedited.
    path: PathBuf,
    initial: Edit,
    states: Vec<Edit>,
    index: usize,
    /// The chosen tool, unless `viewing`.
    tool: Tool,
    /// No tool chosen: the picture as it will be sent, zoomable, under a
    /// spoiler cover when marked as one.
    viewing: bool,
    color: [u8; 3],
    stroke_points: f32,
    text_points: f32,
    /// Draw's eraser: presses remove the lines they touch instead of drawing.
    erasing: bool,
    picture: Option<Picture>,
    loading: Option<Receiver<Option<Loaded>>>,
    failed: bool,
    marks: bool,
    gesture: Option<Gesture>,
    live_crop: Option<Crop>,
    live_text: Option<(usize, f64, f64)>,
    field: Option<Field>,
    opened: bool,
    /// Viewing's zoom, as the media viewer zooms; a chosen tool resets it.
    zoom: Zoom,
    /// The picture blurred as a spoilered attachment's cover is, for a
    /// staged file marked as a spoiler.
    cover: Option<egui::TextureHandle>,
    /// The spoiler cover's fade ([`crate::motion::fade_toward`]).
    cover_fade: Option<(f32, bool, f64)>,
    /// When the decoded picture arrived, for it to fade in over the spinner.
    arrived_at: Option<f64>,
    own: Option<Own>,
    /// The edited file last opened in another app, which the app keeps
    /// until it quits ([`Editor::take_opened`]).
    opened_file: Option<Arc<uploads::Scratch>>,
}

impl Editor {
    /// Starts decoding `path` off the UI thread; `edit` is what the draft
    /// has. It opens viewing, with no tool chosen, for `None`; otherwise on
    /// `tool`, or on Crop for a video this FFmpeg cannot mark.
    pub fn open(
        context: &egui::Context,
        target: Target,
        kind: Kind,
        name: String,
        path: &Path,
        edit: Option<Edit>,
        tool: Option<Tool>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let (source, repaint) = (path.to_owned(), context.clone());
        std::thread::spawn(move || {
            let _ = sender.send(match kind {
                Kind::Image => load_image(&source),
                Kind::Video => load_video(&source),
            });
            repaint.request_repaint();
        });
        let initial = edit.unwrap_or_default();
        Self {
            target,
            kind,
            name,
            path: path.to_owned(),
            states: vec![initial.clone()],
            initial,
            index: 0,
            tool: tool.unwrap_or(Tool::Crop),
            viewing: tool.is_none(),
            color: COLORS[0],
            stroke_points: STROKE_SIZES.2,
            text_points: TEXT_SIZES.2,
            erasing: false,
            picture: None,
            loading: Some(receiver),
            failed: false,
            marks: kind == Kind::Image,
            gesture: None,
            live_crop: None,
            live_text: None,
            field: None,
            opened: false,
            zoom: Zoom::default(),
            cover: None,
            cover_fade: None,
            arrived_at: None,
            own: None,
            opened_file: None,
        }
    }

    #[cfg(test)]
    pub fn loading(&self) -> bool {
        self.loading.is_some()
    }

    /// The chosen tool; `None` while viewing.
    #[cfg(test)]
    pub fn tool(&self) -> Option<Tool> {
        (!self.viewing).then_some(self.tool)
    }

    #[cfg(test)]
    pub fn zoom(&self) -> Zoom {
        self.zoom
    }

    /// The edited file opened in another app, for the app to keep until it
    /// quits: closing the editor must not remove a file still being shown.
    pub fn take_opened(&mut self) -> Option<Arc<uploads::Scratch>> {
        self.opened_file.take()
    }

    /// Chooses `tool` in place, or returns to viewing with `None`.
    fn choose(&mut self, tool: Option<Tool>) {
        self.commit_field();
        self.gesture = None;
        self.live_crop = None;
        self.live_text = None;
        self.erasing = false;
        self.zoom = Zoom::default();
        self.viewing = tool.is_none();
        if let Some(tool) = tool {
            self.tool = tool;
        }
    }

    /// Opens a text field with `text` at `x`, `y` (layout fixtures).
    pub fn place_text(&mut self, x: f64, y: f64, text: &str) {
        self.field = Some(Field {
            index: None,
            x,
            y,
            text: text.into(),
            color: self.color,
            size: 0.04,
            focus: true,
        });
    }

    /// The edit as it stands, with every committed change.
    pub fn current(&self) -> &Edit {
        &self.states[self.index]
    }

    fn square_edit(&self, mut edit: Edit) -> Edit {
        if matches!(self.target, Target::Picture { .. })
            && let Some(picture) = &self.picture
        {
            let (width, height) =
                markup::rotated_size(picture.width, picture.height, edit.rotation);
            edit.crop = crate::pictures::square_frame(edit.crop, width, height);
        }
        edit
    }

    /// Adds a change as one undo step, dropping anything undone.
    fn commit(&mut self, next: Edit) {
        let next = self.square_edit(next);
        if &next == self.current() {
            return;
        }
        self.states.truncate(self.index + 1);
        self.states.push(next);
        self.index += 1;
    }

    fn commit_field(&mut self) {
        if let Some(field) = self.field.take() {
            let next = with_field(self.current(), Some(&field));
            self.commit(next);
        }
    }

    fn undo(&mut self) {
        self.field = None;
        self.index = self.index.saturating_sub(1);
    }

    fn redo(&mut self) {
        self.field = None;
        self.index = (self.index + 1).min(self.states.len() - 1);
    }

    /// A thumbnail of the edited picture from what is shown, for the
    /// composer until the prepared file's own arrives.
    pub fn preview(&self, edit: Option<&Edit>) -> Option<egui::ColorImage> {
        let picture = self.picture.as_ref()?;
        let shown = image::DynamicImage::ImageRgba8(picture.pixels.clone());
        let rendered = match edit {
            Some(edit) => markup::render(&shown, edit)?,
            None => shown,
        };
        let edge = crate::attachments::THUMBNAIL_EDGE;
        Some(color_image(&rendered.thumbnail(edge, edge).to_rgba8()))
    }

    fn receive(&mut self, context: &egui::Context) {
        let Some(receiver) = &self.loading else {
            return;
        };
        let loaded = match receiver.try_recv() {
            Ok(loaded) => loaded,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        self.loading = None;
        match loaded {
            Some(loaded) => {
                self.marks = loaded.marks;
                if !self.marks {
                    self.tool = Tool::Crop;
                }
                let image = color_image(&loaded.image);
                // Only a staged file can be marked as a spoiler.
                if !matches!(self.target, Target::Picture { .. }) {
                    self.cover = Some(context.load_texture(
                        "attachment-editor-cover",
                        crate::attachments::blur_thumbnail(&image),
                        egui::TextureOptions::LINEAR,
                    ));
                }
                self.arrived_at = Some(context.input(|input| input.time));
                self.picture = Some(Picture {
                    texture: context.load_texture(
                        "attachment-editor",
                        image,
                        egui::TextureOptions::LINEAR,
                    ),
                    pixels: loaded.image,
                    width: loaded.width,
                    height: loaded.height,
                });
                if matches!(self.target, Target::Picture { .. }) {
                    let initial = self.square_edit(Edit::default());
                    self.states = vec![initial.clone()];
                    self.initial = initial;
                }
            }
            None => self.failed = true,
        }
    }

    /// A profile card's edit, once prepared, opens as it was asked to.
    fn receive_prepared(&mut self, context: &egui::Context) {
        let Some(Own::Preparing(preparation)) = &self.own else {
            return;
        };
        let prepared = match preparation.result.try_recv() {
            Ok(prepared) => prepared,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        match prepared {
            Some(prepared) => {
                let prepared = Arc::new(prepared);
                self.own = Some(Own::Ready(prepared.clone()));
                self.open_sent(context, ToSend::Prepared(prepared));
            }
            None => self.own = Some(Own::Failed),
        }
    }

    /// Draws the editor over everything; returns what the person chose.
    /// `to_send` is the draft's file as it stands this frame, which Open in
    /// default app opens. `spoiler` is whether a staged file that can still
    /// change is marked as one; it adds Spoiler and Remove to the tools.
    /// `None` shows only the editor's tools.
    pub fn show(
        &mut self,
        context: &egui::Context,
        to_send: ToSend,
        spoiler: Option<bool>,
    ) -> Outcome {
        if !self.opened {
            self.opened = true;
            // The composer below must not keep typing.
            if let Some(focused) = context.memory(|memory| memory.focused()) {
                context.memory_mut(|memory| memory.surrender_focus(focused));
            }
        }
        self.receive(context);
        self.receive_prepared(context);
        let to_send = match (to_send, &self.own) {
            (ToSend::AtSend(_), Some(Own::Preparing(_))) => ToSend::Preparing,
            (ToSend::AtSend(_), Some(Own::Ready(prepared))) => ToSend::Prepared(prepared.clone()),
            (ToSend::AtSend(_), Some(Own::Failed)) => ToSend::Failed,
            (to_send, _) => to_send,
        };
        // Escape closes an open colour picker first, which handles it itself.
        let popup = egui::Popup::is_any_open(context);
        let escape = !popup
            && context
                .input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        if escape {
            if self.field.is_some() {
                self.field = None;
            } else {
                return Outcome::Cancel;
            }
        }
        if self.field.is_none() && self.picture.is_some() && self.gesture.is_none() {
            let (redo, undo) = context.input_mut(|input| {
                let redo = input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    egui::Key::Z,
                )) || input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::Y,
                ));
                let undo = input.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::Z,
                ));
                (redo, undo)
            });
            if redo {
                self.redo();
            }
            if undo {
                self.undo();
            }
        }
        let screen = context.content_rect();
        let modal = egui::Modal::new(egui::Id::new("attachment-editor"))
            .frame(egui::Frame::NONE)
            .backdrop_color(BLACKOUT)
            .show(context, |ui| self.contents(ui, screen, to_send, spoiler));
        // It arrives with a short fade and settles from just smaller.
        crate::motion::modal_zoom(context, modal.response.layer_id, screen.center());
        // Nothing below sees Escape while the editor is open.
        context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        modal.inner
    }

    /// One screen for viewing and editing: the top bar, the picture's
    /// place and the tool row stay the same whichever tool is chosen.
    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        screen: Rect,
        to_send: ToSend,
        spoiler: Option<bool>,
    ) -> Outcome {
        let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, BLACKOUT);
        let mut outcome = Outcome::Open;
        let picture_target = matches!(self.target, Target::Picture { .. });
        let profile_picture = matches!(
            self.target,
            Target::Picture {
                destination: crate::pictures::Target::Avatar,
                ..
            }
        );
        let top = Rect::from_min_size(rect.min, egui::vec2(rect.width(), TOP_BAR));
        let tools = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - TOOLS), rect.max);
        // The options strip keeps its height in every state (empty while
        // viewing), so choosing a tool never moves the picture.
        let options_height = if rect.width() < NARROW {
            OPTIONS * 2.0 - 12.0
        } else {
            OPTIONS
        };
        let options = Rect::from_min_max(
            egui::pos2(rect.left(), tools.top() - options_height),
            egui::pos2(rect.right(), tools.top()),
        );
        let hint = (self.kind == Kind::Video && self.picture.is_some()).then(|| {
            Rect::from_min_max(
                egui::pos2(rect.left(), options.top() - HINT),
                egui::pos2(rect.right(), options.top()),
            )
        });
        let stage = Rect::from_min_max(
            egui::pos2(rect.left(), top.bottom()),
            egui::pos2(rect.right(), hint.map_or(options.top(), |hint| hint.top())),
        );
        let layout = self.layout(stage);
        let edge = layout.map_or(1.0, |layout| layout.edge / self.zoom.scale);
        let ready = self.picture.is_some();

        // Top bar: Close, the name, Open in default app, Undo, Redo, Reset
        // and Done, the same in every state.
        let painter = ui.painter_at(rect);
        painter.line_segment(
            [top.left_bottom(), top.right_bottom()],
            Stroke::new(1.0, BORDER),
        );
        let close = Rect::from_center_size(
            egui::pos2(top.left() + 16.0 + 18.0, top.center().y),
            egui::vec2(36.0, 36.0),
        );
        if icon_button(ui, close, NavIcon::Close, CLOSE, "Close", true).clicked() {
            return Outcome::Cancel;
        }
        let mut right = top.right() - 16.0;
        let mut place = |width: f32| {
            right -= width;
            let rect = Rect::from_min_size(
                egui::pos2(right, top.center().y - 18.0),
                egui::vec2(width, 36.0),
            );
            right -= 8.0;
            rect
        };
        let done = place(if profile_picture { 112.0 } else { 64.0 });
        // With nothing changed, Done just closes.
        let can_finish = ready || !picture_target;
        let finish_label = if profile_picture {
            "Apply crop"
        } else {
            "Done"
        };
        let done_response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(done), |ui| {
                crate::primary_button(ui, finish_label, can_finish)
            })
            .inner;
        done_response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, can_finish, finish_label)
        });
        if done_response.clicked() && can_finish {
            let fin = with_field(self.current(), self.field.as_ref());
            outcome = if fin == self.initial && !picture_target {
                Outcome::Cancel
            } else {
                Outcome::Save((fin.is_edited() || picture_target).then_some(fin))
            };
        }
        let reset = place(64.0);
        let can_reset = ready && self.current().is_edited();
        if text_button(ui, reset, "Reset", can_reset).clicked() {
            self.field = None;
            self.commit(Edit::default());
        }
        let redo = place(36.0);
        let can_redo = ready && self.index + 1 < self.states.len();
        if icon_button(
            ui,
            redo,
            NavIcon::Redo,
            "Redo",
            "Redo (Ctrl+Shift+Z)",
            can_redo,
        )
        .clicked()
        {
            self.redo();
        }
        let undo = place(36.0);
        let can_undo = ready && self.index > 0;
        if icon_button(ui, undo, NavIcon::Undo, "Undo", "Undo (Ctrl+Z)", can_undo).clicked() {
            self.undo();
        }
        // What will be sent, in its default app (a video plays there).
        let mut title_right = undo.left();
        if !picture_target {
            let open = place(36.0);
            title_right = open.left();
            match to_send {
                ToSend::Preparing => {
                    let response = ui.interact(
                        open,
                        ui.id().with("attachment-editor-preparing"),
                        egui::Sense::hover(),
                    );
                    egui::Spinner::new().size(16.0).color(MUTED).paint_at(
                        ui,
                        Rect::from_center_size(open.center(), egui::vec2(16.0, 16.0)),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Button, false, OPEN_SENT)
                    });
                    response.on_hover_text(EDIT_PENDING);
                }
                ToSend::Failed => {
                    icon_button(ui, open, NavIcon::ExternalLink, OPEN_SENT, OPEN_SENT, false)
                        .on_hover_text(markup::FAILED);
                }
                to_send => {
                    if icon_button(ui, open, NavIcon::ExternalLink, OPEN_SENT, OPEN_SENT, true)
                        .clicked()
                    {
                        self.open_sent(ui.ctx(), to_send);
                    }
                }
            }
        }
        let title_left = close.right() + 12.0;
        let mut job = egui::text::LayoutJob::simple_singleline(
            self.name.clone(),
            egui::FontId::new(14.0, egui::FontFamily::Name("Satoshi Bold".into())),
            TEXT,
        );
        job.wrap =
            egui::text::TextWrapping::truncate_at_width((title_right - 12.0 - title_left).max(0.0));
        let title = ui.painter().layout_job(job);
        painter.galley(
            egui::pos2(title_left, top.center().y - title.size().y / 2.0),
            title,
            TEXT,
        );

        // The stage: the picture, its marks and the crop frame, or the
        // picture as it will be sent while viewing.
        if self.failed {
            painter.text(
                stage.center(),
                egui::Align2::CENTER_CENTER,
                "This file can’t be edited here.",
                egui::FontId::proportional(14.0),
                MUTED,
            );
        } else if let Some(layout) = layout {
            self.stage(ui, stage, layout, options, spoiler == Some(true));
        } else {
            egui::Spinner::new().size(24.0).color(MUTED).paint_at(
                ui,
                Rect::from_center_size(stage.center(), egui::vec2(24.0, 24.0)),
            );
        }
        if let Some(hint) = hint {
            painter.text(
                hint.center(),
                egui::Align2::CENTER_CENTER,
                if self.marks {
                    VIDEO_HINT
                } else {
                    VIDEO_CROP_HINT
                },
                egui::FontId::proportional(12.0),
                MUTED,
            );
        }

        // The chosen tool's options fade in; switching away is instant.
        if ready && !self.viewing {
            let from = crate::motion::mark(ui);
            self.options(ui, options, edge);
            let to = crate::motion::mark(ui);
            let shown = crate::motion::entrance(
                ui.ctx(),
                ui.id().with(("attachment-editor-options", self.tool)),
                crate::motion::FAST,
            );
            crate::motion::settle_range(
                ui.ctx(),
                ui.layer_id(),
                from..to,
                shown,
                egui::emath::TSTransform::IDENTITY,
            );
        }

        // The tool row: a staged file's Spoiler, then Crop, Draw and Text,
        // each chosen in place (again to go back to viewing), then Remove.
        painter.line_segment(
            [options.left_top(), options.right_top()],
            Stroke::new(1.0, BORDER),
        );
        let (group, buttons, dividers) = preview_tool_layout(
            tools.center().x,
            tools.bottom() - 3.0,
            rect.width() - 2.0 * PADDING,
            spoiler.is_some(),
        );
        ui.painter().rect(
            group,
            8.0,
            SURFACE,
            Stroke::new(1.0, BORDER),
            egui::StrokeKind::Inside,
        );
        for x in dividers {
            ui.painter().vline(
                x + 0.5,
                group.shrink2(egui::vec2(0.0, 12.0)).y_range(),
                Stroke::new(1.0, BORDER),
            );
        }
        for (tool, button) in buttons {
            match tool {
                PreviewTool::Spoiler => {
                    let marked = spoiler.unwrap_or(false);
                    if tool_button(ui, button, NavIcon::EyeOff, "Spoiler", marked, true).clicked() {
                        outcome = Outcome::Spoiler;
                    }
                }
                PreviewTool::Remove => {
                    let response = tool_button_toned(
                        ui,
                        button,
                        NavIcon::Trash,
                        "Remove",
                        false,
                        true,
                        Some(ERROR),
                    );
                    if response.clicked() {
                        outcome = Outcome::Remove;
                    }
                }
                PreviewTool::Edit(tool) => {
                    let (_, icon, label) = TOOL_BUTTONS
                        .into_iter()
                        .find(|(candidate, ..)| *candidate == tool)
                        .expect("every tool has a button");
                    let enabled = ready && (tool == Tool::Crop || self.marks);
                    let selected = !self.viewing && self.tool == tool;
                    let response = tool_button(ui, button, icon, label, selected, enabled);
                    // Only Draw and Text are ever unavailable, and only for videos.
                    let response = if ready && !enabled {
                        response.on_hover_text(VIDEO_MARKS_UNAVAILABLE)
                    } else {
                        response
                    };
                    if response.clicked() && enabled {
                        if !selected {
                            self.choose(Some(tool));
                        } else if !picture_target {
                            // A profile picture's adjustment stays on Crop.
                            self.choose(None);
                        }
                    }
                }
            }
        }

        outcome
    }

    /// Opens what will be sent in its default app: the original, or the
    /// prepared edit from a file this app owns. A profile card's edit is
    /// prepared first and opens once ready.
    fn open_sent(&mut self, context: &egui::Context, to_send: ToSend) {
        let path = match to_send {
            ToSend::Original => self.path.clone(),
            ToSend::Prepared(prepared) => {
                // The edit can't change while the preview is open.
                let Ok(file) = self
                    .opened_file
                    .clone()
                    .map_or_else(|| prepared.on_disk(), Ok)
                else {
                    return;
                };
                let path = file.path().to_owned();
                self.opened_file = Some(file);
                path
            }
            ToSend::AtSend(settings) => return self.prepare(context, settings),
            ToSend::Preparing | ToSend::Failed => return,
        };
        if let Ok(url) = url::Url::from_file_path(&path) {
            context.open_url(egui::OpenUrl::new_tab(url));
        }
    }

    /// Prepares a profile card's edit as its Send will, off the UI thread.
    fn prepare(&mut self, context: &egui::Context, settings: Compression) {
        // The saved edit, which Send prepares.
        let edit = self.initial.clone();
        if !edit.is_edited() {
            return;
        }
        let (sender, result) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let (path, name, stop, repaint) = (
            self.path.clone(),
            self.name.clone(),
            cancel.clone(),
            context.clone(),
        );
        std::thread::spawn(move || {
            let prepared =
                uploads::prepare_edited_path(&path, &name, &edit, &settings, &|_| {}, &stop);
            let _ = sender.send(prepared.ok());
            repaint.request_repaint();
        });
        self.own = Some(Own::Preparing(Preparation { result, cancel }));
    }

    fn layout(&self, stage: Rect) -> Option<Layout> {
        let picture = self.picture.as_ref()?;
        let (width, height) =
            markup::rotated_size(picture.width, picture.height, self.current().rotation);
        // One padding in every state (what the crop frame's handles need),
        // so choosing a tool never moves or resizes an uncropped picture.
        let region = if self.tool == Tool::Crop && !self.viewing {
            Crop::FULL
        } else {
            self.current().crop
        };
        let mut layout = Layout::new(stage, (width as f32, height as f32), region, CROP_PADDING);
        if self.viewing {
            let mut zoom = self.zoom;
            let placed = zoom.place(stage, layout.rect.size());
            layout.edge *= placed.width() / layout.rect.width();
            layout.rect = placed;
        }
        Some(layout)
    }

    /// What is on screen: the committed edit plus anything in progress.
    fn shown(&self) -> Edit {
        let mut shown = self.current().clone();
        if let Some(crop) = self.live_crop {
            shown.crop = crop;
        }
        if let Some((index, x, y)) = self.live_text
            && let Some(Mark::Text {
                x: at_x, y: at_y, ..
            }) = shown.marks.get_mut(index)
        {
            (*at_x, *at_y) = (x, y);
        }
        // Both index the committed marks, so they go together.
        let field = self.field.as_ref().and_then(|field| field.index);
        let erased = match &self.gesture {
            Some(Gesture::Erase { erased, .. }) => erased.as_slice(),
            _ => &[],
        };
        if field.is_some() || !erased.is_empty() {
            let mut index = 0;
            shown.marks.retain(|_| {
                let keep = Some(index) != field && !erased.contains(&index);
                index += 1;
                keep
            });
        }
        if let Some(Gesture::Draw { mark, .. }) = &self.gesture {
            shown.marks.push(mark.clone());
        }
        shown
    }

    fn stage(
        &mut self,
        ui: &mut egui::Ui,
        stage: Rect,
        layout: Layout,
        options: Rect,
        spoiler: bool,
    ) {
        let Some(picture) = &self.picture else {
            return;
        };
        let shown = self.shown();
        // It fades in over the spinner once decoded.
        let arrival = self.arrived_at.map_or(1.0, |at| {
            crate::motion::progress(ui.ctx(), at, crate::motion::BASE)
        });
        if arrival < 1.0 {
            let from = crate::motion::mark(ui);
            egui::Spinner::new().size(24.0).color(MUTED).paint_at(
                ui,
                Rect::from_center_size(stage.center(), egui::vec2(24.0, 24.0)),
            );
            crate::motion::settle(
                ui.ctx(),
                ui.layer_id(),
                from,
                1.0 - arrival,
                egui::emath::TSTransform::IDENTITY,
            );
        }
        let from = crate::motion::mark(ui);
        let painter = ui.painter_at(stage);
        paint_picture(
            &painter,
            &picture.texture,
            &layout,
            shown.rotation,
            Color32::WHITE,
        );
        paint_marks(
            &ui.painter_at(layout.rect.intersect(stage)),
            &layout,
            &shown.marks,
        );
        // While viewing, a spoiler shows covered, as the people it is sent
        // to will see it; a chosen tool shows the picture plainly.
        let cover = if self.viewing {
            crate::motion::fade_toward(ui.ctx(), &mut self.cover_fade, spoiler)
        } else {
            // Back to viewing, the cover fades in again.
            self.cover_fade = Some((0.0, false, f64::NEG_INFINITY));
            0.0
        };
        if cover > 0.0
            && let Some(blurred) = &self.cover
        {
            paint_spoiler_cover(&painter, blurred, &layout, shown.rotation, stage, cover);
        }
        crate::motion::settle(
            ui.ctx(),
            ui.layer_id(),
            from,
            arrival,
            egui::emath::TSTransform::IDENTITY,
        );
        if self.viewing {
            let shown_rect = layout.rect.intersect(stage);
            let response = ui.interact(
                shown_rect,
                ui.id().with("attachment-editor-stage"),
                egui::Sense::click_and_drag(),
            );
            let mut label = match self.kind {
                Kind::Image => format!("Image {}", self.name),
                Kind::Video => format!("Still from {}", self.name),
            };
            if spoiler {
                label.push_str(SPOILER_LABEL);
            }
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, &label));
            let base = zoom_base(&layout, self.zoom);
            self.zoom.interact(ui, &response, stage, base);
            // Keep it covering the stage however far it is dragged.
            self.zoom.place(stage, base);
            return;
        }
        let response = ui.interact(
            layout.rect,
            ui.id().with("attachment-editor-stage"),
            if self.tool == Tool::Crop {
                egui::Sense::hover()
            } else {
                egui::Sense::drag()
            },
        );
        let label = if self.kind == Kind::Video {
            "Video frame being edited"
        } else {
            "Image being edited"
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, label));
        let response = if self.tool == Tool::Text {
            response.on_hover_cursor(egui::CursorIcon::Text)
        } else {
            response
        };
        match self.tool {
            Tool::Crop => self.crop_frame(ui, &layout, shown.crop),
            Tool::Draw => {
                self.draw(ui, &response, &layout);
                self.brush(ui, &response, &layout, stage);
            }
            Tool::Text => {
                self.text(ui, &response, &layout);
                self.field(ui, &layout, options);
            }
        }
    }

    fn draw(&mut self, ui: &egui::Ui, response: &egui::Response, layout: &Layout) {
        if self.erasing {
            self.erase(ui, response, layout);
            return;
        }
        if response.drag_started()
            && let Some(press) = ui.input(|input| input.pointer.press_origin())
        {
            let (x, y) = layout.fraction(press);
            self.gesture = Some(Gesture::Draw {
                mark: Mark::Stroke {
                    color: self.color,
                    width: stroke_width(self.stroke_points, layout.edge),
                    points: vec![x, y],
                },
                last: press,
            });
        }
        if let Some(Gesture::Draw {
            mark: Mark::Stroke { points, .. },
            last,
        }) = &mut self.gesture
        {
            // Every pointer sample this frame, not only where it ended up.
            let moves: Vec<Pos2> = ui.input(|input| {
                input
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::PointerMoved(pos) => Some(*pos),
                        _ => None,
                    })
                    .collect()
            });
            for pos in moves {
                if pos.distance(*last) < SAMPLE_POINTS {
                    continue;
                }
                let (x, y) = layout.fraction(pos);
                points.extend([x, y]);
                *last = pos;
            }
            if response.drag_stopped() || !ui.input(|input| input.pointer.primary_down()) {
                if let Some(Gesture::Draw { mark, .. }) = self.gesture.take() {
                    let mut next = self.current().clone();
                    next.marks.push(mark);
                    self.commit(next);
                }
            } else {
                ui.ctx().request_repaint();
            }
        }
    }

    /// Draw's eraser: a press removes the committed lines its ring passes
    /// over, as one step to undo.
    fn erase(&mut self, ui: &egui::Ui, response: &egui::Response, layout: &Layout) {
        if response.drag_started()
            && let Some(press) = ui.input(|input| input.pointer.press_origin())
        {
            let erased = self.touched_along(layout, press, press, &[]);
            self.gesture = Some(Gesture::Erase {
                erased,
                last: press,
            });
        }
        let Some(Gesture::Erase { erased, last }) = &self.gesture else {
            return;
        };
        let (mut erased, mut last) = (erased.clone(), *last);
        let moves: Vec<Pos2> = ui.input(|input| {
            input
                .events
                .iter()
                .filter_map(|event| match event {
                    egui::Event::PointerMoved(pos) => Some(*pos),
                    _ => None,
                })
                .collect()
        });
        for pos in moves {
            erased = self.touched_along(layout, last, pos, &erased);
            last = pos;
        }
        if response.drag_stopped() || !ui.input(|input| input.pointer.primary_down()) {
            self.gesture = None;
            if !erased.is_empty() {
                let mut next = self.current().clone();
                let mut index = 0;
                next.marks.retain(|_| {
                    let keep = !erased.contains(&index);
                    index += 1;
                    keep
                });
                self.commit(next);
            }
        } else {
            self.gesture = Some(Gesture::Erase { erased, last });
            ui.ctx().request_repaint();
        }
    }

    /// `erased` with the committed lines the eraser's ring touches on its way
    /// from `from` (already checked) to `to`, sampled closely enough that a
    /// fast swipe skips none.
    fn touched_along(&self, layout: &Layout, from: Pos2, to: Pos2, erased: &[usize]) -> Vec<usize> {
        let radius = brush_diameter(self.stroke_points, layout.edge) / 2.0;
        let steps = (from.distance(to) / (radius / 2.0).max(1.0))
            .ceil()
            .max(1.0) as usize;
        let full = layout.full();
        let mut erased = erased.to_vec();
        for step in 1..=steps {
            let at = from.lerp(to, step as f32 / steps as f32);
            for index in markup::strokes_touched(
                &self.current().marks,
                layout.fraction(at),
                f64::from(radius),
                (f64::from(full.x), f64::from(full.y)),
            ) {
                if !erased.contains(&index) {
                    erased.push(index);
                }
            }
        }
        erased
    }

    /// A ring as wide and coloured as the brush in place of the cursor over
    /// the picture, following the pointer while drawing too. Touch has no
    /// hover, so it gets none.
    fn brush(&self, ui: &egui::Ui, response: &egui::Response, layout: &Layout, stage: Rect) {
        let (pos, touch) = ui.input(|input| (input.pointer.hover_pos(), input.any_touches()));
        let Some(pos) =
            pos.filter(|pos| response.hovered() && !touch && layout.rect.contains(*pos))
        else {
            return;
        };
        ui.ctx().set_cursor_icon(egui::CursorIcon::None);
        paint_brush(
            &ui.painter_at(stage),
            pos,
            brush_diameter(self.stroke_points, layout.edge),
            if self.erasing {
                [255, 255, 255]
            } else {
                self.color
            },
        );
    }

    /// The topmost text mark under a screen position, by its on-screen box.
    fn text_at(&self, layout: &Layout, pos: Pos2) -> Option<usize> {
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
                let pixels = *size as f32 * layout.edge;
                let width = markup::text_metrics(text, pixels).map_or(0.0, |metrics| metrics.width);
                let centre = layout.screen(*x, *y);
                ((pos.x - centre.x).abs() <= width / 2.0 + 8.0
                    && (pos.y - centre.y).abs() <= pixels * 0.6 + 8.0)
                    .then_some(index)
            })
    }

    fn text(&mut self, ui: &egui::Ui, response: &egui::Response, layout: &Layout) {
        if response.drag_started()
            && let Some(press) = ui.input(|input| input.pointer.press_origin())
        {
            if self.field.is_some() {
                // A press outside the open field commits it, like a blur.
                self.commit_field();
                return;
            }
            let index = self.text_at(layout, press);
            let from = layout.fraction(press);
            let origin = match index.and_then(|index| self.current().marks.get(index)) {
                Some(Mark::Text { x, y, .. }) => (*x, *y),
                _ => from,
            };
            self.gesture = Some(Gesture::Text {
                index,
                from,
                origin,
                press,
                moved: false,
            });
        }
        let Some(Gesture::Text {
            index,
            from,
            origin,
            press,
            moved,
        }) = &mut self.gesture
        else {
            return;
        };
        if let (Some(index), Some(pos)) = (*index, ui.input(|input| input.pointer.latest_pos())) {
            *moved |= pos.distance(*press) > MOVE_POINTS;
            if *moved {
                let (x, y) = layout.fraction(pos);
                self.live_text = Some((
                    index,
                    (origin.0 + x - from.0).clamp(0.0, 1.0),
                    (origin.1 + y - from.1).clamp(0.0, 1.0),
                ));
            }
        }
        if !response.drag_stopped() && ui.input(|input| input.pointer.primary_down()) {
            ui.ctx().request_repaint();
            return;
        }
        let Some(Gesture::Text {
            index, from, moved, ..
        }) = self.gesture.take()
        else {
            return;
        };
        let live = self.live_text.take();
        match index {
            None => {
                self.field = Some(Field {
                    index: None,
                    x: from.0,
                    y: from.1,
                    text: String::new(),
                    color: self.color,
                    size: f64::from(self.text_points / layout.edge),
                    focus: true,
                });
            }
            Some(_) if moved => {
                if let Some((index, x, y)) = live {
                    let mut next = self.current().clone();
                    if let Some(Mark::Text {
                        x: at_x, y: at_y, ..
                    }) = next.marks.get_mut(index)
                    {
                        (*at_x, *at_y) = (x, y);
                    }
                    self.commit(next);
                }
            }
            Some(index) => {
                if let Some(Mark::Text {
                    color,
                    size,
                    x,
                    y,
                    text,
                }) = self.current().marks.get(index).cloned()
                {
                    self.color = color;
                    self.text_points = (size as f32 * layout.edge)
                        .round()
                        .clamp(TEXT_SIZES.0, TEXT_SIZES.1);
                    self.field = Some(Field {
                        index: Some(index),
                        x,
                        y,
                        text,
                        color,
                        size,
                        focus: true,
                    });
                }
            }
        }
    }

    /// The open text field, single line, in the mark's colour and size.
    fn field(&mut self, ui: &mut egui::Ui, layout: &Layout, options: Rect) {
        let Some(field) = &mut self.field else {
            return;
        };
        let points = field.size as f32 * layout.edge;
        let width = markup::text_metrics(
            if field.text.is_empty() {
                "M"
            } else {
                &field.text
            },
            points,
        )
        .map_or(points, |metrics| metrics.width);
        let size = egui::vec2((width + points).max(48.0), points * 1.3 + 8.0);
        let rect = Rect::from_center_size(layout.screen(field.x, field.y), size);
        let [red, green, blue] = field.color;
        ui.painter().rect(
            rect,
            8.0,
            Color32::from_black_alpha(90),
            Stroke::new(1.0, Color32::from_white_alpha(140)),
            egui::StrokeKind::Outside,
        );
        let id = egui::Id::new("attachment-editor-text");
        let edit = ui.put(
            rect,
            egui::TextEdit::singleline(&mut field.text)
                .id(id)
                .font(egui::FontId::new(
                    points.min(MAX_GLYPH_POINTS),
                    egui::FontFamily::Name("Satoshi Bold".into()),
                ))
                .text_color(Color32::from_rgb(red, green, blue))
                .horizontal_align(egui::Align::Center)
                .vertical_align(egui::Align::Center)
                .frame(false)
                .margin(egui::vec2(4.0, 2.0))
                .char_limit(MAX_TEXT)
                .desired_width(size.x),
        );
        edit.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::TextEdit,
                true,
                if field.index.is_none() {
                    "New text"
                } else {
                    "Edit text"
                },
            )
        });
        if field.focus {
            field.focus = false;
            edit.request_focus();
        }
        if edit.lost_focus() {
            let entered = ui.input(|input| input.key_pressed(egui::Key::Enter));
            // Moving to the colour or size controls keeps the field open.
            let options_press = ui
                .input(|input| input.pointer.press_origin())
                .is_some_and(|press| options.contains(press))
                || egui::Popup::is_any_open(ui.ctx());
            if !entered && options_press {
                field.focus = true;
            } else {
                self.commit_field();
            }
        }
    }

    fn crop_frame(&mut self, ui: &egui::Ui, layout: &Layout, crop: Crop) {
        let frame = layout.crop_rect(crop);
        let painter = ui.painter_at(layout.rect.expand(CROP_PADDING));
        // The part cropped away is dimmed.
        for shade in [
            Rect::from_min_max(
                layout.rect.min,
                egui::pos2(layout.rect.right(), frame.top()),
            ),
            Rect::from_min_max(
                egui::pos2(layout.rect.left(), frame.bottom()),
                layout.rect.max,
            ),
            Rect::from_min_max(
                egui::pos2(layout.rect.left(), frame.top()),
                egui::pos2(frame.left(), frame.bottom()),
            ),
            Rect::from_min_max(
                egui::pos2(frame.right(), frame.top()),
                egui::pos2(layout.rect.right(), frame.bottom()),
            ),
        ] {
            if shade.is_positive() {
                painter.rect_filled(shade, 0.0, SHADE);
            }
        }
        painter.rect_stroke(
            frame,
            0.0,
            Stroke::new(1.5, Color32::from_white_alpha(230)),
            egui::StrokeKind::Middle,
        );
        // Inside moves the frame; corners and edges resize it, on top.
        let mut held = None;
        let inside = ui.interact(
            frame,
            ui.id().with("attachment-editor-crop"),
            egui::Sense::drag(),
        );
        inside
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, "Crop frame"));
        if inside.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        }
        if inside.drag_started() {
            held = Some(Handle::Move);
        }
        let mut dragging = inside.dragged();
        let mut stopped = inside.drag_stopped();
        for handle in Handle::EDGES {
            let (ax, ay) = handle.anchor();
            let at = egui::pos2(
                frame.left() + frame.width() * ax,
                frame.top() + frame.height() * ay,
            );
            paint_handle(&painter, at, handle);
            let response = ui.interact(
                Rect::from_center_size(at, egui::vec2(HANDLE_HIT, HANDLE_HIT)),
                ui.id().with(("attachment-editor-handle", handle as u8)),
                egui::Sense::drag(),
            );
            if response.hovered() {
                ui.ctx().set_cursor_icon(match handle {
                    Handle::N | Handle::S => egui::CursorIcon::ResizeVertical,
                    Handle::E | Handle::W => egui::CursorIcon::ResizeHorizontal,
                    Handle::NW | Handle::SE => egui::CursorIcon::ResizeNwSe,
                    _ => egui::CursorIcon::ResizeNeSw,
                });
            }
            if response.drag_started() {
                held = Some(handle);
            }
            dragging |= response.dragged();
            stopped |= response.drag_stopped();
        }
        if let Some(handle) = held
            && let Some(from) = ui.input(|input| input.pointer.press_origin())
        {
            self.gesture = Some(Gesture::Crop {
                handle,
                start: self.current().crop,
                from,
            });
        }
        let Some(Gesture::Crop {
            handle,
            start,
            from,
        }) = &self.gesture
        else {
            self.crop_keys(ui);
            return;
        };
        if let Some(pos) = ui.input(|input| input.pointer.latest_pos())
            && (dragging || stopped)
        {
            let full = layout.full();
            let min = (
                f64::from(MIN_CROP_POINTS / full.x).clamp(0.02, 1.0),
                f64::from(MIN_CROP_POINTS / full.y).clamp(0.02, 1.0),
            );
            let drag = if matches!(self.target, Target::Picture { .. }) {
                markup::drag_square_crop
            } else {
                markup::drag_crop
            };
            self.live_crop = Some(drag(
                *start,
                *handle,
                f64::from((pos.x - from.x) / full.x),
                f64::from((pos.y - from.y) / full.y),
                min,
            ));
        }
        if stopped || !ui.input(|input| input.pointer.primary_down()) {
            self.gesture = None;
            if let Some(crop) = self.live_crop.take() {
                let next = Edit {
                    crop: markup::clamp_crop(crop, 0.02),
                    ..self.current().clone()
                };
                self.commit(next);
            }
        } else {
            ui.ctx().request_repaint();
        }
    }

    /// Arrow keys move the frame; with Shift they resize it from its corner.
    fn crop_keys(&mut self, ui: &egui::Ui) {
        if egui::Popup::is_any_open(ui.ctx()) {
            return;
        }
        let step = 0.01;
        let delta = ui.input_mut(|input| {
            [
                (egui::Key::ArrowLeft, (-step, 0.0)),
                (egui::Key::ArrowRight, (step, 0.0)),
                (egui::Key::ArrowUp, (0.0, -step)),
                (egui::Key::ArrowDown, (0.0, step)),
            ]
            .into_iter()
            .find_map(|(key, delta)| {
                if input.consume_key(egui::Modifiers::SHIFT, key) {
                    Some((Handle::SE, delta))
                } else if input.consume_key(egui::Modifiers::NONE, key) {
                    Some((Handle::Move, delta))
                } else {
                    None
                }
            })
        });
        if let Some((handle, (dx, dy))) = delta {
            let drag = if matches!(self.target, Target::Picture { .. }) {
                markup::drag_square_crop
            } else {
                markup::drag_crop
            };
            let crop = drag(self.current().crop, handle, dx, dy, (0.02, 0.02));
            let next = Edit {
                crop,
                ..self.current().clone()
            };
            self.commit(next);
        }
    }

    fn options(&mut self, ui: &mut egui::Ui, options: Rect, edge: f32) {
        if self.tool == Tool::Crop {
            let button = Rect::from_center_size(options.center(), egui::vec2(132.0, 36.0));
            if action_button(ui, button, NavIcon::RotateLeft, "Rotate left").clicked() {
                let next = self.current().rotate_left();
                self.commit(next);
            }
            return;
        }
        // Colours (and Draw's eraser), then the size: one row, or two on a narrow window.
        let draw = self.tool == Tool::Draw;
        let swatches_width = COLORS.len() as f32 * 32.0 + 32.0 + if draw { 44.0 } else { 0.0 };
        let (colors, size) = if options.width() < NARROW {
            let colors = Rect::from_center_size(
                egui::pos2(options.center().x, options.top() + 26.0),
                egui::vec2(swatches_width, 36.0),
            );
            let size = Rect::from_center_size(
                egui::pos2(options.center().x, options.top() + 66.0),
                egui::vec2(options.width() - 32.0, 36.0),
            );
            (colors, size)
        } else {
            let row = Rect::from_center_size(
                options.center(),
                egui::vec2(options.width().min(600.0) - 32.0, 36.0),
            );
            let colors = Rect::from_min_size(row.min, egui::vec2(swatches_width, 36.0));
            (
                colors,
                Rect::from_min_max(egui::pos2(colors.right() + 16.0, row.top()), row.max),
            )
        };
        let mut x = colors.left();
        for (color, name) in COLORS.into_iter().zip(COLOR_NAMES) {
            let rect = Rect::from_min_size(
                egui::pos2(x + 4.0, colors.top() + 6.0),
                egui::vec2(24.0, 24.0),
            );
            if swatch(ui, rect, color, name, !self.erasing && self.color == color).clicked() {
                self.choose_color(color);
            }
            x += 32.0;
        }
        let rect = Rect::from_min_size(
            egui::pos2(x + 4.0, colors.top() + 6.0),
            egui::vec2(24.0, 24.0),
        );
        let custom = !COLORS.contains(&self.color);
        let response = custom_swatch(ui, rect, self.color, custom && !self.erasing);
        let mut picked = {
            let [red, green, blue] = self.color;
            Color32::from_rgb(red, green, blue)
        };
        egui::Popup::from_toggle_button_response(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                egui::color_picker::color_picker_color32(
                    ui,
                    &mut picked,
                    egui::color_picker::Alpha::Opaque,
                )
            });
        let picked = [picked.r(), picked.g(), picked.b()];
        if picked != self.color {
            self.choose_color(picked);
        }
        if draw {
            let rect =
                Rect::from_min_size(egui::pos2(x + 40.0, colors.top()), egui::vec2(36.0, 36.0));
            if eraser_button(ui, rect, self.erasing).clicked() {
                self.erasing = !self.erasing;
            }
        }
        // The size, with a live preview of the dot or text and its value.
        let (min, max) = if draw {
            (STROKE_SIZES.0, STROKE_SIZES.1)
        } else {
            (TEXT_SIZES.0, TEXT_SIZES.1)
        };
        let mut value = if draw {
            self.stroke_points
        } else {
            self.field.as_ref().map_or(self.text_points, |field| {
                // Whole points, as the slider sets them: a size that
                // rounds away from its own value would read as a change.
                (field.size as f32 * edge).round().clamp(min, max)
            })
        };
        let preview = Rect::from_min_size(size.min, egui::vec2(44.0, 36.0));
        let [red, green, blue] = self.color;
        let color = Color32::from_rgb(red, green, blue);
        if draw && self.erasing {
            ui.painter()
                .circle_stroke(preview.center(), value / 2.0, Stroke::new(1.5, MUTED));
        } else if draw {
            ui.painter()
                .circle_filled(preview.center(), value / 2.0, color);
            ui.painter().circle_stroke(
                preview.center(),
                value / 2.0 + 0.5,
                Stroke::new(1.0, BORDER),
            );
        } else {
            ui.painter().text(
                preview.center(),
                egui::Align2::CENTER_CENTER,
                "Aa",
                egui::FontId::new(
                    value.min(32.0),
                    egui::FontFamily::Name("Satoshi Bold".into()),
                ),
                color,
            );
        }
        let label = Rect::from_min_size(
            egui::pos2(preview.right() + 8.0, size.top()),
            egui::vec2(32.0, 36.0),
        );
        ui.painter().text(
            label.left_center(),
            egui::Align2::LEFT_CENTER,
            "Size",
            egui::FontId::proportional(12.0),
            MUTED,
        );
        let slider = Rect::from_min_max(
            egui::pos2(label.right(), size.top()),
            egui::pos2(size.right() - 32.0, size.bottom()),
        );
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(slider), |ui| {
                ui.spacing_mut().slider_width = slider.width();
                crate::default_slider::default_slider(
                    ui,
                    &mut value,
                    min..=max,
                    if draw { 6.0 } else { 32.0 },
                    false,
                )
            })
            .inner;
        ui.painter().text(
            egui::pos2(size.right(), size.center().y),
            egui::Align2::RIGHT_CENTER,
            format!("{}", value.round()),
            egui::FontId::proportional(12.0),
            MUTED,
        );
        let size_label = match (draw, self.erasing) {
            (false, _) => "Text size",
            (true, true) => "Eraser size",
            (true, false) => "Line size",
        };
        response.widget_info(|| egui::WidgetInfo::slider(true, f64::from(value), size_label));
        if response.changed() {
            if draw {
                self.stroke_points = value;
            } else {
                self.text_points = value;
                if let Some(field) = &mut self.field {
                    field.size = f64::from(value / edge);
                    field.focus = true;
                }
            }
        }
    }

    fn choose_color(&mut self, color: [u8; 3]) {
        self.color = color;
        self.erasing = false;
        if let Some(field) = &mut self.field {
            field.color = color;
            field.focus = true;
        }
    }
}

/// One of the tool row's buttons.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PreviewTool {
    Spoiler,
    Edit(Tool),
    Remove,
}

/// Where the tool row goes, centred on `center_x` with its bottom
/// at `bottom`: Spoiler, then Crop, Draw and Text, then Remove when
/// `staged` (a staged file that can still change), or only the editor's
/// tools. Buttons narrow to fit `available` width. Returns the bordered
/// group, each button, and where the dividers between groups go.
pub fn preview_tool_layout(
    center_x: f32,
    bottom: f32,
    available: f32,
    staged: bool,
) -> (Rect, Vec<(PreviewTool, Rect)>, Vec<f32>) {
    let mut tools: Vec<PreviewTool> = Vec::new();
    if staged {
        tools.push(PreviewTool::Spoiler);
    }
    tools.extend(
        TOOL_BUTTONS
            .iter()
            .map(|(tool, ..)| PreviewTool::Edit(*tool)),
    );
    if staged {
        tools.push(PreviewTool::Remove);
    }
    let count = tools.len() as f32;
    let groups = if staged { 2.0 } else { 0.0 };
    let gaps = (count - 1.0 - groups) * PREVIEW_TOOL_GAP + groups * PREVIEW_GROUP_GAP;
    let width = ((available - 10.0 - gaps) / count).clamp(MIN_PREVIEW_TOOL, TOOL_WIDTH);
    let group = Rect::from_center_size(
        egui::pos2(center_x, bottom - PREVIEW_TOOLS_HEIGHT / 2.0),
        egui::vec2(count * width + gaps + 10.0, PREVIEW_TOOLS_HEIGHT),
    );
    let mut left = group.left() + 5.0;
    let mut buttons = Vec::new();
    let mut dividers = Vec::new();
    for (index, tool) in tools.iter().enumerate() {
        if index > 0 {
            let boundary = matches!(
                (tools[index - 1], tool),
                (PreviewTool::Spoiler, _) | (_, PreviewTool::Remove)
            );
            if boundary {
                dividers.push(left + PREVIEW_GROUP_GAP / 2.0 - 0.5);
                left += PREVIEW_GROUP_GAP;
            } else {
                left += PREVIEW_TOOL_GAP;
            }
        }
        buttons.push((
            *tool,
            Rect::from_min_size(egui::pos2(left, group.top() + 5.0), egui::vec2(width, 40.0)),
        ));
        left += width;
    }
    (group, buttons, dividers)
}

/// The file as it will be sent, which the editor's Open in default app opens.
#[derive(Clone)]
pub enum ToSend {
    /// Unedited: the original itself.
    Original,
    /// The edited file, prepared for the draft's upload.
    Prepared(Arc<uploads::Prepared>),
    /// The edit is still being prepared.
    Preparing,
    /// The edit couldn't be applied.
    Failed,
    /// A profile card's file, whose edit is applied only at Send: the
    /// preview prepares it itself, with these settings, when asked.
    AtSend(Compression),
}

/// A profile card's edit being prepared to open; closing the preview stops it.
struct Preparation {
    result: Receiver<Option<uploads::Prepared>>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Preparation {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// [`ToSend::AtSend`] once asked for.
enum Own {
    Preparing(Preparation),
    Ready(Arc<uploads::Prepared>),
    Failed,
}

/// The picture under a spoiler cover, as the people it is sent to see it
/// until they reveal it: blurred and dimmed over blackout, with a
/// crossed-out eye and "Spoiler" centred on what shows, at `opacity`.
fn paint_spoiler_cover(
    painter: &egui::Painter,
    blurred: &egui::TextureHandle,
    layout: &Layout,
    rotation: u16,
    stage: Rect,
    opacity: f32,
) {
    let placed = layout.rect;
    let shown = placed.intersect(stage);
    painter.rect_filled(placed, 0.0, BLACKOUT.gamma_multiply(opacity));
    paint_picture(
        painter,
        blurred,
        layout,
        rotation,
        Color32::WHITE.gamma_multiply(opacity),
    );
    painter.rect_filled(
        placed,
        0.0,
        Color32::from_black_alpha(70).gamma_multiply(opacity),
    );
    let text = painter.layout_no_wrap(
        "Spoiler".into(),
        egui::FontId::new(13.0, egui::FontFamily::Name("Satoshi Bold".into())),
        TEXT.gamma_multiply(opacity),
    );
    let content = 14.0 + 6.0 + text.size().x;
    let badge = Rect::from_center_size(shown.center(), egui::vec2(content + 24.0, 32.0));
    painter.rect(
        badge,
        8.0,
        BLACKOUT.gamma_multiply(0.9 * opacity),
        Stroke::new(1.0, BORDER.gamma_multiply(opacity)),
        egui::StrokeKind::Inside,
    );
    let left = badge.center().x - content / 2.0;
    crate::paint_icon(
        painter,
        Rect::from_center_size(
            egui::pos2(left + 7.0, badge.center().y),
            egui::vec2(14.0, 14.0),
        ),
        NavIcon::EyeOff,
        TEXT.gamma_multiply(opacity),
    );
    painter.galley(
        egui::pos2(left + 20.0, badge.center().y - text.size().y / 2.0),
        text,
        TEXT,
    );
}

/// The fitted (unzoomed) size of viewing's picture.
fn zoom_base(layout: &Layout, zoom: Zoom) -> Vec2 {
    layout.rect.size() / zoom.scale
}

/// The edit with an open text field applied, as committing it would.
fn with_field(edit: &Edit, field: Option<&Field>) -> Edit {
    let Some(field) = field else {
        return edit.clone();
    };
    let text: String = field.text.trim().chars().take(MAX_TEXT).collect();
    let mut marks = edit.marks.clone();
    match field.index {
        Some(index) => {
            let Some(Mark::Text {
                color,
                size,
                text: old,
                ..
            }) = marks.get(index)
            else {
                return edit.clone();
            };
            if text.is_empty() {
                marks.remove(index);
            } else if text != *old || field.color != *color || (field.size - size).abs() > 1e-9 {
                if let Some(Mark::Text {
                    color,
                    size,
                    text: old,
                    ..
                }) = marks.get_mut(index)
                {
                    (*color, *size, *old) = (field.color, field.size, text);
                }
            } else {
                return edit.clone();
            }
        }
        None if !text.is_empty() => marks.push(Mark::Text {
            color: field.color,
            size: field.size,
            x: field.x,
            y: field.y,
            text,
        }),
        None => return edit.clone(),
    }
    Edit {
        marks,
        ..edit.clone()
    }
}

/// The upright texture coordinates of a point in fractions of the rotated picture.
fn upright(rotation: u16, x: f64, y: f64) -> Pos2 {
    let (u, v) = match rotation {
        90 => (y, 1.0 - x),
        180 => (1.0 - x, 1.0 - y),
        270 => (1.0 - y, x),
        _ => (x, y),
    };
    egui::pos2(u as f32, v as f32)
}

fn paint_picture(
    painter: &egui::Painter,
    texture: &egui::TextureHandle,
    layout: &Layout,
    rotation: u16,
    tint: Color32,
) {
    let region = layout.region;
    let rect = layout.rect;
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for (pos, (x, y)) in [
        (rect.left_top(), (region.x, region.y)),
        (rect.right_top(), (region.x + region.width, region.y)),
        (
            rect.right_bottom(),
            (region.x + region.width, region.y + region.height),
        ),
        (rect.left_bottom(), (region.x, region.y + region.height)),
    ] {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: upright(rotation, x, y),
            color: tint,
        });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// Marks with the export's geometry: round-capped strokes (joined with
/// dots) and Satoshi Bold text centred on its point.
fn paint_marks(painter: &egui::Painter, layout: &Layout, marks: &[Mark]) {
    for mark in marks {
        let [red, green, blue] = mark.color();
        let color = Color32::from_rgb(red, green, blue);
        match mark {
            Mark::Stroke { width, points, .. } => {
                let line = *width as f32 * layout.edge;
                let points: Vec<Pos2> = points
                    .chunks_exact(2)
                    .map(|pair| layout.screen(pair[0], pair[1]))
                    .collect();
                if points.len() > 1 {
                    painter.add(egui::Shape::line(points.clone(), Stroke::new(line, color)));
                }
                for point in points {
                    painter.circle_filled(point, line / 2.0, color);
                }
            }
            Mark::Text {
                size, x, y, text, ..
            } => {
                if text.is_empty() {
                    continue;
                }
                let pixels = *size as f32 * layout.edge;
                if pixels < 0.5 {
                    continue;
                }
                let base = pixels.min(MAX_GLYPH_POINTS);
                let scale = pixels / base;
                let galley = painter.layout_no_wrap(
                    text.clone(),
                    egui::FontId::new(base, egui::FontFamily::Name("Satoshi Bold".into())),
                    color,
                );
                // The galley's own baseline, placed where the export puts it.
                let baseline = galley
                    .rows
                    .first()
                    .and_then(|row| row.row.glyphs.first().map(|glyph| row.pos.y + glyph.pos.y))
                    .unwrap_or(base * 0.8);
                let centre = layout.screen(*x, *y);
                let target = markup::text_metrics(text, pixels)
                    .map_or(centre.y, |metrics| metrics.baseline(centre.y));
                let top_left = egui::pos2(
                    centre.x - galley.size().x * scale / 2.0,
                    target - baseline * scale,
                );
                let mut shape = egui::Shape::galley(Pos2::ZERO, galley, color);
                shape.transform(egui::emath::TSTransform::new(top_left.to_vec2(), scale));
                painter.add(shape);
            }
        }
    }
}

/// A new stroke's width: `points` on screen, as a fraction of the picture's
/// longer edge shown `edge` points long.
fn stroke_width(points: f32, edge: f32) -> f64 {
    f64::from(points / edge)
}

/// How wide a stroke drawn now looks on screen, sized as strokes are, so
/// the brush ring matches it; never too small to see.
fn brush_diameter(points: f32, edge: f32) -> f32 {
    (stroke_width(points, edge) as f32 * edge).max(MIN_BRUSH)
}

/// The brush ring: its colour, faint, inside a white ring edged in dark on
/// both sides, so it reads on light and dark pictures alike.
fn paint_brush(painter: &egui::Painter, at: Pos2, diameter: f32, color: [u8; 3]) {
    let radius = diameter / 2.0;
    let [red, green, blue] = color;
    let edge = Stroke::new(1.0, BLACKOUT.gamma_multiply(0.7));
    painter.circle_filled(
        at,
        radius - 1.0,
        Color32::from_rgb(red, green, blue).gamma_multiply(0.3),
    );
    painter.circle_stroke(at, radius + 0.5, edge);
    painter.circle_stroke(at, radius - 0.75, Stroke::new(1.5, Color32::WHITE));
    painter.circle_stroke(at, radius - 2.0, edge);
}

fn paint_handle(painter: &egui::Painter, at: Pos2, handle: Handle) {
    let stroke = Stroke::new(3.0, TEXT);
    let length = 18.0;
    let (ax, ay) = handle.anchor();
    // Corners are L-shaped brackets; edges a short bar along the edge.
    let inward = egui::vec2(1.0 - ax * 2.0, 1.0 - ay * 2.0);
    if ax != 0.5 && ay != 0.5 {
        painter.line_segment([at, at + egui::vec2(inward.x * length, 0.0)], stroke);
        painter.line_segment([at, at + egui::vec2(0.0, inward.y * length)], stroke);
        painter.circle_filled(at, 1.5, TEXT);
    } else if ax == 0.5 {
        painter.line_segment(
            [
                at - egui::vec2(length / 2.0, 0.0),
                at + egui::vec2(length / 2.0, 0.0),
            ],
            stroke,
        );
    } else {
        painter.line_segment(
            [
                at - egui::vec2(0.0, length / 2.0),
                at + egui::vec2(0.0, length / 2.0),
            ],
            stroke,
        );
    }
}

fn icon_button(
    ui: &mut egui::Ui,
    rect: Rect,
    icon: NavIcon,
    label: &str,
    hover: &str,
    enabled: bool,
) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.id().with(("attachment-editor-button", label)),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let hot = enabled && (response.hovered() || response.has_focus());
    if hot {
        ui.painter().rect_filled(rect, 8.0, RAISED);
    }
    if response.has_focus() {
        ui.painter().rect_stroke(
            rect,
            8.0,
            Stroke::new(1.0, TERRACOTTA),
            egui::StrokeKind::Inside,
        );
    }
    crate::paint_icon(
        ui.painter(),
        Rect::from_center_size(rect.center(), egui::vec2(20.0, 20.0)),
        icon,
        if !enabled {
            Color32::from_white_alpha(60)
        } else if hot {
            TEXT
        } else {
            MUTED
        },
    );
    let label = label.to_owned();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &label));
    if enabled {
        response.on_hover_text(hover)
    } else {
        response
    }
}

fn text_button(ui: &mut egui::Ui, rect: Rect, label: &str, enabled: bool) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.id().with(("attachment-editor-text-button", label)),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let hot = enabled && (response.hovered() || response.has_focus());
    if hot {
        ui.painter().rect_filled(rect, 8.0, RAISED);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::new(13.0, egui::FontFamily::Name("Satoshi Bold".into())),
        if !enabled {
            Color32::from_white_alpha(60)
        } else if hot {
            TEXT
        } else {
            MUTED
        },
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label));
    response
}

/// Crop, Draw or Text: terracotta marks the one in use.
fn tool_button(
    ui: &mut egui::Ui,
    rect: Rect,
    icon: NavIcon,
    label: &str,
    selected: bool,
    enabled: bool,
) -> egui::Response {
    tool_button_toned(ui, rect, icon, label, selected, enabled, None)
}

/// A tool button; `tone` colours a destructive one, such as Remove. A
/// button too narrow for its label shows only its icon, still labelled
/// for assistive technology and on hover.
fn tool_button_toned(
    ui: &mut egui::Ui,
    rect: Rect,
    icon: NavIcon,
    label: &str,
    selected: bool,
    enabled: bool,
    tone: Option<Color32>,
) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.id().with(("attachment-editor-tool", label)),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let hot = enabled && (response.hovered() || response.has_focus());
    if selected {
        ui.painter().rect(
            rect,
            8.0,
            SURFACE,
            Stroke::new(1.0, TERRACOTTA),
            egui::StrokeKind::Inside,
        );
    } else if hot {
        ui.painter().rect_filled(rect, 8.0, RAISED);
    }
    let color = if !enabled {
        Color32::from_white_alpha(60)
    } else if selected {
        crate::TERRACOTTA_BRIGHT
    } else if let Some(tone) = tone {
        tone
    } else if hot {
        TEXT
    } else {
        MUTED
    };
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::new(13.0, egui::FontFamily::Name("Satoshi Bold".into())),
        color,
    );
    let labelled = 18.0 + 6.0 + galley.size().x <= rect.width() - 8.0;
    let width = if labelled {
        18.0 + 6.0 + galley.size().x
    } else {
        18.0
    };
    let left = rect.center().x - width / 2.0;
    crate::paint_icon(
        ui.painter(),
        Rect::from_min_size(
            egui::pos2(left, rect.center().y - 9.0),
            egui::vec2(18.0, 18.0),
        ),
        icon,
        color,
    );
    if labelled {
        ui.painter().galley(
            egui::pos2(left + 24.0, rect.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label)
    });
    if labelled {
        response
    } else {
        response.on_hover_text(label)
    }
}

/// Rotate left: a neutral bordered button with its icon.
fn action_button(ui: &mut egui::Ui, rect: Rect, icon: NavIcon, label: &str) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.id().with(("attachment-editor-action", label)),
        egui::Sense::click(),
    );
    let hot = response.hovered() || response.has_focus();
    ui.painter().rect(
        rect,
        8.0,
        if hot { RAISED } else { SURFACE },
        Stroke::new(
            1.0,
            if response.has_focus() {
                TERRACOTTA
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    let galley = ui.painter().layout_no_wrap(
        label.to_owned(),
        egui::FontId::new(13.0, egui::FontFamily::Name("Satoshi Bold".into())),
        TEXT,
    );
    let width = 18.0 + 6.0 + galley.size().x;
    let left = rect.center().x - width / 2.0;
    crate::paint_icon(
        ui.painter(),
        Rect::from_min_size(
            egui::pos2(left, rect.center().y - 9.0),
            egui::vec2(18.0, 18.0),
        ),
        icon,
        TEXT,
    );
    ui.painter().galley(
        egui::pos2(left + 24.0, rect.center().y - galley.size().y / 2.0),
        galley,
        TEXT,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, label));
    response
}

/// A preset colour; terracotta rings the one in use.
fn swatch(
    ui: &mut egui::Ui,
    rect: Rect,
    color: [u8; 3],
    name: &str,
    selected: bool,
) -> egui::Response {
    let response = ui.interact(
        rect.expand(4.0),
        ui.id().with(("attachment-editor-swatch", name)),
        egui::Sense::click(),
    );
    let [red, green, blue] = color;
    ui.painter().circle_filled(
        rect.center(),
        rect.width() / 2.0,
        Color32::from_rgb(red, green, blue),
    );
    ui.painter()
        .circle_stroke(rect.center(), rect.width() / 2.0, Stroke::new(1.0, BORDER));
    if selected || response.has_focus() {
        ui.painter().circle_stroke(
            rect.center(),
            rect.width() / 2.0 + 3.5,
            Stroke::new(2.0, TERRACOTTA),
        );
    } else if response.hovered() {
        ui.painter().circle_stroke(
            rect.center(),
            rect.width() / 2.0 + 3.0,
            Stroke::new(1.0, ACTION),
        );
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::RadioButton, true, selected, name)
    });
    response.on_hover_text(name)
}

/// Draw's eraser, after the colours: on, presses erase lines; a colour or
/// another tool turns it off.
fn eraser_button(ui: &mut egui::Ui, rect: Rect, selected: bool) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.id().with("attachment-editor-eraser"),
        egui::Sense::click(),
    );
    let hot = response.hovered() || response.has_focus();
    if selected {
        ui.painter()
            .rect_filled(rect, 8.0, TERRACOTTA.gamma_multiply(0.16));
    } else if hot {
        ui.painter().rect_filled(rect, 8.0, RAISED);
    }
    ui.painter().rect_stroke(
        rect,
        8.0,
        Stroke::new(
            1.0,
            if selected || response.has_focus() {
                TERRACOTTA
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    crate::paint_icon(
        ui.painter(),
        Rect::from_center_size(rect.center(), egui::vec2(18.0, 18.0)),
        NavIcon::Eraser,
        if selected || hot { TEXT } else { MUTED },
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, selected, "Eraser")
    });
    response.on_hover_text("Eraser")
}

/// The custom colour: a hue ring around the chosen colour, opening the picker.
fn custom_swatch(ui: &mut egui::Ui, rect: Rect, color: [u8; 3], selected: bool) -> egui::Response {
    let response = ui.interact(
        rect.expand(4.0),
        ui.id().with("attachment-editor-custom-colour"),
        egui::Sense::click(),
    );
    let painter = ui.painter();
    let centre = rect.center();
    let radius = rect.width() / 2.0;
    let steps = 24;
    for step in 0..steps {
        let angle = |step: usize| step as f32 / steps as f32 * std::f32::consts::TAU;
        let (start, end) = (angle(step), angle(step + 1) + 0.02);
        let hue = egui::ecolor::Hsva::new(step as f32 / steps as f32, 0.85, 0.95, 1.0);
        painter.add(egui::Shape::convex_polygon(
            vec![
                centre,
                centre + radius * egui::vec2(start.cos(), start.sin()),
                centre + radius * egui::vec2(end.cos(), end.sin()),
            ],
            Color32::from(hue),
            Stroke::NONE,
        ));
    }
    let [red, green, blue] = color;
    painter.circle(
        centre,
        radius - 5.0,
        if selected {
            Color32::from_rgb(red, green, blue)
        } else {
            BLACKOUT
        },
        Stroke::new(1.0, BLACKOUT),
    );
    if selected || response.has_focus() {
        painter.circle_stroke(centre, radius + 3.5, Stroke::new(2.0, TERRACOTTA));
    } else if response.hovered() {
        painter.circle_stroke(centre, radius + 3.0, Stroke::new(1.0, ACTION));
    }
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::ColorButton,
            true,
            selected,
            "Custom colour",
        )
    });
    response.on_hover_text("Custom colour")
}

fn color_image(image: &image::RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )
}

/// The upright still, downscaled for the screen, with its native size.
fn load_image(path: &Path) -> Option<Loaded> {
    let bytes = std::fs::read(path).ok()?;
    let (image, _) = crate::compress::decode_upright(&bytes)?;
    drop(bytes);
    let (width, height) = (image.width(), image.height());
    let shown = if width.max(height) > DISPLAY_EDGE {
        image.thumbnail(DISPLAY_EDGE, DISPLAY_EDGE)
    } else {
        image
    };
    Some(Loaded {
        image: shown.to_rgba8(),
        width,
        height,
        marks: true,
    })
}

/// A still from just after the start, as posters are, in the display orientation.
fn load_video(path: &Path) -> Option<Loaded> {
    let binary = crate::ffmpeg::binary()?;
    let probe = crate::ffmpeg::probe(binary, path).ok()?;
    let video = probe.video.as_ref()?;
    let frame = crate::ffmpeg::poster(binary, path, &probe, DISPLAY_EDGE)?;
    Some(Loaded {
        image: image::DynamicImage::ImageRgb8(frame).to_rgba8(),
        width: video.width,
        height: video.height,
        marks: crate::ffmpeg::overlays(binary),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_map_between_the_screen_and_the_picture() {
        let stage = Rect::from_min_size(Pos2::ZERO, egui::vec2(432.0, 232.0));
        // A 400×200 picture fits exactly inside 16 points of padding.
        let layout = Layout::new(stage, (400.0, 200.0), Crop::FULL, 16.0);
        assert_eq!(
            layout.rect,
            Rect::from_min_size(egui::pos2(16.0, 16.0), egui::vec2(400.0, 200.0))
        );
        assert_eq!(layout.edge, 400.0);
        assert_eq!(layout.fraction(egui::pos2(216.0, 66.0)), (0.5, 0.25));
        assert_eq!(layout.screen(0.5, 0.25), egui::pos2(216.0, 66.0));
        // Cropped to its right half, which is shown twice as large: sizes
        // stay fractions of the whole picture's edge on screen.
        let half = Crop {
            x: 0.5,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        };
        let layout = Layout::new(stage, (400.0, 200.0), half, 16.0);
        assert_eq!(layout.rect.size(), egui::vec2(200.0, 200.0));
        assert_eq!(layout.edge, 400.0);
        assert_eq!(layout.fraction(layout.rect.min), (0.5, 0.0));
        assert_eq!(layout.full(), egui::vec2(400.0, 200.0));
        assert_eq!(upright(0, 0.25, 0.5), egui::pos2(0.25, 0.5));
        // The rotated picture's top-left is the original's bottom-left after
        // a clockwise turn, and its top-right after an anticlockwise one.
        assert_eq!(upright(90, 0.0, 0.0), egui::pos2(0.0, 1.0));
        assert_eq!(upright(270, 0.0, 0.0), egui::pos2(1.0, 0.0));
        assert_eq!(upright(180, 0.0, 0.0), egui::pos2(1.0, 1.0));
    }

    #[test]
    fn the_editor_opens_on_the_chosen_tool_unless_a_video_cannot_take_marks() {
        let context = egui::Context::default();
        let open = |kind, tool| {
            Editor::open(
                &context,
                Target::Draft(1),
                kind,
                "clip".into(),
                Path::new("missing"),
                None,
                tool,
            )
        };
        let still = || Loaded {
            image: image::RgbaImage::new(4, 2),
            width: 4,
            height: 2,
            marks: false,
        };
        for tool in [Tool::Crop, Tool::Draw, Tool::Text] {
            assert_eq!(open(Kind::Image, Some(tool)).tool(), Some(tool));
            // A video keeps it until its FFmpeg turns out unable to mark.
            let mut editor = open(Kind::Video, Some(tool));
            assert_eq!(editor.tool(), Some(tool));
            let (sender, receiver) = mpsc::channel();
            editor.loading = Some(receiver);
            sender.send(Some(still())).unwrap();
            editor.receive(&context);
            assert_eq!(editor.tool(), Some(Tool::Crop), "{tool:?}");
        }
        // Without a tool it opens viewing, and stays so for any video.
        let mut editor = open(Kind::Video, None);
        assert_eq!(editor.tool(), None);
        let (sender, receiver) = mpsc::channel();
        editor.loading = Some(receiver);
        sender.send(Some(still())).unwrap();
        editor.receive(&context);
        assert_eq!(editor.tool(), None);
        let mut editor = open(Kind::Video, Some(Tool::Text));
        let (sender, receiver) = mpsc::channel();
        editor.loading = Some(receiver);
        sender
            .send(Some(Loaded {
                marks: true,
                ..still()
            }))
            .unwrap();
        editor.receive(&context);
        assert_eq!(editor.tool(), Some(Tool::Text));
    }

    #[test]
    fn the_brush_ring_is_as_wide_as_a_stroke_drawn_now() {
        // Strokes keep a fraction of the picture's longer edge and show it
        // at whatever size the picture is on screen, so the ring is the
        // chosen size wherever the picture is cropped or scaled.
        for edge in [180.0, 400.0, 1234.5] {
            for points in [6.0, 17.0, STROKE_SIZES.1] {
                let line = stroke_width(points, edge) as f32 * edge;
                assert!((brush_diameter(points, edge) - line).abs() < 1e-3);
                assert!((line - points).abs() < 1e-3);
            }
            assert_eq!(brush_diameter(STROKE_SIZES.0, edge), MIN_BRUSH);
        }
    }

    #[test]
    fn text_fields_commit_new_changed_and_emptied_text() {
        let field = |index, text: &str| Field {
            index,
            x: 0.25,
            y: 0.75,
            text: text.into(),
            color: COLORS[4],
            size: 0.05,
            focus: false,
        };
        let added = with_field(&Edit::default(), Some(&field(None, "  Look  ")));
        assert_eq!(
            added.marks,
            [Mark::Text {
                color: COLORS[4],
                size: 0.05,
                x: 0.25,
                y: 0.75,
                text: "Look".into()
            }]
        );
        assert_eq!(
            with_field(&Edit::default(), Some(&field(None, " "))),
            Edit::default()
        );
        // Opening a mark and leaving it as it was changes nothing.
        assert_eq!(with_field(&added, Some(&field(Some(0), "Look"))), added);
        let changed = with_field(&added, Some(&field(Some(0), "There")));
        assert!(matches!(&changed.marks[0], Mark::Text { text, .. } if text == "There"));
        assert!(
            with_field(&added, Some(&field(Some(0), "")))
                .marks
                .is_empty()
        );
        let long = "x".repeat(300);
        let capped = with_field(&Edit::default(), Some(&field(None, &long)));
        assert!(matches!(&capped.marks[0], Mark::Text { text, .. } if text.len() == MAX_TEXT));
    }

    #[test]
    fn preview_tools_put_spoiler_and_remove_either_side_of_the_editors_tools() {
        let (group, buttons, dividers) = preview_tool_layout(720.0, 880.0, 1408.0, true);
        let tools: Vec<_> = buttons.iter().map(|(tool, _)| *tool).collect();
        assert_eq!(
            tools,
            [
                PreviewTool::Spoiler,
                PreviewTool::Edit(Tool::Crop),
                PreviewTool::Edit(Tool::Draw),
                PreviewTool::Edit(Tool::Text),
                PreviewTool::Remove,
            ]
        );
        assert!((group.center().x - 720.0).abs() < 0.01);
        assert_eq!(group.bottom(), 880.0);
        // A divider between each group, inside the gap that holds it.
        assert_eq!(dividers.len(), 2);
        assert!(buttons[0].1.right() < dividers[0] && dividers[0] + 1.0 < buttons[1].1.left());
        assert!(buttons[3].1.right() < dividers[1] && dividers[1] + 1.0 < buttons[4].1.left());
        for (_, button) in &buttons {
            assert_eq!(button.size(), egui::vec2(TOOL_WIDTH, 40.0));
            assert!(group.shrink(4.0).contains_rect(*button));
        }
        // Without a staged file, only the editor's tools and no dividers.
        let (plain, buttons, dividers) = preview_tool_layout(720.0, 880.0, 1408.0, false);
        assert_eq!(buttons.len(), 3);
        assert!(dividers.is_empty());
        assert!((plain.center().x - 720.0).abs() < 0.01);
        // A narrow window narrows the buttons to fit.
        let (narrow, buttons, _) = preview_tool_layout(180.0, 600.0, 328.0, true);
        assert!(narrow.width() <= 328.0 + 0.01, "{narrow:?}");
        assert!(
            buttons
                .iter()
                .all(|(_, button)| button.width() < TOOL_WIDTH)
        );
    }

    /// A staged picture's editor, opened viewing; its picture is decoded
    /// once `send` is called.
    fn staged_editor(context: &egui::Context) -> (Editor, impl FnOnce()) {
        crate::configure(context);
        let mut editor = Editor::open(
            context,
            Target::Draft(1),
            Kind::Image,
            "photo.png".into(),
            Path::new("missing"),
            None,
            None,
        );
        let (sender, receiver) = mpsc::channel();
        editor.loading = Some(receiver);
        let send = move || {
            sender
                .send(Some(Loaded {
                    image: image::RgbaImage::from_pixel(640, 320, image::Rgba([200, 40, 40, 255])),
                    width: 640,
                    height: 320,
                    marks: true,
                }))
                .unwrap();
        };
        (editor, send)
    }

    const SCREEN: Vec2 = egui::vec2(1000.0, 700.0);

    /// One pass of `editor` at `time` seconds in a `screen`-sized window,
    /// with motion when `motion`; returns its output and outcome.
    fn pass_in(
        context: &egui::Context,
        editor: &mut Editor,
        time: f64,
        spoiler: Option<bool>,
        motion: bool,
        screen: Vec2,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Outcome) {
        context.enable_accesskit();
        let mut outcome = Outcome::Open;
        let output = context.run(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)),
                events,
                ..Default::default()
            },
            |context| {
                if motion {
                    crate::motion::enable(context);
                }
                outcome = editor.show(context, ToSend::Original, spoiler);
            },
        );
        (output, outcome)
    }

    fn editor_pass(
        context: &egui::Context,
        editor: &mut Editor,
        time: f64,
        spoiler: Option<bool>,
        motion: bool,
    ) -> egui::FullOutput {
        pass_in(context, editor, time, spoiler, motion, SCREEN, vec![]).0
    }

    /// Clicks `at` over two passes; returns the outcome of the release.
    fn click(
        context: &egui::Context,
        editor: &mut Editor,
        time: f64,
        spoiler: Option<bool>,
        screen: Vec2,
        at: Pos2,
    ) -> Outcome {
        let button = |pressed| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        pass_in(
            context,
            editor,
            time,
            spoiler,
            false,
            screen,
            vec![egui::Event::PointerMoved(at), button(true)],
        );
        pass_in(
            context,
            editor,
            time + 0.01,
            spoiler,
            false,
            screen,
            vec![button(false)],
        )
        .1
    }

    fn node(output: &egui::FullOutput, label: &str) -> Option<Rect> {
        output
            .platform_output
            .accesskit_update
            .as_ref()?
            .nodes
            .iter()
            .find(|(_, node)| node.label() == Some(label))
            .and_then(|(_, node)| node.bounds())
            .map(|bounds| {
                Rect::from_min_max(
                    egui::pos2(bounds.x0 as f32, bounds.y0 as f32),
                    egui::pos2(bounds.x1 as f32, bounds.y1 as f32),
                )
            })
    }

    /// The opacity of `texture` painted across `over`.
    fn mesh_opacity(
        output: &egui::FullOutput,
        texture: &egui::TextureHandle,
        over: Rect,
    ) -> Option<f32> {
        output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Mesh(mesh)
                    if mesh.texture_id == texture.id()
                        && (mesh.calc_bounds().center() - over.center()).length() < 1.0 =>
                {
                    Some(f32::from(mesh.vertices[0].color.a()) / 255.0)
                }
                _ => None,
            })
    }

    /// The spoiler cover's centred badge over `picture`.
    fn spoiler_badge(output: &egui::FullOutput, picture: Rect) -> Option<egui::epaint::RectShape> {
        output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect)
                    if rect.stroke.width == 1.0
                        && (rect.rect.center() - picture.center()).length() < 1.0
                        && rect.rect.width() < picture.width() =>
                {
                    Some(rect.clone())
                }
                _ => None,
            })
    }

    fn spoiler_text_over(output: &egui::FullOutput, badge: Rect) -> bool {
        output.shapes.iter().any(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => {
                text.galley.text() == "Spoiler" && badge.contains_rect(text.visual_bounding_rect())
            }
            _ => false,
        })
    }

    const VIEWING: &str = "Image photo.png";
    const EDITING: &str = "Image being edited";

    #[test]
    fn staged_files_open_viewing_on_one_screen_with_tools_chosen_in_place() {
        let context = egui::Context::default();
        let (mut editor, send) = staged_editor(&context);
        send();
        editor_pass(&context, &mut editor, 1.0, Some(false), false);
        let output = editor_pass(&context, &mut editor, 1.1, Some(false), false);
        assert_eq!(editor.tool(), None, "viewing");
        // The top bar, the same in every state.
        let bar = [CLOSE, OPEN_SENT, "Undo", "Redo", "Reset", "Done"];
        let top: Vec<_> = bar.map(|label| node(&output, label).expect(label)).into();
        // The tool row: Spoiler | Crop Draw Text | Remove.
        let row = ["Spoiler", "Crop", "Draw", "Text", "Remove"];
        let tools: Vec<_> = row.map(|label| node(&output, label).unwrap()).into();
        assert!(
            tools
                .windows(2)
                .all(|pair| pair[0].right() < pair[1].left())
        );
        let viewing = node(&output, VIEWING).expect("the picture");
        assert!(tools[0].top() > viewing.bottom());
        // Choosing Draw keeps the picture exactly where it was.
        click(
            &context,
            &mut editor,
            2.0,
            Some(false),
            SCREEN,
            tools[2].center(),
        );
        assert_eq!(editor.tool(), Some(Tool::Draw));
        let output = editor_pass(&context, &mut editor, 2.1, Some(false), false);
        assert_eq!(node(&output, EDITING), Some(viewing));
        assert!(node(&output, VIEWING).is_none());
        for (label, rect) in bar.iter().zip(&top) {
            assert_eq!(node(&output, label).as_ref(), Some(rect), "{label}");
        }
        for (label, rect) in row.iter().zip(&tools) {
            assert_eq!(node(&output, label).as_ref(), Some(rect), "{label}");
        }
        // Crop too, for an uncropped picture; Text switches in place.
        click(
            &context,
            &mut editor,
            3.0,
            Some(false),
            SCREEN,
            tools[1].center(),
        );
        assert_eq!(editor.tool(), Some(Tool::Crop));
        let output = editor_pass(&context, &mut editor, 3.1, Some(false), false);
        assert_eq!(node(&output, EDITING), Some(viewing));
        assert!(node(&output, "Rotate left").is_some());
        click(
            &context,
            &mut editor,
            4.0,
            Some(false),
            SCREEN,
            tools[3].center(),
        );
        assert_eq!(editor.tool(), Some(Tool::Text));
        // Choosing it again returns to viewing.
        click(
            &context,
            &mut editor,
            5.0,
            Some(false),
            SCREEN,
            tools[3].center(),
        );
        assert_eq!(editor.tool(), None);
        let output = editor_pass(&context, &mut editor, 5.1, Some(false), false);
        assert_eq!(node(&output, VIEWING), Some(viewing));
        assert!(
            node(&output, "Rotate left").is_none(),
            "no options while viewing"
        );
        // Spoiler toggles without leaving; Remove asks to remove and close.
        assert!(matches!(
            click(
                &context,
                &mut editor,
                6.0,
                Some(false),
                SCREEN,
                tools[0].center()
            ),
            Outcome::Spoiler
        ));
        assert_eq!(editor.tool(), None);
        assert!(matches!(
            click(
                &context,
                &mut editor,
                7.0,
                Some(false),
                SCREEN,
                tools[4].center()
            ),
            Outcome::Remove
        ));
        // Done with nothing changed just closes.
        assert!(matches!(
            click(
                &context,
                &mut editor,
                8.0,
                Some(false),
                SCREEN,
                top[5].center()
            ),
            Outcome::Cancel
        ));
    }

    #[test]
    fn a_narrow_window_keeps_the_options_height_while_viewing() {
        let context = egui::Context::default();
        let (mut editor, send) = staged_editor(&context);
        send();
        let narrow = egui::vec2(480.0, 760.0);
        let pass = |editor: &mut Editor, time| {
            pass_in(&context, editor, time, Some(false), false, narrow, vec![]).0
        };
        pass(&mut editor, 1.0);
        let output = pass(&mut editor, 1.1);
        let viewing = node(&output, VIEWING).unwrap();
        let draw = node(&output, "Draw").unwrap();
        click(
            &context,
            &mut editor,
            2.0,
            Some(false),
            narrow,
            draw.center(),
        );
        assert_eq!(editor.tool(), Some(Tool::Draw));
        let output = pass(&mut editor, 2.1);
        assert_eq!(node(&output, EDITING), Some(viewing));
        // Colours and the size take a row each, inside the reserved strip.
        let size = node(&output, "Line size").expect("the size");
        assert!(size.bottom() < draw.top() && size.top() > viewing.bottom());
    }

    #[test]
    fn a_spoiler_shows_covered_only_while_viewing() {
        let context = egui::Context::default();
        let (mut editor, send) = staged_editor(&context);
        send();
        editor_pass(&context, &mut editor, 0.9, Some(false), false);
        let output = editor_pass(&context, &mut editor, 1.0, Some(false), false);
        let picture = node(&output, VIEWING).expect("the picture");
        let original = editor.picture.as_ref().unwrap().texture.clone();
        let cover = editor.cover.clone().expect("a blurred cover");
        assert_eq!(mesh_opacity(&output, &original, picture), Some(1.0));
        assert!(mesh_opacity(&output, &cover, picture).is_none());
        assert!(spoiler_badge(&output, picture).is_none());

        // Marked, the picture is covered at once without motion: blurred,
        // dimmed, and the crossed-out eye with "Spoiler" in a blackout badge.
        let output = editor_pass(&context, &mut editor, 1.1, Some(true), false);
        let label = format!("{VIEWING}{SPOILER_LABEL}");
        assert_eq!(node(&output, &label), Some(picture));
        assert_eq!(mesh_opacity(&output, &cover, picture), Some(1.0));
        let shape = spoiler_badge(&output, picture).expect("the badge");
        assert_eq!(shape.fill, BLACKOUT.gamma_multiply(0.9));
        assert_eq!(shape.stroke, Stroke::new(1.0, BORDER));
        assert_eq!(shape.corner_radius, egui::CornerRadius::same(8));
        assert!(spoiler_text_over(&output, shape.rect));
        assert!(picture.contains_rect(shape.rect));

        // A chosen tool shows the picture plainly.
        let draw = node(&output, "Draw").unwrap();
        click(
            &context,
            &mut editor,
            2.0,
            Some(true),
            SCREEN,
            draw.center(),
        );
        let output = editor_pass(&context, &mut editor, 2.1, Some(true), false);
        assert_eq!(node(&output, EDITING), Some(picture));
        assert!(mesh_opacity(&output, &cover, picture).is_none());
        assert!(spoiler_badge(&output, picture).is_none());
        assert_eq!(mesh_opacity(&output, &original, picture), Some(1.0));
        // Back to viewing it is covered again; unmarked, uncovered.
        click(
            &context,
            &mut editor,
            3.0,
            Some(true),
            SCREEN,
            draw.center(),
        );
        let output = editor_pass(&context, &mut editor, 3.1, Some(true), false);
        assert!(spoiler_badge(&output, picture).is_some());
        let output = editor_pass(&context, &mut editor, 3.2, Some(false), false);
        assert!(mesh_opacity(&output, &cover, picture).is_none());
        assert!(spoiler_badge(&output, picture).is_none());
    }

    #[test]
    fn the_spoiler_cover_fades_in_and_out_with_motion() {
        let context = egui::Context::default();
        let (mut editor, send) = staged_editor(&context);
        send();
        // Past the editor's own arrival.
        editor_pass(&context, &mut editor, 1.0, Some(false), true);
        let output = editor_pass(&context, &mut editor, 3.0, Some(false), true);
        let picture = node(&output, VIEWING).unwrap();
        let cover = editor.cover.clone().unwrap();
        let opacity =
            |output: &egui::FullOutput| mesh_opacity(output, &cover, picture).unwrap_or(0.0);
        let base = f64::from(crate::motion::BASE);
        let at = |editor: &mut Editor, time, marked| {
            opacity(&editor_pass(&context, editor, time, Some(marked), true))
        };
        assert_eq!(at(&mut editor, 4.0, true), 0.0);
        let midway = at(&mut editor, 4.0 + base / 2.0, true);
        assert!(midway > 0.5 && midway < 1.0, "decelerating: {midway}");
        let output = editor_pass(&context, &mut editor, 4.0 + base, Some(true), true);
        assert_eq!(opacity(&output), 1.0);
        assert!(spoiler_badge(&output, picture).is_some());
        // Unmarked, it fades out from where it was.
        assert_eq!(at(&mut editor, 5.0, false), 1.0);
        let midway = at(&mut editor, 5.0 + base / 2.0, false);
        assert!(midway > 0.0 && midway < 0.5, "decelerating: {midway}");
        let output = editor_pass(&context, &mut editor, 5.0 + base, Some(false), true);
        assert_eq!(opacity(&output), 0.0);
        assert!(spoiler_badge(&output, picture).is_none());
        // Opened on a spoilered file, it starts covered, without a fade.
        let (mut covered, send) = staged_editor(&context);
        send();
        editor_pass(&context, &mut covered, 6.0, Some(true), true);
        let output = editor_pass(&context, &mut covered, 6.0 + base / 2.0, Some(true), true);
        let cover = covered.cover.clone().unwrap();
        // The picture itself is still fading in; the cover is fully on it.
        let arriving = mesh_opacity(&output, &cover, picture).unwrap();
        let texture = covered.picture.as_ref().unwrap().texture.clone();
        let original = mesh_opacity(&output, &texture, picture).unwrap();
        assert!(arriving > 0.5 && arriving < 1.0, "{arriving}");
        assert_eq!(arriving, original);
    }

    /// The bordered tool row's painted rect, near the bottom of the window.
    fn tool_row(output: &egui::FullOutput) -> Option<egui::epaint::RectShape> {
        output
            .shapes
            .iter()
            .find_map(|clipped| match &clipped.shape {
                egui::Shape::Rect(rect)
                    if rect.stroke.width > 0.9 && rect.rect.center().y > 600.0 =>
                {
                    Some(rect.clone())
                }
                _ => None,
            })
    }

    #[test]
    fn the_editor_arrives_with_a_short_fade_and_its_picture_fades_in_over_the_spinner() {
        let context = egui::Context::default();
        let (mut editor, send) = staged_editor(&context);
        let base = f64::from(crate::motion::BASE);
        // Opening: faded out and just smaller, then settling in (after
        // the modal's first, sizing pass).
        editor_pass(&context, &mut editor, 1.0, Some(false), true);
        let row = |editor: &mut Editor, time| {
            tool_row(&editor_pass(&context, editor, time, Some(false), true)).unwrap()
        };
        let first = row(&mut editor, 1.0);
        let midway = row(&mut editor, 1.0 + base / 2.0);
        let landed = row(&mut editor, 1.0 + base);
        assert_eq!(first.fill.a(), 0);
        assert!((first.rect.width() - landed.rect.width() * 0.98).abs() < 0.01);
        assert!(
            midway.rect.width() > first.rect.width() && midway.rect.width() < landed.rect.width()
        );
        assert!(midway.fill.a() > 0 && midway.fill.a() < landed.fill.a());
        assert_eq!(landed.fill, SURFACE);
        // Waiting, a spinner; once decoded the picture fades in over it.
        let spinner = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .find_map(|clipped| match &clipped.shape {
                    egui::Shape::Path(path) => Some(path.stroke.color.clone()),
                    _ => None,
                })
        };
        let waiting = editor_pass(&context, &mut editor, 2.0, Some(false), true);
        assert!(spinner(&waiting).is_some() && editor.picture.is_none());
        send();
        let output = editor_pass(&context, &mut editor, 3.0, Some(false), true);
        let picture = node(&output, VIEWING).unwrap();
        let original = editor.picture.as_ref().unwrap().texture.clone();
        assert_eq!(mesh_opacity(&output, &original, picture), Some(0.0));
        let output = editor_pass(&context, &mut editor, 3.0 + base / 2.0, Some(false), true);
        let shown = mesh_opacity(&output, &original, picture).unwrap();
        assert!(shown > 0.5 && shown < 1.0, "{shown}");
        assert!(spinner(&output).is_some_and(
            |color| matches!(color, egui::epaint::ColorMode::Solid(color) if color.a() > 0)
        ));
        let output = editor_pass(&context, &mut editor, 3.0 + base, Some(false), true);
        assert_eq!(mesh_opacity(&output, &original, picture), Some(1.0));
        assert!(spinner(&output).is_none());
        // With reduced motion it is all there at once (the app also turns
        // egui's own fades off then).
        let context = egui::Context::default();
        let (mut still, send) = staged_editor(&context);
        context.all_styles_mut(|style| style.animation_time = 0.0);
        send();
        let output = editor_pass(&context, &mut still, 1.0, Some(false), false);
        assert_eq!(tool_row(&output), None, "the sizing pass");
        let output = editor_pass(&context, &mut still, 1.0, Some(false), false);
        assert_eq!(tool_row(&output).unwrap().fill, SURFACE);
        let texture = still.picture.as_ref().unwrap().texture.clone();
        assert_eq!(mesh_opacity(&output, &texture, picture), Some(1.0));
    }

    #[test]
    fn a_profile_picture_adjusts_on_crop_without_spoiler_or_remove() {
        let context = egui::Context::default();
        crate::configure(&context);
        let mut editor = Editor::open(
            &context,
            Target::Picture {
                destination: crate::pictures::Target::Avatar,
                epoch: 0,
                path: PathBuf::from("missing"),
                scratch: None,
            },
            Kind::Image,
            "Picture".into(),
            Path::new("missing"),
            None,
            Some(Tool::Crop),
        );
        let (sender, receiver) = mpsc::channel();
        editor.loading = Some(receiver);
        sender
            .send(Some(Loaded {
                image: image::RgbaImage::new(64, 64),
                width: 64,
                height: 64,
                marks: true,
            }))
            .unwrap();
        editor_pass(&context, &mut editor, 1.0, None, false);
        let output = editor_pass(&context, &mut editor, 1.1, None, false);
        assert_eq!(editor.tool(), Some(Tool::Crop));
        assert!(node(&output, "Apply crop").is_some());
        assert!(node(&output, "Spoiler").is_none() && node(&output, "Remove").is_none());
        assert!(node(&output, OPEN_SENT).is_none());
        let crop = node(&output, "Crop").unwrap();
        click(&context, &mut editor, 2.0, None, SCREEN, crop.center());
        assert_eq!(editor.tool(), Some(Tool::Crop), "it stays on Crop");
        if let Target::Picture { destination, .. } = &mut editor.target {
            *destination = crate::pictures::Target::SpaceIcon("space".into());
        }
        let output = editor_pass(&context, &mut editor, 2.1, None, false);
        assert!(node(&output, "Done").is_some());
        assert!(node(&output, "Apply crop").is_none());
    }
}
