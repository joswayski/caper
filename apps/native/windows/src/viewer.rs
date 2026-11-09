//! The in-app media viewer, as the egui client's (`apps/native/desktop/src/viewer.rs`,
//! after web's `MediaViewer.tsx`): clicking a ready image or video opens it
//! almost full window over the conversation, with the rest of that
//! message's images and videos one arrow (or Left/Right) away.
//!
//! Images load at full size through the shared media cache
//! (`Media::full`, up to `FULL_EDGE`; GIF and WebP animate unless Windows
//! reduces motion), with the inline preview standing in meanwhile. Videos
//! play in `caper_client::player`, streamed through the voice player's
//! loopback `MediaProxy`. Signed URLs come from `Media::resolve`: an
//! expiring one is refreshed before it is used, and a load that fails with
//! an expired URL asks for one refresh and retries.
//!
//! The backdrop and controls draw on the dialog panel; the media draws on
//! its own panel over the stage, so opening and paging move only it
//! (compositor opacity and offset animations) and zooming redraws only it.
use crate::files::{D2d, hidden, line, media_kind};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::theme::*;
use crate::{App, EV_FOCUS, EV_SCREEN};
use caper_client::media::{ImageState, Textures, full_key};
use caper_client::model::{Attachment, AttachmentKind};
use caper_client::player::{self, Player, State};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::{Interface, Result, w};

/// The next GIF frame or video frame.
pub const TIMER_VIEWER: usize = 18;
const TOP_BAR: f32 = 56.0;
const CONTROLS: f32 = 52.0;
const SIDE: f32 = 72.0;
const MAX_ZOOM: f32 = 8.0;
const DOUBLE_CLICK_ZOOM: f32 = 2.5;
/// One wheel notch zooms by this much, about the pointer.
const WHEEL_ZOOM: f32 = 1.25;
/// How far the media rises on opening and a turned-to file slides in from.
const RISE: f32 = 8.0;
const PAGE_OFFSET: f32 = 40.0;
/// `--motion-fast` and `--motion-base`, in seconds.
const FAST: f64 = 0.14;
const BASE: f64 = 0.2;
const NOTICE: Duration = Duration::from_secs(4);
/// Videos decode at most at 1080p however large the window.
const MAX_VIDEO: [f32; 2] = [1920.0, 1080.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The whole overlay: the UI Automation container.
    Panel,
    Title,
    Notice,
    Stage,
    /// Copy media link.
    Link,
    Copy,
    Save,
    Open,
    Close,
    Previous,
    Next,
    Play,
    Seek,
    Mute,
}

impl Part {
    pub fn index(self) -> usize {
        self as usize
    }
}

/// What the viewer shows: ready images and videos with a URL.
pub fn viewable(a: &Attachment) -> bool {
    media_kind(a) && crate::files::clickable(a)
}

/// `natural` scaled to fit `bounds`, never enlarged.
fn fit(natural: (f32, f32), bounds: (f32, f32)) -> (f32, f32) {
    let (w, h) = (natural.0.max(1.0), natural.1.max(1.0));
    let k = (bounds.0 / w).min(bounds.1 / h).clamp(0.0, 1.0);
    (w * k, h * k)
}

/// Keep a zoomed picture covering the stage: it moves only as far as it overhangs.
fn clamp_pan(pan: (f32, f32), shown: (f32, f32), stage: (f32, f32)) -> (f32, f32) {
    let room = (
        ((shown.0 - stage.0) / 2.0).max(0.0),
        ((shown.1 - stage.1) / 2.0).max(0.0),
    );
    (pan.0.clamp(-room.0, room.0), pan.1.clamp(-room.1, room.1))
}

/// How far a picture is zoomed past its fitted size, and moved from the
/// stage's centre, as in egui.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Zoom {
    scale: f32,
    pan: (f32, f32),
}

impl Default for Zoom {
    fn default() -> Self {
        Self {
            scale: 1.0,
            pan: (0.0, 0.0),
        }
    }
}

impl Zoom {
    fn zoomed(&self) -> bool {
        self.scale > 1.0
    }

    /// Where a picture fitted at `base` is drawn on `stage`.
    fn place(&mut self, stage: D2D_RECT_F, base: (f32, f32)) -> D2D_RECT_F {
        let shown = (base.0 * self.scale, base.1 * self.scale);
        let size = (stage.right - stage.left, stage.bottom - stage.top);
        self.pan = clamp_pan(self.pan, shown, size);
        let cx = (stage.left + stage.right) / 2.0 + self.pan.0;
        let cy = (stage.top + stage.bottom) / 2.0 + self.pan.1;
        rect(
            cx - shown.0 / 2.0,
            cy - shown.1 / 2.0,
            cx + shown.0 / 2.0,
            cy + shown.1 / 2.0,
        )
    }

    /// Scales to `scale` (between fit and the most), keeping the spot
    /// under `pointer` still.
    fn zoom_to(&mut self, scale: f32, pointer: (f32, f32), stage: D2D_RECT_F, base: (f32, f32)) {
        let scale = scale.clamp(1.0, MAX_ZOOM);
        let from = (
            pointer.0 - (stage.left + stage.right) / 2.0 - self.pan.0,
            pointer.1 - (stage.top + stage.bottom) / 2.0 - self.pan.1,
        );
        let k = scale / self.scale - 1.0;
        self.pan = (self.pan.0 - from.0 * k, self.pan.1 - from.1 * k);
        self.scale = scale;
        let size = (stage.right - stage.left, stage.bottom - stage.top);
        self.pan = clamp_pan(self.pan, (base.0 * scale, base.1 * scale), size);
    }
}

enum VideoState {
    Probing(Receiver<Option<caper_client::ffmpeg::Probe>>),
    Ready(Box<Player>),
    Failed(String),
}

struct Video {
    id: String,
    url: String,
    local: Option<String>,
    state: VideoState,
    /// One refresh-and-retry after a failure.
    retried: bool,
    frame: Option<ID2D1Bitmap1>,
    /// The time label last drawn, so the controls redraw once a second.
    shown_ms: u64,
}

/// Pixels for the clipboard: a DIBV5 (with alpha) and a PNG.
struct Copied {
    dib: Vec<u8>,
    png: Vec<u8>,
}

/// Which entrance the media panel plays at its next draw.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Enter {
    Open,
    Page(f32),
}

pub struct Viewer {
    items: Vec<Attachment>,
    index: usize,
    zoom: Zoom,
    pub focus: Part,
    /// Focus moved by the keyboard, so its ring shows.
    keyboard: bool,
    hover: Option<Part>,
    pressed: Option<Part>,
    /// A press on the dark area around the media, which closes on release.
    backdrop: bool,
    /// A drag panning the zoomed picture: where it started and the pan then.
    panning: Option<((f32, f32), (f32, f32))>,
    /// The seek bar while dragged, 0–1.
    scrub: Option<f32>,
    controls: Vec<(Part, D2D_RECT_F)>,
    stage: D2D_RECT_F,
    /// Where the picture is drawn (client DIPs), for clicks and zoom.
    picture: D2D_RECT_F,
    video: Option<Video>,
    copying: Option<Receiver<std::result::Result<Copied, String>>>,
    /// A Save: the dialog, then the download, on a worker; its notice
    /// (none when the dialog was cancelled).
    saving: Option<Receiver<Option<String>>>,
    notice: Option<(String, Instant)>,
    enter: Option<Enter>,
    /// The media panel's last static offset (pixels), set again only when
    /// the stage moves so a running entrance is not cut short.
    placed: Option<(f32, f32)>,
    chrome: bool,
    media: bool,
}

impl Viewer {
    pub fn open(items: Vec<Attachment>, index: usize) -> Self {
        Self {
            index: index.min(items.len().saturating_sub(1)),
            items,
            zoom: Zoom::default(),
            focus: Part::Close,
            keyboard: false,
            hover: None,
            pressed: None,
            backdrop: false,
            panning: None,
            scrub: None,
            controls: Vec::new(),
            stage: rect(0.0, 0.0, 1.0, 1.0),
            picture: rect(0.0, 0.0, 0.0, 0.0),
            video: None,
            copying: None,
            saving: None,
            notice: None,
            enter: Some(Enter::Open),
            placed: None,
            chrome: true,
            media: true,
        }
    }

    fn current(&self) -> &Attachment {
        &self.items[self.index]
    }

    fn is_video(&self) -> bool {
        self.current().kind == AttachmentKind::Video
    }

    /// Turns to file `index`; arrows stop at the ends (no wrap, like web).
    fn go(&mut self, index: usize) -> bool {
        if index >= self.items.len() || index == self.index {
            return false;
        }
        self.enter = Some(Enter::Page(if index > self.index { 1.0 } else { -1.0 }));
        self.index = index;
        self.zoom = Zoom::default();
        self.scrub = None;
        self.panning = None;
        if !self.enabled(self.focus) {
            self.focus = Part::Close;
        }
        true
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

    fn hit(&self, x: f32, y: f32) -> Option<Part> {
        let inside = |r: &D2D_RECT_F| x >= r.left && x < r.right && y >= r.top && y < r.bottom;
        // The stage's media (and its controls) take only what they cover.
        let mut found = None;
        for (p, r) in &self.controls {
            match p {
                Part::Panel | Part::Title | Part::Notice => {}
                Part::Stage => {
                    if inside(r) && inside(&self.picture) {
                        found = found.or(Some(Part::Stage));
                    }
                }
                _ if inside(r) => return Some(*p),
                _ => {}
            }
        }
        found
    }

    pub fn enabled(&self, p: Part) -> bool {
        match p {
            Part::Previous => self.index > 0,
            Part::Next => self.index + 1 < self.items.len(),
            Part::Copy => self.copying.is_none(),
            Part::Save => self.saving.is_none(),
            Part::Play | Part::Seek | Part::Mute => matches!(
                self.video.as_ref().map(|v| &v.state),
                Some(VideoState::Ready(_))
            ),
            _ => true,
        }
    }

    /// Focusable with Tab and announced as such.
    pub fn focusable(p: Part) -> bool {
        !matches!(p, Part::Panel | Part::Title | Part::Notice | Part::Stage)
    }

    fn player(&self) -> Option<&Player> {
        match self.video.as_ref().map(|v| &v.state) {
            Some(VideoState::Ready(p)) => Some(p),
            _ => None,
        }
    }

    fn position(&self) -> String {
        format!("{} of {}", self.index + 1, self.items.len())
    }

    /// The overlay's and its parts' accessible names.
    pub fn label(&self, p: Part) -> String {
        let a = self.current();
        let kind = if a.kind == AttachmentKind::Video {
            "Video"
        } else {
            "Image"
        };
        match p {
            Part::Panel => {
                if self.items.len() > 1 {
                    format!("Media viewer, {}, {}", a.name, self.position())
                } else {
                    format!("Media viewer, {}", a.name)
                }
            }
            Part::Title => self.subtitle().map_or_else(
                || a.name.clone(),
                |sub| format!("{}, {}", a.name, sub.replace("  \u{b7}  ", ", ")),
            ),
            Part::Notice => self
                .notice
                .as_ref()
                .map(|(n, _)| n.clone())
                .unwrap_or_default(),
            Part::Stage => format!("{kind} {}", a.name),
            Part::Copy => if self.copying.is_some() {
                "Copying\u{2026}"
            } else {
                "Copy image"
            }
            .into(),
            Part::Link => "Copy media link".into(),
            Part::Save => if self.saving.is_some() {
                "Saving\u{2026}"
            } else {
                "Save"
            }
            .into(),
            Part::Open => "Open in browser".into(),
            Part::Close => "Close viewer".into(),
            Part::Previous => "Previous file".into(),
            Part::Next => "Next file".into(),
            Part::Play => if self.player().is_some_and(|p| p.state() == State::Playing) {
                "Pause"
            } else {
                "Play"
            }
            .into(),
            Part::Seek => {
                let (at, of) = self.times();
                format!(
                    "Seek, {} of {}",
                    player::time_label(at),
                    player::time_label(of)
                )
            }
            Part::Mute => if self.player().is_some_and(Player::muted) {
                "Unmute"
            } else {
                "Mute"
            }
            .into(),
        }
    }

    /// "2 of 3  ·  1.2 MB" under the name.
    fn subtitle(&self) -> Option<String> {
        let a = self.current();
        let size = (a.size > 0).then(|| caper_client::uploads::format_bytes(a.size));
        match (self.items.len() > 1, size) {
            (true, Some(size)) => Some(format!("{}  \u{b7}  {size}", self.position())),
            (true, None) => Some(self.position()),
            (false, size) => size,
        }
    }

    /// The playing position (or the scrubbed one) and the duration, in ms.
    fn times(&self) -> (u64, u64) {
        let Some(p) = self.player() else {
            return (0, self.current().duration_ms.unwrap_or(0));
        };
        let of = p.duration_ms().unwrap_or(0);
        let at = self
            .scrub
            .map_or(p.position_ms(), |f| (f * of as f32) as u64);
        (at, of)
    }

    /// Lays out the overlay in a `w` × `h` window.
    fn layout(&mut self, w: f32, h: f32) {
        let video = self.is_video();
        let animated = self.current().animated;
        let side = SIDE.min(w / 8.0);
        let bottom = if video && !animated { CONTROLS } else { 16.0 };
        self.stage = rect(
            side,
            TOP_BAR + 8.0,
            (w - side).max(side + 1.0),
            (h - bottom - 8.0).max(TOP_BAR + 9.0),
        );
        let mid = TOP_BAR / 2.0;
        let mut controls = vec![(Part::Panel, rect(0.0, 0.0, w, h))];
        let mut x = w - 16.0;
        let mut button = |p: Part, width: f32, height: f32| {
            x -= width;
            let r = rect(x, mid - height / 2.0, x + width, mid + height / 2.0);
            x -= 8.0;
            (p, r)
        };
        controls.push(button(Part::Close, 36.0, 36.0));
        controls.push(button(Part::Open, 148.0, 32.0));
        controls.push(button(Part::Save, 100.0, 32.0));
        if !video {
            controls.push(button(Part::Copy, 124.0, 32.0));
        }
        controls.push(button(Part::Link, 156.0, 32.0));
        let title_right = x.max(20.0 + 40.0);
        if self.notice.is_some() {
            controls.push((
                Part::Notice,
                rect(
                    (title_right - 220.0).max(20.0),
                    mid - 9.0,
                    title_right,
                    mid + 9.0,
                ),
            ));
        }
        controls.insert(
            1,
            (Part::Title, rect(20.0, 8.0, title_right, TOP_BAR - 6.0)),
        );
        controls.push((Part::Stage, self.stage));
        if self.items.len() > 1 {
            let cy = (self.stage.top + self.stage.bottom) / 2.0;
            for (p, cx) in [(Part::Previous, side / 2.0), (Part::Next, w - side / 2.0)] {
                controls.push((p, rect(cx - 22.0, cy - 22.0, cx + 22.0, cy + 22.0)));
            }
        }
        if video && !animated {
            let s = self.stage;
            let (top, bottom) = (s.bottom + 8.0, s.bottom + 8.0 + CONTROLS);
            let cy = (top + bottom) / 2.0;
            controls.push((
                Part::Play,
                rect(s.left, cy - 18.0, s.left + 36.0, cy + 18.0),
            ));
            controls.push((
                Part::Mute,
                rect(s.right - 36.0, cy - 18.0, s.right, cy + 18.0),
            ));
            let left = s.left
                + 36.0
                + if s.right - s.left < 420.0 {
                    12.0
                } else {
                    120.0
                };
            controls.push((
                Part::Seek,
                rect(left, cy - 12.0, (s.right - 48.0).max(left + 1.0), cy + 12.0),
            ));
        }
        self.controls = controls;
        self.chrome = true;
        self.media = true;
    }

    /// The size the current media fits from: its decoded size once known.
    fn natural(&self, decoded: Option<(f32, f32)>) -> (f32, f32) {
        if let Some(d) = decoded {
            return d;
        }
        let a = self.current();
        match (a.width, a.height) {
            (Some(w), Some(h)) if w > 0 && h > 0 => (w as f32, h as f32),
            _ if a.kind == AttachmentKind::Video => (16.0, 9.0),
            _ => (
                self.stage.right - self.stage.left,
                self.stage.bottom - self.stage.top,
            ),
        }
    }

    fn base(&self, decoded: Option<(f32, f32)>) -> (f32, f32) {
        fit(
            self.natural(decoded),
            (
                self.stage.right - self.stage.left,
                self.stage.bottom - self.stage.top,
            ),
        )
    }

    /// Draws the backdrop, top bar, arrows and video controls.
    fn draw_chrome(&self, g: &Gfx, dc: &ID2D1DeviceContext) -> Result<()> {
        unsafe {
            dc.Clear(Some(&gfx::color(BLACKOUT, 1.0)));
        }
        for (p, r) in &self.controls {
            let hot = self.hover == Some(*p) && self.enabled(*p);
            let ring = self.keyboard && self.focus == *p;
            match p {
                Part::Panel | Part::Stage => {}
                Part::Title => {
                    let width = (r.right - r.left).max(1.0);
                    let name = line(g, &self.current().name, &g.formats.name, width)?;
                    no_wrap(&name)?;
                    clip(dc, *r, || {
                        crate::text::draw(dc, &name, r.left, r.top + 2.0, &g.brushes.text)
                    });
                    if let Some(sub) = self.subtitle() {
                        let sub = line(g, &sub, &g.formats.file_detail, width)?;
                        no_wrap(&sub)?;
                        clip(dc, *r, || {
                            crate::text::draw(dc, &sub, r.left, r.top + 23.0, &g.brushes.muted)
                        });
                    }
                }
                Part::Notice => {
                    let text = self.label(*p);
                    let l = line(g, &text, &g.formats.file_detail, r.right - r.left)?;
                    unsafe {
                        l.SetTextAlignment(
                            windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_TRAILING,
                        )?;
                    }
                    no_wrap(&l)?;
                    crate::text::draw(dc, &l, r.left, r.top, &g.brushes.muted);
                }
                Part::Link | Part::Copy | Part::Save | Part::Open => {
                    // Open in browser is the primary action: eggshell, blackout label.
                    let primary = *p == Part::Open;
                    let (fill, label_brush) = if primary {
                        (
                            alpha_brush(dc, if hot { ACTION_HOVER } else { ACTION }, 1.0)?,
                            &g.brushes.blackout,
                        )
                    } else if self.enabled(*p) {
                        (
                            alpha_brush(dc, if hot { SURFACE_RAISED } else { 0x151719 }, 1.0)?,
                            &g.brushes.text,
                        )
                    } else {
                        (alpha_brush(dc, 0x151719, 1.0)?, &g.brushes.muted)
                    };
                    fill_round(dc, *r, RADIUS, &fill);
                    if !primary {
                        stroke_round(dc, *r, RADIUS, &g.brushes.border);
                    }
                    let icon = match p {
                        Part::Open => Icon::ExternalLink,
                        Part::Save => Icon::Download,
                        Part::Link => Icon::Link,
                        _ => Icon::Copy,
                    };
                    let color = if primary { BLACKOUT } else { TEXT };
                    let cy = (r.top + r.bottom) / 2.0;
                    icons::draw(
                        dc,
                        icon,
                        rect(r.left + 12.0, cy - 8.0, r.left + 28.0, cy + 8.0),
                        color,
                        if self.enabled(*p) { 1.0 } else { 0.5 },
                    );
                    let text = self.label(*p);
                    let l = line(g, &text, &g.formats.button, r.right - r.left - 40.0)?;
                    no_wrap(&l)?;
                    let m = gfx::metrics(&l);
                    crate::text::draw(dc, &l, r.left + 34.0, cy - m.height / 2.0, label_brush);
                }
                Part::Close | Part::Previous | Part::Next | Part::Play | Part::Mute => {
                    let enabled = self.enabled(*p);
                    let fill = if hot {
                        alpha_brush(dc, 0xFFFFFF, 0.16)?
                    } else {
                        alpha_brush(dc, 0x000000, 0.47)?
                    };
                    let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
                    let radius = (r.right - r.left) / 2.0;
                    let ellipse = D2D1_ELLIPSE {
                        point: windows_numerics::Vector2 { X: cx, Y: cy },
                        radiusX: radius,
                        radiusY: radius,
                    };
                    unsafe {
                        dc.FillEllipse(&ellipse, &fill);
                    }
                    let icon = match p {
                        Part::Close => Icon::Close,
                        Part::Previous => Icon::ChevronLeft,
                        Part::Next => Icon::ChevronRight,
                        Part::Play
                            if self.player().is_some_and(|p| p.state() == State::Playing) =>
                        {
                            Icon::Pause
                        }
                        Part::Play => Icon::Play,
                        _ if self
                            .player()
                            .is_some_and(|p| p.muted() || p.volume() == 0.0) =>
                        {
                            Icon::VolumeX
                        }
                        _ => Icon::Volume,
                    };
                    icons::draw(
                        dc,
                        icon,
                        rect(cx - 10.0, cy - 10.0, cx + 10.0, cy + 10.0),
                        TEXT,
                        if enabled { 1.0 } else { 0.24 },
                    );
                    if ring {
                        unsafe {
                            dc.DrawEllipse(&ellipse, &g.brushes.terracotta, 1.5, None);
                        }
                    }
                    continue;
                }
                Part::Seek => {
                    let (at, of) = self.times();
                    let cy = (r.top + r.bottom) / 2.0;
                    let label = format!("{} / {}", player::time_label(at), player::time_label(of));
                    let wide = self.stage.right - self.stage.left >= 420.0;
                    if wide {
                        let l = line(g, &label, &g.formats.mono, 120.0)?;
                        no_wrap(&l)?;
                        let m = gfx::metrics(&l);
                        crate::text::draw(
                            dc,
                            &l,
                            self.stage.left + 46.0,
                            cy - m.height / 2.0,
                            &g.brushes.muted,
                        );
                    }
                    let track = rect(r.left, cy - 2.0, r.right, cy + 2.0);
                    fill_round(dc, track, 2.0, &alpha_brush(dc, 0xFFFFFF, 0.2)?);
                    let fraction = if of > 0 {
                        (at as f32 / of as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let knob = r.left + (r.right - r.left) * fraction;
                    fill_round(
                        dc,
                        rect(r.left, cy - 2.0, knob, cy + 2.0),
                        2.0,
                        &g.brushes.text,
                    );
                    if self.enabled(*p) {
                        let ellipse = D2D1_ELLIPSE {
                            point: windows_numerics::Vector2 { X: knob, Y: cy },
                            radiusX: 6.0,
                            radiusY: 6.0,
                        };
                        unsafe {
                            dc.FillEllipse(&ellipse, &g.brushes.text);
                        }
                    }
                }
            }
            if ring {
                stroke_round(dc, *r, RADIUS, &g.brushes.terracotta);
            }
        }
        Ok(())
    }
}

fn no_wrap(l: &windows::Win32::Graphics::DirectWrite::IDWriteTextLayout) -> Result<()> {
    unsafe {
        l.SetWordWrapping(windows::Win32::Graphics::DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP)
    }
}

fn clip(dc: &ID2D1DeviceContext, r: D2D_RECT_F, f: impl FnOnce()) {
    unsafe {
        dc.PushAxisAlignedClip(&r, D2D1_ANTIALIAS_MODE_ALIASED);
    }
    f();
    unsafe {
        dc.PopAxisAlignedClip();
    }
}

fn alpha_brush(dc: &ID2D1DeviceContext, c: u32, a: f32) -> Result<ID2D1SolidColorBrush> {
    crate::files::alpha(dc, c, a)
}

/// `--ease-out`, as egui approximates it.
fn ease_out(t: f64) -> f64 {
    if t >= 1.0 {
        1.0
    } else {
        1.0 - 2f64.powf(-10.0 * t.max(0.0))
    }
}

/// A compositor animation from `from` to `to` over `seconds`, eased out
/// (piecewise linear: eight steps are finer than a frame).
fn animation(g: &Gfx, from: f32, to: f32, seconds: f64) -> Result<IDCompositionAnimation> {
    const STEPS: usize = 8;
    unsafe {
        let a = g.dcomp.CreateAnimation()?;
        for i in 0..STEPS {
            let (t0, t1) = (i as f64 / STEPS as f64, (i + 1) as f64 / STEPS as f64);
            let v0 = from as f64 + (to - from) as f64 * ease_out(t0);
            let v1 = from as f64 + (to - from) as f64 * ease_out(t1);
            a.AddCubic(
                t0 * seconds,
                v0 as f32,
                ((v1 - v0) / ((t1 - t0) * seconds)) as f32,
                0.0,
                0.0,
            )?;
        }
        a.End(seconds, to)?;
        Ok(a)
    }
}

fn opacity(visual: &IDCompositionVisual2, value: Option<&IDCompositionAnimation>) -> Result<()> {
    let v: IDCompositionVisual3 = visual.cast()?;
    unsafe {
        match value {
            Some(a) => v.SetOpacity(a),
            None => v.SetOpacity2(1.0),
        }
    }
}

/// Unmultiplied RGBA to the clipboard's bottom-up 32-bit DIBV5 (straight
/// alpha, which Office and browsers read) and a PNG.
fn clipboard_image(image: &image::RgbaImage) -> Option<Copied> {
    let (w, h) = image.dimensions();
    let mut dib = Vec::with_capacity(124 + (w * h * 4) as usize);
    let header = BITMAPV5HEADER_SIZE;
    dib.extend_from_slice(&header.to_le_bytes());
    dib.extend_from_slice(&(w as i32).to_le_bytes());
    dib.extend_from_slice(&(h as i32).to_le_bytes());
    dib.extend_from_slice(&1u16.to_le_bytes());
    dib.extend_from_slice(&32u16.to_le_bytes());
    dib.extend_from_slice(&3u32.to_le_bytes()); // BI_BITFIELDS
    dib.extend_from_slice(&(w * h * 4).to_le_bytes());
    dib.extend_from_slice(&[0; 8]); // pixels per metre
    dib.extend_from_slice(&[0; 8]); // colours used, important
    for mask in [0x00FF_0000u32, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000] {
        dib.extend_from_slice(&mask.to_le_bytes());
    }
    dib.extend_from_slice(b"BGRs"); // LCS_sRGB, little-endian 'sRGB'
    dib.resize(dib.len() + 36 + 12, 0); // endpoints, gamma
    dib.extend_from_slice(&4u32.to_le_bytes()); // LCS_GM_IMAGES
    dib.resize(header as usize, 0); // profile data, size, reserved
    for row in (0..h).rev() {
        for x in 0..w {
            let [r, g, b, a] = image.get_pixel(x, row).0;
            dib.extend_from_slice(&[b, g, r, a]);
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(Copied {
        dib,
        png: png.into_inner(),
    })
}

const BITMAPV5HEADER_SIZE: u32 = 124;

/// Puts `copied` on the clipboard (as the only content). False when the
/// clipboard was busy.
fn set_clipboard_image(hwnd: HWND, copied: &Copied) -> bool {
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return false;
        }
        let _ = EmptyClipboard();
        let put = |format: u32, bytes: &[u8]| {
            let Ok(h) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) else {
                return false;
            };
            let p = GlobalLock(h).cast::<u8>();
            if p.is_null() {
                let _ = GlobalFree(Some(h));
                return false;
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
            let _ = GlobalUnlock(h);
            if SetClipboardData(format, Some(HANDLE(h.0))).is_err() {
                let _ = GlobalFree(Some(h));
                return false;
            }
            true
        };
        let dib = put(windows::Win32::System::Ole::CF_DIBV5.0 as u32, &copied.dib);
        let png = put(RegisterClipboardFormatW(w!("PNG")), &copied.png);
        let _ = CloseClipboard();
        dib || png
    }
}

/// Uploads one video frame (unmultiplied RGBA; video is opaque) to `frame`.
fn show_frame(
    dc: &ID2D1DeviceContext,
    frame: &mut Option<ID2D1Bitmap1>,
    size: [usize; 2],
    rgba: &[u8],
) {
    let image = caper_client::Rgba {
        size,
        pixels: rgba.to_vec(),
    };
    let same = frame.as_ref().is_some_and(|b| unsafe {
        let s = b.GetPixelSize();
        s.width as usize == size[0] && s.height as usize == size[1]
    });
    let mut tex = D2d(dc.clone());
    match frame {
        Some(b) if same => tex.update(b, &image),
        _ => *frame = Some(tex.create("viewer-video", &image)),
    }
}

impl App {
    /// Opens the viewer on `items` (a message's ready images and videos) at `index`.
    pub fn open_viewer(&mut self, items: Vec<Attachment>, index: usize) {
        if items.is_empty() || self.live.is_none() {
            return;
        }
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        self.close_menu();
        self.close_dialog();
        self.viewer = Some(Viewer::open(items, index));
        self.viewer_changed();
        self.events |= EV_SCREEN | EV_FOCUS;
    }

    /// Closes the viewer at once (closing never animates).
    pub fn close_viewer(&mut self) {
        let Some(v) = self.viewer.take() else {
            return;
        };
        self.stop_viewer_video(v.video);
        if let Some(live) = &mut self.live {
            for a in &v.items {
                live.media.release(&full_key(&a.id));
            }
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_VIEWER);
            let _ = ReleaseCapture();
        }
        let _ = opacity(&self.dialog_p.visual, None);
        let _ = opacity(&self.viewer_p.visual, None);
        let _ = self.viewer_p.offset(-100_000.0, 0.0);
        self.dirty.overlay = true;
        self.need = true;
        self.events |= EV_SCREEN | EV_FOCUS;
        self.set_composer_focus(true);
    }

    fn stop_viewer_video(&mut self, video: Option<Video>) {
        if let Some(Video {
            local: Some(local), ..
        }) = video
            && let Some(proxy) = self.playback.proxy_if_started()
        {
            proxy.remove(&local);
        }
    }

    /// After the window, the file shown or what loaded changed.
    pub fn viewer_changed(&mut self) {
        let (w, h) = (self.w, self.h);
        let Some(v) = &mut self.viewer else {
            return;
        };
        if v.notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= NOTICE)
        {
            v.notice = None;
        }
        v.layout(w, h);
        self.prepare_viewer_video();
        self.dirty.overlay = true;
        self.need = true;
    }

    /// Starts (or keeps) the current video, or stops one no longer shown.
    fn prepare_viewer_video(&mut self) {
        let Some(v) = &self.viewer else {
            return;
        };
        let Some(live) = &self.live else {
            return;
        };
        let current = live.media.resolve(v.current());
        if current.kind != AttachmentKind::Video {
            let video = self.viewer.as_mut().and_then(|v| v.video.take());
            self.stop_viewer_video(video);
            return;
        }
        let Some(url) = current.url.clone() else {
            return;
        };
        if let Some(video) = &v.video {
            let failed = matches!(video.state, VideoState::Failed(_));
            // Same video: keep playing; after a failure, a refreshed URL retries once.
            if video.id == current.id && (!failed || video.url == url || video.retried) {
                return;
            }
        }
        let retried = v.video.as_ref().is_some_and(|video| video.id == current.id);
        let old = self.viewer.as_mut().and_then(|v| v.video.take());
        self.stop_viewer_video(old);
        let failed = |message: &str| Video {
            id: current.id.clone(),
            url: url.clone(),
            local: None,
            state: VideoState::Failed(message.into()),
            retried: true,
            frame: None,
            shown_ms: 0,
        };
        let video = match caper_client::ffmpeg::binary() {
            None => failed("Video playback needs the FFmpeg that ships with Caper."),
            Some(binary) => match (self.playback.proxy(), caper_client::api::media_url(&url)) {
                (Some(proxy), Some(_)) => {
                    let local = proxy.route(&url);
                    let (tx, rx) = channel();
                    let input = local.clone();
                    let window = self.hwnd.0 as isize;
                    std::thread::spawn(move || {
                        let probe =
                            caper_client::ffmpeg::probe(binary, std::path::Path::new(&input));
                        let _ = tx.send(probe.ok());
                        wake(window);
                    });
                    Video {
                        id: current.id.clone(),
                        url: url.clone(),
                        local: Some(local),
                        state: VideoState::Probing(rx),
                        retried,
                        frame: None,
                        shown_ms: 0,
                    }
                }
                _ => failed("This video could not be streamed."),
            },
        };
        if let Some(v) = &mut self.viewer {
            v.video = Some(video);
            v.media = true;
            v.chrome = true;
        }
    }

    /// Results from the viewer's workers: a probed video, a copied image.
    pub fn poll_viewer(&mut self) {
        let Some(v) = &mut self.viewer else {
            return;
        };
        let mut changed = false;
        if let Some(rx) = &v.copying
            && let Ok(result) = rx.try_recv()
        {
            v.copying = None;
            let text = match result {
                Ok(copied) if set_clipboard_image(self.hwnd, &copied) => "Image copied.".into(),
                Ok(_) => "Couldn\u{2019}t access the clipboard.".into(),
                Err(e) => e,
            };
            v.notice = Some((text, Instant::now()));
            changed = true;
        }
        if let Some(rx) = &v.saving {
            match rx.try_recv() {
                Ok(result) => {
                    v.saving = None;
                    if let Some(text) = result {
                        v.notice = Some((text, Instant::now()));
                    }
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    v.saving = None;
                    v.notice = Some((
                        "Couldn\u{2019}t save the file. Please try again.".into(),
                        Instant::now(),
                    ));
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let stage = (v.stage.right - v.stage.left, v.stage.bottom - v.stage.top);
        let animated = v.current().animated;
        let duration = v.current().duration_ms;
        let id = v.current().id.clone();
        let mut refresh = false;
        if let Some(video) = &mut v.video
            && let VideoState::Probing(rx) = &video.state
            && let Ok(probe) = rx.try_recv()
        {
            video.state = match (probe, caper_client::ffmpeg::binary(), &video.local) {
                (Some(probe), Some(binary), Some(local)) if probe.video.is_some() => {
                    let stream = probe.video.as_ref().expect("checked");
                    let fit = [
                        (stage.0 * self.scale).min(MAX_VIDEO[0]),
                        (stage.1 * self.scale).min(MAX_VIDEO[1]),
                    ];
                    let mut player = Player::new(
                        binary,
                        local.clone(),
                        stream.width,
                        stream.height,
                        probe.duration_ms.or(duration),
                        stream.fps,
                        fit,
                    );
                    // GIF-style videos loop silently, like a GIF.
                    if animated {
                        player.toggle_mute();
                    }
                    player.play();
                    VideoState::Ready(Box::new(player))
                }
                _ => {
                    refresh = !video.retried;
                    VideoState::Failed(
                        "This video can\u{2019}t be played here. Open it in your browser instead."
                            .into(),
                    )
                }
            };
            changed = true;
        }
        if refresh && let Some(live) = &mut self.live {
            live.media.refresh_now(&id);
        }
        if changed {
            self.viewer_changed();
            self.viewer_tick();
        }
    }

    /// The viewer's clock: the video's next frame, a GIF's next frame, and
    /// a notice running out.
    pub fn viewer_tick(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_VIEWER);
        }
        let Some(v) = &mut self.viewer else {
            return;
        };
        let mut next: Option<Duration> = None;
        let dc = self.g.dc.clone();
        let animated = v.current().animated;
        let scrubbing = v.scrub.is_some();
        if let Some(video) = &mut v.video
            && let VideoState::Ready(player) = &mut video.state
        {
            let frame = &mut video.frame;
            let mut shown = false;
            let due = player.update(|size, rgba| {
                show_frame(&dc, frame, size, rgba);
                shown = true;
            });
            // Looping GIF-style videos never stop.
            if animated && player.state() == State::Ended {
                player.seek(0, true);
            }
            next = due;
            if shown {
                v.media = true;
            }
            let at = player.position_ms() / 250;
            if at != video.shown_ms || scrubbing {
                video.shown_ms = at;
                v.chrome = true;
            }
            if matches!(player.state(), State::Playing | State::Loading) {
                next = Some(next.unwrap_or(Duration::from_millis(33)));
            }
        }
        if v.video.is_none() {
            // An image's timer is its GIF's next frame, drawn with the media.
            v.media = true;
        }
        if v.notice
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= NOTICE)
        {
            v.notice = None;
            let (w, h) = (self.w, self.h);
            v.layout(w, h);
        } else if let Some((_, at)) = &v.notice {
            let left = NOTICE.saturating_sub(at.elapsed());
            next = Some(next.map_or(left, |n| n.min(left)));
        }
        if v.chrome || v.media {
            self.dirty.overlay = true;
            self.need = true;
        }
        if let Some(next) = next {
            self.arm_viewer(next);
        }
    }

    fn arm_viewer(&self, after: Duration) {
        unsafe {
            SetTimer(
                Some(self.hwnd),
                TIMER_VIEWER,
                after.as_millis().clamp(10, u32::MAX as u128) as u32,
                None,
            );
        }
    }

    pub fn viewer_activate(&mut self, p: Part) {
        let unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let Some(v) = &mut self.viewer else {
            return;
        };
        if !v.enabled(p) {
            return;
        }
        match p {
            Part::Close => return self.close_viewer(),
            Part::Previous => {
                let i = v.index.saturating_sub(1);
                v.go(i);
            }
            Part::Next => {
                let i = v.index + 1;
                v.go(i);
            }
            Part::Open => {
                let Some(live) = &mut self.live else {
                    return;
                };
                let a = live.media.resolve(v.current());
                // An expiring URL opens once refreshed (`Update::Open`).
                if let Some(url) = live.media.open(&a, unix) {
                    crate::open_link(&url);
                }
            }
            Part::Copy => {
                let Some(live) = &self.live else {
                    return;
                };
                let a = live.media.resolve(v.current());
                let (Some(api), Some(url)) = (live.media.api().cloned(), a.url.clone()) else {
                    v.notice = Some((
                        "Couldn\u{2019}t copy: media is unavailable.".into(),
                        Instant::now(),
                    ));
                    return self.viewer_changed();
                };
                let (tx, rx) = channel();
                v.copying = Some(rx);
                v.notice = Some(("Copying\u{2026}".into(), Instant::now()));
                let window = self.hwnd.0 as isize;
                let content_type = a.content_type.clone();
                std::thread::spawn(move || {
                    let result = api
                        .fetch_media(&url)
                        .map_err(|_| "Couldn\u{2019}t load the original image.".to_owned())
                        .and_then(|bytes| {
                            caper_client::attachments::decode_original(&bytes, &content_type)
                                .ok_or_else(|| {
                                    "Couldn\u{2019}t decode the original image.".to_owned()
                                })
                        })
                        .and_then(|image| {
                            clipboard_image(&image)
                                .ok_or_else(|| "Couldn\u{2019}t copy the image.".to_owned())
                        });
                    let _ = tx.send(result);
                    wake(window);
                });
            }
            Part::Link => {
                let url = self
                    .live
                    .as_ref()
                    .and_then(|live| live.media.resolve(v.current()).url.clone());
                let text = match url {
                    Some(url) => {
                        crate::set_clipboard(self.hwnd, &url.encode_utf16().collect::<Vec<_>>());
                        "Media link copied."
                    }
                    None => "Couldn\u{2019}t copy: media is unavailable.",
                };
                v.notice = Some((text.into(), Instant::now()));
            }
            Part::Save => {
                let Some(live) = &self.live else {
                    return;
                };
                let a = live.media.resolve(v.current());
                let (Some(api), Some(url)) = (live.media.api().cloned(), a.url.clone()) else {
                    return;
                };
                let (tx, rx) = channel();
                v.saving = Some(rx);
                let window = self.hwnd.0 as isize;
                let (name, content_type) = (a.name.clone(), a.content_type.clone());
                // The system dialog blocks its own thread, never the UI.
                std::thread::spawn(move || {
                    let result = save_path(window, &name, &content_type).map(|path| {
                        api.download_media(&url, &path).map_or_else(
                            // A refused, broken or stalled download ends
                            // here, never leaving Save stuck.
                            |_| "Couldn\u{2019}t save the file. Please try again.".to_owned(),
                            |()| {
                                let file = path.file_name().map_or(name.clone(), |file| {
                                    file.to_string_lossy().into_owned()
                                });
                                format!("Saved {file}")
                            },
                        )
                    });
                    let _ = tx.send(result);
                    wake(window);
                });
            }
            Part::Play | Part::Stage => {
                if let Some(Video {
                    state: VideoState::Ready(player),
                    ..
                }) = &mut v.video
                {
                    player.toggle();
                }
            }
            Part::Mute => {
                if let Some(Video {
                    state: VideoState::Ready(player),
                    ..
                }) = &mut v.video
                {
                    player.toggle_mute();
                }
            }
            Part::Panel | Part::Title | Part::Notice | Part::Seek => {}
        }
        if Viewer::focusable(p) {
            if let Some(v) = &mut self.viewer {
                v.focus = if v.enabled(p) { p } else { Part::Close };
            }
            self.events |= EV_FOCUS;
        }
        self.viewer_changed();
        self.viewer_tick();
    }

    /// Seeks to where `x` falls on the seek bar; `done` ends a drag.
    fn viewer_scrub(&mut self, x: f32, done: bool) {
        let Some(v) = &mut self.viewer else {
            return;
        };
        let Some((_, r)) = v.controls.iter().find(|(p, _)| *p == Part::Seek) else {
            return;
        };
        let fraction = ((x - r.left) / (r.right - r.left).max(1.0)).clamp(0.0, 1.0);
        if done {
            v.scrub = None;
            if let Some(Video {
                state: VideoState::Ready(player),
                ..
            }) = &mut v.video
                && let Some(of) = player.duration_ms()
            {
                let play = player.state() == State::Playing || player.state() == State::Ended;
                player.seek((fraction * of as f32) as u64, play);
            }
        } else {
            v.scrub = Some(fraction);
        }
        v.chrome = true;
        self.dirty.overlay = true;
        self.need = true;
        self.viewer_tick();
    }

    /// Zooms the picture to `scale` × fit about client point `pointer`.
    fn viewer_zoom(&mut self, scale: impl FnOnce(f32) -> f32, pointer: (f32, f32)) {
        let decoded = self.viewer_decoded();
        let Some(v) = &mut self.viewer else {
            return;
        };
        if v.is_video() {
            return;
        }
        let base = v.base(decoded);
        let target = scale(v.zoom.scale);
        v.zoom.zoom_to(target, pointer, v.stage, base);
        v.media = true;
        self.dirty.overlay = true;
        self.need = true;
    }

    /// The current image's decoded size, once its original arrived.
    fn viewer_decoded(&self) -> Option<(f32, f32)> {
        let v = self.viewer.as_ref()?;
        let b = self
            .live
            .as_ref()?
            .media
            .texture(&full_key(&v.current().id))?;
        let s = unsafe { b.GetPixelSize() };
        (s.width > 1 || s.height > 1).then_some((s.width as f32, s.height as f32))
    }

    pub fn viewer_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        shift: bool,
    ) -> Option<Option<LRESULT>> {
        let v = self.viewer.as_mut()?;
        let done = Some(Some(LRESULT(0)));
        match msg {
            WM_LBUTTONDOWN => {
                self.pointer = Some((x, y));
                v.keyboard = false;
                v.backdrop = false;
                match v.hit(x, y) {
                    Some(Part::Stage) if !v.is_video() => {
                        if v.zoom.zoomed() {
                            v.panning = Some(((x, y), v.zoom.pan));
                        }
                        v.pressed = Some(Part::Stage);
                    }
                    Some(p) if v.enabled(p) => v.pressed = Some(p),
                    Some(_) => {}
                    None => v.backdrop = true,
                }
                v.chrome = true;
                let seek = v.pressed == Some(Part::Seek);
                unsafe {
                    SetCapture(self.hwnd);
                }
                if seek {
                    self.viewer_scrub(x, false);
                }
                self.dirty.overlay = true;
                self.need = true;
            }
            WM_LBUTTONDBLCLK => {
                if v.hit(x, y) == Some(Part::Stage) && !v.is_video() {
                    let zoomed = v.zoom.zoomed();
                    self.viewer_zoom(|_| if zoomed { 1.0 } else { DOUBLE_CLICK_ZOOM }, (x, y));
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                } else if v.hit(x, y).is_some() {
                    // A quick second click on a button counts as another click.
                    return self.viewer_input(WM_LBUTTONDOWN, wp, x, y, shift);
                }
            }
            WM_MOUSEMOVE => {
                self.pointer = Some((x, y));
                if let Some((start, pan)) = v.panning {
                    v.zoom.pan = (pan.0 + x - start.0, pan.1 + y - start.1);
                    v.media = true;
                    self.dirty.overlay = true;
                    self.need = true;
                } else if v.pressed == Some(Part::Seek) {
                    self.viewer_scrub(x, false);
                } else {
                    let hover = v.hit(x, y);
                    if hover != v.hover {
                        v.hover = hover;
                        v.chrome = true;
                        self.dirty.overlay = true;
                        self.need = true;
                    }
                }
            }
            WM_LBUTTONUP => {
                unsafe {
                    let _ = ReleaseCapture();
                }
                let v = self.viewer.as_mut()?;
                let pressed = v.pressed.take();
                let panned = v
                    .panning
                    .take()
                    .is_some_and(|(start, _)| (x - start.0).abs() + (y - start.1).abs() > 3.0);
                let hit = v.hit(x, y);
                if std::mem::take(&mut v.backdrop) && hit.is_none() {
                    // A click on the dark area around the media closes, as on web.
                    self.close_viewer();
                } else if pressed == Some(Part::Seek) {
                    self.viewer_scrub(x, true);
                } else if let Some(p) = pressed.filter(|p| Some(*p) == hit && !panned) {
                    if p != Part::Stage || v.is_video() {
                        self.viewer_activate(p);
                    }
                } else {
                    v.chrome = true;
                    self.dirty.overlay = true;
                    self.need = true;
                }
            }
            WM_CAPTURECHANGED => {
                v.panning = None;
                v.backdrop = false;
                if v.pressed.take() == Some(Part::Seek) {
                    v.scrub = None;
                }
            }
            WM_MOUSEWHEEL => {
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32 / 120.0;
                if let Some(pointer) = self.pointer
                    && v.hit(pointer.0, pointer.1) == Some(Part::Stage)
                {
                    self.viewer_zoom(|s| s * WHEEL_ZOOM.powf(delta), pointer);
                }
            }
            WM_SETCURSOR => {
                let hit = self.pointer.and_then(|(x, y)| v.hit(x, y));
                let cursor = match hit {
                    Some(Part::Stage) if v.panning.is_some() || v.zoom.zoomed() => IDC_SIZEALL,
                    Some(Part::Stage) if v.is_video() => IDC_HAND,
                    Some(Part::Stage) => IDC_ARROW,
                    Some(p) if v.enabled(p) => IDC_HAND,
                    _ => IDC_ARROW,
                };
                unsafe {
                    SetCursor(LoadCursorW(None, cursor).ok());
                }
                return Some(Some(LRESULT(1)));
            }
            WM_KEYDOWN => {
                let vk = VIRTUAL_KEY(wp.0 as u16);
                match vk {
                    VK_ESCAPE => self.close_viewer(),
                    VK_LEFT => self.viewer_activate(Part::Previous),
                    VK_RIGHT => self.viewer_activate(Part::Next),
                    VK_TAB => {
                        let parts: Vec<_> = v
                            .parts()
                            .filter(|p| Viewer::focusable(*p) && v.enabled(*p))
                            .collect();
                        if !parts.is_empty() {
                            let i = parts.iter().position(|p| *p == v.focus).unwrap_or(0);
                            v.focus =
                                parts[(i + if shift { parts.len() - 1 } else { 1 }) % parts.len()];
                            v.keyboard = true;
                            v.chrome = true;
                            self.events |= EV_FOCUS;
                            self.dirty.overlay = true;
                            self.need = true;
                        }
                    }
                    VK_SPACE
                        if v.is_video()
                            && !matches!(
                                v.focus,
                                Part::Link
                                    | Part::Copy
                                    | Part::Save
                                    | Part::Open
                                    | Part::Close
                                    | Part::Previous
                                    | Part::Next
                            ) =>
                    {
                        self.viewer_activate(Part::Play)
                    }
                    VK_SPACE | VK_RETURN => {
                        let p = v.focus;
                        self.viewer_activate(p);
                    }
                    _ => {}
                }
            }
            WM_CHAR | WM_RBUTTONDOWN | WM_RBUTTONUP | WM_MOUSEHWHEEL | WM_MBUTTONDOWN => {}
            _ => return None,
        }
        done
    }

    /// Redraws the viewer's panels when they changed, and starts their entrance.
    pub fn draw_viewer(&mut self) -> Result<()> {
        let s = self.scale;
        let px = |v: f32| (v * s).ceil().max(1.0) as u32;
        let (w, h) = (self.w, self.h);
        let decoded = self.viewer_decoded();
        let Some(v) = &mut self.viewer else {
            return Ok(());
        };
        let g = &self.g;
        if std::mem::take(&mut v.chrome) {
            self.dialog_p.ensure(g, px(w), px(h))?;
            self.dialog_p.offset(0.0, 0.0)?;
            self.dialog_p.draw(s, |dc| v.draw_chrome(g, dc))?;
        }
        let stage = v.stage;
        let (sw, sh) = (stage.right - stage.left, stage.bottom - stage.top);
        let mut next = None;
        if std::mem::take(&mut v.media) {
            let Some(live) = &mut self.live else {
                return Ok(());
            };
            let mut tex = D2d(g.dc.clone());
            let now = Instant::now();
            let unix = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            let current = live.media.resolve(v.current());
            live.media.visible(&current, unix, now);
            // This file and its neighbours stay decoded, never the whole message.
            for (i, a) in v.items.iter().enumerate() {
                if i + 1 < v.index || i > v.index + 1 {
                    live.media.release(&full_key(&a.id));
                }
            }
            let base = v.base(decoded);
            v.picture = v.zoom.place(stage, base);
            if current.kind == AttachmentKind::Video {
                let shown = match v.video.as_ref().and_then(|video| video.frame.as_ref()) {
                    Some(b) => {
                        let s = unsafe { b.GetPixelSize() };
                        fit((s.width as f32, s.height as f32), (sw, sh))
                    }
                    None => base,
                };
                v.picture = rect(
                    stage.left + (sw - shown.0) / 2.0,
                    stage.top + (sh - shown.1) / 2.0,
                    stage.left + (sw + shown.0) / 2.0,
                    stage.top + (sh + shown.1) / 2.0,
                );
            }
            let local = rect(
                v.picture.left - stage.left,
                v.picture.top - stage.top,
                v.picture.right - stage.left,
                v.picture.bottom - stage.top,
            );
            let (full, due) = if current.kind == AttachmentKind::Image {
                live.media
                    .full(&mut tex, &current, crate::files::motion(), now)
            } else {
                (ImageState::Loading, None)
            };
            next = due;
            if matches!(full, ImageState::Failed) {
                live.media.refresh_now(&current.id);
            }
            let preview = match live.media.image(
                &current.id,
                caper_client::attachments::inline_source(&current),
            ) {
                ImageState::Ready(_) => live.media.texture(&current.id).cloned(),
                _ => None,
            };
            let picture = match full {
                ImageState::Ready(_) => live.media.texture(&full_key(&current.id)).cloned(),
                _ => None,
            };
            // Queued loads and refreshes start at the live drain's next poll.
            if live.media.busy() {
                unsafe {
                    SetTimer(Some(self.hwnd), crate::TIMER_LIVE, 100, None);
                }
            }
            self.viewer_p.ensure(g, px(sw), px(sh))?;
            let at = ((stage.left * s).round(), (stage.top * s).round());
            if v.placed != Some(at) {
                self.viewer_p.offset(at.0, at.1)?;
                v.placed = Some(at);
            }
            let video = v.video.as_ref();
            let failed_image =
                matches!(full, ImageState::Failed) && current.kind == AttachmentKind::Image;
            self.viewer_p.draw(s, |dc| {
                unsafe {
                    dc.Clear(Some(&gfx::color(0, 0.0)));
                }
                let bitmap = picture
                    .as_ref()
                    .or(video.and_then(|v| v.frame.as_ref()))
                    .or(preview.as_ref());
                match bitmap {
                    Some(b) => unsafe {
                        dc.DrawBitmap(
                            b,
                            Some(&local),
                            1.0,
                            D2D1_INTERPOLATION_MODE_HIGH_QUALITY_CUBIC,
                            None,
                            None,
                        );
                    },
                    None if !failed_image => {
                        fill_round(dc, local, RADIUS, &g.brushes.surface);
                    }
                    None => {}
                }
                let message = match video.map(|v| &v.state) {
                    Some(VideoState::Failed(m)) => Some(m.as_str()),
                    _ if failed_image && preview.is_none() => {
                        Some("Couldn\u{2019}t load this image. Open it in your browser instead.")
                    }
                    _ => None,
                };
                let loading = picture.is_none()
                    && message.is_none()
                    && !failed_image
                    && video.is_none_or(|v| v.frame.is_none());
                let center = (sw / 2.0, sh / 2.0);
                if let Some(message) = message {
                    let band = rect(
                        center.0 - (sw.min(460.0)) / 2.0,
                        center.1 - 22.0,
                        center.0 + (sw.min(460.0)) / 2.0,
                        center.1 + 22.0,
                    );
                    fill_round(dc, band, RADIUS, &alpha_brush(dc, 0x000000, 0.8)?);
                    let l = line(
                        g,
                        message,
                        &g.formats.file_detail,
                        band.right - band.left - 24.0,
                    )?;
                    unsafe {
                        l.SetTextAlignment(
                            windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER,
                        )?;
                    }
                    let m = gfx::metrics(&l);
                    crate::text::draw(
                        dc,
                        &l,
                        band.left + 12.0,
                        center.1 - m.height / 2.0,
                        &g.brushes.text,
                    );
                } else if loading {
                    let pill = rect(
                        center.0 - 52.0,
                        center.1 - 15.0,
                        center.0 + 52.0,
                        center.1 + 15.0,
                    );
                    fill_round(dc, pill, RADIUS, &alpha_brush(dc, 0x000000, 0.7)?);
                    let l = line(g, "Loading\u{2026}", &g.formats.file_detail, 104.0)?;
                    unsafe {
                        l.SetTextAlignment(
                            windows::Win32::Graphics::DirectWrite::DWRITE_TEXT_ALIGNMENT_CENTER,
                        )?;
                    }
                    let m = gfx::metrics(&l);
                    crate::text::draw(
                        dc,
                        &l,
                        pill.left,
                        center.1 - m.height / 2.0,
                        &g.brushes.text,
                    );
                }
                if let Some(VideoState::Ready(p)) = video.map(|v| &v.state)
                    && matches!(p.state(), State::Paused | State::Ended)
                    && video.is_some_and(|v| v.frame.is_some())
                {
                    let c = (
                        (local.left + local.right) / 2.0,
                        (local.top + local.bottom) / 2.0,
                    );
                    let ellipse = D2D1_ELLIPSE {
                        point: windows_numerics::Vector2 { X: c.0, Y: c.1 },
                        radiusX: 30.0,
                        radiusY: 30.0,
                    };
                    unsafe {
                        dc.FillEllipse(&ellipse, &alpha_brush(dc, 0x000000, 0.67)?);
                    }
                    icons::draw(
                        dc,
                        Icon::Play,
                        rect(c.0 - 11.0, c.1 - 13.0, c.0 + 15.0, c.1 + 13.0),
                        TEXT,
                        1.0,
                    );
                }
                Ok(())
            })?;
        }
        if let Some(enter) = v.enter.take() {
            let at = v.placed.unwrap_or_default();
            if crate::files::motion() {
                match enter {
                    Enter::Open => {
                        opacity(&self.dialog_p.visual, Some(&animation(g, 0.0, 1.0, FAST)?))?;
                        opacity(&self.viewer_p.visual, Some(&animation(g, 0.0, 1.0, BASE)?))?;
                        unsafe {
                            self.viewer_p.visual.SetOffsetY(&animation(
                                g,
                                at.1 + RISE * s,
                                at.1,
                                BASE,
                            )?)?;
                        }
                    }
                    Enter::Page(side) => {
                        opacity(&self.viewer_p.visual, Some(&animation(g, 0.0, 1.0, BASE)?))?;
                        unsafe {
                            self.viewer_p.visual.SetOffsetX(&animation(
                                g,
                                at.0 + side * PAGE_OFFSET * s,
                                at.0,
                                BASE,
                            )?)?;
                        }
                    }
                }
            } else {
                opacity(&self.dialog_p.visual, None)?;
                opacity(&self.viewer_p.visual, None)?;
            }
        }
        if let Some(next) = next {
            self.arm_viewer(next);
        }
        Ok(())
    }
}

/// What "Save" suggests: `name` without characters Windows refuses in file
/// names, and the extension to keep (from the name, else a video's type).
fn save_name(name: &str, content_type: &str) -> (String, Option<String>) {
    let name: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let extension = std::path::Path::new(&name)
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .or_else(|| match content_type {
            "video/mp4" => Some("mp4".into()),
            "video/webm" => Some("webm".into()),
            _ => None,
        });
    (name, extension)
}

/// "Save": the system save dialog owned by `window`, suggesting `name`.
/// None when cancelled.
fn save_path(window: isize, name: &str, content_type: &str) -> Option<std::path::PathBuf> {
    use windows::Win32::System::Com::*;
    use windows::Win32::UI::Shell::*;
    let (name, extension) = save_name(name, content_type);
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().ok()?;
        let path = (|| {
            let d: IFileSaveDialog =
                CoCreateInstance(&FileSaveDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            d.SetOptions(d.GetOptions().ok()? | FOS_FORCEFILESYSTEM | FOS_OVERWRITEPROMPT)
                .ok()?;
            d.SetFileName(&windows::core::HSTRING::from(name.as_str()))
                .ok()?;
            // Kept when the person types a name without one.
            if let Some(extension) = &extension {
                d.SetDefaultExtension(&windows::core::HSTRING::from(extension.as_str()))
                    .ok()?;
            }
            d.Show(Some(HWND(window as _))).ok()?;
            let shown = d.GetResult().ok()?.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let path = std::path::PathBuf::from(shown.to_string().ok()?);
            CoTaskMemFree(Some(shown.0 as _));
            Some(path)
        })();
        CoUninitialize();
        path
    }
}

/// Wakes the window's live drain from a worker thread.
fn wake(window: isize) {
    unsafe {
        let _ = PostMessageW(
            Some(HWND(window as _)),
            crate::live::WM_LIVE,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

/// The message's viewable media, starting at `id`: (items, index).
pub fn gallery(
    attachments: &[Attachment],
    revealed: &std::collections::HashSet<String>,
    id: &str,
) -> (Vec<Attachment>, usize) {
    let items: Vec<Attachment> = attachments
        .iter()
        .filter(|a| viewable(a) && !hidden(a, revealed))
        .cloned()
        .collect();
    let index = items.iter().position(|a| a.id == id).unwrap_or(0);
    (items, index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use caper_client::model::AttachmentStatus;
    use std::collections::HashSet;

    fn attachment(id: &str, kind: AttachmentKind) -> Attachment {
        Attachment {
            spoiler: false,
            id: id.into(),
            kind,
            content_type: "image/png".into(),
            name: format!("{id}.png"),
            size: 1_258_291,
            width: Some(800),
            height: Some(600),
            duration_ms: None,
            preview: false,
            url: Some(format!("https://cdn.test/original/{id}?exp=1&sig=s")),
            preview_url: None,
            unavailable: false,
            status: AttachmentStatus::Ready,
            animated: false,
            voice: false,
            waveform: None,
            transcript: None,
        }
    }

    #[test]
    fn a_message_opens_its_ready_media_in_order_without_hidden_spoilers() {
        let mut spoiler = attachment("s", AttachmentKind::Image);
        spoiler.spoiler = true;
        let mut processing = attachment("p", AttachmentKind::Video);
        processing.status = AttachmentStatus::Processing;
        let list = vec![
            attachment("a", AttachmentKind::Image),
            attachment("f", AttachmentKind::File),
            spoiler,
            processing,
            attachment("v", AttachmentKind::Video),
            attachment("c", AttachmentKind::Image),
        ];
        let (items, index) = gallery(&list, &HashSet::new(), "v");
        let ids: Vec<_> = items.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["a", "v", "c"]);
        assert_eq!(index, 1);
        // A revealed spoiler is part of the set.
        let (items, index) = gallery(&list, &HashSet::from(["s".to_owned()]), "s");
        assert_eq!((items.len(), index), (4, 1));
    }

    #[test]
    fn arrows_stop_at_the_ends_and_paging_resets_zoom() {
        let items = vec![
            attachment("a", AttachmentKind::Image),
            attachment("b", AttachmentKind::Video),
            attachment("c", AttachmentKind::Image),
        ];
        let mut v = Viewer::open(items, 9);
        assert_eq!(v.index, 2);
        assert!(!v.enabled(Part::Next) && v.enabled(Part::Previous));
        assert!(!v.go(3));
        v.zoom.scale = 3.0;
        assert!(v.go(0));
        assert_eq!(v.index, 0);
        assert_eq!(v.zoom, Zoom::default());
        assert_eq!(
            v.enter,
            Some(Enter::Page(-1.0)),
            "it slides in from the left"
        );
        assert!(!v.enabled(Part::Previous));
    }

    #[test]
    fn the_overlay_names_its_file_position_and_controls() {
        let items = vec![
            attachment("a", AttachmentKind::Image),
            attachment("b", AttachmentKind::Image),
            attachment("c", AttachmentKind::Video),
        ];
        let mut v = Viewer::open(items, 1);
        v.layout(1200.0, 800.0);
        assert_eq!(v.label(Part::Panel), "Media viewer, b.png, 2 of 3");
        assert_eq!(v.label(Part::Title), "b.png, 2 of 3, 1.2 MB");
        assert_eq!(v.label(Part::Stage), "Image b.png");
        let parts: Vec<_> = v.parts().collect();
        for p in [
            Part::Close,
            Part::Open,
            Part::Save,
            Part::Copy,
            Part::Link,
            Part::Previous,
            Part::Next,
        ] {
            assert!(parts.contains(&p), "{p:?}");
        }
        assert!(!parts.contains(&Part::Play));
        // A video swaps Copy image for its playback controls.
        v.go(2);
        v.layout(1200.0, 800.0);
        let parts: Vec<_> = v.parts().collect();
        assert!(!parts.contains(&Part::Copy));
        assert!(parts.contains(&Part::Save) && parts.contains(&Part::Link));
        assert!(parts.contains(&Part::Play) && parts.contains(&Part::Seek));
        assert!(!v.enabled(Part::Play), "not until the video is ready");
        // One file alone has no arrows or position.
        let mut one = Viewer::open(vec![attachment("a", AttachmentKind::Image)], 0);
        one.layout(1200.0, 800.0);
        assert_eq!(one.label(Part::Panel), "Media viewer, a.png");
        assert!(!one.parts().any(|p| p == Part::Next));
    }

    #[test]
    fn clicks_on_the_dark_area_miss_the_media_and_controls() {
        let mut v = Viewer::open(
            vec![
                attachment("a", AttachmentKind::Image),
                attachment("b", AttachmentKind::Image),
            ],
            0,
        );
        v.layout(1200.0, 800.0);
        let mut zoom = v.zoom;
        v.picture = zoom.place(v.stage, v.base(None));
        let (cx, cy) = (600.0, (v.stage.top + v.stage.bottom) / 2.0);
        assert_eq!(v.hit(cx, cy), Some(Part::Stage));
        // Beside the 800 × 600 picture, still on the stage: the backdrop.
        assert_eq!(v.hit(v.picture.left - 20.0, cy), None);
        assert_eq!(v.hit(1200.0 - 36.0, cy), Some(Part::Next));
        assert_eq!(v.hit(1200.0 - 34.0, 28.0), Some(Part::Close));
    }

    #[test]
    fn media_fits_without_enlarging_and_zoom_keeps_the_spot_under_the_pointer() {
        assert_eq!(fit((4000.0, 3000.0), (800.0, 800.0)), (800.0, 600.0));
        assert_eq!(fit((400.0, 300.0), (800.0, 800.0)), (400.0, 300.0));
        assert_eq!(
            clamp_pan((500.0, -500.0), (1600.0, 1200.0), (800.0, 600.0)),
            (400.0, -300.0)
        );
        let stage = rect(100.0, 50.0, 900.0, 650.0);
        let base = (800.0, 600.0);
        let pointer = (700.0, 200.0);
        let mut zoom = Zoom::default();
        let spot = |zoom: &mut Zoom| {
            let r = zoom.place(stage, base);
            (
                (pointer.0 - r.left) / (r.right - r.left),
                (pointer.1 - r.top) / (r.bottom - r.top),
            )
        };
        let before = spot(&mut zoom);
        zoom.zoom_to(2.0, pointer, stage, base);
        let after = spot(&mut zoom);
        assert!((before.0 - after.0).abs() < 1e-5 && (before.1 - after.1).abs() < 1e-5);
        zoom.zoom_to(100.0, pointer, stage, base);
        assert_eq!(zoom.scale, MAX_ZOOM);
        zoom.zoom_to(0.5, pointer, stage, base);
        assert_eq!(zoom, Zoom::default(), "back at fit it is centred again");
    }

    #[test]
    fn copied_images_keep_their_size_alpha_and_orientation_on_the_clipboard() {
        let mut image = image::RgbaImage::from_pixel(3, 2, image::Rgba([10, 20, 30, 128]));
        image.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let copied = clipboard_image(&image).unwrap();
        let header = &copied.dib[..124];
        assert_eq!(u32::from_le_bytes(header[0..4].try_into().unwrap()), 124);
        assert_eq!(i32::from_le_bytes(header[4..8].try_into().unwrap()), 3);
        assert_eq!(i32::from_le_bytes(header[8..12].try_into().unwrap()), 2);
        assert_eq!(copied.dib.len(), 124 + 3 * 2 * 4);
        // Bottom-up BGRA: the top-left red pixel starts the second row.
        assert_eq!(copied.dib[124 + 12..124 + 16], [0, 0, 255, 255]);
        assert_eq!(copied.dib[124..128], [30, 20, 10, 128]);
        let png = image::load_from_memory(&copied.png).unwrap().to_rgba8();
        assert_eq!(png, image);
    }

    #[test]
    fn save_suggests_a_name_windows_accepts_and_keeps_its_extension() {
        assert_eq!(
            save_name("photo.final.PNG", "image/png"),
            ("photo.final.PNG".into(), Some("PNG".into()))
        );
        // A link preview's video is named after its card, maybe with a path.
        assert_eq!(
            save_name("Clip: a/b?", "video/mp4"),
            ("Clip_ a_b_".into(), Some("mp4".into()))
        );
        assert_eq!(
            save_name("notes/v2.webm", "video/mp4"),
            ("notes_v2.webm".into(), Some("webm".into()))
        );
        assert_eq!(save_name("scan", "image/jpeg"), ("scan".into(), None));
    }
}
