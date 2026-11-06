//! The desktop's Audio test: microphone and speaker pickers with their
//! volumes, a speaker test, and a microphone test that compares the natural
//! and enhanced recording. Choices save to `audio-preferences-v1`. A child of
//! `app`.

use super::{App, clear};
use crate::ui::label;
use caper_client::voice::{MicrophoneState, VoiceOperation};
use gtk::{glib, prelude::*};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// The desktop's key for device choices and volumes.
pub(super) const PREFERENCES: &str = "audio-preferences-v1";
/// The input meter and recording timer redraw this often.
const METER: Duration = Duration::from_millis(80);

pub(super) struct AudioDialog {
    window: gtk::Window,
    pickers: [Picker; 2],
    speaker_gain: gtk::Scale,
    microphone_gain: gtk::Scale,
    speaker_test: gtk::Button,
    speaker_error: gtk::Label,
    device_error: gtk::Label,
    refresh: gtk::Button,
    title: gtk::Label,
    elapsed: gtk::Label,
    strength: gtk::Scale,
    strength_value: gtk::Label,
    microphone_button: gtk::Button,
    meter: gtk::LevelBar,
    microphone_error: gtk::Label,
    samples: gtk::Box,
    shown_samples: RefCell<String>,
    /// Set while the dialog itself moves a control, so its handler skips.
    syncing: Rc<Cell<bool>>,
}

struct Picker {
    input: bool,
    list: gtk::DropDown,
    ids: RefCell<Vec<Option<String>>>,
    shown: RefCell<String>,
    hint: gtk::Label,
}

fn slider(min: f64, max: f64) -> gtk::Scale {
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, 1.0);
    scale.set_draw_value(true);
    scale.set_value_pos(gtk::PositionType::Right);
    scale.set_digits(0);
    scale.set_hexpand(true);
    scale
}

impl App {
    fn default_device_label(&self, input: bool) -> String {
        let voice = self.voice.borrow();
        let name = if input {
            &voice.default_input
        } else {
            &voice.default_output
        };
        name.as_ref().map_or_else(
            || {
                if voice.refreshing_devices {
                    "System default — finding device…".into()
                } else {
                    "System default — name unavailable".into()
                }
            },
            |name| format!("System default — {name}"),
        )
    }

    pub(super) fn open_audio(self: &Rc<Self>) {
        if let Some(dialog) = self.audio.borrow().as_ref() {
            dialog.window.present();
            return;
        }
        self.voice.borrow_mut().refresh_devices();
        let window = gtk::Window::builder()
            .transient_for(&self.ui.window)
            .modal(true)
            .title("Audio test")
            .default_width(560)
            .default_height(780)
            .build();
        window.add_css_class("caper");
        window.add_css_class("dialog");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("dialog-body");
        root.append(&label("Audio test", &["dialog-title"]));
        root.append(&label(
            "Only you can hear these tests.",
            &["muted", "small"],
        ));
        let syncing = Rc::new(Cell::new(false));

        let columns = gtk::Box::new(gtk::Orientation::Horizontal, 24);
        columns.set_homogeneous(true);
        let mut pickers = Vec::new();
        let mut gains = Vec::new();
        let speaker_test = gtk::Button::with_label("Test speakers");
        let speaker_error = label(
            "Couldn’t play audio. Check your output and try again.",
            &["error", "small"],
        );
        speaker_error.set_wrap(true);
        for input in [true, false] {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
            let kind = if input { "Microphone" } else { "Speaker" };
            column.append(&label(kind, &["muted", "small", "strong"]));
            let list = crate::app::named::<gtk::DropDown>("GtkDropDown", kind);
            list.set_model(Some(&gtk::StringList::new(&[])));
            column.append(&list);
            let hint = label("", &["muted", "small"]);
            hint.set_wrap(true);
            column.append(&hint);
            let gain_title = if input {
                "Microphone volume"
            } else {
                "Speaker volume"
            };
            column.append(&label(gain_title, &["small", "strong"]));
            let gain = slider(0.0, 200.0);
            gain.set_value(f64::from(if input {
                self.voice.borrow().preferences.input_percent
            } else {
                self.voice.borrow().preferences.master_percent
            }));
            column.append(&gain);
            if !input {
                speaker_test.set_halign(gtk::Align::Start);
                column.append(&speaker_test);
                column.append(&speaker_error);
            }
            columns.append(&column);
            let weak = Rc::downgrade(self);
            let guard = syncing.clone();
            list.connect_selected_notify(move |list| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                if guard.get() {
                    return;
                }
                let id = app.audio.borrow().as_ref().and_then(|dialog| {
                    dialog.pickers[usize::from(!input)]
                        .ids
                        .borrow()
                        .get(list.selected() as usize)
                        .cloned()
                });
                let Some(id) = id else {
                    return;
                };
                app.voice.borrow_mut().command(match (input, id) {
                    (true, None) => VoiceOperation::DefaultInput,
                    (false, None) => VoiceOperation::DefaultOutput,
                    (true, Some(id)) => VoiceOperation::Input(id),
                    (false, Some(id)) => VoiceOperation::Output(id),
                });
                app.save_voice();
                app.render_audio();
            });
            let weak = Rc::downgrade(self);
            gain.connect_value_changed(move |scale| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                let percent = scale.value().round() as u16;
                {
                    let mut voice = app.voice.borrow_mut();
                    if input {
                        let strength = voice.preferences.processing_strength;
                        voice.set_input_processing(percent, strength);
                    } else {
                        voice.set_master_gain(percent);
                    }
                }
                app.effects.borrow_mut().slider(f32::from(percent) / 200.0);
                app.save_voice();
            });
            pickers.push(Picker {
                input,
                list,
                ids: RefCell::default(),
                shown: RefCell::default(),
                hint,
            });
            gains.push(gain);
        }
        root.append(&columns);
        let device_error = label("", &["error", "small"]);
        device_error.set_wrap(true);
        root.append(&device_error);
        let refresh = gtk::Button::with_label("Refresh devices");
        refresh.set_halign(gtk::Align::Start);
        root.append(&refresh);
        root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

        let heading = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = label("Try your microphone", &["settings-section"]);
        title.set_hexpand(true);
        heading.append(&title);
        let elapsed = label("", &["strong"]);
        heading.append(&elapsed);
        root.append(&heading);
        root.append(&label("Less noise. Clearer voice.", &["muted"]));
        let enhancement = gtk::Box::new(gtk::Orientation::Vertical, 6);
        enhancement.add_css_class("audio-panel");
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let name = label("Voice enhancement", &["small", "strong"]);
        name.set_hexpand(true);
        top.append(&name);
        let strength_value = label("", &["small", "strong", "accent"]);
        top.append(&strength_value);
        enhancement.append(&top);
        let strength = slider(0.0, 100.0);
        strength.set_draw_value(false);
        strength.set_value(f64::from(
            self.voice.borrow().preferences.processing_strength,
        ));
        enhancement.append(&strength);
        let ends = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let natural = label("Natural", &["muted", "small"]);
        natural.set_hexpand(true);
        ends.append(&natural);
        ends.append(&label("Enhanced", &["muted", "small"]));
        enhancement.append(&ends);
        root.append(&enhancement);
        let test = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let microphone_button = gtk::Button::with_label("Test microphone");
        microphone_button.add_css_class("primary");
        microphone_button.set_size_request(160, 44);
        test.append(&microphone_button);
        let level = gtk::Box::new(gtk::Orientation::Vertical, 6);
        level.set_hexpand(true);
        level.set_valign(gtk::Align::Center);
        level.append(&label("Input level", &["muted", "small", "strong"]));
        let meter = gtk::LevelBar::for_interval(0.0, 1.0);
        meter.set_mode(gtk::LevelBarMode::Continuous);
        meter.add_css_class("input-meter");
        level.append(&meter);
        test.append(&level);
        root.append(&test);
        let microphone_error = label("", &["error", "small"]);
        microphone_error.set_wrap(true);
        root.append(&microphone_error);
        let samples = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        samples.set_homogeneous(true);
        root.append(&samples);

        let weak = Rc::downgrade(self);
        let guard = syncing.clone();
        strength.connect_value_changed(move |scale| {
            let Some(app) = weak.upgrade() else {
                return;
            };
            if guard.get() {
                return;
            }
            let strength = scale.value().round() as u8;
            {
                let mut voice = app.voice.borrow_mut();
                let gain = voice.preferences.input_percent;
                voice.set_input_processing(gain, strength);
            }
            app.effects.borrow_mut().slider(f32::from(strength) / 100.0);
            app.save_voice();
            app.render_audio();
        });
        let weak = Rc::downgrade(self);
        microphone_button.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                {
                    let mut voice = app.voice.borrow_mut();
                    if matches!(voice.microphone, MicrophoneState::Recording(_)) {
                        voice.finish_mic_recording();
                    } else {
                        voice.start_mic_test();
                    }
                }
                app.render_audio();
            }
        });
        let weak = Rc::downgrade(self);
        speaker_test.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                {
                    let mut voice = app.voice.borrow_mut();
                    if voice.speaker_testing {
                        voice.stop_speaker_test();
                    } else {
                        voice.start_speaker_test();
                    }
                }
                app.render_audio();
            }
        });
        let weak = Rc::downgrade(self);
        refresh.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.voice.borrow_mut().refresh_devices();
                app.render_audio();
            }
        });
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(app) = weak.upgrade() {
                app.close_audio();
            }
            glib::Propagation::Stop
        });
        let keys = gtk::EventControllerKey::new();
        let weak = window.downgrade();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape
                && let Some(window) = weak.upgrade()
            {
                window.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        window.add_controller(keys);
        let mut pickers = pickers.into_iter();
        let mut gains = gains.into_iter();
        let (Some(input), Some(output), Some(microphone_gain), Some(speaker_gain)) =
            (pickers.next(), pickers.next(), gains.next(), gains.next())
        else {
            return;
        };
        window.set_child(Some(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .child(&root)
                .build(),
        ));
        *self.audio.borrow_mut() = Some(AudioDialog {
            window: window.clone(),
            pickers: [input, output],
            speaker_gain,
            microphone_gain,
            speaker_test,
            speaker_error,
            device_error,
            refresh,
            title,
            elapsed,
            strength,
            strength_value,
            microphone_button,
            meter,
            microphone_error,
            samples,
            shown_samples: RefCell::default(),
            syncing,
        });
        self.render_audio();
        window.present();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(METER, move || {
            let Some(app) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if app.audio.borrow().is_none() {
                return glib::ControlFlow::Break;
            }
            app.voice.borrow_mut().receive();
            app.render_audio();
            glib::ControlFlow::Continue
        });
    }

    /// Closing ends any test, as on the desktop.
    pub(super) fn close_audio(&self) {
        let Some(dialog) = self.audio.borrow_mut().take() else {
            return;
        };
        {
            let mut voice = self.voice.borrow_mut();
            if !matches!(voice.microphone, MicrophoneState::Idle) {
                voice.stop_mic_test();
            }
            if voice.speaker_testing {
                voice.stop_speaker_test();
            }
        }
        self.save_voice();
        dialog.window.destroy();
    }

    pub(super) fn render_audio(self: &Rc<Self>) {
        let audio = self.audio.borrow();
        let Some(dialog) = audio.as_ref() else {
            return;
        };
        let labels = [
            self.default_device_label(true),
            self.default_device_label(false),
        ];
        let mut voice = self.voice.borrow_mut();
        voice.sample_input_meter(Instant::now());
        let idle = matches!(voice.microphone, MicrophoneState::Idle);
        dialog.syncing.set(true);
        for picker in &dialog.pickers {
            let (preferred, devices) = if picker.input {
                (&voice.preferences.input, &voice.inputs)
            } else {
                (&voice.preferences.output, &voice.outputs)
            };
            let default = &labels[usize::from(!picker.input)];
            let mut ids = vec![None];
            let mut names = vec![default.clone()];
            for (id, name) in devices {
                ids.push(Some(id.clone()));
                names.push(name.clone());
            }
            let selected = match ids.iter().position(|id| id == preferred) {
                Some(index) => index,
                None => {
                    ids.push(preferred.clone());
                    names.push("Saved device (unavailable)".into());
                    ids.len() - 1
                }
            };
            let key = names.join("\n");
            if *picker.shown.borrow() != key {
                let model = gtk::StringList::new(&[]);
                for name in &names {
                    model.append(name);
                }
                picker.list.set_model(Some(&model));
                *picker.shown.borrow_mut() = key;
            }
            *picker.ids.borrow_mut() = ids;
            if picker.list.selected() != selected as u32 {
                picker.list.set_selected(selected as u32);
            }
            picker.list.set_sensitive(idle);
            picker
                .list
                .set_tooltip_text((!idle).then_some("End the microphone test to change devices."));
            picker.hint.set_label(if voice.refreshing_devices {
                "Finding devices…"
            } else if devices.is_empty() {
                "No devices found. Check system audio settings."
            } else {
                ""
            });
            picker.hint.set_visible(!picker.hint.label().is_empty());
        }
        let preferences = &voice.preferences;
        for (scale, value) in [
            (&dialog.microphone_gain, preferences.input_percent),
            (&dialog.speaker_gain, preferences.master_percent),
            (&dialog.strength, u16::from(preferences.processing_strength)),
        ] {
            if scale.value().round() as u16 != value {
                scale.set_value(f64::from(value));
            }
        }
        dialog
            .strength_value
            .set_label(&format!("{}%", preferences.processing_strength));
        dialog.syncing.set(false);
        dialog.refresh.set_sensitive(!voice.refreshing_devices);
        let device_error = voice.device_error.clone().unwrap_or_default();
        dialog.device_error.set_label(&device_error);
        dialog.device_error.set_visible(!device_error.is_empty());
        dialog.speaker_test.set_label(if voice.speaker_testing {
            "Stop speaker test"
        } else {
            "Test speakers"
        });
        dialog.speaker_error.set_visible(voice.speaker_failed);

        let state = voice.microphone.clone();
        let recording = match state {
            MicrophoneState::Recording(started) => Some(started),
            _ => None,
        };
        dialog.title.set_label(if recording.is_some() {
            "Recording…"
        } else {
            "Try your microphone"
        });
        dialog
            .elapsed
            .set_label(&recording.map_or_else(String::new, |started| {
                format!("{:.1}s", started.elapsed().as_secs_f32().min(30.0))
            }));
        dialog.strength.set_sensitive(recording.is_none());
        let (text, busy) = match state {
            MicrophoneState::Recording(_) => ("Stop recording", false),
            MicrophoneState::Preparing | MicrophoneState::Processing(_) => ("Preparing…", true),
            _ => ("Test microphone", false),
        };
        dialog.microphone_button.set_label(text);
        dialog.microphone_button.set_sensitive(!busy);
        let level = voice.input_meter.iter().copied().fold(0.0_f32, f32::max);
        dialog.meter.set_value(if recording.is_some() {
            f64::from(level.clamp(0.0, 1.0))
        } else {
            0.0
        });
        let microphone_error = voice.microphone_error.clone().unwrap_or_default();
        dialog.microphone_error.set_label(&microphone_error);
        dialog
            .microphone_error
            .set_visible(!microphone_error.is_empty());
        drop(voice);

        let (recorded, playing, processing) = match state {
            MicrophoneState::Ready(recorded) => (Some(recorded), None, false),
            MicrophoneState::Playing { recorded, enhanced } => {
                (Some(recorded), Some(enhanced), false)
            }
            MicrophoneState::Processing(recorded) => (Some(recorded), None, true),
            _ => (None, None, false),
        };
        let key = format!("{recorded:?}|{playing:?}|{processing}");
        if *dialog.shown_samples.borrow() == key {
            return;
        }
        *dialog.shown_samples.borrow_mut() = key;
        clear(&dialog.samples);
        let Some(recorded) = recorded else {
            dialog.samples.set_visible(false);
            return;
        };
        dialog.samples.set_visible(true);
        for enhanced in [false, true] {
            let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
            card.add_css_class("audio-panel");
            if enhanced && !processing {
                card.add_css_class("latest");
            }
            let name = if enhanced { "Enhanced" } else { "Natural" };
            card.append(&label(name, &["small", "strong"]));
            if enhanced && processing {
                card.append(&label("Preparing…", &["muted", "small"]));
                dialog.samples.append(&card);
                continue;
            }
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let this = playing == Some(enhanced);
            let play: gtk::Button = crate::app::named(
                "GtkButton",
                &format!("{} {name} audio sample", if this { "Stop" } else { "Play" }),
            );
            play.set_label(if this { "Stop" } else { "Play" });
            play.set_sensitive(!processing && (playing.is_none() || this));
            let weak = Rc::downgrade(self);
            play.connect_clicked(move |_| {
                let Some(app) = weak.upgrade() else {
                    return;
                };
                {
                    let mut voice = app.voice.borrow_mut();
                    if this {
                        voice.stop_mic_playback();
                    } else {
                        voice.play_mic_sample(enhanced);
                    }
                }
                let weak = Rc::downgrade(&app);
                glib::idle_add_local_once(move || {
                    if let Some(app) = weak.upgrade() {
                        app.render_audio();
                    }
                });
            });
            line.append(&play);
            line.append(&label(
                &format!("{:.1}s", recorded.seconds),
                &["muted", "small"],
            ));
            card.append(&line);
            if recorded.silent {
                let silent = label(
                    "No audible signal detected. Check your mic and try again.",
                    &["error", "small"],
                );
                silent.set_wrap(true);
                card.append(&silent);
            }
            dialog.samples.append(&card);
        }
    }
}

impl App {
    /// The account bar's settings button: Settings… and Audio test.
    pub(super) fn settings_menu(self: &Rc<Self>, anchor: &gtk::Button) {
        let popover = gtk::Popover::new();
        popover.add_css_class("menu");
        let items = gtk::Box::new(gtk::Orientation::Vertical, 2);
        for (text, audio) in [("Settings…", false), ("Audio test", true)] {
            let item = gtk::Button::with_label(text);
            item.add_css_class("flat");
            item.add_css_class("menu-item");
            let (weak, menu) = (Rc::downgrade(self), popover.clone());
            item.connect_clicked(move |_| {
                menu.popdown();
                let weak = weak.clone();
                glib::idle_add_local_once(move || {
                    if let Some(app) = weak.upgrade() {
                        if audio {
                            app.open_audio();
                        } else {
                            app.open_settings();
                        }
                    }
                });
            });
            items.append(&item);
        }
        popover.set_child(Some(&items));
        super::popup(&popover, anchor.upcast_ref());
        self.effects.borrow().toggle(true);
    }
}
