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

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Vec2};

use crate::attachment_edit::{
    self as markup, COLOR_NAMES, COLORS, Crop, Edit, Handle, Kind, MAX_TEXT, Mark, STROKE_SIZES,
    TEXT_SIZES,
};
use crate::{ACTION, BLACKOUT, BORDER, MUTED, NavIcon, RAISED, SURFACE, TERRACOTTA, TEXT};

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
pub const VIDEO_MARKS_UNAVAILABLE: &str =
    "Drawing on videos needs a newer FFmpeg than this build includes.";
const VIDEO_HINT: &str = "Drawings and text stay on screen for the whole video.";
const VIDEO_CROP_HINT: &str = "Crop and rotation apply to the whole video.";

/// The draft being edited.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A conversation or thread composer's file, by its upload key.
    Draft(u64),
    /// A profile card's staged file, by its place and path.
    Profile {
        person: String,
        index: usize,
        path: PathBuf,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Crop,
    Draw,
    Text,
}

pub enum Outcome {
    Open,
    Cancel,
    /// A change; `None` returns the draft to its original.
    Save(Option<Edit>),
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
    initial: Edit,
    states: Vec<Edit>,
    index: usize,
    tool: Tool,
    color: [u8; 3],
    stroke_points: f32,
    text_points: f32,
    picture: Option<Picture>,
    loading: Option<Receiver<Option<Loaded>>>,
    failed: bool,
    marks: bool,
    gesture: Option<Gesture>,
    live_crop: Option<Crop>,
    live_text: Option<(usize, f64, f64)>,
    field: Option<Field>,
    opened: bool,
}

impl Editor {
    /// Starts decoding `path` off the UI thread; `edit` is what the draft has.
    pub fn open(
        context: &egui::Context,
        target: Target,
        kind: Kind,
        name: String,
        path: &Path,
        edit: Option<Edit>,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let (path, repaint) = (path.to_owned(), context.clone());
        std::thread::spawn(move || {
            let _ = sender.send(match kind {
                Kind::Image => load_image(&path),
                Kind::Video => load_video(&path),
            });
            repaint.request_repaint();
        });
        let initial = edit.unwrap_or_default();
        Self {
            target,
            kind,
            name,
            states: vec![initial.clone()],
            initial,
            index: 0,
            tool: Tool::Draw,
            color: COLORS[0],
            stroke_points: STROKE_SIZES.2,
            text_points: TEXT_SIZES.2,
            picture: None,
            loading: Some(receiver),
            failed: false,
            marks: kind == Kind::Image,
            gesture: None,
            live_crop: None,
            live_text: None,
            field: None,
            opened: false,
        }
    }

    #[cfg(test)]
    pub fn loading(&self) -> bool {
        self.loading.is_some()
    }

    #[cfg(test)]
    pub fn tool(&self) -> Tool {
        self.tool
    }

    /// Opens on another tool (layout fixtures).
    pub fn choose(&mut self, tool: Tool) {
        self.tool = tool;
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

    /// Adds a change as one undo step, dropping anything undone.
    fn commit(&mut self, next: Edit) {
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
                self.picture = Some(Picture {
                    texture: context.load_texture(
                        "attachment-editor",
                        color_image(&loaded.image),
                        egui::TextureOptions::LINEAR,
                    ),
                    pixels: loaded.image,
                    width: loaded.width,
                    height: loaded.height,
                });
            }
            None => self.failed = true,
        }
    }

    /// Draws the editor over everything; returns what the person chose.
    pub fn show(&mut self, context: &egui::Context) -> Outcome {
        if !self.opened {
            self.opened = true;
            // The composer below must not keep typing.
            if let Some(focused) = context.memory(|memory| memory.focused()) {
                context.memory_mut(|memory| memory.surrender_focus(focused));
            }
        }
        self.receive(context);
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
            .show(context, |ui| self.contents(ui, screen));
        crate::motion::modal(context, modal.response.layer_id);
        // Nothing below sees Escape while the editor is open.
        context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        modal.inner
    }

    fn contents(&mut self, ui: &mut egui::Ui, screen: Rect) -> Outcome {
        let (rect, _) = ui.allocate_exact_size(screen.size(), egui::Sense::hover());
        ui.painter().rect_filled(rect, 0.0, BLACKOUT);
        let mut outcome = Outcome::Open;
        let top = Rect::from_min_size(rect.min, egui::vec2(rect.width(), TOP_BAR));
        let tools = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - TOOLS), rect.max);
        let options_height = if rect.width() < NARROW && self.tool != Tool::Crop {
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
        let edge = layout.map_or(1.0, |layout| layout.edge);
        let ready = self.picture.is_some();

        // Top bar: Cancel, the title and name, Undo, Redo, Reset and Done.
        let painter = ui.painter_at(rect);
        painter.line_segment(
            [top.left_bottom(), top.right_bottom()],
            Stroke::new(1.0, BORDER),
        );
        let cancel = Rect::from_center_size(
            egui::pos2(top.left() + 16.0 + 18.0, top.center().y),
            egui::vec2(36.0, 36.0),
        );
        if icon_button(ui, cancel, NavIcon::Close, "Cancel editing", "Cancel", true).clicked() {
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
        let done = place(64.0);
        let done_response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(done), |ui| {
                crate::primary_button(ui, "Done", ready)
            })
            .inner;
        done_response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, ready, "Done"));
        if done_response.clicked() {
            let fin = with_field(self.current(), self.field.as_ref());
            outcome = if fin == self.initial {
                Outcome::Cancel
            } else {
                Outcome::Save(fin.is_edited().then_some(fin))
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
        let title_left = cancel.right() + 12.0;
        let title_width = (undo.left() - 12.0 - title_left).max(0.0);
        let mut job = egui::text::LayoutJob::simple_singleline(
            if self.kind == Kind::Video {
                "Edit video".into()
            } else {
                "Edit image".into()
            },
            egui::FontId::new(14.0, egui::FontFamily::Name("Satoshi Bold".into())),
            TEXT,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(title_width);
        let title = ui.painter().layout_job(job);
        let mut job = egui::text::LayoutJob::simple_singleline(
            self.name.clone(),
            egui::FontId::proportional(12.0),
            MUTED,
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(title_width);
        let name = ui.painter().layout_job(job);
        let height = title.size().y + name.size().y;
        let title_top = top.center().y - height / 2.0;
        painter.galley(egui::pos2(title_left, title_top), title.clone(), TEXT);
        painter.galley(
            egui::pos2(title_left, title_top + title.size().y),
            name,
            MUTED,
        );

        // The stage: the picture, its marks and the crop frame.
        if self.failed {
            painter.text(
                stage.center(),
                egui::Align2::CENTER_CENTER,
                "This file can’t be edited here.",
                egui::FontId::proportional(14.0),
                MUTED,
            );
        } else if let Some(layout) = layout {
            self.stage(ui, stage, layout, options);
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

        // The active tool's options, then the tools.
        painter.line_segment(
            [options.left_top(), options.right_top()],
            Stroke::new(1.0, BORDER),
        );
        if ready {
            self.options(ui, options, edge);
        }
        let width = 96.0;
        let start = tools.center().x - width * 1.5 - 8.0;
        for (index, (tool, icon, label)) in [
            (Tool::Crop, NavIcon::Crop, "Crop"),
            (Tool::Draw, NavIcon::Brush, "Draw"),
            (Tool::Text, NavIcon::Type, "Text"),
        ]
        .into_iter()
        .enumerate()
        {
            let button = Rect::from_min_size(
                egui::pos2(start + index as f32 * (width + 8.0), tools.top() + 8.0),
                egui::vec2(width, 40.0),
            );
            let enabled = ready && (tool == Tool::Crop || self.marks);
            let response = tool_button(ui, button, icon, label, self.tool == tool, enabled);
            // Only Draw and Text are ever unavailable, and only for videos.
            let response = if ready && !enabled {
                response.on_hover_text(VIDEO_MARKS_UNAVAILABLE)
            } else {
                response
            };
            if response.clicked() && enabled && self.tool != tool {
                self.commit_field();
                self.gesture = None;
                self.tool = tool;
            }
        }
        outcome
    }

    fn layout(&self, stage: Rect) -> Option<Layout> {
        let picture = self.picture.as_ref()?;
        let (width, height) =
            markup::rotated_size(picture.width, picture.height, self.current().rotation);
        let (region, padding) = if self.tool == Tool::Crop {
            (Crop::FULL, CROP_PADDING)
        } else {
            (self.current().crop, PADDING)
        };
        Some(Layout::new(
            stage,
            (width as f32, height as f32),
            region,
            padding,
        ))
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
        if let Some(index) = self.field.as_ref().and_then(|field| field.index)
            && index < shown.marks.len()
        {
            shown.marks.remove(index);
        }
        if let Some(Gesture::Draw { mark, .. }) = &self.gesture {
            shown.marks.push(mark.clone());
        }
        shown
    }

    fn stage(&mut self, ui: &mut egui::Ui, stage: Rect, layout: Layout, options: Rect) {
        let Some(picture) = &self.picture else {
            return;
        };
        let shown = self.shown();
        let painter = ui.painter_at(stage);
        paint_picture(&painter, &picture.texture, &layout, shown.rotation);
        paint_marks(&ui.painter_at(layout.rect), &layout, &shown.marks);
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
        let response = match self.tool {
            Tool::Draw => response.on_hover_cursor(egui::CursorIcon::Crosshair),
            Tool::Text => response.on_hover_cursor(egui::CursorIcon::Text),
            Tool::Crop => response,
        };
        match self.tool {
            Tool::Crop => self.crop_frame(ui, &layout, shown.crop),
            Tool::Draw => self.draw(ui, &response, &layout),
            Tool::Text => {
                self.text(ui, &response, &layout);
                self.field(ui, &layout, options);
            }
        }
    }

    fn draw(&mut self, ui: &egui::Ui, response: &egui::Response, layout: &Layout) {
        if response.drag_started()
            && let Some(press) = ui.input(|input| input.pointer.press_origin())
        {
            let (x, y) = layout.fraction(press);
            self.gesture = Some(Gesture::Draw {
                mark: Mark::Stroke {
                    color: self.color,
                    width: f64::from(self.stroke_points / layout.edge),
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
            self.live_crop = Some(markup::drag_crop(
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
            let crop = markup::drag_crop(self.current().crop, handle, dx, dy, (0.02, 0.02));
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
        // Colours, then the size: one row, or two on a narrow window.
        let swatches_width = COLORS.len() as f32 * 32.0 + 32.0;
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
            if swatch(ui, rect, color, name, self.color == color).clicked() {
                self.choose_color(color);
            }
            x += 32.0;
        }
        let rect = Rect::from_min_size(
            egui::pos2(x + 4.0, colors.top() + 6.0),
            egui::vec2(24.0, 24.0),
        );
        let custom = !COLORS.contains(&self.color);
        let response = custom_swatch(ui, rect, self.color, custom);
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
        // The size, with a live preview of the dot or text.
        let draw = self.tool == Tool::Draw;
        let (min, max) = if draw {
            (STROKE_SIZES.0, STROKE_SIZES.1)
        } else {
            (TEXT_SIZES.0, TEXT_SIZES.1)
        };
        let mut value = if draw {
            self.stroke_points
        } else {
            self.field.as_ref().map_or(self.text_points, |field| {
                (field.size as f32 * edge).clamp(min, max)
            })
        };
        let preview = Rect::from_min_size(size.min, egui::vec2(44.0, 36.0));
        let [red, green, blue] = self.color;
        let color = Color32::from_rgb(red, green, blue);
        if draw {
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
        let slider = Rect::from_min_max(egui::pos2(label.right(), size.top()), size.max);
        let response = ui
            .scope_builder(egui::UiBuilder::new().max_rect(slider), |ui| {
                ui.spacing_mut().slider_width = slider.width();
                ui.add(
                    egui::Slider::new(&mut value, min..=max)
                        .show_value(false)
                        .step_by(1.0),
                )
            })
            .inner;
        let size_label = if draw { "Line size" } else { "Text size" };
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
        if let Some(field) = &mut self.field {
            field.color = color;
            field.focus = true;
        }
    }
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
            color: Color32::WHITE,
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
    let width = 18.0 + 6.0 + galley.size().x;
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
    ui.painter().galley(
        egui::pos2(left + 24.0, rect.center().y - galley.size().y / 2.0),
        galley,
        color,
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, label)
    });
    response
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
}
