//! Videos play in the app, as on the egui desktop: in the viewer, and in
//! place in a message once its tile is clicked. FFmpeg probes the video,
//! then the shared `caper_client::player` decodes it through a loopback
//! route (`crate::playback::Streams`) at most at 1080p, its frames shown
//! over the poster. Under it, Play or Pause, the time, a seek bar, Mute and
//! the volume; clicking the picture plays or pauses. GIF-style videos loop
//! silently with no controls. Playback stops when the stage leaves the
//! window.

use crate::images::{Icon, Images};
use crate::playback::Streams;
use caper_client::attachments::inline_source;
use caper_client::model::Attachment;
use caper_client::player::{self, Player, State};
use gtk::{gdk, gio, glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// Videos decode at most at 1080p however large the stage.
const MAX_VIDEO: [f32; 2] = [1920.0, 1080.0];
/// Below this the time and the volume make way, as on the desktop.
const COMPACT: i32 = 420;
/// The control bar's height.
const CONTROLS: i32 = 52;
/// How long a seek waits for the bar to settle while it is dragged.
const SEEK_SETTLE: Duration = Duration::from_millis(150);

struct Stage {
    attachment: Attachment,
    streams: Rc<Streams>,
    local: RefCell<Option<String>>,
    player: RefCell<Option<Player>>,
    frame: gtk::Picture,
    glyph: gtk::Widget,
    spinner: gtk::Spinner,
    notice: gtk::Label,
    controls: Option<Controls>,
    ticking: Cell<bool>,
    /// The seek bar's value while it is dragged, and the pending seek.
    scrub: Cell<Option<f64>>,
    seek: RefCell<Option<glib::SourceId>>,
}

struct Controls {
    /// Shown once the video plays, as on the desktop.
    bar: gtk::Box,
    play: gtk::Button,
    pause: gtk::Button,
    time: gtk::Label,
    seek: gtk::Scale,
    mute: gtk::Button,
    unmute: gtk::Button,
    volume: gtk::Scale,
}

/// A video stage on screen.
pub struct Video {
    pub root: gtk::Box,
    stage: Rc<Stage>,
}

impl Video {
    /// Plays or pauses (Space in the viewer).
    pub fn toggle(&self) {
        self.stage.toggle();
    }
}

/// The video `attachment` playing, `width` × `height` overall (the picture
/// and, unless it is GIF-style, its controls under it).
pub fn stage(images: &Images, attachment: &Attachment, width: i32, height: i32) -> Video {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    root.add_css_class("video-stage");
    let overlay = gtk::Overlay::new();
    overlay.set_vexpand(true);
    // The poster until the first frame, then the frames over it.
    let poster = picture();
    if let Some(url) = inline_source(attachment).filter(|_| !attachment.unavailable) {
        images.media.bind(&poster, &attachment.id, url);
    }
    overlay.set_child(Some(&poster));
    let frame = picture();
    overlay.add_overlay(&frame);
    let glyph = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    glyph.add_css_class("play");
    glyph.add_css_class("video-paused");
    glyph.append(&images.icon_image(Icon::Play, "#f3f4f5", 26));
    centered(&glyph);
    glyph.set_visible(false);
    overlay.add_overlay(&glyph);
    let spinner = gtk::Spinner::new();
    spinner.set_size_request(28, 28);
    centered(&spinner);
    spinner.start();
    overlay.add_overlay(&spinner);
    let notice = crate::ui::label("", &["video-notice"]);
    notice.set_wrap(true);
    notice.set_max_width_chars(48);
    notice.set_xalign(0.5);
    notice.set_justify(gtk::Justification::Center);
    centered(&notice);
    notice.set_visible(false);
    overlay.add_overlay(&notice);
    root.append(&overlay);
    let controls = (!attachment.animated).then(|| controls(images, width < COMPACT));
    if let Some(controls) = &controls {
        // The bar's room is kept while it is hidden, so the picture never
        // jumps when it appears.
        overlay.set_vexpand(false);
        overlay.set_size_request(-1, (height - CONTROLS - 8).max(1));
        root.append(&controls.bar);
    }
    root.set_size_request(width, height);

    let stage = Rc::new(Stage {
        attachment: attachment.clone(),
        streams: images.streams.clone(),
        local: RefCell::default(),
        player: RefCell::default(),
        frame,
        glyph: glyph.upcast(),
        spinner,
        notice,
        controls,
        ticking: Cell::new(false),
        scrub: Cell::new(None),
        seek: RefCell::default(),
    });
    stage.connect();
    // Clicking the picture plays or pauses.
    let clicks = gtk::GestureClick::new();
    let shown = Rc::downgrade(&stage);
    clicks.connect_released(move |_, _, _, _| {
        if let Some(stage) = shown.upgrade() {
            stage.toggle();
        }
    });
    overlay.add_controller(clicks);
    // Leaving the window (closing the viewer, stepping on, its row leaving
    // the timeline) stops it and frees its route.
    let shown = Rc::downgrade(&stage);
    root.connect_unrealize(move |_| {
        if let Some(stage) = shown.upgrade() {
            stage.stop();
        }
    });
    // (Held by the root: the stage lives as long as it does.)
    let picked = stage.clone();
    root.connect_realize(move |root| {
        let scale = root.scale_factor() as f32;
        let fit = [
            (width as f32 * scale).min(MAX_VIDEO[0]),
            (height as f32 * scale).min(MAX_VIDEO[1]),
        ];
        picked.start(fit);
    });
    Video { root, stage }
}

fn picture() -> gtk::Picture {
    let picture = gtk::Picture::new();
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture
}

fn centered(widget: &impl IsA<gtk::Widget>) {
    widget.set_halign(gtk::Align::Center);
    widget.set_valign(gtk::Align::Center);
}

fn controls(images: &Images, compact: bool) -> Controls {
    let button = |icon, name| {
        let button: gtk::Button = crate::app::named("GtkButton", name);
        let glyph = images.icon_image(icon, "#f3f4f5", 18);
        centered(&glyph);
        button.set_child(Some(&glyph));
        button.set_tooltip_text(Some(name));
        button.add_css_class("video-button");
        button.set_valign(gtk::Align::Center);
        button
    };
    let scale = |name: &str, max: f64| {
        let scale: gtk::Scale = crate::app::named("GtkScale", name);
        scale.set_adjustment(&gtk::Adjustment::new(
            0.0,
            0.0,
            max,
            max / 100.0,
            max / 10.0,
            0.0,
        ));
        scale.set_draw_value(false);
        scale.add_css_class("video-track");
        scale.set_valign(gtk::Align::Center);
        scale
    };
    let time = crate::ui::label("0:00 / 0:00", &["muted", "video-time"]);
    time.set_visible(!compact);
    let seek = scale("Seek", 1.0);
    seek.set_hexpand(true);
    let volume = scale("Volume", 1.0);
    volume.set_value(1.0);
    volume.set_size_request(80, -1);
    volume.set_visible(!compact);
    let (play, pause) = (button(Icon::Play, "Play"), button(Icon::Pause, "Pause"));
    let (mute, unmute) = (
        button(Icon::Volume, "Mute"),
        button(Icon::VolumeOff, "Unmute"),
    );
    pause.set_visible(false);
    unmute.set_visible(false);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    bar.add_css_class("video-controls");
    for widget in [
        play.upcast_ref::<gtk::Widget>(),
        pause.upcast_ref(),
        time.upcast_ref(),
        seek.upcast_ref(),
        mute.upcast_ref(),
        unmute.upcast_ref(),
        volume.upcast_ref(),
    ] {
        bar.append(widget);
    }
    bar.set_visible(false);
    Controls {
        bar,
        play,
        pause,
        time,
        seek,
        mute,
        unmute,
        volume,
    }
}

impl Stage {
    fn connect(self: &Rc<Self>) {
        let Some(controls) = &self.controls else {
            return;
        };
        for button in [&controls.play, &controls.pause] {
            let shown = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(stage) = shown.upgrade() {
                    stage.toggle();
                }
            });
        }
        for button in [&controls.mute, &controls.unmute] {
            let shown = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(stage) = shown.upgrade() {
                    if let Some(player) = stage.player.borrow_mut().as_mut() {
                        player.toggle_mute();
                    }
                    stage.render();
                }
            });
        }
        let shown = Rc::downgrade(self);
        controls.volume.connect_change_value(move |_, _, value| {
            if let Some(stage) = shown.upgrade()
                && let Some(player) = stage.player.borrow_mut().as_mut()
            {
                player.set_volume(value.clamp(0.0, 1.0) as f32);
                if player.muted() {
                    player.toggle_mute();
                }
            }
            if let Some(stage) = shown.upgrade() {
                stage.render();
            }
            glib::Propagation::Proceed
        });
        // Dragging shows the time under the pointer; the seek waits until
        // the bar settles, since each one restarts FFmpeg.
        let shown = Rc::downgrade(self);
        controls.seek.connect_change_value(move |_, _, value| {
            let Some(stage) = shown.upgrade() else {
                return glib::Propagation::Proceed;
            };
            stage.scrub.set(Some(value));
            if let Some(pending) = stage.seek.take() {
                pending.remove();
            }
            let settled = Rc::downgrade(&stage);
            stage
                .seek
                .replace(Some(glib::timeout_add_local_once(SEEK_SETTLE, move || {
                    let Some(stage) = settled.upgrade() else {
                        return;
                    };
                    stage.seek.take();
                    if let Some(at) = stage.scrub.take()
                        && let Some(player) = stage.player.borrow_mut().as_mut()
                    {
                        player.seek(at.max(0.0) as u64, false);
                    }
                    stage.tick();
                })));
            stage.render();
            glib::Propagation::Proceed
        });
    }

    /// Probes the video, then plays it from the start (sound on, unless it
    /// is GIF-style).
    fn start(self: &Rc<Self>, fit: [f32; 2]) {
        let failed = |stage: &Self, message: &str| {
            stage.spinner.stop();
            stage.spinner.set_visible(false);
            stage.notice.set_label(message);
            stage.notice.set_visible(true);
        };
        let Some(binary) = caper_client::ffmpeg::binary() else {
            return failed(
                self,
                "Video playback needs the FFmpeg that ships with Caper.",
            );
        };
        let Some(local) = self
            .attachment
            .url
            .as_deref()
            .and_then(|url| self.streams.route(url))
        else {
            return failed(self, "This video could not be streamed.");
        };
        self.local.replace(Some(local.clone()));
        let stage = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let input = local.clone();
            let probe = gio::spawn_blocking(move || {
                caper_client::ffmpeg::probe(binary, std::path::Path::new(&input)).ok()
            })
            .await
            .ok()
            .flatten();
            let Some(stage) = stage.upgrade() else {
                return;
            };
            // Stopped (or restarted) while probing.
            if stage.local.borrow().as_deref() != Some(local.as_str()) {
                return;
            }
            let Some((probe, video)) = probe.and_then(|probe| {
                let video = probe.video.clone()?;
                Some((probe, video))
            }) else {
                return failed(
                    &stage,
                    "This video can’t be played here. Open it in your browser instead.",
                );
            };
            let mut player = Player::new(
                binary,
                local,
                video.width,
                video.height,
                probe.duration_ms.or(stage.attachment.duration_ms),
                video.fps,
                fit,
            );
            if stage.attachment.animated {
                player.toggle_mute();
            }
            player.play();
            if let Some(controls) = &stage.controls {
                controls.bar.set_visible(true);
                let duration = player.duration_ms().unwrap_or(0) as f64;
                controls.seek.adjustment().set_upper(duration.max(1.0));
                controls.seek.set_sensitive(duration > 0.0);
            }
            stage.player.replace(Some(player));
            stage.tick();
        });
    }

    fn toggle(self: &Rc<Self>) {
        if let Some(player) = self.player.borrow_mut().as_mut() {
            player.toggle();
        }
        self.render();
        self.tick();
    }

    fn stop(&self) {
        self.player.take();
        if let Some(pending) = self.seek.take() {
            pending.remove();
        }
        if let Some(local) = self.local.take() {
            self.streams.remove(&local);
        }
    }

    /// Shows frames as they fall due, for as long as the player has any.
    fn tick(self: &Rc<Self>) {
        if self.ticking.replace(true) {
            return;
        }
        self.schedule(Duration::ZERO);
    }

    fn schedule(self: &Rc<Self>, delay: Duration) {
        let stage = Rc::downgrade(self);
        glib::timeout_add_local_once(delay, move || {
            let Some(stage) = stage.upgrade() else {
                return;
            };
            match stage.update() {
                Some(next) => stage.schedule(next),
                None => stage.ticking.set(false),
            }
        });
    }

    fn update(&self) -> Option<Duration> {
        let next = {
            let mut player = self.player.borrow_mut();
            let player = player.as_mut()?;
            let frame = &self.frame;
            let next = player.update(|[width, height], rgba| {
                let texture = gdk::MemoryTexture::new(
                    width as i32,
                    height as i32,
                    gdk::MemoryFormat::R8g8b8a8,
                    &glib::Bytes::from(rgba),
                    width * 4,
                );
                frame.set_paintable(Some(&texture));
            });
            // GIF-style videos loop, like a GIF.
            if self.attachment.animated && player.state() == State::Ended {
                player.seek(0, true);
                Some(Duration::ZERO)
            } else {
                next
            }
        };
        self.render();
        next
    }

    /// The controls, spinner and paused glyph for where playback is.
    fn render(&self) {
        let player = self.player.borrow();
        let Some(player) = player.as_ref() else {
            return;
        };
        let state = player.state();
        let shown = self.frame.paintable().is_some();
        self.spinner.set_visible(!shown && player.error().is_none());
        if let Some(error) = player.error() {
            self.notice.set_label(error);
        }
        self.notice.set_visible(player.error().is_some());
        self.glyph
            .set_visible(shown && matches!(state, State::Paused | State::Ended));
        let Some(controls) = &self.controls else {
            return;
        };
        let playing = state == State::Playing;
        swap(&controls.play, &controls.pause, playing);
        let muted = player.muted() || player.volume() == 0.0;
        swap(&controls.mute, &controls.unmute, muted);
        if !muted {
            controls.volume.set_value(f64::from(player.volume()));
        } else {
            controls.volume.set_value(0.0);
        }
        let duration = player.duration_ms().unwrap_or(0);
        let position = self
            .scrub
            .get()
            .map_or(player.position_ms(), |at| at.max(0.0) as u64);
        if self.scrub.get().is_none() {
            controls.seek.set_value(position as f64);
        }
        controls.time.set_label(&format!(
            "{} / {}",
            player::time_label(position),
            player::time_label(duration)
        ));
    }
}

/// Shows `on` in place of `off` when `which`, keeping keyboard focus on
/// whichever is showing.
fn swap(off: &gtk::Button, on: &gtk::Button, which: bool) {
    if on.is_visible() == which {
        return;
    }
    let focused = off.has_focus() || on.has_focus();
    off.set_visible(!which);
    on.set_visible(which);
    if focused {
        if which {
            on.grab_focus();
        } else {
            off.grab_focus();
        }
    }
}
