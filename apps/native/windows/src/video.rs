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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Play,
    Seek,
    Mute,
    Expand,
    Time,
}

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
    pub position: u64,
    pub duration: u64,
    pub scrub: Option<f32>,
    pub error: Option<String>,
}

pub fn playable(a: &Attachment) -> bool {
    a.kind == AttachmentKind::Video && files::clickable(a)
}

pub fn controls(r: crate::overlay::Rect) -> Vec<(Part, crate::overlay::Rect)> {
    let (x, y, w, h) = r;
    vec![
        (Part::Play, (x + 4.0, y + h - 40.0, 28.0, 28.0)),
        (
            Part::Seek,
            (x + 40.0, y + h - 44.0, (w - 80.0).max(1.0), 18.0),
        ),
        (Part::Mute, (x + w - 32.0, y + h - 40.0, 28.0, 28.0)),
        (Part::Expand, (x + 4.0, y + 4.0, 28.0, 28.0)),
        (
            Part::Time,
            (x + 40.0, y + h - 24.0, (w - 80.0).max(1.0), 18.0),
        ),
    ]
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
    r: windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F,
) -> Result<()> {
    let v = view.filter(|v| v.id == a.id);
    let bounds = (r.left, r.top, r.right - r.left, r.bottom - r.top);
    if let Some(frame) = v.and_then(|v| v.frame.as_ref()) {
        // Fit, rather than crop, so the controls never conceal the picture.
        let stage = rect(r.left, r.top, r.right, (r.bottom - 48.0).max(r.top + 1.0));
        unsafe {
            dc.FillRectangle(&r, &g.brushes.blackout);
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
    gfx::fill_round(
        dc,
        rect(r.left, r.bottom - 48.0, r.right, r.bottom),
        0.0,
        &shade,
    );
    for (part, (x, y, w, h)) in controls(bounds) {
        let rr = rect(x, y, x + w, y + h);
        match part {
            Part::Play | Part::Mute | Part::Expand => {
                if part == Part::Expand {
                    gfx::fill_round(dc, rr, 6.0, &shade);
                }
                let icon = match part {
                    Part::Play if v.is_some_and(|v| v.playing) => Icon::Pause,
                    Part::Play => Icon::Play,
                    Part::Mute if v.is_some_and(|v| v.muted) => Icon::VolumeX,
                    Part::Mute => Icon::Volume,
                    _ => Icon::ExternalLink,
                };
                icons::draw(
                    dc,
                    icon,
                    rect(x + 6.0, y + 6.0, x + w - 6.0, y + h - 6.0),
                    0xFFFFFF,
                    1.0,
                );
            }
            Part::Seek => {
                let fraction = v
                    .map_or(0.0, |v| {
                        v.scrub.unwrap_or(if v.duration > 0 {
                            v.position as f32 / v.duration as f32
                        } else {
                            0.0
                        })
                    })
                    .clamp(0.0, 1.0);
                gfx::fill_round(
                    dc,
                    rect(x, y + 7.0, x + w, y + 11.0),
                    2.0,
                    &g.brushes.border,
                );
                if fraction > 0.0 {
                    gfx::fill_round(
                        dc,
                        rect(x, y + 7.0, x + w * fraction, y + 11.0),
                        2.0,
                        &g.brushes.text,
                    );
                }
            }
            Part::Time => {
                let text = if let Some(error) = v.and_then(|v| v.error.as_deref()) {
                    error.to_owned()
                } else if v.is_some_and(|v| !v.ready) {
                    "Loading…".into()
                } else {
                    format!(
                        "{} / {}",
                        caper_client::player::time_label(v.map_or(0, |v| v.position)),
                        caper_client::player::time_label(
                            v.map_or(a.duration_ms.unwrap_or(0), |v| v.duration)
                        )
                    )
                };
                let l = g.layout(&gfx::wide(&text), &g.formats.file_detail, w)?;
                crate::text::draw(dc, &l, x, y, &g.brushes.text);
            }
        }
        if focus == Some(part) {
            gfx::stroke_round(dc, rr, 4.0, &g.brushes.terracotta);
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
                for part in [Part::Play, Part::Seek, Part::Mute, Part::Expand, Part::Time] {
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
            Part::Mute => if v.is_some_and(|v| v.muted) {
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
            Part::Seek | Part::Mute => self.inline_video_view().is_some_and(|v| {
                v.id == a.id && v.ready && (c.part != Part::Seek || v.duration > 0)
            }),
            _ => true,
        })
    }

    pub fn video_gauge(&self, c: Control) -> Option<(u16, u16, u16)> {
        let a = self.video_attachment(c)?;
        let v = self.inline_video_view().filter(|v| v.id == a.id)?;
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
                    outside: false,
                });
            }
        } else if c.part == Part::Mute
            && let Some(v) = &mut self.playback.video
            && v.attachment.id == a.id
            && let VideoState::Ready(p) = &mut v.video.state
        {
            p.toggle_mute();
        }
        self.focus_video(c);
        self.inline_video_tick();
    }

    pub fn video_seek(&mut self, c: Control, fraction: f32) {
        let Some(target) = self.video_target(c) else {
            return;
        };
        if let Some(v) = &mut self.playback.video
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
                if c.part == Part::Seek && self.video_enabled(c) {
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
                .is_some_and(|v| v.scrub.is_some())
            && let Some(c) = focused
        {
            self.video_scrub(c, x, msg == WM_LBUTTONUP);
            if msg == WM_LBUTTONUP {
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
                    VK_LEFT | VK_RIGHT | VK_HOME | VK_END if c.part == Part::Seek => {
                        if let Some((value, _, _)) = self.video_gauge(c) {
                            let fraction = match key {
                                VK_HOME => 0.0,
                                VK_END => 1.0,
                                VK_LEFT => value as f32 / 1000.0 - 0.05,
                                _ => value as f32 / 1000.0 + 0.05,
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
            .and_then(|r| controls(r).into_iter().find(|(p, _)| *p == Part::Seek))
        else {
            return;
        };
        let fraction = ((x - r.0) / r.2).clamp(0.0, 1.0);
        if done {
            self.video_seek(c, fraction);
        } else if let Some(v) = &mut self.playback.video {
            v.scrub = Some(fraction);
            self.video_redraw();
        }
    }
}
