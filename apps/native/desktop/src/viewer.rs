//! The in-app media viewer, like web's `MediaViewer.tsx`: clicking an image or
//! video opens it almost full screen over the conversation, with the rest of
//! that message's media one arrow (or Left/Right) away.
//!
//! Images load at full size: PNG/JPEG/WebP/GIF with the `image` codecs
//! (animated GIF and WebP play), AVIF and HEIC through the bundled FFmpeg.
//! Videos play in [`Player`], streamed through [`MediaProxy`]. Signed URLs
//! come from [`Media::resolve`], so a refreshed URL is picked up, and a load
//! that fails with an expired URL asks for one refresh.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, Stroke};

use crate::attachments::{ImageState, Media};
use crate::media_proxy::MediaProxy;
use crate::model::{Attachment, AttachmentKind, AttachmentStatus};
use crate::player::{self, Player, State};
use crate::{BORDER, MUTED, NavIcon, TEXT, paint_icon};

const TOP_BAR: f32 = 56.0;
const CONTROLS: f32 = 52.0;
const SIDE: f32 = 72.0;
const MAX_ZOOM: f32 = 8.0;
const DOUBLE_CLICK_ZOOM: f32 = 2.5;
/// Full images are decoded at most this large on their long side.
const MAX_EDGE: u32 = 4096;
const MAX_ANIMATION_FRAMES: usize = 500;
const MAX_ANIMATION_BYTES: usize = 512 * 1024 * 1024;
/// Videos decode at most at 1080p however large the window.
const MAX_VIDEO: egui::Vec2 = egui::vec2(1920.0, 1080.0);
const NOTICE: Duration = Duration::from_secs(4);
// egui's Context::copy_image keeps its clipboard alive, but its output API
// reports no write result (egui-winit only logs failures). Keep our fallible
// writer alive for the process instead: dropping it loses Linux ownership,
// including when the viewer closes. Access happens only on copy workers.
static CLIPBOARD: std::sync::Mutex<Option<arboard::Clipboard>> = std::sync::Mutex::new(None);
/// One-finger drags past these, while not zoomed, change file or close
/// (web's `viewer.ts`).
const SWIPE_DISTANCE: f32 = 60.0;
const CLOSE_DISTANCE: f32 = 100.0;
/// How far a finger moves before it swipes rather than taps.
const SWIPE_SLOP: f32 = 10.0;
/// How far a file turned to by a swipe or arrow slides in from.
const PAGE_OFFSET: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipeAction {
    Next,
    Previous,
    Close,
}

/// What a finished one-finger drag means while not zoomed, as web's
/// `swipeAction`: left shows the next file, like paging; down closes.
pub fn swipe_action(dx: f32, dy: f32) -> Option<SwipeAction> {
    if dx.abs() > dy.abs() {
        if dx <= -SWIPE_DISTANCE {
            Some(SwipeAction::Next)
        } else if dx >= SWIPE_DISTANCE {
            Some(SwipeAction::Previous)
        } else {
            None
        }
    } else if dy >= CLOSE_DISTANCE {
        Some(SwipeAction::Close)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    Horizontal,
    Vertical,
}

/// A finger on the media. It locks to one axis once it moves past the slop,
/// and the media follows it on that axis (downwards only, to close).
struct Swipe {
    device: egui::TouchDeviceId,
    id: egui::TouchId,
    start: egui::Pos2,
    axis: Option<Axis>,
    offset: egui::Vec2,
}

impl Swipe {
    fn follow(&mut self, pos: egui::Pos2) {
        let delta = pos - self.start;
        if self.axis.is_none() && delta.length() > SWIPE_SLOP {
            self.axis = Some(if delta.x.abs() > delta.y.abs() {
                Axis::Horizontal
            } else {
                Axis::Vertical
            });
        }
        self.offset = match self.axis {
            Some(Axis::Horizontal) => egui::vec2(delta.x, 0.0),
            Some(Axis::Vertical) => egui::vec2(0.0, delta.y.max(0.0)),
            None => egui::Vec2::ZERO,
        };
    }

    fn action(&self, pos: egui::Pos2) -> Option<SwipeAction> {
        let delta = pos - self.start;
        match self.axis? {
            Axis::Horizontal => swipe_action(delta.x, 0.0),
            Axis::Vertical => swipe_action(0.0, delta.y),
        }
    }
}

/// What the viewer shows: ready images and videos with a URL.
pub fn viewable(attachment: &Attachment) -> bool {
    !attachment.unavailable
        && matches!(
            attachment.kind,
            AttachmentKind::Image | AttachmentKind::Video
        )
        && attachment.status == AttachmentStatus::Ready
        && attachment.url.is_some()
}

enum Full {
    Loading { url: String },
    Ready(Picture),
    Failed { url: String },
}

/// One still or an animation's frames with their delays.
struct Picture {
    frames: Vec<(egui::TextureHandle, Duration)>,
    size: egui::Vec2,
    started: Instant,
}

impl Picture {
    fn texture(&self) -> &egui::TextureHandle {
        let total: Duration = self.frames.iter().map(|(_, delay)| *delay).sum();
        if self.frames.len() < 2 || total.is_zero() {
            return &self.frames[0].0;
        }
        let mut at =
            Duration::from_nanos((self.started.elapsed().as_nanos() % total.as_nanos()) as u64);
        for (texture, delay) in &self.frames {
            if at < *delay {
                return texture;
            }
            at -= *delay;
        }
        &self.frames[0].0
    }

    /// Time until the next animation frame.
    fn next_frame(&self) -> Option<Duration> {
        (self.frames.len() > 1).then_some(Duration::from_millis(16))
    }
}

struct Loaded {
    id: String,
    url: String,
    result: Result<Vec<(egui::ColorImage, Duration)>, Option<u16>>,
}

enum VideoState {
    Probing(Receiver<Option<crate::ffmpeg::Probe>>),
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
}

pub struct Viewer {
    items: Vec<Attachment>,
    index: usize,
    zoom: Zoom,
    full: HashMap<String, Full>,
    sender: Sender<Loaded>,
    receiver: Receiver<Loaded>,
    video: Option<Video>,
    /// The seek bar while it is dragged, 0–1.
    scrub: Option<f32>,
    saving: Option<Receiver<Result<String, String>>>,
    copying: Option<Receiver<Result<String, String>>>,
    notice: Option<(String, Instant)>,
    opened: bool,
    swipe: Option<Swipe>,
    /// An unfinished swipe's offset easing back, from when it let go.
    returning: Option<(egui::Vec2, f64)>,
    /// The side (-1 left, 1 right) a newly turned-to file slides in from, and
    /// when it started once drawn.
    paged: Option<(f32, Option<f64>)>,
    /// Where the video's control bar starts on the layer, so swipes move
    /// only the picture.
    controls_mark: Option<usize>,
    /// Original message-local, ephemeral spoiler reveal state (not draft previews).
    reveal_scope: Option<egui::Id>,
}

impl Viewer {
    /// Profile pictures use stable asset keys, not message attachment IDs.
    pub fn profile_picture(source: crate::pictures::Source<'_>, name: &str) -> Option<Attachment> {
        Some(Attachment {
            id: format!("profile-picture:{}", source.asset),
            name: format!("{name} profile picture"),
            kind: AttachmentKind::Image,
            content_type: "image/webp".into(),
            url: Some(source.url?.to_owned()),
            preview_url: None,
            size: 0,
            width: None,
            height: None,
            duration_ms: None,
            preview: false,
            unavailable: false,
            status: AttachmentStatus::Ready,
            animated: false,
            spoiler: false,
        })
    }

    pub fn picture_menu(&mut self, ui: &mut egui::Ui, current: &Attachment, media: &mut Media) {
        if crate::message_menu_item(ui, NavIcon::Copy, "Copy image", self.copying.is_none())
            .clicked()
        {
            self.copy(current, media, ui.ctx(), true);
            ui.close();
        }
        if crate::message_menu_item(ui, NavIcon::ExternalLink, "Open in browser", true).clicked() {
            open_browser(ui.ctx(), current, media, chrono::Utc::now().timestamp());
            ui.close();
        }
    }

    pub fn open(items: Vec<Attachment>, index: usize) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            index: index.min(items.len().saturating_sub(1)),
            items,
            zoom: Zoom::default(),
            full: HashMap::new(),
            sender,
            receiver,
            video: None,
            scrub: None,
            saving: None,
            copying: None,
            notice: None,
            opened: false,
            swipe: None,
            returning: None,
            paged: None,
            controls_mark: None,
            reveal_scope: None,
        }
    }

    pub fn with_reveal_scope(mut self, scope: egui::Id) -> Self {
        self.reveal_scope = Some(scope);
        self
    }

    fn hide_spoiler(&self, context: &egui::Context, current: &Attachment) {
        if let Some(scope) = self.reveal_scope {
            context.data_mut(|data| {
                data.insert_temp(scope.with(("attachment-spoiler", &current.id)), false)
            });
        }
    }

    #[cfg(test)]
    pub fn index(&self) -> usize {
        self.index
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    fn go(&mut self, index: usize) {
        if index < self.items.len() && index != self.index {
            self.paged = Some((if index > self.index { 1.0 } else { -1.0 }, None));
            self.swipe = None;
            self.returning = None;
            self.index = index;
            self.zoom = Zoom::default();
            self.scrub = None;
        }
    }

    /// Stop playback and free the stream route (closing or leaving a video).
    pub fn stop_video(&mut self, proxy: Option<&MediaProxy>) {
        if let Some(video) = self.video.take()
            && let (Some(proxy), Some(local)) = (proxy, &video.local)
        {
            proxy.remove(local);
        }
    }

    /// Use the same streaming player inside a message, without modal handling.
    pub fn show_inline(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        current: &Attachment,
        media: &mut Media,
        proxy: &mut Option<MediaProxy>,
    ) {
        self.prepare_video(current, media, proxy);
        // Keep the controls inside the attachment rather than over its reactions.
        let mut stage = rect;
        if !current.animated {
            stage.max.y = (rect.bottom() - CONTROLS - 8.0).max(rect.top() + 1.0);
        }
        ui.push_id(("inline-video", &current.id), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(rect));
            self.video_stage(ui, stage, current, media);
        });
    }

    /// Draw the viewer over everything. Returns false once it should close.
    pub fn show(
        &mut self,
        context: &egui::Context,
        media: &mut Media,
        proxy: &mut Option<MediaProxy>,
        unix_now: i64,
    ) -> bool {
        if !self.opened {
            self.opened = true;
            // The composer or a field below must not keep typing.
            if let Some(focused) = context.memory(|memory| memory.focused()) {
                context.memory_mut(|memory| memory.surrender_focus(focused));
            }
        }
        if self.items.is_empty() {
            return false;
        }
        self.receive(context);
        if let Some(copying) = &self.copying {
            match copying.try_recv() {
                Ok(result) => {
                    self.copying = None;
                    self.notice = Some((result.unwrap_or_else(|error| error), Instant::now()));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.copying = None;
                    self.notice = Some(("Couldn’t copy the media.".into(), Instant::now()));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(saving) = &self.saving {
            // A failure (including a stalled download) re-enables Save.
            match saving.try_recv() {
                Ok(result) => {
                    self.saving = None;
                    self.notice = Some((result.unwrap_or_else(|error| error), Instant::now()));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.saving = None;
                    self.notice = Some((
                        "Couldn’t save the file. Please try again.".into(),
                        Instant::now(),
                    ));
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        let (previous, next, toggle) = context.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft),
                input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight),
                input.consume_key(egui::Modifiers::NONE, egui::Key::Space),
            )
        });
        if previous && self.index > 0 {
            self.go(self.index - 1);
        }
        if next {
            self.go(self.index + 1);
        }
        // Touch swipes turn the page or close before anything is drawn, so
        // the whole frame shows the file they leave.
        let screen = context.content_rect();
        let video = self.items[self.index].kind == AttachmentKind::Video;
        let now = context.input(|input| input.time);
        if self.track_swipe(context, stage_rect(screen, video), now) {
            self.stop_video(proxy.as_ref());
            return false;
        }
        let current = media.resolve(&self.items[self.index]);
        if !viewable(&current) {
            // Removed while open.
            self.stop_video(proxy.as_ref());
            return false;
        }
        match current.kind {
            AttachmentKind::Video => self.prepare_video(&current, media, proxy),
            _ => {
                self.stop_video(proxy.as_ref());
                self.prepare_image(context, &current, media);
            }
        }
        if toggle
            && let Some(VideoState::Ready(player)) =
                self.video.as_mut().map(|video| &mut video.state)
        {
            player.toggle();
        }
        let response = egui::Modal::new(egui::Id::new("media-viewer"))
            .frame(egui::Frame::NONE)
            // Nearly opaque: GPUs that blend in linear light make lighter
            // alphas look much brighter than on web.
            .backdrop_color(Color32::from_black_alpha(250))
            .show(context, |ui| {
                self.contents(ui, screen, &current, media, unix_now)
            });
        let close = response.inner || response.should_close();
        if close {
            self.stop_video(proxy.as_ref());
        }
        !close
    }

    fn receive(&mut self, context: &egui::Context) {
        for loaded in self.receiver.try_iter() {
            let Some(Full::Loading { url }) = self.full.get(&loaded.id) else {
                continue;
            };
            if *url != loaded.url {
                continue;
            }
            let entry = match loaded.result {
                Ok(frames) if !frames.is_empty() => {
                    let [width, height] = frames[0].0.size;
                    Full::Ready(Picture {
                        frames: frames
                            .into_iter()
                            .enumerate()
                            .map(|(index, (image, delay))| {
                                (
                                    context.load_texture(
                                        format!("viewer-{}-{index}", loaded.id),
                                        image,
                                        egui::TextureOptions::LINEAR,
                                    ),
                                    delay,
                                )
                            })
                            .collect(),
                        size: egui::vec2(width as f32, height as f32),
                        started: Instant::now(),
                    })
                }
                _ => Full::Failed { url: loaded.url },
            };
            self.full.insert(loaded.id, entry);
        }
    }

    fn prepare_image(&mut self, context: &egui::Context, current: &Attachment, media: &mut Media) {
        let Some(url) = current.url.clone() else {
            return;
        };
        let start = match self.full.get(&current.id) {
            None => true,
            // A refreshed URL gets one more try.
            Some(Full::Failed { url: failed }) => *failed != url,
            _ => false,
        };
        if !start {
            return;
        }
        // Only this and its neighbours stay decoded: paging through a
        // message's full-size images must not keep every texture alive.
        let keep: Vec<String> = self
            .items
            .iter()
            .skip(self.index.saturating_sub(1))
            .take(3)
            .map(|item| item.id.clone())
            .collect();
        self.full.retain(|id, _| keep.contains(id));
        let Some(api) = media.api().cloned() else {
            return;
        };
        self.full
            .insert(current.id.clone(), Full::Loading { url: url.clone() });
        let sender = self.sender.clone();
        let context = context.clone();
        let (id, content_type) = (current.id.clone(), current.content_type.clone());
        // Some GPUs (and GLES fallbacks) take less than MAX_EDGE.
        let edge = context.input(|input| input.max_texture_side);
        let edge = MAX_EDGE.min(u32::try_from(edge).unwrap_or(u32::MAX));
        std::thread::spawn(move || {
            let result = api
                .fetch_media(&url)
                .and_then(|bytes| decode_full(&bytes, &content_type, edge).ok_or(None));
            let _ = sender.send(Loaded { id, url, result });
            context.request_repaint();
        });
    }

    fn prepare_video(
        &mut self,
        current: &Attachment,
        media: &mut Media,
        proxy: &mut Option<MediaProxy>,
    ) {
        let Some(url) = current.url.clone() else {
            return;
        };
        if let Some(video) = &self.video {
            let failed = matches!(video.state, VideoState::Failed(_));
            // Same video: keep playing; after a failure, a refreshed URL retries once.
            if video.id == current.id && (!failed || video.url == url || video.retried) {
                return;
            }
        }
        let retried = self
            .video
            .as_ref()
            .is_some_and(|video| video.id == current.id);
        self.stop_video(proxy.as_ref());
        let Some(binary) = crate::ffmpeg::binary() else {
            self.video = Some(Video {
                id: current.id.clone(),
                url,
                local: None,
                state: VideoState::Failed(
                    "Video playback needs the FFmpeg that ships with Caper.".into(),
                ),
                retried: true,
            });
            return;
        };
        if proxy.is_none() {
            *proxy = MediaProxy::start().ok();
        }
        let (Some(proxy), Some(_)) = (proxy.as_ref(), crate::api::media_url(&url)) else {
            self.video = Some(Video {
                id: current.id.clone(),
                url,
                local: None,
                state: VideoState::Failed("This video could not be streamed.".into()),
                retried: true,
            });
            return;
        };
        let local = proxy.route(&url);
        let (sender, receiver) = mpsc::channel();
        let input = local.clone();
        std::thread::spawn(move || {
            let _ = sender.send(crate::ffmpeg::probe(binary, Path::new(&input)).ok());
        });
        if !retried {
            // A new video plays from the start with sound.
            let _ = media;
        }
        self.video = Some(Video {
            id: current.id.clone(),
            url,
            local: Some(local),
            state: VideoState::Probing(receiver),
            retried,
        });
    }

    /// The whole screen: top bar, media, arrows and video controls.
    fn contents(
        &mut self,
        ui: &mut egui::Ui,
        screen: egui::Rect,
        current: &Attachment,
        media: &mut Media,
        unix_now: i64,
    ) -> bool {
        let (rect, backdrop) = ui.allocate_exact_size(screen.size(), egui::Sense::click());
        let mut close = false;
        let top = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), TOP_BAR));
        let stage = stage_rect(rect, current.kind == AttachmentKind::Video);

        // Top bar: name and position on the left, actions on the right.
        let painter = ui.painter_at(rect);
        let mut title = current.name.clone();
        if self.items.len() > 1 {
            title = format!("{title}  ·  {} / {}", self.index + 1, self.items.len());
        }
        let notice = self
            .notice
            .as_ref()
            .filter(|(_, at)| at.elapsed() < NOTICE)
            .map(|(text, _)| text.clone());
        let title_rect = egui::Rect::from_min_max(
            top.min + egui::vec2(20.0, 0.0),
            egui::pos2(top.right() - 186.0, top.bottom()),
        );
        let mut job = egui::text::LayoutJob::single_section(
            title,
            egui::TextFormat::simple(
                egui::FontId::new(14.0, egui::FontFamily::Name("Satoshi Bold".into())),
                TEXT,
            ),
        );
        if let Some(size) = size_label(current) {
            job.append(
                &size,
                0.0,
                egui::TextFormat::simple(egui::FontId::proportional(13.0), MUTED),
            );
        }
        job.wrap.max_width = title_rect.width();
        let galley = ui.painter().layout_job(job);
        painter.galley(
            egui::pos2(title_rect.left(), top.center().y - galley.size().y / 2.0),
            galley,
            TEXT,
        );
        if let Some(notice) = notice {
            painter.text(
                egui::pos2(top.right() - 196.0, top.center().y),
                egui::Align2::RIGHT_CENTER,
                notice,
                egui::FontId::proportional(12.0),
                MUTED,
            );
            ui.ctx().request_repaint_after(Duration::from_millis(500));
        }
        let mut x = top.right() - 16.0;
        let mut action = |ui: &mut egui::Ui, icon: NavIcon, label: &str| {
            x -= 36.0;
            let button = egui::Rect::from_min_size(
                egui::pos2(x, top.center().y - 18.0),
                egui::vec2(36.0, 36.0),
            );
            icon_button(ui, button, icon, label, true)
        };
        if action(ui, NavIcon::Close, "Close viewer").clicked() {
            close = true;
        }
        let more = action(ui, NavIcon::More, "More media actions");
        if action(ui, NavIcon::ExternalLink, "Open in browser").clicked() {
            open_browser(ui.ctx(), current, media, unix_now);
        }
        let saving = self.saving.is_some();
        if action(
            ui,
            NavIcon::Download,
            if saving { "Saving…" } else { "Save" },
        )
        .clicked()
            && !saving
        {
            self.save(current, media, ui.ctx());
        }
        egui::Popup::menu(&more)
            .align(egui::RectAlign::BOTTOM_END)
            .show(|ui| {
                ui.set_width(200.0);
                if current.kind == AttachmentKind::Image
                    && crate::message_menu_item(
                        ui,
                        NavIcon::Copy,
                        "Copy image",
                        self.copying.is_none(),
                    )
                    .clicked()
                {
                    self.copy(current, media, ui.ctx(), true);
                    ui.close();
                }
                if crate::message_menu_item(
                    ui,
                    NavIcon::Link,
                    "Copy media link",
                    self.copying.is_none(),
                )
                .clicked()
                {
                    self.copy(current, media, ui.ctx(), false);
                    ui.close();
                }
            });
        if current.spoiler && self.reveal_scope.is_some() {
            // Keep the label out of the filename and toolbar, including narrow windows.
            let bounds = egui::Rect::from_min_max(
                top.left_bottom(),
                egui::pos2(top.right() - 12.0, top.bottom() + 38.0),
            );
            if crate::spoiler_hide_button(ui, bounds, ui.id().with("viewer-hide-spoiler")).clicked()
            {
                self.hide_spoiler(ui.ctx(), current);
                return true;
            }
        }

        // The media itself, carried by a finger or sliding in after a page turn.
        let mut over_media = false;
        let from = crate::motion::mark(ui);
        self.controls_mark = None;
        match current.kind {
            AttachmentKind::Video => over_media |= self.video_stage(ui, stage, current, media),
            _ => over_media |= self.image_stage(ui, stage, current, media),
        }
        let until = self.controls_mark.take().unwrap_or(usize::MAX);
        let (offset, opacity) = self.media_pose(ui.ctx(), ui.input(|input| input.time));
        crate::motion::carry(
            ui.ctx(),
            ui.layer_id(),
            from..until,
            opacity,
            egui::emath::TSTransform::from_translation(offset),
        );

        // Previous and next, disabled at the ends (no wrap, like web).
        if self.items.len() > 1 {
            let size = egui::vec2(44.0, 44.0);
            let left = egui::Rect::from_center_size(
                egui::pos2(rect.left() + SIDE / 2.0, stage.center().y),
                size,
            );
            let right = egui::Rect::from_center_size(
                egui::pos2(rect.right() - SIDE / 2.0, stage.center().y),
                size,
            );
            if icon_button(
                ui,
                left,
                NavIcon::ChevronLeft,
                "Previous file",
                self.index > 0,
            )
            .clicked()
            {
                self.go(self.index.saturating_sub(1));
            }
            if icon_button(
                ui,
                right,
                NavIcon::ChevronRight,
                "Next file",
                self.index + 1 < self.items.len(),
            )
            .clicked()
            {
                self.go(self.index + 1);
            }
        }
        // A click on the dark area around the media closes, as on web.
        if backdrop.clicked() && !over_media {
            close = true;
        }
        close
    }

    /// Follows one finger on the stage while not zoomed: past the slop it
    /// owns the gesture, so nothing under it (the picture, the dark area, a
    /// video's play toggle) treats its release as a click. Left and right
    /// turn the page, down closes; anything shorter eases back. Returns
    /// whether to close.
    fn track_swipe(&mut self, context: &egui::Context, stage: egui::Rect, now: f64) -> bool {
        let (events, pinching) =
            context.input(|input| (input.events.clone(), input.multi_touch().is_some()));
        if pinching || self.zoom.zoomed() {
            self.cancel_swipe(now);
            return false;
        }
        let mut close = false;
        for event in events {
            let egui::Event::Touch {
                device_id,
                id,
                phase,
                pos,
                ..
            } = event
            else {
                continue;
            };
            if phase == egui::TouchPhase::Start {
                if self.swipe.is_some() {
                    // A second finger: a pinch, not a swipe.
                    self.cancel_swipe(now);
                } else if stage.contains(pos) {
                    self.swipe = Some(Swipe {
                        device: device_id,
                        id,
                        start: pos,
                        axis: None,
                        offset: egui::Vec2::ZERO,
                    });
                }
                continue;
            }
            let Some(swipe) = self
                .swipe
                .as_mut()
                .filter(|swipe| swipe.device == device_id && swipe.id == id)
            else {
                continue;
            };
            if phase == egui::TouchPhase::Cancel {
                self.cancel_swipe(now);
                continue;
            }
            swipe.follow(pos);
            if swipe.axis.is_some() {
                context.set_dragged_id(egui::Id::new("viewer-swipe"));
            }
            if phase != egui::TouchPhase::End {
                continue;
            }
            let action = swipe.action(pos);
            let swiped = swipe.axis.is_some();
            match action {
                Some(SwipeAction::Next) if self.index + 1 < self.items.len() => {
                    self.go(self.index + 1);
                }
                Some(SwipeAction::Previous) if self.index > 0 => self.go(self.index - 1),
                Some(SwipeAction::Close) => {
                    self.swipe = None;
                    close = true;
                }
                _ => self.cancel_swipe(now),
            }
            if swiped {
                context.stop_dragging();
            }
        }
        if self.swipe.is_some() || self.returning.is_some() {
            context.request_repaint();
        }
        close
    }

    /// Lets go of a swipe without acting: the media eases back.
    fn cancel_swipe(&mut self, now: f64) {
        if let Some(swipe) = self.swipe.take()
            && swipe.offset != egui::Vec2::ZERO
        {
            self.returning = Some((swipe.offset, now));
        }
    }

    /// Where the media is drawn, relative to its place, and its opacity.
    fn media_pose(&mut self, context: &egui::Context, now: f64) -> (egui::Vec2, f32) {
        if let Some(swipe) = &self.swipe {
            return (swipe.offset, 1.0);
        }
        let mut offset = egui::Vec2::ZERO;
        if let Some((from, started)) = self.returning {
            let progress = crate::motion::progress(context, started, crate::motion::BASE);
            if progress >= 1.0 {
                self.returning = None;
            } else {
                offset += from * (1.0 - progress);
            }
        }
        let mut opacity = 1.0;
        if let Some((side, started)) = &mut self.paged {
            let started = *started.get_or_insert(now);
            let progress = crate::motion::progress(context, started, crate::motion::BASE);
            if progress >= 1.0 {
                self.paged = None;
            } else {
                offset.x += *side * PAGE_OFFSET * (1.0 - progress);
                opacity = progress;
            }
        }
        (offset, opacity)
    }

    /// Returns whether the pointer is over the picture.
    fn image_stage(
        &mut self,
        ui: &mut egui::Ui,
        stage: egui::Rect,
        current: &Attachment,
        media: &mut Media,
    ) -> bool {
        let painter = ui.painter_at(stage);
        let (texture, natural, loading) = match self.full.get(&current.id) {
            Some(Full::Ready(picture)) => {
                if let Some(next) = picture.next_frame().filter(|_| !crate::motion::reduced()) {
                    ui.ctx().request_repaint_after(next);
                }
                let texture = if crate::motion::reduced() {
                    &picture.frames[0].0
                } else {
                    picture.texture()
                };
                (Some(texture.id()), picture.size, false)
            }
            other => {
                // The inline preview stands in until the original arrives.
                let failed = matches!(other, Some(Full::Failed { .. }));
                if failed && !current.id.starts_with("profile-picture:") {
                    media.refresh_now(&current.id);
                }
                let preview = match media.image(&current.id, current.preview_url.as_deref()) {
                    ImageState::Ready(texture) => Some(texture),
                    _ => None,
                };
                let natural = match (current.width, current.height) {
                    (Some(width), Some(height)) if width > 0 && height > 0 => {
                        egui::vec2(width as f32, height as f32)
                    }
                    _ => egui::vec2(stage.width(), stage.height()),
                };
                if failed && preview.is_none() {
                    painter.text(
                        stage.center(),
                        egui::Align2::CENTER_CENTER,
                        "Couldn’t load this image. Open it in your browser instead.",
                        egui::FontId::proportional(13.0),
                        MUTED,
                    );
                    return false;
                }
                (preview, natural, !failed)
            }
        };
        let base = fit(natural, stage.size());
        let image_rect = self.zoom.place(stage, base);
        let shown = image_rect.size();
        let response = ui.interact(
            image_rect.intersect(stage),
            ui.id().with(("viewer-image", &current.id)),
            egui::Sense::click_and_drag(),
        );
        if let Some(texture) = texture {
            egui::Image::new((texture, shown)).paint_at(ui, image_rect);
        }
        if loading {
            egui::Spinner::new().size(28.0).color(TEXT).paint_at(
                ui,
                egui::Rect::from_center_size(stage.center(), egui::vec2(28.0, 28.0)),
            );
        }
        self.zoom.interact(ui, &response, stage, base);
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Image,
                true,
                format!("Image {}", current.name),
            )
        });
        response.context_menu(|ui| self.picture_menu(ui, current, media));
        response.hovered() || response.dragged()
    }

    /// The video and its control bar. Returns whether the pointer is over them.
    fn video_stage(
        &mut self,
        ui: &mut egui::Ui,
        stage: egui::Rect,
        current: &Attachment,
        media: &mut Media,
    ) -> bool {
        let pixels = ui.ctx().pixels_per_point();
        let Some(video) = &mut self.video else {
            return false;
        };
        if let VideoState::Probing(receiver) = &video.state
            && let Ok(probe) = receiver.try_recv()
        {
            video.state = match (probe, crate::ffmpeg::binary(), &video.local) {
                (Some(probe), Some(binary), Some(local)) if probe.video.is_some() => {
                    let stream = probe.video.as_ref().expect("checked");
                    let fit_size = (stage.size() * pixels).min(MAX_VIDEO);
                    let mut player = Player::new(
                        binary,
                        local.clone(),
                        stream.width,
                        stream.height,
                        probe.duration_ms.or(current.duration_ms),
                        stream.fps,
                        fit_size,
                    );
                    // GIF-style videos loop silently, like a GIF.
                    if current.animated {
                        player.toggle_mute();
                    }
                    player.play();
                    VideoState::Ready(Box::new(player))
                }
                _ => {
                    if !video.retried {
                        media.refresh_now(&current.id);
                    }
                    VideoState::Failed(
                        "This video can’t be played here. Open it in your browser instead.".into(),
                    )
                }
            };
        }
        let painter = ui.painter_at(stage);
        let mut over = false;
        let natural = match (current.width, current.height) {
            (Some(width), Some(height)) if width > 0 && height > 0 => {
                egui::vec2(width as f32, height as f32)
            }
            _ => egui::vec2(16.0, 9.0),
        };
        match &mut video.state {
            VideoState::Probing(_) => {
                ui.ctx().request_repaint_after(Duration::from_millis(50));
                poster(ui, stage, natural, current, media, true);
            }
            VideoState::Failed(message) => {
                poster(ui, stage, natural, current, media, false);
                let band = egui::Rect::from_center_size(
                    stage.center(),
                    egui::vec2(stage.width().min(460.0), 44.0),
                );
                painter.rect_filled(band, 8.0, Color32::from_black_alpha(200));
                painter.text(
                    band.center(),
                    egui::Align2::CENTER_CENTER,
                    message.as_str(),
                    egui::FontId::proportional(13.0),
                    TEXT,
                );
            }
            VideoState::Ready(player) => {
                if let Some(next) = player.update(ui.ctx()) {
                    ui.ctx().request_repaint_after(next);
                }
                // Looping GIF-style videos never stop.
                if current.animated && player.state() == State::Ended {
                    player.seek(0, true);
                }
                let shown = fit(player.frame_size().max(egui::vec2(1.0, 1.0)), stage.size());
                let picture = egui::Rect::from_center_size(stage.center(), shown);
                match player.texture() {
                    Some(texture) => {
                        egui::Image::new((texture.id(), shown)).paint_at(ui, picture);
                    }
                    None => poster(ui, stage, natural, current, media, player.error().is_none()),
                }
                let response = ui.interact(
                    picture,
                    ui.id().with(("viewer-video", &current.id)),
                    egui::Sense::click(),
                );
                if response.clicked() {
                    player.toggle();
                }
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Play or pause")
                });
                over |= response.hovered();
                if matches!(player.state(), State::Paused | State::Ended)
                    && player.texture().is_some()
                {
                    let center = picture.center();
                    painter.circle_filled(center, 30.0, Color32::from_black_alpha(170));
                    paint_icon(
                        &painter,
                        egui::Rect::from_center_size(
                            center + egui::vec2(2.0, 0.0),
                            egui::vec2(26.0, 26.0),
                        ),
                        NavIcon::Play,
                        TEXT,
                    );
                }
                let bar = egui::Rect::from_min_max(
                    egui::pos2(stage.left(), stage.bottom() + 8.0),
                    egui::pos2(stage.right(), stage.bottom() + 8.0 + CONTROLS),
                );
                self.controls_mark = Some(crate::motion::mark(ui));
                // GIF-style videos have no timeline, like a GIF.
                if !current.animated {
                    over |= controls(ui, bar, player, &mut self.scrub);
                }
            }
        }
        over
    }

    fn copy(&mut self, current: &Attachment, media: &Media, context: &egui::Context, image: bool) {
        let Some(url) = current.url.clone() else {
            self.notice = Some((
                "Couldn’t copy: media is unavailable.".into(),
                Instant::now(),
            ));
            return;
        };
        let api = media.api().cloned();
        let content_type = current.content_type.clone();
        let context = context.clone();
        let (sender, receiver) = mpsc::channel();
        self.copying = Some(receiver);
        self.notice = Some(("Copying…".into(), Instant::now()));
        std::thread::spawn(move || {
            let result = (|| {
                let pixels = if image {
                    let bytes = api
                        .ok_or("Couldn’t load the original image.")?
                        .fetch_media(&url)
                        .map_err(|_| "Couldn’t load the original image.")?;
                    Some(
                        decode_original(&bytes, &content_type)
                            .ok_or("Couldn’t decode the original image.")?,
                    )
                } else {
                    None
                };
                let mut owner = CLIPBOARD
                    .lock()
                    .map_err(|_| "Couldn’t access the clipboard.")?;
                if owner.is_none() {
                    *owner = Some(
                        arboard::Clipboard::new().map_err(|_| "Couldn’t access the clipboard.")?,
                    );
                }
                let clipboard = owner.as_mut().expect("initialized");
                if let Some(pixels) = pixels {
                    clipboard
                        .set_image(arboard::ImageData {
                            width: pixels.width() as usize,
                            height: pixels.height() as usize,
                            bytes: std::borrow::Cow::Owned(pixels.into_raw()),
                        })
                        .map_err(|_| "Couldn’t copy the image.")?;
                    Ok("Image copied.".to_owned())
                } else {
                    clipboard
                        .set_text(url)
                        .map_err(|_| "Couldn’t copy the media link.")?;
                    Ok("Media link copied.".to_owned())
                }
            })()
            .map_err(str::to_owned);
            let _ = sender.send(result);
            context.request_repaint();
        });
    }

    fn save(&mut self, current: &Attachment, media: &Media, context: &egui::Context) {
        let (Some(api), Some(url)) = (media.api().cloned(), current.url.clone()) else {
            return;
        };
        let context = context.clone();
        let (sender, receiver) = mpsc::channel();
        let name = current.name.clone();
        // The native dialog blocks its own thread, never the UI.
        std::thread::spawn(move || {
            let Some(path) = rfd::FileDialog::new().set_file_name(&name).save_file() else {
                let _ = sender.send(Err(String::new()));
                return;
            };
            let result = api
                .download_media(&url, &path)
                .map(|()| {
                    format!(
                        "Saved {}",
                        path.file_name()
                            .map_or(name.clone(), |file| file.to_string_lossy().into_owned())
                    )
                })
                // A refused, broken or stalled download (no bytes for
                // `MEDIA_STALL`) ends here, never leaving Save stuck.
                .map_err(|_| "Couldn’t save the file. Please try again.".to_owned());
            let _ = sender.send(result);
            context.request_repaint();
        });
        self.saving = Some(receiver);
    }
}

/// Open picture URLs directly; attachment URLs retain their refresh contract.
fn open_browser(context: &egui::Context, current: &Attachment, media: &mut Media, now: i64) {
    if current.id.starts_with("profile-picture:") {
        // Picture URLs are supplied by account/profile refreshes, not the
        // message-attachment URL endpoint.
        if let Some(url) = &current.url {
            context.open_url(egui::OpenUrl::new_tab(url));
        }
    } else {
        media.open(context, current, now);
    }
}

/// Where media sits: below the top bar and above video controls.
fn stage_rect(screen: egui::Rect, video: bool) -> egui::Rect {
    let bottom = if video { CONTROLS } else { 16.0 };
    egui::Rect::from_min_max(
        egui::pos2(screen.left() + SIDE, screen.top() + TOP_BAR + 8.0),
        egui::pos2(screen.right() - SIDE, screen.bottom() - bottom - 8.0),
    )
}

/// The poster (inline preview) and a spinner while a video starts.
fn poster(
    ui: &mut egui::Ui,
    stage: egui::Rect,
    natural: egui::Vec2,
    current: &Attachment,
    media: &mut Media,
    spinner: bool,
) {
    let shown = fit(natural, stage.size());
    let rect = egui::Rect::from_center_size(stage.center(), shown);
    if let ImageState::Ready(texture) = media.image(&current.id, current.preview_url.as_deref()) {
        egui::Image::new((texture, shown)).paint_at(ui, rect);
    } else {
        ui.painter().rect_filled(rect, 8.0, Color32::from_gray(20));
    }
    if spinner {
        egui::Spinner::new().size(28.0).color(TEXT).paint_at(
            ui,
            egui::Rect::from_center_size(stage.center(), egui::vec2(28.0, 28.0)),
        );
    }
}

/// Play/pause, time, seek bar, mute and volume. Returns whether hovered.
fn controls(
    ui: &mut egui::Ui,
    bar: egui::Rect,
    player: &mut Player,
    scrub: &mut Option<f32>,
) -> bool {
    let painter = ui.painter_at(bar);
    let middle = bar.center().y;
    let playing = player.state() == State::Playing;
    let play = egui::Rect::from_center_size(
        egui::pos2(bar.left() + 18.0, middle),
        egui::vec2(36.0, 36.0),
    );
    let mut hovered = false;
    let response = icon_button(
        ui,
        play,
        if playing {
            NavIcon::Pause
        } else {
            NavIcon::Play
        },
        if playing { "Pause" } else { "Play" },
        true,
    );
    hovered |= response.hovered();
    if response.clicked() {
        player.toggle();
    }
    let duration = player.duration_ms().unwrap_or(0);
    let position = scrub.map_or(player.position_ms(), |fraction| {
        (fraction * duration as f32) as u64
    });
    let time = format!(
        "{} / {}",
        player::time_label(position),
        player::time_label(duration)
    );
    let time_galley = ui
        .painter()
        .layout_no_wrap(time, egui::FontId::monospace(12.0), MUTED);
    let compact = bar.width() < 420.0;
    let time_width = if compact { 0.0 } else { time_galley.size().x };
    if !compact {
        painter.galley(
            egui::pos2(play.right() + 10.0, middle - time_galley.size().y / 2.0),
            time_galley,
            MUTED,
        );
    }

    let volume_slider = egui::Rect::from_center_size(
        egui::pos2(bar.right() - 50.0, middle),
        egui::vec2(80.0, 20.0),
    );
    let mute = egui::Rect::from_center_size(
        egui::pos2(
            if compact {
                bar.right() - 18.0
            } else {
                volume_slider.left() - 22.0
            },
            middle,
        ),
        egui::vec2(36.0, 36.0),
    );
    let response = icon_button(
        ui,
        mute,
        if player.muted() || player.volume() == 0.0 {
            NavIcon::VolumeX
        } else {
            NavIcon::Volume
        },
        if player.muted() { "Unmute" } else { "Mute" },
        true,
    );
    hovered |= response.hovered();
    if response.clicked() {
        player.toggle_mute();
    }
    // Narrow message tiles retain mute and seeking without overlapping controls.
    if !compact {
        let volume = ui.interact(
            volume_slider,
            ui.id().with("viewer-volume"),
            egui::Sense::click_and_drag(),
        );
        if let Some(pointer) = volume.interact_pointer_pos()
            && (volume.dragged() || volume.clicked())
        {
            player.set_volume(
                ((pointer.x - volume_slider.left()) / volume_slider.width()).clamp(0.0, 1.0),
            );
            if player.muted() {
                player.toggle_mute();
            }
        }
        let level = if player.muted() { 0.0 } else { player.volume() };
        track(&painter, volume_slider, level);
        volume.widget_info(|| egui::WidgetInfo::slider(true, f64::from(level) * 100.0, "Volume"));
        hovered |= volume.hovered();
    }

    // Seek bar between the time and the volume.
    let seek_rect = egui::Rect::from_min_max(
        egui::pos2(play.right() + 20.0 + time_width, middle - 10.0),
        egui::pos2(mute.left() - 12.0, middle + 10.0),
    );
    if seek_rect.width() > 40.0 && duration > 0 {
        let seek = ui.interact(
            seek_rect,
            ui.id().with("viewer-seek"),
            egui::Sense::click_and_drag(),
        );
        let fraction_at = |pointer: egui::Pos2| {
            ((pointer.x - seek_rect.left()) / seek_rect.width()).clamp(0.0, 1.0)
        };
        if seek.dragged()
            && let Some(pointer) = seek.interact_pointer_pos()
        {
            *scrub = Some(fraction_at(pointer));
        }
        if (seek.drag_stopped() || seek.clicked())
            && let Some(pointer) = seek.interact_pointer_pos()
        {
            *scrub = None;
            player.seek((fraction_at(pointer) * duration as f32) as u64, false);
        }
        let fraction = scrub.unwrap_or(position as f32 / duration as f32);
        track(&painter, seek_rect, fraction);
        seek.widget_info(|| egui::WidgetInfo::slider(true, f64::from(fraction) * 100.0, "Seek"));
        hovered |= seek.hovered() || seek.dragged();
    }
    hovered
}

fn track(painter: &egui::Painter, rect: egui::Rect, fraction: f32) {
    let line = egui::Rect::from_center_size(rect.center(), egui::vec2(rect.width(), 4.0));
    painter.rect_filled(line, 2.0, Color32::from_white_alpha(50));
    let filled = egui::Rect::from_min_size(
        line.min,
        egui::vec2(line.width() * fraction.clamp(0.0, 1.0), 4.0),
    );
    painter.rect_filled(filled, 2.0, TEXT);
    painter.circle_filled(egui::pos2(filled.right(), line.center().y), 6.0, TEXT);
}

/// A round icon button on the dark backdrop.
fn icon_button(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icon: NavIcon,
    label: &str,
    enabled: bool,
) -> egui::Response {
    let response = ui.interact(
        rect,
        ui.id().with(("viewer-button", label)),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let painter = ui.painter_at(rect.expand(2.0));
    let hot = enabled && (response.hovered() || response.has_focus());
    painter.circle_filled(
        rect.center(),
        rect.width() / 2.0,
        if hot {
            Color32::from_white_alpha(40)
        } else {
            Color32::from_black_alpha(120)
        },
    );
    if response.has_focus() {
        painter.circle_stroke(rect.center(), rect.width() / 2.0, Stroke::new(1.5, BORDER));
    }
    paint_icon(
        &painter,
        egui::Rect::from_center_size(rect.center(), egui::vec2(20.0, 20.0)),
        icon,
        if enabled {
            TEXT
        } else {
            Color32::from_white_alpha(60)
        },
    );
    let label = label.to_owned();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, label.clone())
    });
    if enabled {
        response.on_hover_text(hover_text(icon))
    } else {
        response
    }
}

/// Hover text with the keyboard shortcut, where there is one.
fn hover_text(icon: NavIcon) -> &'static str {
    match icon {
        NavIcon::Close => "Close (Esc)",
        NavIcon::ChevronLeft => "Previous (←)",
        NavIcon::ChevronRight => "Next (→)",
        NavIcon::Download => "Save",
        NavIcon::ExternalLink => "Open in browser",
        NavIcon::More => "More media actions",
        NavIcon::Pause => "Pause (Space)",
        NavIcon::Play => "Play (Space)",
        NavIcon::VolumeX => "Unmute",
        NavIcon::Volume => "Mute",
        _ => "",
    }
}

/// "  ·  1.2 MB" after the title: the stored size, which is what saving downloads.
fn size_label(attachment: &Attachment) -> Option<String> {
    (attachment.size > 0).then(|| format!("  ·  {}", crate::uploads::format_bytes(attachment.size)))
}

/// `natural` scaled to fit `bounds`, never enlarged.
pub fn fit(natural: egui::Vec2, bounds: egui::Vec2) -> egui::Vec2 {
    let natural = natural.max(egui::vec2(1.0, 1.0));
    let scale = (bounds.x / natural.x)
        .min(bounds.y / natural.y)
        .clamp(0.0, 1.0);
    natural * scale
}

/// Keep a zoomed image covering the stage: it can move only as far as it overhangs.
pub fn clamp_pan(pan: egui::Vec2, shown: egui::Vec2, stage: egui::Vec2) -> egui::Vec2 {
    let room = ((shown - stage) / 2.0).max(egui::Vec2::ZERO);
    egui::vec2(pan.x.clamp(-room.x, room.x), pan.y.clamp(-room.y, room.y))
}

/// How far a picture is zoomed past its fitted size, and moved from the
/// stage's centre: the viewer's, and the draft preview's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zoom {
    pub scale: f32,
    pub pan: egui::Vec2,
}

impl Default for Zoom {
    fn default() -> Self {
        Self {
            scale: 1.0,
            pan: egui::Vec2::ZERO,
        }
    }
}

impl Zoom {
    pub fn zoomed(&self) -> bool {
        self.scale > 1.0
    }

    /// Where a picture fitted at `base` is drawn on `stage`.
    pub fn place(&mut self, stage: egui::Rect, base: egui::Vec2) -> egui::Rect {
        let shown = base * self.scale;
        self.pan = clamp_pan(self.pan, shown, stage.size());
        egui::Rect::from_center_size(stage.center() + self.pan, shown)
    }

    /// Double-click toggles, pinch / Ctrl+scroll scales, both about the
    /// pointer; dragging pans while zoomed. `response` senses clicks and drags.
    pub fn interact(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        stage: egui::Rect,
        base: egui::Vec2,
    ) {
        let pointer = response.hover_pos();
        if response.double_clicked() {
            let target = if self.zoomed() {
                1.0
            } else {
                DOUBLE_CLICK_ZOOM
            };
            self.zoom_to(target, pointer, stage, base);
        } else if response.hovered() {
            let delta = ui.ctx().input(|input| input.zoom_delta());
            if (delta - 1.0).abs() > f32::EPSILON {
                self.zoom_to(self.scale * delta, pointer, stage, base);
            }
        }
        if response.dragged() && self.zoomed() {
            self.pan += response.drag_delta();
        }
    }

    fn zoom_to(
        &mut self,
        scale: f32,
        pointer: Option<egui::Pos2>,
        stage: egui::Rect,
        base: egui::Vec2,
    ) {
        let scale = scale.clamp(1.0, MAX_ZOOM);
        // Keep the point under the pointer still.
        if let Some(pointer) = pointer {
            let from_center = pointer - stage.center() - self.pan;
            self.pan -= from_center * (scale / self.scale - 1.0);
        }
        self.scale = scale;
        self.pan = clamp_pan(self.pan, base * scale, stage.size());
    }
}

/// Clipboard pixels are never the viewer's scaled texture or inline preview.
/// ImageReader decodes the first frame of GIF/WebP without loading the animation.
/// Reject excessive originals rather than silently shrinking them.
pub(crate) fn decode_original(bytes: &[u8], content_type: &str) -> Option<image::RgbaImage> {
    use image::ImageDecoder;
    if matches!(content_type, "image/avif" | "image/heic" | "image/heif") {
        return decode_ffmpeg_pixels(bytes, content_type);
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(MAX_ANIMATION_BYTES as u64);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder.orientation().ok()?;
    let mut image = image::DynamicImage::from_decoder(decoder).ok()?;
    image.apply_orientation(orientation);
    Some(image.to_rgba8())
}

/// Full-size frames: animated GIF/WebP with their delays, other stills once,
/// all at most `edge` on their long side.
/// Formats the `image` codecs cannot read (AVIF, HEIC) go through FFmpeg.
fn decode_full(
    bytes: &[u8],
    content_type: &str,
    edge: u32,
) -> Option<Vec<(egui::ColorImage, Duration)>> {
    use image::{AnimationDecoder, ImageDecoder};
    let content_type = match image::guess_format(bytes).ok() {
        Some(image::ImageFormat::Gif) => "image/gif",
        Some(image::ImageFormat::WebP) => "image/webp",
        _ => content_type,
    };
    let cursor = || std::io::Cursor::new(bytes);
    let frames = match content_type {
        "image/gif" => {
            image::codecs::gif::GifDecoder::new(cursor())
                .ok()
                .and_then(|mut decoder| {
                    decoder.set_limits(limits()).ok()?;
                    animation(decoder.into_frames(), edge, MAX_ANIMATION_BYTES)
                })
        }
        "image/webp" => image::codecs::webp::WebPDecoder::new(cursor())
            .ok()
            .filter(image::codecs::webp::WebPDecoder::has_animation)
            .and_then(|mut decoder| {
                decoder.set_limits(limits()).ok()?;
                animation(decoder.into_frames(), edge, MAX_ANIMATION_BYTES)
            }),
        _ => None,
    };
    if frames.is_some() {
        return frames;
    }
    if crate::attachments::decodable(content_type) {
        let mut reader = image::ImageReader::new(cursor())
            .with_guessed_format()
            .ok()?;
        reader.limits(limits());
        let mut image = reader.decode().ok()?;
        if image.width() > edge || image.height() > edge {
            image = image.thumbnail(edge, edge);
        }
        return Some(vec![(color_image(image.to_rgba8()), Duration::ZERO)]);
    }
    crate::ffmpeg::decode_image(bytes, content_type, edge)
        .map(|image| vec![(image, Duration::ZERO)])
}

fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(MAX_ANIMATION_BYTES as u64);
    limits
}

/// Frames shrunk to `edge`. Past `budget` bytes the first frame stands in as
/// a still, as inline GIFs do, rather than looping a cut-short animation.
fn animation(
    frames: image::Frames<'_>,
    edge: u32,
    budget: usize,
) -> Option<Vec<(egui::ColorImage, Duration)>> {
    let mut decoded = Vec::new();
    let mut bytes = 0;
    for frame in frames.take(MAX_ANIMATION_FRAMES) {
        let frame = frame.ok()?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        // Browsers treat delays under 20 ms as 100 ms.
        let delay = Duration::from_millis(u64::from(numerator / denominator.max(1)));
        let delay = if delay < Duration::from_millis(20) {
            Duration::from_millis(100)
        } else {
            delay
        };
        let buffer = fit_edge(frame.into_buffer(), edge);
        bytes += buffer.len();
        if bytes > budget {
            decoded.truncate(1);
            break;
        }
        decoded.push((color_image(buffer), delay));
    }
    (!decoded.is_empty()).then_some(decoded)
}

/// Scales down, keeping the aspect ratio, so neither side exceeds `edge`.
fn fit_edge(image: image::RgbaImage, edge: u32) -> image::RgbaImage {
    if image.width() <= edge && image.height() <= edge {
        return image;
    }
    image::DynamicImage::ImageRgba8(image)
        .thumbnail(edge, edge)
        .into_rgba8()
}

fn color_image(image: image::RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )
}

/// Clipboard originals need full-resolution, straight-alpha pixels rather than
/// the bounded, premultiplied display image returned by ffmpeg::decode_image.
fn decode_ffmpeg_pixels(bytes: &[u8], content_type: &str) -> Option<image::RgbaImage> {
    let binary = crate::ffmpeg::binary()?;
    let extension = match content_type {
        "image/avif" => "avif",
        "image/heic" | "image/heif" => "heic",
        _ => return None,
    };
    let scratch = crate::uploads::Scratch::new(extension);
    std::fs::write(scratch.path(), bytes).ok()?;
    let probe = crate::ffmpeg::probe(binary, scratch.path()).ok()?;
    let stream = probe.video?;
    let [width, height] = [stream.width as usize, stream.height as usize];
    if width == 0
        || height == 0
        || width > 16_384
        || height > 16_384
        || width.checked_mul(height)?.checked_mul(4)? > MAX_ANIMATION_BYTES
    {
        return None;
    }
    let output = crate::ffmpeg::command(binary)
        .args(["-loglevel", "error", "-i"])
        .arg(scratch.path())
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale={width}:{height}:flags=lanczos,format=rgba"),
        ])
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() != width * height * 4 {
        return None;
    }
    image::RgbaImage::from_raw(width as u32, height as u32, output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(id: &str, kind: AttachmentKind) -> Attachment {
        Attachment {
            spoiler: false,
            id: id.into(),
            kind,
            content_type: "image/png".into(),
            name: format!("{id}.png"),
            size: 10,
            width: Some(800),
            height: Some(600),
            duration_ms: None,
            preview: false,
            url: Some(format!("https://cdn.test/original/{id}?exp=1&sig=s")),
            preview_url: None,
            unavailable: false,
            status: AttachmentStatus::Ready,
            animated: false,
        }
    }

    #[test]
    fn profile_pictures_open_only_with_an_original_url() {
        let source = crate::pictures::Source {
            asset: "avatars/person/asset",
            url: Some("https://cdn.test/picture?exp=123&sig=s"),
        };
        let item = Viewer::profile_picture(source, "Person").unwrap();
        assert!(viewable(&item));
        assert_eq!(item.url.as_deref(), source.url);
        assert!(item.preview_url.is_none());
        let viewer = Viewer::open(vec![item], 0);
        assert_eq!((viewer.index(), viewer.len()), (0, 1));
        assert!(
            Viewer::profile_picture(
                crate::pictures::Source {
                    url: None,
                    ..source
                },
                "Person"
            )
            .is_none()
        );
    }

    #[test]
    fn only_ready_images_and_videos_with_urls_open() {
        assert!(viewable(&attachment("a", AttachmentKind::Image)));
        assert!(viewable(&attachment("v", AttachmentKind::Video)));
        assert!(!viewable(&attachment("f", AttachmentKind::File)));
        assert!(!viewable(&attachment("s", AttachmentKind::Audio)));
        let mut removed = attachment("r", AttachmentKind::Image);
        removed.unavailable = true;
        assert!(!viewable(&removed));
        let mut processing = attachment("p", AttachmentKind::Video);
        processing.status = AttachmentStatus::Processing;
        assert!(!viewable(&processing));
        let mut unsigned = attachment("u", AttachmentKind::Image);
        unsigned.url = None;
        assert!(!viewable(&unsigned));
    }

    #[test]
    fn the_title_shows_the_stored_size() {
        let mut item = attachment("a", AttachmentKind::Image);
        item.size = 1_258_291;
        assert_eq!(size_label(&item).as_deref(), Some("  ·  1.2 MB"));
        item.size = 0;
        assert_eq!(size_label(&item), None);
    }

    #[test]
    fn arrows_stop_at_the_ends() {
        let items = vec![
            attachment("a", AttachmentKind::Image),
            attachment("b", AttachmentKind::Video),
            attachment("c", AttachmentKind::Image),
        ];
        let mut viewer = Viewer::open(items, 9);
        assert_eq!((viewer.index(), viewer.len()), (2, 3));
        viewer.go(3);
        assert_eq!(viewer.index(), 2);
        viewer.zoom.scale = 3.0;
        viewer.go(0);
        assert_eq!(viewer.index(), 0);
        assert_eq!(viewer.zoom, Zoom::default(), "moving resets zoom");
    }

    #[test]
    fn swipes_page_close_or_ease_back_as_on_web() {
        assert_eq!(swipe_action(-60.0, 0.0), Some(SwipeAction::Next));
        assert_eq!(swipe_action(60.0, 0.0), Some(SwipeAction::Previous));
        assert_eq!(swipe_action(59.0, 0.0), None);
        assert_eq!(swipe_action(0.0, 100.0), Some(SwipeAction::Close));
        assert_eq!(swipe_action(0.0, -200.0), None, "up never closes");
        assert_eq!(swipe_action(30.0, 99.0), None);

        let context = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let stage = stage_rect(screen, false);
        let items = vec![
            attachment("a", AttachmentKind::Image),
            attachment("b", AttachmentKind::Image),
        ];
        let mut viewer = Viewer::open(items, 0);
        let mut time = 0.0;
        let mut frame = |viewer: &mut Viewer, touches: &[(egui::TouchPhase, u64, egui::Pos2)]| {
            time += 0.016;
            let events = touches
                .iter()
                .map(|&(phase, id, pos)| egui::Event::Touch {
                    device_id: egui::TouchDeviceId(0),
                    id: egui::TouchId(id),
                    phase,
                    pos,
                    force: None,
                })
                .collect();
            let mut close = false;
            let _ = context.run(
                egui::RawInput {
                    screen_rect: Some(screen),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |context| close = viewer.track_swipe(context, stage, time),
            );
            close
        };
        use egui::TouchPhase::{End, Move, Start};
        let center = stage.center();
        // The media follows a finger on one axis, then turns the page.
        frame(&mut viewer, &[(Start, 1, center)]);
        frame(&mut viewer, &[(Move, 1, center + egui::vec2(-40.0, 12.0))]);
        assert_eq!(
            viewer.swipe.as_ref().map(|swipe| swipe.offset),
            Some(egui::vec2(-40.0, 0.0))
        );
        assert!(!frame(
            &mut viewer,
            &[(End, 1, center + egui::vec2(-90.0, 20.0))]
        ));
        assert_eq!(viewer.index(), 1);
        assert!(viewer.paged.is_some(), "the next file slides in");
        // Past the last file it eases back instead.
        frame(&mut viewer, &[(Start, 1, center)]);
        frame(&mut viewer, &[(End, 1, center + egui::vec2(-90.0, 0.0))]);
        assert_eq!(viewer.index(), 1);
        assert_eq!(
            viewer.returning.map(|(offset, _)| offset),
            Some(egui::vec2(-90.0, 0.0))
        );
        // Short swipes ease back; a tap neither moves nor eases.
        frame(
            &mut viewer,
            &[(Start, 1, center), (End, 1, center + egui::vec2(30.0, 0.0))],
        );
        assert_eq!(viewer.index(), 1);
        viewer.returning = None;
        frame(&mut viewer, &[(Start, 1, center), (End, 1, center)]);
        assert!(viewer.returning.is_none() && viewer.swipe.is_none());
        // Down closes; a finger starting outside the stage (the top bar's
        // buttons) never swipes.
        frame(&mut viewer, &[(Start, 1, center)]);
        assert!(frame(
            &mut viewer,
            &[(End, 1, center + egui::vec2(10.0, 120.0))]
        ));
        let button = egui::pos2(780.0, 20.0);
        frame(&mut viewer, &[(Start, 1, button)]);
        assert!(!frame(
            &mut viewer,
            &[(End, 1, button + egui::vec2(0.0, 200.0))]
        ));
        // A second finger makes a pinch, not a swipe; zoomed media pans.
        frame(&mut viewer, &[(Start, 1, center)]);
        frame(&mut viewer, &[(Start, 2, center + egui::vec2(50.0, 0.0))]);
        frame(&mut viewer, &[(End, 1, center + egui::vec2(120.0, 0.0))]);
        assert_eq!(viewer.index(), 1);
        viewer.zoom.scale = 2.0;
        frame(&mut viewer, &[(Start, 1, center)]);
        frame(&mut viewer, &[(End, 1, center + egui::vec2(120.0, 0.0))]);
        assert_eq!(viewer.index(), 1);
    }

    #[test]
    fn media_fits_without_enlarging_and_zoomed_pans_stay_covered() {
        assert_eq!(
            fit(egui::vec2(4000.0, 3000.0), egui::vec2(800.0, 800.0)),
            egui::vec2(800.0, 600.0)
        );
        assert_eq!(
            fit(egui::vec2(400.0, 300.0), egui::vec2(800.0, 800.0)),
            egui::vec2(400.0, 300.0)
        );
        let stage = egui::vec2(800.0, 600.0);
        assert_eq!(
            clamp_pan(egui::vec2(500.0, -500.0), egui::vec2(1600.0, 1200.0), stage),
            egui::vec2(400.0, -300.0)
        );
        assert_eq!(
            clamp_pan(egui::vec2(50.0, 50.0), egui::vec2(400.0, 300.0), stage),
            egui::Vec2::ZERO
        );
    }

    #[test]
    fn zoom_keeps_the_spot_under_the_pointer_between_fit_and_the_most() {
        let stage = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let base = egui::vec2(800.0, 600.0);
        let pointer = egui::pos2(600.0, 150.0);
        let mut zoom = Zoom::default();
        // Where the pointer is on the picture, in fractions of it.
        let spot = |zoom: &mut Zoom| {
            let rect = zoom.place(stage, base);
            (pointer - rect.min) / rect.size()
        };
        let before = spot(&mut zoom);
        zoom.zoom_to(2.0, Some(pointer), stage, base);
        assert_eq!(zoom.scale, 2.0);
        assert!((spot(&mut zoom) - before).length() < 1e-5);
        zoom.zoom_to(100.0, Some(pointer), stage, base);
        assert_eq!(zoom.scale, MAX_ZOOM);
        // Back at fit it is centred again.
        zoom.zoom_to(0.5, Some(pointer), stage, base);
        assert_eq!(zoom, Zoom::default());
    }

    #[test]
    fn animated_gifs_keep_their_frames_and_delays() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for shade in [0_u8, 255] {
                let frame = image::Frame::from_parts(
                    image::RgbaImage::from_pixel(4, 2, image::Rgba([shade, 0, 0, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(150, 1),
                );
                encoder.encode_frame(frame).unwrap();
            }
        }
        let frames = decode_full(&bytes, "image/gif", MAX_EDGE).unwrap();
        assert_eq!(frames.len(), 2);
        // Profile metadata has no MIME type: sniff the original, not its URL.
        assert_eq!(
            decode_full(&bytes, "image/webp", MAX_EDGE).unwrap().len(),
            2
        );
        assert_eq!(frames[0].0.size, [4, 2]);
        assert_eq!(frames[1].1, Duration::from_millis(150));
        let copied = decode_original(&bytes, "image/gif").unwrap();
        assert_eq!(copied.dimensions(), (4, 2));
        assert_eq!(copied.get_pixel(0, 0).0, [0, 0, 0, 255]);
        // Past the memory budget, the first frame stands in as a still.
        use image::AnimationDecoder;
        let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(&bytes)).unwrap();
        let still = animation(decoder.into_frames(), MAX_EDGE, 4 * 2 * 4 + 1).unwrap();
        assert_eq!(still.len(), 1);
        // Stills decode once; formats needing FFmpeg fail cleanly without it.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(3, 3, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(
            decode_full(&png, "image/png", MAX_EDGE).unwrap()[0].0.size,
            [3, 3]
        );
        // Copy preserves original dimensions and straight alpha, even when the
        // viewer would shrink the image to MAX_EDGE.
        png.clear();
        image::RgbaImage::from_pixel(MAX_EDGE + 1, 1, image::Rgba([123, 45, 67, 128]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let copied = decode_original(&png, "image/png").unwrap();
        assert_eq!(copied.dimensions(), (MAX_EDGE + 1, 1));
        assert_eq!(copied.get_pixel(0, 0).0, [123, 45, 67, 128]);
        assert!(decode_original(b"broken", "image/png").is_none());
        assert!(decode_original(b"broken", "image/webp").is_none());
        if crate::ffmpeg::binary().is_none() {
            assert!(decode_full(b"not an avif", "image/avif", MAX_EDGE).is_none());
            assert!(decode_original(b"not an avif", "image/avif").is_none());
        }
    }

    #[test]
    fn wide_animations_shrink_to_fit_a_texture_like_stills() {
        // A tiny file can still describe a frame wider than the GPU allows.
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for _ in 0..2 {
                let frame = image::Frame::from_parts(
                    image::RgbaImage::from_pixel(8193, 16, image::Rgba([9, 9, 9, 255])),
                    0,
                    0,
                    image::Delay::from_numer_denom_ms(100, 1),
                );
                encoder.encode_frame(frame).unwrap();
            }
        }
        let frames = decode_full(&bytes, "image/gif", MAX_EDGE).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0.size, [4096, 8]);
        // A smaller GPU limit wins over MAX_EDGE, for stills too.
        assert_eq!(
            decode_full(&bytes, "image/gif", 2048).unwrap()[1].0.size,
            [2048, 4]
        );
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(3000, 30, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(
            decode_full(&png, "image/png", 2048).unwrap()[0].0.size,
            [2048, 20]
        );
        let sliver = fit_edge(image::RgbaImage::new(9000, 1), MAX_EDGE);
        assert_eq!((sliver.width(), sliver.height()), (4096, 1));
    }
}
