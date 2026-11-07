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
use caper_client::voice_note;
use gtk::{cairo, glib, pango, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
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
    error: Option<String>,
}

/// A card on screen, updated as playback moves.
struct Card {
    id: String,
    duration: u64,
    play: glib::WeakRef<gtk::Button>,
    pause: glib::WeakRef<gtk::Button>,
    wave: glib::WeakRef<gtk::DrawingArea>,
    time: glib::WeakRef<gtk::Label>,
    error: glib::WeakRef<gtk::Label>,
    /// How much has played (0–1), as the waveform draws it.
    played: Rc<Cell<f32>>,
    /// A drag in progress, shown instead of the player's position.
    scrub: Rc<Cell<Option<f32>>>,
}

#[derive(Default)]
pub struct Voice {
    current: RefCell<Option<Current>>,
    cards: RefCell<Vec<Card>>,
    proxy: RefCell<Option<MediaProxy>>,
    ticking: Cell<bool>,
}

impl Voice {
    /// Plays or pauses `attachment` (or plays it from `at` ms), stopping any
    /// other voice message first.
    pub fn play(self: &Rc<Self>, attachment: &Attachment, at: Option<u64>) {
        {
            let mut current = self.current.borrow_mut();
            if let Some(player) = current
                .as_mut()
                .filter(|current| current.id == attachment.id)
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
            id: attachment.id.clone(),
            player: None,
            local: None,
            error: Some(message.into()),
        };
        let started = (|| {
            let binary = caper_client::ffmpeg::binary()
                .ok_or("Voice playback needs the FFmpeg that ships with Caper.")?;
            let url = attachment
                .url
                .clone()
                .filter(|url| caper_client::api::media_url(url).is_some())
                .ok_or("This voice message can’t be played here.")?;
            let mut proxy = self.proxy.borrow_mut();
            if proxy.is_none() {
                *proxy = MediaProxy::start().ok();
            }
            let local = proxy
                .as_ref()
                .ok_or("This voice message can’t be played here.")?
                .route(&url);
            let mut player = Player::audio(binary, local.clone(), attachment.duration_ms);
            match at {
                Some(ms) => player.seek(ms, true),
                None => player.play(),
            }
            Ok::<_, &str>(Current {
                id: attachment.id.clone(),
                player: Some(player),
                local: Some(local),
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
            && let Some(proxy) = self.proxy.borrow().as_ref()
        {
            proxy.remove(&local);
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
            let (Some(play), Some(pause), Some(wave), Some(time), Some(error_label)) = (
                card.play.upgrade(),
                card.pause.upgrade(),
                card.wave.upgrade(),
                card.time.upgrade(),
                card.error.upgrade(),
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
            error_label.set_label(error.as_deref().unwrap_or_default());
            error_label.set_tooltip_text(error.as_deref());
            error_label.set_visible(error.is_some());
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
                voice.play(&attachment, None);
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
            voice.play(&attachment_ref, Some((at * duration as f32) as u64));
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
        error: error.downgrade(),
        played,
        scrub,
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
