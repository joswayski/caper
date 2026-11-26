//! One inline video, using the media viewer's existing FFmpeg pipeline.
//! Targets follow message/attachment IDs across prepends; clipping pauses,
//! removal stops, and expanding transfers the player instead of decoding twice.
use crate::{
    App, EV_FOCUS, EV_ROWS, files,
    gfx::{self, Gfx, rect},
    icons::{self, Icon},
    theme::*,
    viewer::{Video, VideoState},
};
use caper_client::{
    model::{Attachment, AttachmentKind},
    player::State,
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Direct2D::*,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::Result,
};

pub const TIMER_VIDEO: usize = 46;
/// Clears Copy link's check.
pub const TIMER_VIDEO_COPIED: usize = 54;

/// The control bar under the picture.
pub const BAR: f32 = 44.0;
/// Room for "12:34 / 56:78" in `file_detail`.
const TIME_W: f32 = 84.0;
const VOLUME_W: f32 = 64.0;
/// How long Copy link shows a check after copying, in ms.
const COPIED_MS: u32 = 1500;
/// The bar always sits on a dark scrim, so its ink is light in every theme.
pub const INK: u32 = 0xF4F4F6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Play,
    Seek,
    Mute,
    Expand,
    Time,
    Volume,
    /// Copies the video's link (the signed CDN URL).
    Link,
}

/// Keyboard and screen reader order: the overlay buttons, then the bar.
const ORDER: [Part; 7] = [
    Part::Expand,
    Part::Link,
    Part::Play,
    Part::Time,
    Part::Seek,
    Part::Mute,
    Part::Volume,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Control {
    pub thread: bool,
    pub forward: bool,
    pub row: usize,
    pub file: usize,
    pub part: Part,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub thread: bool,
    pub forward: bool,
    pub message: String,
    pub attachment: String,
}

pub struct Inline {
    pub target: Target,
    pub attachment: Attachment,
    pub video: Video,
    pub scrub: Option<f32>,
    /// The volume slider is being dragged.
    pub volume_drag: bool,
    /// Pausing a pending probe must not start sound when the probe returns.
    pub outside: bool,
}

#[derive(Clone)]
pub struct View {
    pub id: String,
    pub frame: Option<ID2D1Bitmap1>,
    pub playing: bool,
    pub ready: bool,
    pub muted: bool,
    pub volume: f32,
    pub position: u64,
    pub duration: u64,
    pub scrub: Option<f32>,
    pub error: Option<String>,
}

pub fn playable(a: &Attachment) -> bool {
    a.kind == AttachmentKind::Video && files::clickable(a)
}

/// One row under the picture, centred on one line, as on Linux: Play, the
/// time, the seek bar, Mute and the volume. Narrow tiles drop the volume,
/// then the time, so the seek bar keeps its room. Open in viewer and Copy
/// link sit over the picture's top-left (a spoiler's Hide takes the right).
pub fn controls(r: crate::overlay::Rect) -> Vec<(Part, crate::overlay::Rect)> {
    let (x, y, w, h) = r;
    let cy = y + h - BAR / 2.0;
    let button = |left: f32| (left, cy - 14.0, 28.0, 28.0);
    let line = |left: f32, width: f32| (left, cy - 9.0, width.max(1.0), 18.0);
    let mut out = vec![
        (Part::Expand, (x + 6.0, y + 6.0, 28.0, 28.0)),
        (Part::Link, (x + 38.0, y + 6.0, 28.0, 28.0)),
        (Part::Play, button(x + 6.0)),
    ];
    let mut left = x + 6.0 + 28.0 + 8.0;
    if w >= 220.0 {
        out.push((Part::Time, line(left, TIME_W)));
        left += TIME_W + 4.0;
    }
    let mut right = x + w - 8.0;
    let volume = (w >= 300.0).then(|| {
        right -= VOLUME_W;
        (Part::Volume, line(right, VOLUME_W))
    });
    right -= if volume.is_some() { 30.0 } else { 28.0 };
    out.push((Part::Seek, line(left, right - 10.0 - left)));
    out.push((Part::Mute, button(right)));
    out.extend(volume);
    out
}

fn contains(r: crate::overlay::Rect, x: f32, y: f32) -> bool {
    x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
}

pub fn draw(
    g: &Gfx,
    dc: &ID2D1DeviceContext,
    a: &Attachment,
    view: Option<&View>,
    focus: Option<Part>,
    copied: bool,
    r: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F,
) -> Result<()> {
    let v = view.filter(|v| v.id == a.id);
    let error = v.and_then(|v| v.error.as_deref());
    let bounds = (r.left, r.top, r.right - r.left, r.bottom - r.top);
    // The bar's own room: the picture fits above it and never under it.
    let bar = rect(r.left, (r.bottom - BAR).max(r.top), r.right, r.bottom);
    if let Some(frame) = v.and_then(|v| v.frame.as_ref()) {
        let stage = rect(r.left, r.top, r.right, bar.top.max(r.top + 1.0));
        unsafe {
            // Letterboxing is black in every theme, like any video player.
            dc.FillRectangle(&r, &files::alpha(dc, 0x000000, 1.0)?);
            let size = frame.GetPixelSize();
            let scale = ((stage.right - stage.left) / size.width as f32)
                .min((stage.bottom - stage.top) / size.height as f32);
            let (w, h) = (size.width as f32 * scale, size.height as f32 * scale);
            let (x, y) = (
                (stage.left + stage.right - w) / 2.0,
                (stage.top + stage.bottom - h) / 2.0,
            );
            dc.DrawBitmap(
                frame,
                Some(&rect(x, y, x + w, y + h)),
                1.0,
                D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            );
        }
    }
    let shade = files::alpha(dc, 0x090A0C, 0.88)?;
    let track = files::alpha(dc, INK, 0.28)?;
    let ink = files::alpha(dc, INK, 1.0)?;
    unsafe { dc.FillRectangle(&bar, &shade) };
    for (part, (x, y, w, h)) in controls(bounds) {
        let rr = rect(x, y, x + w, y + h);
        match part {
            Part::Play | Part::Mute | Part::Expand | Part::Link => {
                if matches!(part, Part::Expand | Part::Link) {
                    gfx::fill_round(dc, rr, 8.0, &shade);
                }
                let icon = match part {
                    Part::Play if v.is_some_and(|v| v.playing) => Icon::Pause,
                    Part::Play => Icon::Play,
                    Part::Mute if v.is_some_and(|v| v.muted || v.volume == 0.0) => Icon::VolumeX,
                    Part::Mute => Icon::Volume,
                    Part::Link if copied => Icon::Check,
                    Part::Link => Icon::Link,
                    _ => Icon::Maximize,
                };
                icons::draw(
                    dc,
                    icon,
                    rect(x + 6.0, y + 6.0, x + w - 6.0, y + h - 6.0),
                    INK,
                    1.0,
                );
            }
            // An error takes the seek bar's room.
            Part::Seek if error.is_some() => {}
            Part::Seek | Part::Volume => {
                let fraction = if part == Part::Volume {
                    v.map_or(1.0, |v| if v.muted { 0.0 } else { v.volume })
                } else {
                    v.map_or(0.0, |v| {
                        v.scrub.unwrap_or(if v.duration > 0 {
                            v.position as f32 / v.duration as f32
                        } else {
                            0.0
                        })
                    })
                }
                .clamp(0.0, 1.0);
                let cy = y + h / 2.0;
                gfx::fill_round(dc, rect(x, cy - 2.0, x + w, cy + 2.0), 2.0, &track);
                let at = x + w * fraction;
                if fraction > 0.0 {
                    gfx::fill_round(dc, rect(x, cy - 2.0, at, cy + 2.0), 2.0, &ink);
                }
                // The thumb shows where a drag picks up.
                gfx::fill_round(dc, rect(at - 5.0, cy - 5.0, at + 5.0, cy + 5.0), 5.0, &ink);
            }
            Part::Time => {
                let text = if let Some(error) = error {
                    error.to_owned()
                } else if v.is_some_and(|v| !v.ready) {
                    "Loading\u{2026}".into()
                } else {
                    format!(
                        "{} / {}",
                        caper_client::player::time_label(v.map_or(0, |v| v.position)),
                        caper_client::player::time_label(
                            v.map_or(a.duration_ms.unwrap_or(0), |v| v.duration)
                        )
                    )
                };
                // An error runs up to Mute, over the seek bar's room.
                let room = match error {
                    Some(_) => controls(bounds)
                        .iter()
                        .find(|(p, _)| *p == Part::Mute)
                        .map_or(w, |(_, m)| (m.0 - 8.0 - x).max(w)),
                    None => w,
                };
                let l = g.layout(&gfx::wide(&text), &g.formats.file_detail, room)?;
                // Centred on the bar's line, like its buttons.
                let th = gfx::metrics(&l).height;
                crate::text::draw(dc, &l, x, y + (h - th) / 2.0, &ink);
            }
        }
        if focus == Some(part) {
            gfx::stroke_round(dc, rr, 6.0, &g.brushes.terracotta);
        }
    }
    Ok(())
}

impl App {
    pub fn video_attachment(&self, c: Control) -> Option<&Attachment> {
        let conv = if c.thread {
            &self.thread.as_ref()?.c
        } else {
            &self.c
        };
        let m = conv.messages.get(c.row)?;
        let (attachments, revealed) = if c.forward {
            let original = m
                .forward
                .as_ref()?
                .message
                .as_ref()
                .filter(|m| !m.is_deleted())?;
            (&original.content.attachments, &m.forward_revealed)
        } else {
            (&m.attachments, &m.revealed)
        };
        attachments
            .get(c.file)
            .filter(|a| playable(a) && !files::hidden(a, revealed))
    }

    fn video_target(&self, c: Control) -> Option<Target> {
        let conv = if c.thread {
            &self.thread.as_ref()?.c
        } else {
            &self.c
        };
        Some(Target {
            thread: c.thread,
            forward: c.forward,
            message: conv.messages.get(c.row)?.id.clone(),
            attachment: self.video_attachment(c)?.id.clone(),
        })
    }

    fn target_control(&self, t: &Target, part: Part) -> Option<Control> {
        let (conv, ids) = if t.thread {
            let pane = self.thread.as_ref()?;
            (&pane.c, &pane.ids)
        } else {
            (&self.c, &self.ids)
        };
        let row = *ids.rows.get(&t.message)?;
        let m = conv.messages.get(row)?;
        let attachments = if t.forward {
            &m.forward.as_ref()?.message.as_ref()?.content.attachments
        } else {
            &m.attachments
        };
        let file = attachments.iter().position(|a| a.id == t.attachment)?;
        let c = Control {
            thread: t.thread,
            forward: t.forward,
            row,
            file,
            part,
        };
        self.video_attachment(c)?;
        Some(c)
    }

    fn video_tile_rect(&self, c: Control) -> Option<crate::overlay::Rect> {
        if c.forward {
            self.forward_parts(c.thread, c.row)
                .into_iter()
                .find_map(|(p, r, _)| (p == crate::list::ForwardPart::File(c.file)).then_some(r))
        } else if c.thread {
            self.thread_file_rect(c.row, c.file)
        } else {
            self.file_rect(c.row, c.file)
        }
    }

    pub fn video_rect(&self, c: Control) -> Option<crate::overlay::Rect> {
        if !(if c.thread {
            self.thread_shown()
        } else {
            self.chat_shown()
        }) {
            return None;
        }
        self.video_attachment(c)?;
        let r = self.video_tile_rect(c)?;
        let history_y = if c.thread { HEADER_H } else { self.history_y() };
        let bottom = history_y
            + if c.thread {
                self.thread_vh()
            } else {
                self.vh()
            };
        let (p, r) = controls(r).into_iter().find(|(p, _)| *p == c.part)?;
        let _ = p;
        let top = r.1.max(history_y);
        let end = (r.1 + r.3).min(bottom);
        (end > top).then_some((r.0, top, r.2, end - top))
    }

    pub fn video_controls(&self, thread: bool, row: usize) -> Vec<Control> {
        let Some(m) = (if thread {
            self.thread.as_ref().and_then(|t| t.c.messages.get(row))
        } else {
            self.c.messages.get(row)
        }) else {
            return vec![];
        };
        let mut out = Vec::new();
        for forward in [false, true] {
            let n = if forward {
                m.forward
                    .as_ref()
                    .and_then(|f| f.message.as_ref())
                    .map_or(0, |m| m.content.attachments.len())
            } else {
                m.attachments.len()
            };
            for file in 0..n {
                for part in ORDER {
                    let c = Control {
                        thread,
                        forward,
                        row,
                        file,
                        part,
                    };
                    if self.video_rect(c).is_some() {
                        out.push(c);
                    }
                }
            }
        }
        out
    }

    pub fn all_video_controls(&self) -> Vec<Control> {
        self.visible_rows()
            .flat_map(|i| self.video_controls(false, i))
            .chain(
                self.thread.iter().flat_map(|t| {
                    (0..t.c.messages.len()).flat_map(|i| self.video_controls(true, i))
                }),
            )
            .collect()
    }

    pub fn video_focus(&self) -> Option<Control> {
        if self.composer.focused
            || self.thread.as_ref().is_some_and(|t| t.composer.focused)
            || self.kb_control().is_some()
        {
            return None;
        }
        let (target, part) = self.playback.video_focus.as_ref()?;
        self.target_control(target, *part)
            .filter(|c| self.video_rect(*c).is_some())
    }

    pub fn video_focus_view(&self) -> Option<(String, Part)> {
        let c = self.video_focus()?;
        Some((self.video_attachment(c)?.id.clone(), c.part))
    }

    pub fn focus_video(&mut self, c: Control) {
        let Some(target) = self.video_target(c) else {
            return;
        };
        self.set_kb(None);
        self.set_composer_focus(false);
        self.set_thread_focus(false);
        self.playback.video_focus = Some((target, c.part));
        self.video_redraw();
        self.events |= EV_FOCUS;
    }

    pub fn inline_video_view(&self) -> Option<View> {
        let v = self.playback.video.as_ref()?;
        let player = match &v.video.state {
            VideoState::Ready(p) => Some(p),
            _ => None,
        };
        Some(View {
            id: v.attachment.id.clone(),
            frame: v.video.frame.clone(),
            playing: player.is_some_and(|p| matches!(p.state(), State::Playing | State::Loading)),
            ready: player.is_some(),
            muted: player.is_some_and(|p| p.muted()),
            volume: player.map_or(1.0, |p| p.volume()),
            position: player.map_or(0, |p| p.position_ms()),
            duration: player
                .and_then(|p| p.duration_ms())
                .or(v.attachment.duration_ms)
                .unwrap_or(0),
            scrub: v.scrub,
            error: match &v.video.state {
                VideoState::Failed(e) => Some(e.clone()),
                VideoState::Ready(p) => p.error().map(str::to_owned),
                _ => None,
            },
        })
    }

    pub fn video_name(&self, c: Control) -> String {
        let Some(a) = self.video_attachment(c) else {
            return String::new();
        };
        let v = self.inline_video_view().filter(|v| v.id == a.id);
        match c.part {
            Part::Play => format!(
                "{} {}",
                if v.is_some_and(|v| v.playing) {
                    "Pause"
                } else {
                    "Play"
                },
                a.name
            ),
            Part::Link => "Copy video link".into(),
            Part::Volume => "Volume".into(),
            Part::Mute => if v.is_some_and(|v| v.muted || v.volume == 0.0) {
                "Unmute video"
            } else {
                "Mute video"
            }
            .into(),
            Part::Expand => format!("Open {} in viewer", a.name),
            Part::Seek => format!("Seek {}", a.name),
            Part::Time => v.map_or_else(
                || caper_client::player::time_label(a.duration_ms.unwrap_or(0)),
                |v| {
                    v.error.unwrap_or_else(|| {
                        format!(
                            "{} / {}",
                            caper_client::player::time_label(v.position),
                            caper_client::player::time_label(v.duration)
                        )
                    })
                },
            ),
        }
    }

    pub fn video_enabled(&self, c: Control) -> bool {
        self.video_attachment(c).is_some_and(|a| match c.part {
            Part::Time => false,
            Part::Link => a.url.is_some(),
            Part::Seek | Part::Mute | Part::Volume => self.inline_video_view().is_some_and(|v| {
                v.id == a.id && v.ready && (c.part != Part::Seek || v.duration > 0)
            }),
            _ => true,
        })
    }

    pub fn video_gauge(&self, c: Control) -> Option<(u16, u16, u16)> {
        let a = self.video_attachment(c)?;
        let v = self.inline_video_view().filter(|v| v.id == a.id)?;
        if c.part == Part::Volume {
            let volume = if v.muted { 0.0 } else { v.volume };
            return v
                .ready
                .then_some(((volume * 100.0).round() as u16, 100, 10));
        }
        (c.part == Part::Seek && v.ready && v.duration > 0).then(|| {
            (
                (v.position.saturating_mul(1000) / v.duration).min(1000) as u16,
                1000,
                10,
            )
        })
    }

    fn video_redraw(&mut self) {
        self.redraw.extend(self.visible_rows());
        self.dirty.thread = true;
        self.events |= EV_ROWS;
        self.need = true;
    }

    pub fn video_copied_tick(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_VIDEO_COPIED);
        }
        if self.playback.video_copied.take().is_some() {
            self.video_redraw();
        }
    }

    pub fn stop_inline_video(&mut self) {
        if let Some(v) = self.playback.video.take() {
            self.stop_viewer_video(Some(v.video));
            self.video_redraw();
        }
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_VIDEO);
        }
    }

    pub fn inline_video_retry(&mut self) {
        let Some(current) = self.playback.video.as_ref().and_then(|v| {
            self.live
                .as_ref()
                .map(|l| l.media.resolve(&v.attachment))
                .filter(|a| v.video.needs_retry(a))
        }) else {
            return;
        };
        let Some(mut inline) = self.playback.video.take() else {
            return;
        };
        self.stop_viewer_video(Some(inline.video));
        inline.video = self.start_video(&current, true);
        inline.attachment = current;
        self.playback.video = Some(inline);
        self.inline_video_tick();
    }

    pub fn video_activate(&mut self, c: Control) {
        let Some(a) = self.video_attachment(c).cloned() else {
            return;
        };
        if !self.video_enabled(c) {
            return;
        }
        if c.part == Part::Expand {
            let conv = if c.thread {
                &self.thread.as_ref().unwrap().c
            } else {
                &self.c
            };
            let m = &conv.messages[c.row];
            let (items, index) = if c.forward {
                crate::viewer::gallery(
                    &m.forward
                        .as_ref()
                        .unwrap()
                        .message
                        .as_ref()
                        .unwrap()
                        .content
                        .attachments,
                    &m.forward_revealed,
                    &a.id,
                )
            } else {
                crate::viewer::gallery(&m.attachments, &m.revealed, &a.id)
            };
            self.open_viewer(items, index);
            return;
        }
        if c.part == Part::Link {
            // The signed CDN link the app already holds, as the viewer's
            // Copy media link; it expires with its signature.
            if let Some(url) = self.live.as_ref().and_then(|l| l.media.resolve(&a).url) {
                crate::set_clipboard(self.hwnd, &gfx::wide(&url));
                self.playback.video_copied = Some(a.id.clone());
                unsafe {
                    SetTimer(Some(self.hwnd), TIMER_VIDEO_COPIED, COPIED_MS, None);
                }
            }
            self.focus_video(c);
            return;
        }
        if c.part == Part::Play {
            let target = self.video_target(c).unwrap();
            if let Some(v) = &mut self.playback.video
                && v.target == target
            {
                if let VideoState::Ready(p) = &mut v.video.state {
                    p.toggle();
                    v.outside = false;
                } else if matches!(v.video.state, VideoState::Probing(_)) {
                    v.outside = !v.outside;
                }
            } else {
                self.stop_inline_video();
                self.playback.stop();
                let current = self
                    .live
                    .as_ref()
                    .map_or(a.clone(), |l| l.media.resolve(&a));
                let video = self.start_video(&current, false);
                self.playback.video = Some(Inline {
                    target,
                    attachment: current,
                    video,
                    scrub: None,
                    volume_drag: false,
                    outside: false,
                });
            }
        } else if c.part == Part::Mute
            && let Some(v) = &mut self.playback.video
            && v.attachment.id == a.id
            && let VideoState::Ready(p) = &mut v.video.state
        {
            // Unmuting a slider dragged to nothing brings the sound back.
            if p.volume() == 0.0 {
                p.set_volume(1.0);
                if p.muted() {
                    p.toggle_mute();
                }
            } else {
                p.toggle_mute();
            }
        }
        self.focus_video(c);
        self.inline_video_tick();
    }

    pub fn video_seek(&mut self, c: Control, fraction: f32) {
        let Some(target) = self.video_target(c) else {
            return;
        };
        if c.part == Part::Volume {
            if let Some(v) = &mut self.playback.video
                && v.target == target
                && let VideoState::Ready(p) = &mut v.video.state
            {
                p.set_volume(fraction);
                // Turning it up unmutes, as on Linux and the web.
                if p.muted() && fraction > 0.0 {
                    p.toggle_mute();
                }
            }
        } else if let Some(v) = &mut self.playback.video
            && v.target == target
            && let VideoState::Ready(p) = &mut v.video.state
            && let Some(duration) = p.duration_ms()
        {
            let playing = matches!(p.state(), State::Playing | State::Loading | State::Ended);
            p.seek((fraction.clamp(0.0, 1.0) * duration as f32) as u64, playing);
            v.scrub = None;
        }
        self.video_redraw();
        self.inline_video_tick();
    }

    /// Called on scroll/layout as well as the timer; a paused probe cannot
    /// start sound later just because the worker finishes off-screen.
    pub fn inline_video_follow(&mut self) {
        let Some(v) = &self.playback.video else {
            return;
        };
        let Some(c) = self.target_control(&v.target, Part::Play) else {
            self.stop_inline_video();
            return;
        };
        let history_y = if c.thread { HEADER_H } else { self.history_y() };
        let visible = (if c.thread {
            self.thread_shown()
        } else {
            self.chat_shown()
        }) && !self.narrow.menu
            && !(self.members_w() > 0.0 && !self.members_tiled())
            && self.viewer.is_none()
            && self.signin.is_none()
            && self.settings.is_none()
            && self.dialog.is_none()
            && self.pins.is_none()
            && !unsafe { IsIconic(self.hwnd) }.as_bool()
            && self.video_tile_rect(c).is_some_and(|r| {
                r.1 + r.3 > history_y
                    && r.1
                        < history_y
                            + if c.thread {
                                self.thread_vh()
                            } else {
                                self.vh()
                            }
            });
        if !visible && let Some(v) = &mut self.playback.video {
            let changed = !v.outside;
            v.outside = true;
            v.scrub = None;
            if let VideoState::Ready(p) = &mut v.video.state {
                p.pause();
            }
            if changed {
                self.video_redraw();
            }
        }
    }

    pub fn inline_video_tick(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_VIDEO);
        }
        self.inline_video_follow();
        let Some(v) = &mut self.playback.video else {
            return;
        };
        if v.outside && matches!(v.video.state, VideoState::Probing(_)) {
            return;
        }
        let polled = v.video.poll(
            v.attachment.duration_ms,
            v.attachment.animated,
            [1920.0, 1080.0],
        );
        if polled == Some(true)
            && let Some(l) = &mut self.live
        {
            l.media.refresh_now(&v.attachment.id);
        }
        if v.outside
            && let VideoState::Ready(p) = &mut v.video.state
        {
            p.pause();
        }
        let (frame, clock, next) = v
            .video
            .tick(&self.g.dc, v.attachment.animated && !v.outside);
        let busy = matches!(v.video.state, VideoState::Probing(_));
        if frame || clock || polled.is_some() {
            self.video_redraw();
        }
        if let Some(next) = next.or(busy.then_some(std::time::Duration::from_millis(100))) {
            unsafe {
                SetTimer(
                    Some(self.hwnd),
                    TIMER_VIDEO,
                    next.as_millis().clamp(10, 100) as u32,
                    None,
                );
            }
        }
    }

    pub fn video_input(
        &mut self,
        msg: u32,
        wp: WPARAM,
        x: f32,
        y: f32,
        ctrl: bool,
        shift: bool,
    ) -> Option<LRESULT> {
        if self.viewer.is_some()
            || self.signin.is_some()
            || self.settings.is_some()
            || self.dialog.is_some()
            || self.profile.is_some()
            || self.pins.is_some()
            || self.attachment_editor.is_some()
            || self.menu.is_some()
            || self.picker.is_some()
            || self.gifs.is_some()
        {
            return None;
        }
        let focused = self.video_focus();
        if matches!(msg, WM_CAPTURECHANGED | WM_KILLFOCUS | WM_CANCELMODE)
            && let Some(v) = &mut self.playback.video
        {
            v.scrub = None;
            v.volume_drag = false;
        }
        if matches!(
            msg,
            WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MOUSEWHEEL | WM_KILLFOCUS
        ) {
            self.playback.video_focus = None;
            if focused.is_some() {
                self.video_redraw();
                self.events |= EV_FOCUS;
            }
        }
        if msg == WM_LBUTTONDOWN {
            let mut hit = self.all_video_controls().into_iter().find(|c| {
                c.part != Part::Time && self.video_rect(*c).is_some_and(|r| contains(r, x, y))
            });
            if hit.is_none() && (self.in_history(x, y) || self.in_thread(x, y)) {
                hit = self
                    .all_video_controls()
                    .into_iter()
                    .filter(|c| c.part == Part::Play)
                    .find(|c| {
                        self.video_tile_rect(*c).is_some_and(|r| {
                            let hide = files::hide_rect(rect(r.0, r.1, r.0 + r.2, r.1 + r.3)).0;
                            let hide = (
                                hide.left,
                                hide.top,
                                hide.right - hide.left,
                                hide.bottom - hide.top,
                            );
                            contains(r, x, y)
                                && !(self.video_attachment(*c).is_some_and(|a| a.spoiler)
                                    && contains(hide, x, y))
                        })
                    });
            }
            if let Some(c) = hit {
                if c.part == Part::Volume && self.video_enabled(c) {
                    self.focus_video(c);
                    if let Some(v) = &mut self.playback.video {
                        v.volume_drag = true;
                    }
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                    self.video_scrub(c, x, false);
                } else if c.part == Part::Seek && self.video_enabled(c) {
                    self.focus_video(c);
                    if let Some(v) = &mut self.playback.video {
                        v.scrub = Some(0.0);
                    }
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                    self.video_scrub(c, x, false);
                } else {
                    self.video_activate(c);
                }
                return Some(LRESULT(0));
            }
        }
        if matches!(msg, WM_MOUSEMOVE | WM_LBUTTONUP)
            && self
                .playback
                .video
                .as_ref()
                .is_some_and(|v| v.scrub.is_some() || v.volume_drag)
            && let Some(c) = focused
        {
            self.video_scrub(c, x, msg == WM_LBUTTONUP);
            if msg == WM_LBUTTONUP {
                if let Some(v) = &mut self.playback.video {
                    v.volume_drag = false;
                }
                unsafe {
                    let _ = ReleaseCapture();
                }
            }
            return Some(LRESULT(0));
        }
        if msg == WM_KEYDOWN && !ctrl {
            let key = VIRTUAL_KEY(wp.0 as u16);
            if key == VK_TAB
                && (focused.is_some()
                    || self.kb_control().is_none()
                        && (!(self.composer.focused
                            || self.thread.as_ref().is_some_and(|t| t.composer.focused))
                            || shift))
            {
                let all: Vec<_> = self
                    .all_video_controls()
                    .into_iter()
                    .filter(|c| c.part != Part::Time && self.video_enabled(*c))
                    .collect();
                let i = focused.and_then(|c| all.iter().position(|p| *p == c));
                let next = if shift {
                    i.map_or_else(|| all.len().checked_sub(1), |i| i.checked_sub(1))
                } else {
                    Some(i.map_or(0, |i| i + 1))
                };
                if let Some(c) = next.and_then(|i| all.get(i)) {
                    self.focus_video(*c);
                    return Some(LRESULT(0));
                }
                self.playback.video_focus = None;
                self.video_redraw();
                if focused.is_some() && !shift {
                    if focused.is_some_and(|c| c.thread) {
                        self.set_thread_focus(true);
                    } else {
                        self.set_composer_focus(true);
                    }
                    return Some(LRESULT(0));
                }
            } else if let Some(c) = focused {
                match key {
                    VK_RETURN | VK_SPACE => self.video_activate(c),
                    VK_LEFT | VK_RIGHT | VK_HOME | VK_END
                        if matches!(c.part, Part::Seek | Part::Volume) =>
                    {
                        if let Some((value, max, _)) = self.video_gauge(c) {
                            let at = f32::from(value) / f32::from(max);
                            let step = if c.part == Part::Volume { 0.1 } else { 0.05 };
                            let fraction = match key {
                                VK_HOME => 0.0,
                                VK_END => 1.0,
                                VK_LEFT => at - step,
                                _ => at + step,
                            };
                            self.video_seek(c, fraction);
                        }
                    }
                    VK_ESCAPE => {
                        self.playback.video_focus = None;
                        if c.thread {
                            self.set_thread_focus(true);
                        } else {
                            self.set_composer_focus(true);
                        }
                        self.video_redraw();
                    }
                    _ => return None,
                }
                return Some(LRESULT(0));
            }
        }
        None
    }

    fn video_scrub(&mut self, c: Control, x: f32, done: bool) {
        let Some((_, r)) = self
            .video_tile_rect(c)
            .and_then(|r| controls(r).into_iter().find(|(p, _)| *p == c.part))
        else {
            return;
        };
        let fraction = ((x - r.0) / r.2).clamp(0.0, 1.0);
        // Volume follows the pointer; a seek waits for the release.
        if done || c.part == Part::Volume {
            self.video_seek(c, fraction);
        } else if let Some(v) = &mut self.playback.video {
            v.scrub = Some(fraction);
            self.video_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_is_one_line_that_fits_and_never_overlaps() {
        for (width, time, volume) in [
            (180.0, false, false),
            (260.0, true, false),
            (400.0, true, true),
        ] {
            let tile = (10.0, 20.0, width, 300.0);
            let parts = controls(tile);
            let bar: Vec<_> = parts
                .iter()
                .filter(|(_, r)| r.1 > 20.0 + 300.0 - BAR)
                .collect();
            assert_eq!(bar.iter().any(|(p, _)| *p == Part::Time), time, "{width}");
            assert_eq!(
                bar.iter().any(|(p, _)| *p == Part::Volume),
                volume,
                "{width}"
            );
            for (_, r) in &bar {
                // Inside the tile, centred on the bar's line.
                assert!(
                    r.0 >= tile.0 && r.0 + r.2 <= tile.0 + width,
                    "{width}: {r:?}"
                );
                assert!((r.1 + r.3 / 2.0 - (20.0 + 300.0 - BAR / 2.0)).abs() < 0.01);
            }
            let mut spans: Vec<_> = bar.iter().map(|(_, r)| (r.0, r.0 + r.2)).collect();
            spans.sort_by(|a, b| a.0.total_cmp(&b.0));
            assert!(
                spans.windows(2).all(|w| w[0].1 <= w[1].0),
                "{width}: {spans:?}"
            );
            let seek = bar.iter().find(|(p, _)| *p == Part::Seek).unwrap().1;
            assert!(seek.2 >= 60.0, "{width}: the seek bar keeps room");
        }
    }
}
