//! Voice messages play in the app, as on the egui desktop: a card with
//! Play or Pause, the message's waveform (click or drag to seek) and its
//! time (the length, or what remains once started), then its transcript
//! ("Transcribing…" until it lands; three lines until "View transcript").
//! One voice message plays at a time, app-wide, through the bundled FFmpeg
//! (`caper_client::player`) and a loopback route to its media
//! (`caper_client::media_proxy`); reaching the end returns it to idle.

use crate::images::{Icon, Images};
use crate::ui::label;
use caper_client::media_proxy::MediaProxy;
use caper_client::model::{Attachment, TranscriptView};
use caper_client::player::{Player, State};
use caper_client::uploads::{Prepared, Scratch};
use caper_client::voice_note;
use gtk::{cairo, glib, pango, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Waveform bars: 3 px wide, 5 px apart, as on the desktop.
const BAR: f64 = 3.0;
const PITCH: f64 = 5.0;
/// The card's width: the desktop's, in a timeline wider than it.
const WIDTH: i32 = 320;
/// The transcript's, inside its rule (`.voice-transcript`).
const TEXT_WIDTH: i32 = WIDTH - 12;

/// The one voice message playing (or paused, or failed to start).
struct Current {
    id: String,
    player: Option<Player>,
    /// Its loopback route, removed when it stops.
    local: Option<String>,
    /// A staged recording's file, kept while it plays.
    _scratch: Option<Arc<Scratch>>,
    error: Option<String>,
}

/// What a voice player plays: a sent message, or a recording staged in a
/// composer (from its own bytes: it may not have finished uploading).
pub enum Source<'a> {
    Sent(&'a Attachment),
    Staged(&'a Arc<Prepared>),
}

impl Source<'_> {
    fn id(&self) -> String {
        match self {
            Source::Sent(attachment) => attachment.id.clone(),
            Source::Staged(prepared) => staged_id(prepared),
        }
    }

    fn duration_ms(&self) -> Option<u64> {
        match self {
            Source::Sent(attachment) => attachment.duration_ms,
            Source::Staged(prepared) => prepared.duration_ms,
        }
    }
}

/// A staged recording's player id: stable while it stays staged.
fn staged_id(prepared: &Arc<Prepared>) -> String {
    format!("staged-voice-{:p}", Arc::as_ptr(prepared))
}

/// A card on screen, updated as playback moves.
struct Card {
    id: String,
    duration: u64,
    play: glib::WeakRef<gtk::Button>,
    pause: glib::WeakRef<gtk::Button>,
    wave: glib::WeakRef<gtk::DrawingArea>,
    time: glib::WeakRef<gtk::Label>,
    /// Where a sent message shows why it can't play (chips show none).
    error: Option<glib::WeakRef<gtk::Label>>,
    /// How much has played (0–1), as the waveform draws it.
    played: Rc<Cell<f32>>,
    /// A drag in progress, shown instead of the player's position.
    scrub: Rc<Cell<Option<f32>>>,
}

/// Loopback routes to media for FFmpeg (`caper_client::media_proxy`),
/// started on first use and shared by voice messages and videos.
#[derive(Default)]
pub struct Streams(RefCell<Option<MediaProxy>>);

impl Streams {
    /// A local URL for `url`, or `None` when it can't be streamed.
    pub fn route(&self, url: &str) -> Option<String> {
        caper_client::api::media_url(url)?;
        let mut proxy = self.0.borrow_mut();
        if proxy.is_none() {
            *proxy = MediaProxy::start().ok();
        }
        Some(proxy.as_ref()?.route(url))
    }

    pub fn remove(&self, local: &str) {
        if let Some(proxy) = self.0.borrow().as_ref() {
            proxy.remove(local);
        }
    }
}

pub struct Voice {
    streams: Rc<Streams>,
    current: RefCell<Option<Current>>,
    cards: RefCell<Vec<Card>>,
    ticking: Cell<bool>,
}

impl Voice {
    pub fn new(streams: Rc<Streams>) -> Self {
        Self {
            streams,
            current: RefCell::default(),
            cards: RefCell::default(),
            ticking: Cell::new(false),
        }
    }

    /// Plays or pauses `source` (or plays it from `at` ms), stopping any
    /// other voice message first.
    pub fn play(self: &Rc<Self>, source: &Source, at: Option<u64>) {
        let id = source.id();
        {
            let mut current = self.current.borrow_mut();
            if let Some(player) = current
                .as_mut()
                .filter(|current| current.id == id)
                .and_then(|current| current.player.as_mut())
            {
                match at {
                    Some(ms) => player.seek(ms, true),
                    None => player.toggle(),
                }
                drop(current);
                self.render();
                self.tick();
                return;
            }
        }
        self.stop();
        let failed = |message: &str| Current {
            id: id.clone(),
            player: None,
            local: None,
            _scratch: None,
            error: Some(message.into()),
        };
        let started = (|| {
            let binary = caper_client::ffmpeg::binary()
                .ok_or("Voice playback needs the FFmpeg that ships with Caper.")?;
            let (input, local, scratch) = match source {
                Source::Sent(attachment) => {
                    let local = attachment
                        .url
                        .as_deref()
                        .and_then(|url| self.streams.route(url))
                        .ok_or("This voice message can’t be played here.")?;
                    (local.clone(), Some(local), None)
                }
                Source::Staged(prepared) => {
                    let scratch = prepared
                        .on_disk()
                        .map_err(|_| "This recording can’t be played here.")?;
                    let input = scratch.path().to_string_lossy().into_owned();
                    (input, None, Some(scratch))
                }
            };
            let mut player = Player::audio(binary, input, source.duration_ms());
            match at {
                Some(ms) => player.seek(ms, true),
                None => player.play(),
            }
            Ok::<_, &str>(Current {
                id: id.clone(),
                player: Some(player),
                local,
                _scratch: scratch,
                error: None,
            })
        })();
        *self.current.borrow_mut() = Some(started.unwrap_or_else(failed));
        self.render();
        self.tick();
    }

    /// Stops whatever plays and returns its card to idle.
    pub fn stop(&self) {
        let stopped = self.current.borrow_mut().take();
        if let Some(Current {
            local: Some(local), ..
        }) = stopped
        {
            self.streams.remove(&local);
        }
        self.render();
    }

    /// Keeps the clock moving while a voice message is loaded; it stops
    /// at the end, or once its message leaves the screen (another channel,
    /// a closed thread, sign-out).
    fn tick(self: &Rc<Self>) {
        if self.ticking.replace(true) {
            return;
        }
        let voice = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(33), move || {
            let Some(voice) = voice.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let state = voice
                .current
                .borrow_mut()
                .as_mut()
                .and_then(|current| current.player.as_mut())
                .map(|player| {
                    player.update(|_, _| {});
                    player.state()
                });
            voice.cards.borrow_mut().retain(|card| {
                card.wave
                    .upgrade()
                    .is_some_and(|wave| wave.root().is_some())
            });
            let shown = voice.current.borrow().as_ref().is_some_and(|current| {
                voice
                    .cards
                    .borrow()
                    .iter()
                    .any(|card| card.id == current.id)
            });
            if state == Some(State::Ended) || !shown {
                voice.stop();
            } else {
                voice.render();
            }
            if voice.current.borrow().is_some() {
                return glib::ControlFlow::Continue;
            }
            voice.ticking.set(false);
            glib::ControlFlow::Break
        });
    }

    /// Whether `id` plays (or is starting), its position and any error.
    fn state_of(&self, id: &str) -> (bool, Option<u64>, Option<String>) {
        let current = self.current.borrow();
        let Some(current) = current.as_ref().filter(|current| current.id == id) else {
            return (false, None, None);
        };
        let state = current
            .player
            .as_ref()
            .map(|player| (player.state(), player.position_ms()));
        let error = current.error.clone().or_else(|| {
            current
                .player
                .as_ref()
                .and_then(|player| player.error().map(str::to_owned))
        });
        (
            matches!(state, Some((State::Playing | State::Loading, _))),
            state.map(|(_, position)| position),
            error,
        )
    }

    /// Brings every card on screen up to date; gone ones are forgotten.
    fn render(&self) {
        let Ok(mut cards) = self.cards.try_borrow_mut() else {
            return;
        };
        cards.retain(|card| card.wave.upgrade().is_some());
        for card in cards.iter() {
            let (playing, position, error) = self.state_of(&card.id);
            let (Some(play), Some(pause), Some(wave), Some(time)) = (
                card.play.upgrade(),
                card.pause.upgrade(),
                card.wave.upgrade(),
                card.time.upgrade(),
            ) else {
                continue;
            };
            let refocus = (play.has_focus() && playing) || (pause.has_focus() && !playing);
            play.set_visible(!playing);
            pause.set_visible(playing);
            if refocus {
                if playing {
                    pause.grab_focus();
                } else {
                    play.grab_focus();
                }
            }
            let played = card.scrub.get().unwrap_or(match position {
                Some(position) if card.duration > 0 => position as f32 / card.duration as f32,
                _ => 0.0,
            });
            if (card.played.replace(played) - played).abs() > f32::EPSILON {
                wave.queue_draw();
            }
            // Idle shows the length; started, what remains.
            let shown = card.duration.saturating_sub(position.unwrap_or(0));
            let text = voice_note::duration_label(shown);
            if time.label() != text {
                time.set_label(&text);
            }
            if let Some(error_label) = card.error.as_ref().and_then(glib::WeakRef::upgrade) {
                error_label.set_label(error.as_deref().unwrap_or_default());
                error_label.set_tooltip_text(error.as_deref());
                error_label.set_visible(error.is_some());
            }
        }
    }
}

/// A ready voice message's card: player, then its transcript.
pub fn card(images: &Images, attachment: &Attachment) -> gtk::Box {
    let voice = images.voice.clone();
    let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
    root.add_css_class("voice-message");
    root.set_halign(gtk::Align::Start);
    root.set_size_request(WIDTH, -1);
    let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bar.add_css_class("voice-bar");
    let play: gtk::Button = crate::app::named("GtkButton", "Play voice message");
    play.set_child(Some(&images.icon_image(Icon::Play, "#0c0d0f", 14)));
    let pause: gtk::Button = crate::app::named("GtkButton", "Pause voice message");
    pause.set_child(Some(&images.icon_image(Icon::Pause, "#0c0d0f", 14)));
    pause.add_css_class("pause");
    pause.set_visible(false);
    for button in [&play, &pause] {
        if let Some(glyph) = button.child() {
            glyph.set_halign(gtk::Align::Center);
            glyph.set_valign(gtk::Align::Center);
        }
        button.add_css_class("voice-play");
        button.set_valign(gtk::Align::Center);
        bar.append(button);
    }
    let duration = attachment.duration_ms.unwrap_or(0);
    let played = Rc::new(Cell::new(0.0_f32));
    let scrub = Rc::new(Cell::new(None::<f32>));
    let wave: gtk::DrawingArea = crate::app::named("GtkDrawingArea", "Voice message position");
    wave.set_hexpand(true);
    wave.set_content_height(28);
    wave.set_valign(gtk::Align::Center);
    wave.set_tooltip_text(Some("Voice message position"));
    let peaks = attachment.waveform.clone();
    let shown = played.clone();
    wave.set_draw_func(move |_, cr, width, height| {
        draw_wave(cr, width, height, peaks.as_deref(), shown.get());
    });
    bar.append(&wave);
    let time = label(&voice_note::duration_label(duration), &["muted", "small"]);
    time.set_width_chars(5);
    time.set_xalign(1.0);
    bar.append(&time);
    root.append(&bar);
    let error = label("", &["error", "small"]);
    error.set_xalign(0.0);
    // One line, as the transcript: a wrapping label would widen the card.
    error.set_ellipsize(pango::EllipsizeMode::End);
    error.set_max_width_chars(30);
    error.set_visible(false);
    root.append(&error);
    if let Some(transcript) = transcript(attachment) {
        root.append(&transcript);
    }

    for button in [&play, &pause] {
        let (voice, attachment) = (Rc::downgrade(&voice), attachment.clone());
        button.connect_clicked(move |_| {
            if let Some(voice) = voice.upgrade() {
                voice.play(&Source::Sent(&attachment), None);
            }
        });
    }
    // A click or drag across the waveform seeks there.
    // (The gesture's own widget, not a captured one: a captured clone
    // would keep the card alive after its row is gone.)
    let drag = gtk::GestureDrag::new();
    let fraction = |drag: &gtk::GestureDrag, offset_x: f64| {
        let (x, _) = drag.start_point()?;
        let width = drag.widget()?.width().max(1);
        Some(((x + offset_x) / f64::from(width)).clamp(0.0, 1.0) as f32)
    };
    let held = scrub.clone();
    let voice_ref = Rc::downgrade(&voice);
    drag.connect_drag_update(move |drag, offset_x, _| {
        held.set(fraction(drag, offset_x));
        if let Some(voice) = voice_ref.upgrade() {
            voice.render();
        }
    });
    let (held, attachment_ref) = (scrub.clone(), attachment.clone());
    let voice_ref = Rc::downgrade(&voice);
    drag.connect_drag_end(move |drag, offset_x, _| {
        held.set(None);
        if let (Some(voice), Some(at)) = (voice_ref.upgrade(), fraction(drag, offset_x))
            && duration > 0
        {
            voice.play(
                &Source::Sent(&attachment_ref),
                Some((at * duration as f32) as u64),
            );
        }
    });
    wave.add_controller(drag);

    voice.cards.borrow_mut().push(Card {
        id: attachment.id.clone(),
        duration,
        play: play.downgrade(),
        pause: pause.downgrade(),
        wave: wave.downgrade(),
        time: time.downgrade(),
        error: Some(error.downgrade()),
        played,
        scrub,
    });
    voice.render();
    root
}

/// A recording staged in a composer, as on the desktop: Play or Pause, its
/// waveform and length (what remains once started), the upload's progress
/// or "Retry upload", and Remove.
pub fn chip(
    images: &Images,
    prepared: &Arc<Prepared>,
    progress: Option<f32>,
    error: Option<&str>,
    remove: impl Fn() + 'static,
    retry: impl Fn() + 'static,
) -> gtk::Box {
    let voice = images.voice.clone();
    let duration = prepared.duration_ms.unwrap_or(0);
    let summary = format!(
        "Voice message, {}{}",
        voice_note::duration_label(duration),
        match (error, progress) {
            (Some(error), _) => format!(", {error}"),
            (None, Some(_)) => ", uploading".into(),
            (None, None) => String::new(),
        }
    );
    let root: gtk::Box = crate::app::named("GtkBox", &summary);
    root.set_orientation(gtk::Orientation::Horizontal);
    root.set_spacing(8);
    root.add_css_class("voice-chip");
    if error.is_some() {
        root.add_css_class("failed");
    }
    root.set_size_request(236, 52);
    // Its waveform expands within the chip, not the chip along the row.
    root.set_hexpand(false);
    let play: gtk::Button = crate::app::named("GtkButton", "Play voice message");
    let pause: gtk::Button = crate::app::named("GtkButton", "Pause voice message");
    for (button, icon) in [(&play, Icon::Play), (&pause, Icon::Pause)] {
        let glyph = images.icon_image(icon, "#0c0d0f", 12);
        glyph.set_halign(gtk::Align::Center);
        glyph.set_valign(gtk::Align::Center);
        button.set_child(Some(&glyph));
        button.add_css_class("voice-play");
        button.add_css_class("small");
        button.set_valign(gtk::Align::Center);
        root.append(button);
    }
    pause.add_css_class("pause");
    pause.set_visible(false);
    let middle = gtk::Box::new(gtk::Orientation::Vertical, 4);
    middle.set_hexpand(true);
    middle.set_valign(gtk::Align::Center);
    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let played = Rc::new(Cell::new(0.0_f32));
    let wave = gtk::DrawingArea::new();
    wave.set_hexpand(true);
    wave.set_content_height(16);
    let peaks = prepared
        .voice
        .as_ref()
        .and_then(|voice| voice.waveform.clone());
    let shown = played.clone();
    wave.set_draw_func(move |_, cr, width, height| {
        draw_wave(cr, width, height, peaks.as_deref(), shown.get());
    });
    top.append(&wave);
    let time = label(&voice_note::duration_label(duration), &["muted", "tiny"]);
    time.set_width_chars(4);
    time.set_xalign(1.0);
    top.append(&time);
    middle.append(&top);
    // Upload state along the bottom: progress, then nothing once uploaded.
    match (error, progress) {
        (Some(_), _) => {
            let again = gtk::Button::with_label("Retry upload");
            again.add_css_class("flat");
            again.add_css_class("chip-retry");
            again.set_halign(gtk::Align::Start);
            again.connect_clicked(move |_| retry());
            middle.append(&again);
        }
        (None, Some(progress)) => {
            let bar = gtk::ProgressBar::new();
            bar.add_css_class("chip-progress");
            bar.set_fraction(f64::from(progress.clamp(0.0, 1.0)));
            middle.append(&bar);
        }
        (None, None) => {}
    }
    root.append(&middle);
    let close: gtk::Button = crate::app::named("GtkButton", "Remove voice message");
    close.set_child(Some(&images.icon_image(Icon::Close, "#b9bcbe", 14)));
    close.set_tooltip_text(Some("Remove voice message"));
    close.add_css_class("flat");
    close.add_css_class("chip-remove");
    close.set_valign(gtk::Align::Center);
    close.connect_clicked(move |_| remove());
    root.append(&close);
    for button in [&play, &pause] {
        let (voice, prepared) = (Rc::downgrade(&voice), prepared.clone());
        button.connect_clicked(move |_| {
            if let Some(voice) = voice.upgrade() {
                voice.play(&Source::Staged(&prepared), None);
            }
        });
    }
    voice.cards.borrow_mut().push(Card {
        id: staged_id(prepared),
        duration,
        play: play.downgrade(),
        pause: pause.downgrade(),
        wave: wave.downgrade(),
        time: time.downgrade(),
        error: None,
        played,
        scrub: Rc::new(Cell::new(None)),
    });
    voice.render();
    root
}

/// The bars that fit `width`, the played part (0–1) bright over the rest.
fn draw_wave(cr: &cairo::Context, width: i32, height: i32, peaks: Option<&str>, played: f32) {
    let (width, height) = (f64::from(width), f64::from(height));
    let count = (((width + PITCH - BAR) / PITCH).floor() as usize).max(1);
    let levels = voice_note::bars(peaks, count);
    cr.set_line_width(BAR);
    cr.set_line_cap(cairo::LineCap::Round);
    let draw = |cr: &cairo::Context| {
        for (index, level) in levels.iter().enumerate() {
            let x = index as f64 * PITCH + BAR / 2.0;
            // Never shorter than its own width: silence reads as dots.
            let half = ((f64::from(level.clamp(0.0, 1.0)) * height).max(BAR) - BAR) / 2.0;
            cr.move_to(x, height / 2.0 - half);
            cr.line_to(x, height / 2.0 + half);
        }
        let _ = cr.stroke();
    };
    // Muted at 45%, then text colour over the played part.
    cr.set_source_rgba(0.725, 0.737, 0.745, 0.45);
    draw(cr);
    let played = f64::from(played.clamp(0.0, 1.0));
    if played > 0.0 {
        let _ = cr.save();
        cr.rectangle(0.0, 0.0, width * played, height);
        cr.clip();
        cr.set_source_rgb(0.953, 0.957, 0.961);
        draw(cr);
        let _ = cr.restore();
    }
}

/// "Transcribing…" while pending; the text in three lines until "View
/// transcript"; nothing when there is none.
fn transcript(attachment: &Attachment) -> Option<gtk::Box> {
    let text = match attachment.transcript_view() {
        TranscriptView::Hidden => return None,
        TranscriptView::Pending => None,
        TranscriptView::Text(text) => Some(text),
    };
    let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
    root.add_css_class("voice-transcript");
    let Some(text) = text else {
        let pending = label("Transcribing…", &["muted", "transcribing"]);
        pending.set_xalign(0.0);
        root.append(&pending);
        return Some(root);
    };
    // Laid out here, not by GTK: a wrapping label is sized by asking how
    // wide it must be for a height, which widens the card to the text's
    // full length. The card's width is fixed, so its lines are known.
    let words = label(text, &["transcript"]);
    words.set_selectable(true);
    words.set_ellipsize(pango::EllipsizeMode::End);
    words.set_max_width_chars(30);
    root.append(&words);
    let toggle = gtk::Button::with_label("View transcript");
    toggle.add_css_class("flat");
    toggle.add_css_class("transcript-toggle");
    toggle.set_halign(gtk::Align::Start);
    toggle.set_visible(false);
    root.append(&toggle);
    let text = text.to_owned();
    let lines = Rc::new(RefCell::new(Vec::<String>::new()));
    let expanded = Rc::new(Cell::new(false));
    // Three lines, the third ending in an ellipsis, or all of them.
    let show = {
        let (lines, expanded) = (lines.clone(), expanded.clone());
        move |words: &gtk::Label| {
            let lines = lines.borrow();
            words.set_label(&if expanded.get() || lines.len() <= 3 {
                lines.join("\n")
            } else {
                format!("{}\n{}\n{}", lines[0], lines[1], lines[2..].join(" "))
            });
        }
    };
    let (shown, show_toggled) = (words.downgrade(), show.clone());
    toggle.connect_clicked(move |toggle| {
        expanded.set(!expanded.get());
        toggle.set_label(if expanded.get() {
            "Hide transcript"
        } else {
            "View transcript"
        });
        if let Some(words) = shown.upgrade() {
            show_toggled(&words);
        }
    });
    // Once styled (its font comes from the card's CSS).
    let link = toggle.downgrade();
    words.add_tick_callback(move |words, _| {
        let layout = words.create_pango_layout(Some(&text));
        layout.set_width(TEXT_WIDTH * pango::SCALE);
        layout.set_wrap(pango::WrapMode::WordChar);
        *lines.borrow_mut() = layout
            .lines_readonly()
            .iter()
            .filter_map(|line| {
                let start = usize::try_from(line.start_index()).ok()?;
                let end = start + usize::try_from(line.length()).ok()?;
                Some(text.get(start..end)?.trim_end().to_owned())
            })
            .collect();
        if let Some(link) = link.upgrade() {
            link.set_visible(lines.borrow().len() > 3);
        }
        show(words);
        glib::ControlFlow::Break
    });
    Some(root)
}
