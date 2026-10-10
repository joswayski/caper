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
    zoom: f32,
    pan: egui::Vec2,
    full: HashMap<String, Full>,
    sender: Sender<Loaded>,
    receiver: Receiver<Loaded>,
    video: Option<Video>,
    /// The seek bar while it is dragged, 0–1.
    scrub: Option<f32>,
    saving: Option<Receiver<Result<String, String>>>,
    notice: Option<(String, Instant)>,
    opened: bool,
}

impl Viewer {
    pub fn open(items: Vec<Attachment>, index: usize) -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            index: index.min(items.len().saturating_sub(1)),
            items,
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            full: HashMap::new(),
            sender,
            receiver,
            video: None,
            scrub: None,
            saving: None,
            notice: None,
            opened: false,
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
            self.index = index;
            self.zoom = 1.0;
            self.pan = egui::Vec2::ZERO;
            self.scrub = None;
        }
    }

    /// Stop playback and free the stream route (closing or leaving a video).
    fn stop_video(&mut self, proxy: Option<&MediaProxy>) {
        if let Some(video) = self.video.take()
            && let (Some(proxy), Some(local)) = (proxy, &video.local)
        {
            proxy.remove(local);
        }
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
        if let Some(saving) = &self.saving
            && let Ok(result) = saving.try_recv()
        {
            self.saving = None;
            self.notice = Some((result.unwrap_or_else(|error| error), Instant::now()));
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
        let screen = context.content_rect();
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
        if matches!(self.full.get(&current.id), Some(Full::Failed { .. })) || self.full.is_empty() {
            // Only this and its neighbours stay decoded.
            let keep: Vec<String> = self
                .items
                .iter()
                .skip(self.index.saturating_sub(1))
                .take(3)
                .map(|item| item.id.clone())
                .collect();
            self.full.retain(|id, _| keep.contains(id));
        }
        let Some(api) = media.api().cloned() else {
            return;
        };
        self.full
            .insert(current.id.clone(), Full::Loading { url: url.clone() });
        let sender = self.sender.clone();
        let context = context.clone();
        let (id, content_type) = (current.id.clone(), current.content_type.clone());
        std::thread::spawn(move || {
            let result = api
                .fetch_media(&url)
                .and_then(|bytes| decode_full(&bytes, &content_type).ok_or(None));
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
        let video = current.kind == AttachmentKind::Video;
        let top = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), TOP_BAR));
        let bottom = if video { CONTROLS } else { 16.0 };
        let stage = egui::Rect::from_min_max(
            egui::pos2(rect.left() + SIDE, top.bottom() + 8.0),
            egui::pos2(rect.right() - SIDE, rect.bottom() - bottom - 8.0),
        );

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
            egui::pos2(top.right() - 150.0, top.bottom()),
        );
        let galley = ui.painter().layout(
            title,
            egui::FontId::new(14.0, egui::FontFamily::Name("Satoshi Bold".into())),
            TEXT,
            title_rect.width(),
        );
        painter.galley(
            egui::pos2(title_rect.left(), top.center().y - galley.size().y / 2.0),
            galley,
            TEXT,
        );
        if let Some(notice) = notice {
            painter.text(
                egui::pos2(top.right() - 160.0, top.center().y),
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
        if action(ui, NavIcon::ExternalLink, "Open in browser").clicked() {
            media.open(ui.ctx(), current, unix_now);
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
            self.save(current, media);
        }

        // The media itself.
        let mut over_media = false;
        match current.kind {
            AttachmentKind::Video => over_media |= self.video_stage(ui, stage, current, media),
            _ => over_media |= self.image_stage(ui, stage, current, media),
        }

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
                if let Some(next) = picture.next_frame() {
                    ui.ctx().request_repaint_after(next);
                }
                (Some(picture.texture().id()), picture.size, false)
            }
            other => {
                // The inline preview stands in until the original arrives.
                let failed = matches!(other, Some(Full::Failed { .. }));
                if failed {
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
        let shown = base * self.zoom;
        self.pan = clamp_pan(self.pan, shown, stage.size());
        let image_rect = egui::Rect::from_center_size(stage.center() + self.pan, shown);
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
        // Zoom: double-click toggles, pinch / Ctrl+scroll scales, both about the pointer.
        let pointer = response.hover_pos();
        if response.double_clicked() {
            let target = if self.zoom > 1.0 {
                1.0
            } else {
                DOUBLE_CLICK_ZOOM
            };
            self.zoom_to(target, pointer, stage, base);
        } else if response.hovered() {
            let delta = ui.ctx().input(|input| input.zoom_delta());
            if (delta - 1.0).abs() > f32::EPSILON {
                self.zoom_to(self.zoom * delta, pointer, stage, base);
            }
        }
        if response.dragged() && self.zoom > 1.0 {
            self.pan += response.drag_delta();
        }
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::Image,
                true,
                format!("Image {}", current.name),
            )
        });
        response.hovered() || response.dragged()
    }

    fn zoom_to(
        &mut self,
        zoom: f32,
        pointer: Option<egui::Pos2>,
        stage: egui::Rect,
        base: egui::Vec2,
    ) {
        let zoom = zoom.clamp(1.0, MAX_ZOOM);
        // Keep the point under the pointer still.
        if let Some(pointer) = pointer {
            let from_center = pointer - stage.center() - self.pan;
            self.pan -= from_center * (zoom / self.zoom - 1.0);
        }
        self.zoom = zoom;
        self.pan = clamp_pan(self.pan, base * zoom, stage.size());
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
                // GIF-style videos have no timeline, like a GIF.
                if !current.animated {
                    over |= controls(ui, bar, player, &mut self.scrub);
                }
            }
        }
        over
    }

    fn save(&mut self, current: &Attachment, media: &Media) {
        let (Some(api), Some(url)) = (media.api().cloned(), current.url.clone()) else {
            return;
        };
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
                .map_err(|_| "Couldn’t save the file.".to_owned());
            let _ = sender.send(result);
        });
        self.saving = Some(receiver);
    }
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
    let time_width = time_galley.size().x;
    painter.galley(
        egui::pos2(play.right() + 10.0, middle - time_galley.size().y / 2.0),
        time_galley,
        MUTED,
    );

    let volume_slider = egui::Rect::from_center_size(
        egui::pos2(bar.right() - 50.0, middle),
        egui::vec2(80.0, 20.0),
    );
    let mute = egui::Rect::from_center_size(
        egui::pos2(volume_slider.left() - 22.0, middle),
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
    // Volume: a small slider.
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
        NavIcon::Pause => "Pause (Space)",
        NavIcon::Play => "Play (Space)",
        NavIcon::VolumeX => "Unmute",
        NavIcon::Volume => "Mute",
        _ => "",
    }
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

/// Full-size frames: animated GIF/WebP with their delays, other stills once.
/// Formats the `image` codecs cannot read (AVIF, HEIC) go through FFmpeg.
fn decode_full(bytes: &[u8], content_type: &str) -> Option<Vec<(egui::ColorImage, Duration)>> {
    use image::AnimationDecoder;
    let cursor = || std::io::Cursor::new(bytes);
    let frames = match content_type {
        "image/gif" => image::codecs::gif::GifDecoder::new(cursor())
            .ok()
            .and_then(|decoder| animation(decoder.into_frames())),
        "image/webp" => image::codecs::webp::WebPDecoder::new(cursor())
            .ok()
            .filter(image::codecs::webp::WebPDecoder::has_animation)
            .and_then(|decoder| animation(decoder.into_frames())),
        _ => None,
    };
    if frames.is_some() {
        return frames;
    }
    if crate::attachments::decodable(content_type) {
        let mut reader = image::ImageReader::new(cursor())
            .with_guessed_format()
            .ok()?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16_384);
        limits.max_image_height = Some(16_384);
        limits.max_alloc = Some(MAX_ANIMATION_BYTES as u64);
        reader.limits(limits);
        let mut image = reader.decode().ok()?;
        if image.width() > MAX_EDGE || image.height() > MAX_EDGE {
            image = image.thumbnail(MAX_EDGE, MAX_EDGE);
        }
        return Some(vec![(color_image(image.to_rgba8()), Duration::ZERO)]);
    }
    decode_with_ffmpeg(bytes, content_type).map(|image| vec![(image, Duration::ZERO)])
}

fn animation<'a>(frames: image::Frames<'a>) -> Option<Vec<(egui::ColorImage, Duration)>> {
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
        let buffer = frame.into_buffer();
        bytes += buffer.len();
        if bytes > MAX_ANIMATION_BYTES {
            break;
        }
        decoded.push((color_image(buffer), delay));
    }
    (!decoded.is_empty()).then_some(decoded)
}

fn color_image(image: image::RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )
}

/// AVIF and HEIC: one frame through the bundled FFmpeg, from a scratch file
/// (their container needs seeking).
fn decode_with_ffmpeg(bytes: &[u8], content_type: &str) -> Option<egui::ColorImage> {
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
    let [width, height] = player::output_size(
        stream.width,
        stream.height,
        egui::vec2(MAX_EDGE as f32, MAX_EDGE as f32),
    );
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
    (output.status.success() && output.stdout.len() == width * height * 4)
        .then(|| egui::ColorImage::from_rgba_unmultiplied([width, height], &output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attachment(id: &str, kind: AttachmentKind) -> Attachment {
        Attachment {
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
        viewer.zoom = 3.0;
        viewer.go(0);
        assert_eq!(viewer.index(), 0);
        assert_eq!(viewer.zoom, 1.0, "moving resets zoom");
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
        let frames = decode_full(&bytes, "image/gif").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].0.size, [4, 2]);
        assert_eq!(frames[1].1, Duration::from_millis(150));
        // Stills decode once; formats needing FFmpeg fail cleanly without it.
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(3, 3, image::Rgba([1, 2, 3, 255]))
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(decode_full(&png, "image/png").unwrap()[0].0.size, [3, 3]);
        if crate::ffmpeg::binary().is_none() {
            assert!(decode_full(b"not an avif", "image/avif").is_none());
        }
    }
}
