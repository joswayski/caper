//! Pre-send editor. The original file is immutable; this modal owns only an
//! edit description and a screen-sized render. Live swaps the upload only
//! after the replacement succeeds, and discards cancelled/late responses.
use crate::composer::Composer;
use crate::files::{D2d, line};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::{App, EV_FOCUS, EV_SCREEN, EV_TEXT};
use caper_client::attachment_edit::{self as edit, Crop, Edit, Handle, Kind, Mark, Still};
use caper_client::media::Textures;
use caper_client::uploads::{Prepared, Scratch};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::Controls::Dialogs::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::Result;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    View,
    Crop,
    Draw,
    Erase,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Title,
    Status,
    Canvas,
    Field,
    Cancel,
    Done,
    View,
    Crop,
    Rotate,
    Draw,
    Erase,
    Text,
    Undo,
    Redo,
    Reset,
    Color(usize),
    Smaller,
    Larger,
    AddText,
    CustomColor,
    Open,
}

impl Part {
    pub fn index(self) -> usize {
        match self {
            Self::Title => 0,
            Self::Status => 1,
            Self::Canvas => 2,
            Self::Field => 3,
            Self::Cancel => 4,
            Self::Done => 5,
            Self::View => 6,
            Self::Crop => 7,
            Self::Rotate => 8,
            Self::Draw => 9,
            Self::Erase => 10,
            Self::Text => 11,
            Self::Undo => 12,
            Self::Redo => 13,
            Self::Reset => 14,
            Self::Color(i) => 15 + i,
            Self::Smaller => 21,
            Self::Larger => 22,
            Self::AddText => 23,
            Self::CustomColor => 24,
            Self::Open => 25,
        }
    }
}

struct Gesture {
    before: Edit,
    start: (f64, f64),
    handle: Handle,
    text: Option<usize>,
}

pub struct Editor {
    pub key: u64,
    name: String,
    kind: Option<Kind>,
    loading: Receiver<Option<(Kind, Still)>>,
    still: Option<Still>,
    initial: Edit,
    edit: Edit,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    tool: Tool,
    gesture: Option<Gesture>,
    bitmap: Option<ID2D1Bitmap1>,
    repaint: bool,
    pub field: Composer,
    text_at: (f64, f64),
    text_index: Option<usize>,
    text_size: Option<f64>,
    color: [u8; 3],
    stroke: f32,
    size: f32,
    pub busy: bool,
    error: Option<String>,
    pub focus: Part,
    pressed: Option<Part>,
    controls: Vec<(Part, D2D_RECT_F)>,
    canvas: D2D_RECT_F,
    path: PathBuf,
    prepared: Option<Arc<Prepared>>,
    opening: Option<Receiver<std::result::Result<Arc<Scratch>, String>>>,
    color_picker: Option<Receiver<Option<[u8; 3]>>>,
}

fn inside(r: D2D_RECT_F, x: f32, y: f32) -> bool {
    x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
}

impl Editor {
    fn new(hwnd: HWND, key: u64, name: String, path: PathBuf, initial: Option<Edit>) -> Self {
        let (tx, loading) = channel();
        let window = hwnd.0 as isize;
        let source = path.clone();
        std::thread::spawn(move || {
            let loaded = edit::editable(&source).and_then(|kind| {
                let mut still = edit::load_still(&source, kind)?;
                // Only the display copy is reduced. Export re-reads the original.
                still.image = image::imageops::thumbnail(&still.image, 1200, 1200);
                Some((kind, still))
            });
            let _ = tx.send(loaded);
            unsafe {
                let _ = PostMessageW(
                    Some(HWND(window as _)),
                    crate::live::WM_LIVE,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
        });
        let initial = initial.unwrap_or_default();
        Self {
            key,
            name,
            kind: None,
            loading,
            still: None,
            edit: initial.clone(),
            initial,
            undo: Vec::new(),
            redo: Vec::new(),
            tool: Tool::View,
            gesture: None,
            bitmap: None,
            repaint: true,
            field: Composer::default(),
            text_at: (0.5, 0.5),
            text_index: None,
            text_size: None,
            color: edit::COLORS[0],
            stroke: edit::STROKE_SIZES.2,
            size: edit::TEXT_SIZES.2,
            busy: false,
            error: None,
            focus: Part::Cancel,
            pressed: None,
            controls: Vec::new(),
            canvas: rect(0.0, 0.0, 1.0, 1.0),
            path,
            prepared: None,
            opening: None,
            color_picker: None,
        }
    }

    fn remember(&mut self, before: Edit) {
        if before != self.edit {
            self.undo.push(before);
            self.redo.clear();
        }
        self.repaint = true;
    }

    fn with_text(&self) -> Edit {
        let mut edited = self.edit.clone();
        if self.tool != Tool::Text {
            return edited;
        }
        let text: String = String::from_utf16_lossy(&self.field.text)
            .chars()
            .filter(|c| !c.is_control())
            .take(edit::MAX_TEXT)
            .collect();
        if !text.trim().is_empty() {
            let mark = Mark::Text {
                color: self.color,
                size: self
                    .text_size
                    .unwrap_or(self.size as f64 / self.long_edge()),
                x: self.text_at.0,
                y: self.text_at.1,
                text,
            };
            if let Some(i) = self.text_index {
                edited.marks[i] = mark;
            } else {
                edited.marks.push(mark);
            }
        } else if let Some(i) = self.text_index {
            edited.marks.remove(i);
        }
        edited
    }

    fn commit_text(&mut self) {
        if self.tool != Tool::Text {
            return;
        }
        let before = self.edit.clone();
        self.edit = self.with_text();
        self.field.set_text(Vec::new());
        self.text_index = None;
        self.text_size = None;
        self.remember(before);
    }

    fn long_edge(&self) -> f64 {
        (self.canvas.right - self.canvas.left)
            .max(self.canvas.bottom - self.canvas.top)
            .max(1.0) as f64
    }

    fn point(&self, x: f32, y: f32) -> (f64, f64) {
        (
            ((x - self.canvas.left) / (self.canvas.right - self.canvas.left).max(1.0))
                .clamp(0.0, 1.0) as f64,
            ((y - self.canvas.top) / (self.canvas.bottom - self.canvas.top).max(1.0))
                .clamp(0.0, 1.0) as f64,
        )
    }

    fn crop_rect(&self) -> D2D_RECT_F {
        let c = self.edit.crop;
        let r = self.canvas;
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        rect(
            r.left + c.x as f32 * w,
            r.top + c.y as f32 * h,
            r.left + (c.x + c.width) as f32 * w,
            r.top + (c.y + c.height) as f32 * h,
        )
    }

    fn begin(&mut self, x: f32, y: f32) {
        if self.busy || self.still.is_none() || self.tool == Tool::View {
            return;
        }
        let point = self.point(x, y);
        let mut handle = Handle::Move;
        let mut text = None;
        if self.tool == Tool::Crop {
            let r = self.crop_rect();
            handle = Handle::EDGES
                .into_iter()
                .find(|h| {
                    let (a, b) = h.anchor();
                    (x - (r.left + a * (r.right - r.left))).abs() <= 12.0
                        && (y - (r.top + b * (r.bottom - r.top))).abs() <= 12.0
                })
                .unwrap_or(Handle::Move);
            if handle == Handle::Move && !inside(r, x, y) {
                return;
            }
        }
        if self.tool == Tool::Text {
            self.commit_text();
            // A text mark can be selected, reworded, recoloured and dragged.
            text = self.edit.marks.iter().enumerate().rev().find_map(|(i, m)| {
                let Mark::Text {
                    x: mx,
                    y: my,
                    size,
                    text,
                    ..
                } = m
                else {
                    return None;
                };
                let pixels = (*size * self.long_edge()) as f32;
                let metrics = edit::text_metrics(text, pixels)?;
                let cx = self.canvas.left + *mx as f32 * (self.canvas.right - self.canvas.left);
                let cy = self.canvas.top + *my as f32 * (self.canvas.bottom - self.canvas.top);
                ((x - cx).abs() < metrics.width / 2.0 + 8.0 && (y - cy).abs() < pixels / 2.0 + 8.0)
                    .then_some(i)
            });
            self.text_at = point;
            self.text_index = text;
            if let Some(i) = text
                && let Mark::Text {
                    text,
                    color,
                    size,
                    x,
                    y,
                } = &self.edit.marks[i]
            {
                self.field.set_text(text.encode_utf16().collect());
                self.color = *color;
                self.size = (*size * self.long_edge()) as f32;
                self.text_size = Some(*size);
                self.text_at = (*x, *y);
            }
            self.field.focused = true;
            self.focus = Part::Field;
        }
        self.gesture = Some(Gesture {
            before: self.edit.clone(),
            start: point,
            handle,
            text,
        });
        if self.tool == Tool::Draw {
            self.edit.marks.push(Mark::Stroke {
                color: self.color,
                width: self.stroke as f64 / self.long_edge(),
                points: vec![point.0, point.1],
            });
        }
        self.drag(x, y);
    }

    fn drag(&mut self, x: f32, y: f32) {
        let point = self.point(x, y);
        let Some(g) = &self.gesture else {
            return;
        };
        match self.tool {
            Tool::Crop => {
                let (w, h) = self
                    .still
                    .as_ref()
                    .map(|s| edit::rotated_size(s.width, s.height, self.edit.rotation))
                    .unwrap_or((1, 1));
                self.edit.crop = edit::drag_crop(
                    g.before.crop,
                    g.handle,
                    point.0 - g.start.0,
                    point.1 - g.start.1,
                    (1.0 / w.max(1) as f64, 1.0 / h.max(1) as f64),
                );
            }
            Tool::Draw => {
                if let Some(Mark::Stroke { points, .. }) = self.edit.marks.last_mut() {
                    points.extend([point.0, point.1]);
                }
            }
            Tool::Erase => {
                let w = (self.canvas.right - self.canvas.left) as f64;
                let h = (self.canvas.bottom - self.canvas.top) as f64;
                let touched = edit::strokes_touched(&self.edit.marks, point, 12.0, (w, h));
                for i in touched.into_iter().rev() {
                    self.edit.marks.remove(i);
                }
            }
            Tool::Text => {
                if let Some(i) = g.text
                    && let (Mark::Text { x, y, .. }, Mark::Text { x: bx, y: by, .. }) =
                        (&mut self.edit.marks[i], &g.before.marks[i])
                {
                    *x = (bx + point.0 - g.start.0).clamp(0.0, 1.0);
                    *y = (by + point.1 - g.start.1).clamp(0.0, 1.0);
                    self.text_at = (*x, *y);
                }
            }
            Tool::View => {}
        }
        self.repaint = true;
    }

    fn end(&mut self) {
        if let Some(g) = self.gesture.take() {
            self.remember(g.before);
        }
    }

    pub fn parts(&self) -> impl Iterator<Item = Part> + '_ {
        self.controls.iter().map(|(p, _)| *p)
    }
    pub fn part_rect(&self, p: Part) -> Option<(f32, f32, f32, f32)> {
        self.controls
            .iter()
            .find(|(q, _)| *q == p)
            .map(|(_, r)| (r.left, r.top, r.right - r.left, r.bottom - r.top))
    }
    pub fn text_origin(&self) -> (f32, f32) {
        self.part_rect(Part::Field)
            .map_or((0.0, 0.0), |(x, y, _, h)| {
                let (_, cy, ch) = self.field.caret_pos(self.field.display_caret());
                (x + 8.0, y + 8.0 - (cy + ch - (h - 16.0)).max(0.0))
            })
    }
    fn hit(&self, x: f32, y: f32) -> Option<Part> {
        self.controls
            .iter()
            .find(|(_, r)| inside(*r, x, y))
            .map(|(p, _)| *p)
    }
    pub fn enabled(&self, p: Part) -> bool {
        if p == Part::Cancel {
            return true;
        }
        if p == Part::Open {
            return !self.busy
                && self.opening.is_none()
                && (!self.initial.is_edited() || self.prepared.is_some());
        }
        if self.busy || self.still.is_none() {
            return false;
        }
        match p {
            Part::Title | Part::Status => false,
            Part::CustomColor => self.color_picker.is_none(),
            Part::Undo => !self.undo.is_empty(),
            Part::Redo => !self.redo.is_empty(),
            Part::Draw | Part::Erase | Part::Text => self.still.as_ref().is_some_and(|s| s.marks),
            _ => true,
        }
    }
    pub fn label(&self, p: Part) -> String {
        match p {
            Part::Title => self
                .kind
                .map_or("Attachment preview", |k| edit::title(k, false))
                .into(),
            Part::Status => self.error.clone().unwrap_or_else(|| {
                if self.busy {
                    "Preparing and uploading edit. Original kept until success."
                } else if self.still.is_none() {
                    "Loading preview…"
                } else if self.kind == Some(Kind::Video)
                    && !self.still.as_ref().is_some_and(|s| s.marks)
                {
                    "Drawing on videos needs a newer FFmpeg than this build includes."
                } else if self.kind == Some(Kind::Video) {
                    "Edits apply to the whole video; preview shows one frame."
                } else {
                    "Choose a tool. Done keeps changes; Cancel keeps the original draft."
                }
                .into()
            }),
            Part::Canvas => "Attachment preview canvas".into(),
            Part::Field => "Annotation text".into(),
            Part::Cancel => edit::CANCEL_EDITING.into(),
            Part::Done => "Done".into(),
            Part::View => "View".into(),
            Part::Crop => "Crop".into(),
            Part::Rotate => "Rotate left".into(),
            Part::Draw => "Draw".into(),
            Part::Erase => "Erase strokes".into(),
            Part::Text => "Text".into(),
            Part::Undo => "Undo".into(),
            Part::Redo => "Redo".into(),
            Part::Reset => "Reset".into(),
            Part::Color(i) => edit::COLOR_NAMES[i].into(),
            Part::Smaller => "Smaller".into(),
            Part::Larger => "Larger".into(),
            Part::AddText => "Apply text".into(),
            Part::CustomColor => "Custom color".into(),
            Part::Open => "Open in default app".into(),
        }
    }

    fn layout(&mut self, g: &Gfx, w: f32, h: f32) -> Result<()> {
        let mut controls = vec![(Part::Title, rect(16.0, 12.0, (w - 16.0).max(32.0), 58.0))];
        let mut buttons = vec![
            Part::Cancel,
            Part::Done,
            Part::View,
            Part::Crop,
            Part::Rotate,
            Part::Draw,
            Part::Erase,
            Part::Text,
            Part::Undo,
            Part::Redo,
            Part::Reset,
            Part::Open,
        ];
        // Reserve tool options even in viewing mode: choosing Draw or Text
        // must not move the picture under the pointer.
        buttons.extend((0..edit::COLORS.len()).map(Part::Color));
        buttons.extend([
            Part::CustomColor,
            Part::Smaller,
            Part::Larger,
            Part::AddText,
        ]);
        let (mut x, mut y) = (16.0, 68.0);
        for p in buttons {
            let width = match p {
                Part::Cancel => 132.0,
                Part::Open => 148.0,
                Part::CustomColor => 104.0,
                Part::Rotate | Part::Erase | Part::AddText => 104.0,
                _ => 72.0,
            };
            if x + width > w - 16.0 && x > 16.0 {
                x = 16.0;
                y += 38.0;
            }
            let shown = match p {
                Part::Color(_) | Part::CustomColor | Part::Smaller | Part::Larger => {
                    matches!(self.tool, Tool::Draw | Tool::Text)
                }
                Part::AddText => self.tool == Tool::Text,
                _ => true,
            };
            if shown {
                controls.push((p, rect(x, y, x + width, y + 32.0)));
            }
            x += width + 6.0;
        }
        y += 42.0;
        if self.tool == Tool::Text {
            controls.push((Part::Field, rect(16.0, y, (w - 16.0).max(32.0), y + 52.0)));
            self.field.relayout(g, (w - 48.0).max(1.0))?;
        }
        y += 60.0;
        controls.push((Part::Status, rect(16.0, y, (w - 16.0).max(32.0), y + 40.0)));
        y += 44.0;
        let (mut iw, mut ih) = self
            .still
            .as_ref()
            .map(|s| edit::rotated_size(s.width, s.height, self.edit.rotation))
            .unwrap_or((1, 1));
        if self.tool == Tool::View {
            iw = (iw as f64 * self.edit.crop.width).max(1.0) as u32;
            ih = (ih as f64 * self.edit.crop.height).max(1.0) as u32;
        }
        let scale = ((w - 32.0).max(1.0) / iw as f32).min((h - y - 16.0).max(1.0) / ih as f32);
        let (iw, ih) = (iw as f32 * scale, ih as f32 * scale);
        self.canvas = rect((w - iw) / 2.0, y, (w + iw) / 2.0, y + ih);
        controls.push((Part::Canvas, self.canvas));
        self.controls = controls;
        Ok(())
    }

    fn prepare(&mut self, g: &Gfx) {
        if !std::mem::take(&mut self.repaint) {
            return;
        }
        let Some(still) = &self.still else {
            return;
        };
        let mut shown = self.with_text();
        if self.tool != Tool::View {
            shown.crop = Crop::FULL;
        }
        if let Some(image) = edit::render(
            &image::DynamicImage::ImageRgba8(still.image.clone()),
            &shown,
        ) {
            let image = image.to_rgba8();
            self.bitmap = Some(D2d(g.dc.clone()).create(
                "attachment-editor",
                &caper_client::Rgba {
                    size: [image.width() as usize, image.height() as usize],
                    pixels: image.into_raw(),
                },
            ));
        } else {
            self.error = Some(edit::FAILED.into());
        }
    }

    pub fn draw(&self, g: &Gfx, dc: &ID2D1DeviceContext, w: f32) -> Result<()> {
        unsafe {
            dc.Clear(Some(&gfx::color(crate::theme::BLACKOUT, 1.0)));
        }
        for (p, r) in &self.controls {
            if *p == Part::Canvas {
                if let Some(bitmap) = &self.bitmap {
                    unsafe {
                        dc.DrawBitmap(
                            bitmap,
                            Some(r),
                            1.0,
                            D2D1_INTERPOLATION_MODE_LINEAR,
                            None,
                            None,
                        );
                    }
                }
                if self.tool == Tool::Crop {
                    let crop = self.crop_rect();
                    stroke_round(dc, crop, 0.0, &g.brushes.text);
                    for handle in Handle::EDGES {
                        let (x, y) = handle.anchor();
                        let (x, y) = (
                            crop.left + x * (crop.right - crop.left),
                            crop.top + y * (crop.bottom - crop.top),
                        );
                        fill_round(
                            dc,
                            rect(x - 4.0, y - 4.0, x + 4.0, y + 4.0),
                            2.0,
                            &g.brushes.text,
                        );
                    }
                }
                continue;
            }
            if *p == Part::Field {
                fill_round(dc, *r, 8.0, &g.brushes.surface);
                let (x, y) = self.text_origin();
                unsafe {
                    dc.PushAxisAlignedClip(r, D2D1_ANTIALIAS_MODE_ALIASED);
                }
                self.field.draw_text(g, dc, x, y, None);
                unsafe {
                    dc.PopAxisAlignedClip();
                }
                continue;
            }
            let text = self.label(*p);
            let passive = matches!(p, Part::Title | Part::Status);
            if !passive {
                fill_round(
                    dc,
                    *r,
                    8.0,
                    if *p == Part::Done && self.enabled(*p) {
                        &g.brushes.eggshell
                    } else {
                        &g.brushes.surface
                    },
                );
                stroke_round(
                    dc,
                    *r,
                    8.0,
                    if self.focus == *p
                        || matches!(
                            (self.tool, p),
                            (Tool::View, Part::View)
                                | (Tool::Crop, Part::Crop)
                                | (Tool::Draw, Part::Draw)
                                | (Tool::Erase, Part::Erase)
                                | (Tool::Text, Part::Text)
                        )
                        || matches!(p, Part::Color(i) if edit::COLORS[*i] == self.color)
                    {
                        &g.brushes.terracotta
                    } else {
                        &g.brushes.border
                    },
                );
            }
            let brush = if *p == Part::Done && self.enabled(*p) {
                &g.brushes.blackout
            } else if passive || self.enabled(*p) {
                &g.brushes.text
            } else {
                &g.brushes.muted
            };
            let l = line(
                g,
                &text,
                if *p == Part::Title {
                    &g.formats.name
                } else {
                    &g.formats.file_detail
                },
                (r.right - r.left - 12.0).max(1.0),
            )?;
            crate::text::draw(dc, &l, r.left + 6.0, r.top + 7.0, brush);
            if *p == Part::Title && w > edit::NAME_HIDES_AT {
                let l = line(g, &self.name, &g.formats.file_detail, (w - 40.0).max(1.0))?;
                unsafe {
                    l.SetWordWrapping(
                        windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP,
                    )?;
                    dc.PushAxisAlignedClip(r, D2D1_ANTIALIAS_MODE_ALIASED);
                }
                crate::text::draw(dc, &l, r.left + 6.0, r.top + 27.0, &g.brushes.muted);
                unsafe {
                    dc.PopAxisAlignedClip();
                }
            }
        }
        Ok(())
    }
}

impl App {
    /// Only local image/video files enter this editor. Other files keep the
    /// existing external-open route; animated image eligibility is checked
    /// by the shared decoder, never flattened silently.
    pub fn open_attachment_editor(&mut self, key: u64) -> bool {
        let Some(d) = self
            .live
            .as_ref()
            .and_then(|l| l.drafts.iter().find(|d| d.upload.key == key))
        else {
            return false;
        };
        let u = &d.upload;
        if !matches!(
            u.kind,
            caper_client::model::AttachmentKind::Image | caper_client::model::AttachmentKind::Video
        ) {
            return false;
        }
        let Some(path) = u.local_path.clone() else {
            return false;
        };
        let editor = Editor::new(self.hwnd, key, u.source_name.clone(), path, u.edit.clone());
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        self.close_menu();
        self.close_dialog();
        self.send_waiting = None;
        self.attachment_editor = Some(editor);
        self.attachment_editor_changed();
        self.events |= EV_SCREEN | EV_FOCUS;
        true
    }

    pub fn close_attachment_editor(&mut self) {
        if let Some(e) = self.attachment_editor.take() {
            if let Some(live) = &mut self.live {
                live.cancel_draft_edit(e.key);
            }
            unsafe {
                let _ = ReleaseCapture();
            }
            self.dirty.overlay = true;
            self.need = true;
            self.events |= EV_SCREEN | EV_FOCUS;
            self.set_composer_focus(true);
        }
    }

    pub fn attachment_editor_changed(&mut self) {
        if let Some(e) = &mut self.attachment_editor {
            e.field.caret_on = true;
            // The annotation field is plain text, single-line and bounded,
            // including paste, UIA Value and IME commits.
            if !e.field.composing {
                let mut text = Vec::new();
                for c in char::decode_utf16(e.field.text.iter().copied())
                    .filter(|c| !c.as_ref().is_ok_and(|c| c.is_control()))
                    .take(edit::MAX_TEXT)
                {
                    match c {
                        Ok(c) => text.extend_from_slice(c.encode_utf16(&mut [0; 2])),
                        // WM_CHAR delivers a surrogate pair in two messages,
                        // even when inserted in the middle of existing text.
                        Err(c) => text.push(c.unpaired_surrogate()),
                    }
                }
                if text != e.field.text {
                    e.field.set_text(text);
                }
            }
            if let Err(error) = e.layout(&self.g, self.w, self.h) {
                e.error = Some(format!("Could not lay out the editor: {error}"));
            }
            e.repaint |= e.tool == Tool::Text;
            e.prepare(&self.g);
        }
        self.dirty.overlay = true;
        self.events |= EV_TEXT;
        self.need = true;
        self.ime_position();
    }

    pub fn poll_attachment_editor(&mut self) {
        let Some(e) = &mut self.attachment_editor else {
            return;
        };
        if !self
            .live
            .as_ref()
            .is_some_and(|l| l.open_drafts().iter().any(|d| d.upload.key == e.key))
        {
            self.close_attachment_editor();
            return;
        }
        e.prepared = self
            .live
            .as_ref()
            .and_then(|l| l.drafts.iter().find(|d| d.upload.key == e.key))
            .and_then(|d| d.upload.prepared.clone());
        let mut changed = false;
        if let Some(rx) = &e.color_picker
            && let Ok(color) = rx.try_recv()
        {
            e.color_picker = None;
            if let Some(color) = color {
                e.color = color;
                e.repaint = true;
            }
            changed = true;
        }
        if let Some(rx) = &e.opening
            && let Ok(result) = rx.try_recv()
        {
            e.opening = None;
            match result {
                Ok(scratch) => {
                    crate::open_link(&scratch.path().to_string_lossy());
                    self.opened_attachments.push(scratch);
                }
                Err(error) => e.error = Some(error),
            }
            changed = true;
        }
        if let Ok(loaded) = e.loading.try_recv() {
            match loaded {
                Some((kind, still)) => {
                    e.kind = Some(kind);
                    e.still = Some(still);
                    e.repaint = true;
                }
                None => e.error = Some(
                    "This file cannot be edited. Animated images keep their original animation."
                        .into(),
                ),
            }
            changed = true;
        }
        if changed {
            self.attachment_editor_changed();
        }
    }

    pub fn attachment_edit_finished(&mut self, key: u64, error: Option<String>) {
        let Some(e) = self.attachment_editor.as_mut().filter(|e| e.key == key) else {
            return;
        };
        if let Some(error) = error {
            e.busy = false;
            e.error = Some(error);
            self.attachment_editor_changed();
        } else {
            self.close_attachment_editor();
        }
    }

    pub fn attachment_editor_activate(&mut self, p: Part) {
        let Some(e) = &mut self.attachment_editor else {
            return;
        };
        if !e.enabled(p) {
            return;
        }
        if p == Part::Cancel {
            return self.close_attachment_editor();
        }
        e.end();
        e.focus = p;
        e.field.focused = p == Part::Field;
        if !matches!(
            p,
            Part::Field
                | Part::Color(_)
                | Part::CustomColor
                | Part::Smaller
                | Part::Larger
                | Part::Open
        ) {
            e.commit_text();
        }
        let before = e.edit.clone();
        match p {
            Part::Done => {
                if e.edit == e.initial {
                    return self.close_attachment_editor();
                }
                let edit = e.edit.is_edited().then(|| e.edit.clone());
                let result = self
                    .live
                    .as_mut()
                    .ok_or("The attachment is no longer available.")
                    .and_then(|l| l.save_draft_edit(e.key, edit));
                match result {
                    Ok(()) => {
                        e.busy = true;
                        e.field.focused = false;
                        e.error = None;
                    }
                    Err(error) => e.error = Some(error.into()),
                }
            }
            Part::View => e.tool = Tool::View,
            Part::Crop => e.tool = Tool::Crop,
            Part::Draw => e.tool = Tool::Draw,
            Part::Erase => e.tool = Tool::Erase,
            Part::Text => {
                e.tool = Tool::Text;
                e.field.focused = true;
                e.focus = Part::Field;
            }
            Part::Field => e.field.focused = true,
            Part::Rotate => e.edit = e.edit.rotate_left(),
            Part::Reset => e.edit = Edit::default(),
            Part::Undo => {
                if let Some(edit) = e.undo.pop() {
                    e.redo.push(std::mem::replace(&mut e.edit, edit));
                }
            }
            Part::Redo => {
                if let Some(edit) = e.redo.pop() {
                    e.undo.push(std::mem::replace(&mut e.edit, edit));
                }
            }
            Part::Color(i) => e.color = edit::COLORS[i],
            Part::CustomColor => {
                let (tx, rx) = channel();
                e.color_picker = Some(rx);
                let window = self.hwnd.0 as isize;
                let [r, g, b] = e.color;
                // Like Attach files, a blocking native dialog cannot hold the
                // app's mutable UI borrow while its message loop dispatches.
                std::thread::spawn(move || {
                    let owner = HWND(window as _);
                    let mut colors = [COLORREF(0xFFFFFF); 16];
                    let mut picker = CHOOSECOLORW {
                        lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
                        hwndOwner: owner,
                        rgbResult: COLORREF(u32::from(r) | u32::from(g) << 8 | u32::from(b) << 16),
                        lpCustColors: colors.as_mut_ptr(),
                        Flags: CC_RGBINIT | CC_FULLOPEN,
                        ..Default::default()
                    };
                    let chosen = unsafe { ChooseColorW(&mut picker) }.as_bool();
                    let c = picker.rgbResult.0;
                    let _ = tx.send(chosen.then_some([c as u8, (c >> 8) as u8, (c >> 16) as u8]));
                    unsafe {
                        let _ =
                            PostMessageW(Some(owner), crate::live::WM_LIVE, WPARAM(0), LPARAM(0));
                    }
                });
            }
            Part::Open => {
                // Open the staged version, never unsaved edits. Edited drafts
                // use the shared exact-sendable-byte materialization contract.
                if !e.initial.is_edited() {
                    crate::open_link(&e.path.to_string_lossy());
                } else if let Some(prepared) = e.prepared.clone() {
                    let (tx, rx) = channel();
                    e.opening = Some(rx);
                    let window = self.hwnd.0 as isize;
                    std::thread::spawn(move || {
                        let _ = tx.send(prepared.on_disk());
                        unsafe {
                            let _ = PostMessageW(
                                Some(HWND(window as _)),
                                crate::live::WM_LIVE,
                                WPARAM(0),
                                LPARAM(0),
                            );
                        }
                    });
                }
            }
            Part::Smaller | Part::Larger => {
                let delta = if p == Part::Smaller { -2.0 } else { 2.0 };
                if e.tool == Tool::Text {
                    e.size = (e.size + delta).clamp(edit::TEXT_SIZES.0, edit::TEXT_SIZES.1);
                    e.text_size = Some(e.size as f64 / e.long_edge());
                } else {
                    e.stroke = (e.stroke + delta).clamp(edit::STROKE_SIZES.0, edit::STROKE_SIZES.1);
                }
            }
            _ => {}
        }
        if !matches!(p, Part::Undo | Part::Redo) {
            e.remember(before);
        }
        e.repaint = true;
        self.events |= EV_SCREEN | EV_FOCUS;
        self.attachment_editor_changed();
    }

    pub fn attachment_editor_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        let e = self.attachment_editor.as_mut()?;
        let done = Some(Some(LRESULT(0)));
        match msg {
            WM_LBUTTONDOWN => {
                let hit = e.hit(x, y);
                if let Some(p) = hit.filter(|p| e.enabled(*p)) {
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                    e.pressed = Some(p);
                    e.field.focused = p == Part::Field;
                    e.focus = p;
                    if p == Part::Canvas {
                        e.begin(x, y);
                    } else if p == Part::Field {
                        let (tx, ty) = e.text_origin();
                        e.field.move_to(e.field.hit(x - tx, y - ty), shift);
                    }
                    self.events |= EV_FOCUS;
                    self.attachment_editor_changed();
                }
            }
            WM_MOUSEMOVE => {
                self.pointer = Some((x, y));
                if e.gesture.is_some() {
                    e.drag(x, y);
                    self.attachment_editor_changed();
                } else if e.pressed == Some(Part::Field) {
                    let (tx, ty) = e.text_origin();
                    e.field.move_to(e.field.hit(x - tx, y - ty), true);
                    self.attachment_editor_changed();
                }
            }
            WM_LBUTTONUP => {
                e.end();
                let pressed = e.pressed.take();
                let activated = pressed.filter(|p| {
                    Some(*p) == e.hit(x, y) && !matches!(p, Part::Canvas | Part::Field)
                });
                unsafe {
                    let _ = ReleaseCapture();
                }
                if let Some(p) = activated {
                    self.attachment_editor_activate(p);
                } else {
                    self.attachment_editor_changed();
                }
            }
            WM_CAPTURECHANGED => {
                e.end();
                e.pressed = None;
                self.attachment_editor_changed();
            }
            WM_SETCURSOR => {
                let cursor = match self.pointer.and_then(|(x, y)| e.hit(x, y)) {
                    Some(Part::Field) => IDC_IBEAM,
                    Some(Part::Canvas) if e.tool != Tool::View => IDC_CROSS,
                    Some(p) if e.enabled(p) => IDC_HAND,
                    _ => IDC_ARROW,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                return Some(Some(LRESULT(1)));
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                if vk == VK_ESCAPE {
                    self.close_attachment_editor();
                } else if vk == VK_TAB {
                    let parts: Vec<_> = e
                        .parts()
                        .filter(|p| e.enabled(*p) && *p != Part::Canvas)
                        .collect();
                    if !parts.is_empty() {
                        let i = parts.iter().position(|p| *p == e.focus).unwrap_or(0);
                        e.focus =
                            parts[(i + if shift { parts.len() - 1 } else { 1 }) % parts.len()];
                        e.field.focused = e.focus == Part::Field;
                        self.events |= EV_FOCUS;
                        self.attachment_editor_changed();
                    }
                } else if !e.busy {
                    if ctrl && vk == VIRTUAL_KEY('Z' as u16) && !e.field.focused {
                        self.attachment_editor_activate(if shift {
                            Part::Redo
                        } else {
                            Part::Undo
                        });
                    } else if ctrl && vk == VIRTUAL_KEY('Y' as u16) && !e.field.focused {
                        self.attachment_editor_activate(Part::Redo);
                    } else if vk == VK_RETURN {
                        let p = if ctrl {
                            Part::Done
                        } else if e.field.focused {
                            Part::AddText
                        } else {
                            e.focus
                        };
                        self.attachment_editor_activate(p);
                    } else if e.field.focused {
                        if ctrl && vk == VIRTUAL_KEY('C' as u16) {
                            crate::set_clipboard(self.hwnd, &e.field.copied());
                        } else if e.field.edit_key(self.hwnd, vk, ctrl, shift, false) {
                            self.attachment_editor_changed();
                        }
                    } else if vk == VK_SPACE {
                        let p = e.focus;
                        self.attachment_editor_activate(p);
                    }
                }
            }
            WM_CHAR => {
                let c = wp.0 as u16;
                if e.field.focused && !e.busy && !ctrl && c >= 0x20 && c != 0x7f {
                    e.field.insert(&[c]);
                    self.attachment_editor_changed();
                }
            }
            WM_LBUTTONDBLCLK | WM_MOUSEWHEEL | WM_RBUTTONDOWN | WM_RBUTTONUP => {}
            _ => return None,
        }
        done
    }
}
