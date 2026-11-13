//! Dictation and voice messages from a composer, as on the egui desktop.
//! The microphone ("Dictate") replaces the input row with a recording bar:
//! Discard, a live dot and timer, the input meter, "Voice note" and "Done,
//! insert text". Done transcribes the recording (`POST /api/dictation`)
//! and inserts the words at the caret; "Voice note" keeps it as a voice
//! message instead, staged in the composer as a chip that plays and
//! uploads at once and goes with the next Send. A failed dictation offers
//! Retry, Keep as voice note and Discard. Escape discards; the recorder
//! stops itself at 10:00. A child of `app`.

use super::{App, State};
use crate::uploads::DraftFile;
use caper_client::uploads::{Body, Prepared};
use caper_client::voice_note::{self, LiveMeter, Recorder, RecorderEvent};
use caper_client::worker::Command;
use gtk::{cairo, gdk, glib, prelude::*};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often the bar redraws while recording.
const FRAME: Duration = Duration::from_millis(33);

/// Where a recording was made: its conversation and composer.
#[derive(Clone)]
pub(super) struct Origin {
    channel: String,
    /// The thread's root for its composer; `None` for the conversation's.
    root: Option<String>,
}

pub(super) struct Recording {
    origin: Origin,
    recorder: Recorder,
    /// When the microphone opened; the timer counts from here.
    started: Option<Instant>,
    /// Done (or 10:00): waiting for the encoded file.
    sending: bool,
    /// "Voice note": keep this one as a voice message.
    as_voice: bool,
    meter: LiveMeter,
}

impl Recording {
    fn elapsed(&self) -> Duration {
        self.started
            .map(|started| started.elapsed())
            .unwrap_or_default()
            .min(voice_note::MAX_DURATION)
    }
}

/// A voice message sending on its own once uploaded, with the reply that
/// was above its composer.
pub(super) struct VoiceSend {
    pub(super) file: DraftFile,
    reply: Option<String>,
}

/// A finished dictation: transcribing, or failed with its audio kept.
pub(super) struct Dictation {
    key: u64,
    origin: Origin,
    prepared: Arc<Prepared>,
    error: Option<String>,
}

impl State {
    fn origin(&self, thread: bool) -> Option<Origin> {
        Some(Origin {
            channel: self.channel.clone()?,
            root: App::composer_root(self, thread),
        })
    }

    /// Whether `origin` is the composer on screen for `thread`.
    fn shows(&self, origin: &Origin, thread: bool) -> bool {
        self.channel.as_ref() == Some(&origin.channel)
            && App::composer_root(self, thread) == origin.root
            && (!thread || origin.root.is_some())
    }

    /// Why a recording can't be kept as a voice message here: one per
    /// message, within the attachment limit.
    fn voice_note_blocked(&self, origin: &Origin) -> Option<&'static str> {
        let mut staged = self
            .files
            .iter()
            .filter(|file| file.channel == origin.channel && file.thread_root_id == origin.root);
        let voice = staged.clone().any(|file| file.voice().is_some());
        (voice
            || staged
                .nth(caper_client::uploads::MAX_ATTACHMENTS - 1)
                .is_some())
        .then_some("Remove the voice message to record another")
    }
}

impl App {
    pub(super) fn connect_recording(self: &Rc<Self>) {
        // While recording, Escape discards and an Enter nothing else took
        // (focus left with the hidden input row) finishes.
        let keys = gtk::EventControllerKey::new();
        let app = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(app) = app.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if app.recording.borrow().is_none() || !modifiers.is_empty() {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::Escape => app.discard_recording(),
                gdk::Key::Return | gdk::Key::KP_Enter => app.finish_recording(false),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        self.ui.window.add_controller(keys);
        for thread in [false, true] {
            let composer = if thread {
                &self.ui.thread_composer
            } else {
                &self.ui.composer
            };
            let app = Rc::downgrade(self);
            composer.mic.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    app.start_recording(thread);
                }
            });
            let bar = &composer.recording;
            let app = Rc::downgrade(self);
            bar.discard.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    app.discard_recording();
                }
            });
            let app = Rc::downgrade(self);
            bar.done.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    app.finish_recording(false);
                }
            });
            let app = Rc::downgrade(self);
            bar.voice_note.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    app.finish_recording(true);
                }
            });
            let app = Rc::downgrade(self);
            bar.meter.set_draw_func(move |_, cr, width, height| {
                if let Some(app) = app.upgrade()
                    && let Ok(recording) = app.recording.try_borrow()
                    && let Some(recording) = recording.as_ref()
                {
                    let scroll = if gtk::Settings::default()
                        .is_none_or(|settings| settings.is_gtk_enable_animations())
                    {
                        recording.meter.scroll(Instant::now())
                    } else {
                        0.0
                    };
                    let levels: Vec<f32> = recording.meter.bars.iter().copied().collect();
                    draw_levels(
                        cr,
                        width,
                        height,
                        &levels,
                        recording.meter.pending(),
                        scroll,
                    );
                }
            });
            let row = &composer.dictation;
            let app = Rc::downgrade(self);
            row.retry.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    app.send_dictation();
                }
            });
            let app = Rc::downgrade(self);
            row.keep.connect_clicked(move |_| {
                if let Some(app) = app.upgrade()
                    && let Some(dictation) = app.dictation.take()
                {
                    app.keep_voice(dictation.origin, dictation.prepared);
                }
            });
            let app = Rc::downgrade(self);
            row.drop.connect_clicked(move |_| {
                if let Some(app) = app.upgrade() {
                    // A late answer for a dropped dictation matches no key.
                    app.dictation.take();
                    app.render();
                }
            });
        }
    }

    /// Whether a composer's microphone records, and why not: recordings go
    /// wherever files can be attached, one at a time.
    fn mic_state(&self, state: &State, thread: bool) -> (bool, &'static str) {
        let root = Self::composer_root(state, thread);
        let mut staged = state.files.iter().filter(|file| {
            state.channel.as_ref() == Some(&file.channel)
                && file.thread_root_id.as_deref() == root.as_deref()
        });
        if !Self::can_attach(state) {
            (false, "File uploads are unavailable")
        } else if self.recording.borrow().is_some() {
            (false, "Finish the current voice message first")
        } else if self.dictation.borrow().is_some() {
            (false, "Finish the current dictation first")
        } else if self.gif_picker.borrow().is_some() {
            (false, "Close the GIF picker first")
        } else if staged.clone().any(|file| file.voice().is_some()) {
            (false, "Remove the voice message to record another")
        } else if staged
            .nth(caper_client::uploads::MAX_ATTACHMENTS - 1)
            .is_some()
        {
            (false, "Remove a file to add a voice message")
        } else {
            (true, "")
        }
    }

    /// Opens the microphone, stopping any voice message that plays.
    fn start_recording(self: &Rc<Self>, thread: bool) {
        let origin = {
            let mut state = self.state.borrow_mut();
            if !self.mic_state(&state, thread).0 {
                return;
            }
            state.send_error = None;
            state.origin(thread)
        };
        let Some(origin) = origin else {
            return;
        };
        self.images.voice.stop();
        let (input, gain, strength) = {
            let voice = self.voice.borrow();
            (
                voice.preferences.input.clone(),
                voice.preferences.input_percent,
                voice.preferences.processing_strength,
            )
        };
        let recorder = Recorder::start(input, gain, strength, || {});
        self.recording.replace(Some(Recording {
            origin,
            recorder,
            started: None,
            sending: false,
            as_voice: false,
            meter: LiveMeter::default(),
        }));
        self.tick_recording();
        self.render();
    }

    /// Trash or Escape: dropping the recorder closes the microphone and
    /// nothing is uploaded.
    fn discard_recording(self: &Rc<Self>) {
        if self.recording.take().is_some() {
            self.render();
        }
    }

    /// Done (`as_voice` false) or "Voice note": stop, then transcribe or
    /// keep once encoded.
    fn finish_recording(self: &Rc<Self>, as_voice: bool) {
        if let Some(recording) = self.recording.borrow_mut().as_mut()
            && recording.started.is_some()
            && !recording.sending
        {
            recording.sending = true;
            recording.as_voice |= as_voice;
            recording.recorder.finish();
        }
        self.render();
    }

    /// Reads the recorder every frame while it runs.
    fn tick_recording(self: &Rc<Self>) {
        if self.recording_ticking.replace(true) {
            return;
        }
        let app = Rc::downgrade(self);
        glib::timeout_add_local(FRAME, move || {
            let Some(app) = app.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if app.recording.borrow().is_none() {
                app.recording_ticking.set(false);
                return glib::ControlFlow::Break;
            }
            // One at a time: a finished recorder reports a failure on
            // every poll after its last event, and each final event ends
            // the recording.
            loop {
                let event = app
                    .recording
                    .borrow()
                    .as_ref()
                    .and_then(|recording| recording.recorder.poll());
                let Some(event) = event else {
                    break;
                };
                app.recorder_event(event);
            }
            if let Some(recording) = app.recording.borrow_mut().as_mut()
                && recording.started.is_some()
                && !recording.sending
            {
                let level = voice_note::perceptual(recording.recorder.level());
                recording.meter.sample(level, Instant::now());
                // The recorder stops itself at 10:00; the message then
                // goes on as Done would.
                if recording.elapsed() >= voice_note::MAX_DURATION {
                    recording.sending = true;
                }
            }
            app.render_recording();
            glib::ControlFlow::Continue
        });
    }

    fn recorder_event(self: &Rc<Self>, event: RecorderEvent) {
        match event {
            RecorderEvent::Started => {
                let thread = match self.recording.borrow_mut().as_mut() {
                    Some(recording) => {
                        recording.started = Some(Instant::now());
                        recording.origin.root.is_some()
                    }
                    None => return,
                };
                // Done takes the focus the hidden input row had.
                self.render_recording();
                let composer = if thread {
                    &self.ui.thread_composer
                } else {
                    &self.ui.composer
                };
                composer.recording.done.grab_focus();
            }
            // Under a second is a tap: nothing is uploaded.
            RecorderEvent::TooShort => {
                self.recording.take();
            }
            RecorderEvent::Failed(error) => {
                self.recording.take();
                self.state.borrow_mut().send_error = Some(error);
            }
            RecorderEvent::Ready(prepared) => {
                let Some(recording) = self.recording.take() else {
                    return;
                };
                let lost = recording.recorder.input_lost();
                let prepared = Arc::new(*prepared);
                if recording.as_voice {
                    self.keep_voice(recording.origin, prepared);
                } else {
                    self.dictation.replace(Some(Dictation {
                        key: 0,
                        origin: recording.origin,
                        prepared,
                        error: None,
                    }));
                    self.send_dictation();
                }
                // What was recorded is kept; say why it ended early.
                if lost {
                    self.state.borrow_mut().send_error = Some(voice_note::INPUT_LOST.into());
                }
            }
        }
        self.render();
    }

    /// Transcribes the dictation (again, for Retry).
    fn send_dictation(self: &Rc<Self>) {
        {
            let mut state = self.state.borrow_mut();
            let mut dictation = self.dictation.borrow_mut();
            let Some(dictation) = dictation.as_mut() else {
                return;
            };
            let Body::Memory(audio) = &dictation.prepared.body else {
                dictation.error = Some(voice_note::dictation_error(Some(400)).into());
                return;
            };
            let Some(token) = state.token.clone() else {
                dictation.error = Some(voice_note::dictation_error(Some(401)).into());
                return;
            };
            dictation.error = None;
            state.file_keys += 1;
            dictation.key = state.file_keys;
            self.worker.send(Command::Dictate {
                token,
                key: dictation.key,
                audio: audio.clone(),
                content_type: dictation.prepared.content_type.clone(),
            });
        }
        self.render();
    }

    /// The server's answer: the words go in at the caret of the composer
    /// they were spoken into.
    pub(super) fn dictated(self: &Rc<Self>, key: u64, result: Result<String, Option<u16>>) {
        let origin = match self.dictation.borrow().as_ref() {
            Some(dictation) if dictation.key == key => dictation.origin.clone(),
            _ => return,
        };
        let thread = origin.root.is_some();
        let here = self.state.borrow().shows(&origin, thread);
        let failure = match result {
            Err(status) => Some(voice_note::dictation_error(status).to_owned()),
            Ok(words) if words.trim().is_empty() => Some("No words heard.".to_owned()),
            Ok(_) if !here => Some("Open the conversation again to insert them.".to_owned()),
            Ok(words) => (!self.insert_dictation(thread, &words))
                .then(|| "Messages can be at most 4,000 characters.".to_owned()),
        };
        match failure {
            Some(error) => {
                if let Some(dictation) = self.dictation.borrow_mut().as_mut() {
                    dictation.error = Some(error);
                }
            }
            None => {
                self.dictation.take();
            }
        }
        self.render();
    }

    /// Inserts dictated words over the composer's selection, spaced from
    /// the words around them. False when the draft would be too long.
    fn insert_dictation(&self, thread: bool, words: &str) -> bool {
        let composer = if thread {
            &self.ui.thread_composer
        } else {
            &self.ui.composer
        };
        let buffer = composer.view.buffer();
        let text = composer.text();
        let (start, end) = buffer.selection_bounds().map_or_else(
            || {
                let caret = buffer.iter_at_mark(&buffer.get_insert()).offset();
                (caret, caret)
            },
            |(start, end)| (start.offset(), end.offset()),
        );
        let (start, end) = (start.max(0) as usize, end.max(0) as usize);
        let before: String = text.chars().take(start).collect();
        let after: String = text.chars().skip(end).collect();
        let spaced = voice_note::dictation_text(&before, &after, words);
        let token = caper_client::emoji::Token {
            start,
            end,
            query: String::new(),
        };
        let Some((value, caret)) = caper_client::emoji::insert(&text, &token, &spaced) else {
            return false;
        };
        buffer.set_text(&value);
        buffer.place_cursor(&buffer.iter_at_offset(caret as i32));
        composer.view.grab_focus();
        true
    }

    /// A stopped voice message waits in its composer, uploading now (so
    /// transcription starts) and going with the next Send. With "Send voice
    /// messages as soon as I stop recording" it goes on its own instead,
    /// once uploaded, with the reply shown above.
    fn keep_voice(self: &Rc<Self>, origin: Origin, prepared: Arc<Prepared>) {
        let send_on_stop = self.voice_send_on_stop();
        {
            let mut state = self.state.borrow_mut();
            let (Some(token), Some(settings)) = (state.token.clone(), state.uploads.clone()) else {
                return;
            };
            state.file_keys += 1;
            let thread = origin.root.is_some();
            let mut file =
                DraftFile::recorded(state.file_keys, origin.channel, origin.root, prepared);
            if file.upload.restart().is_some() {
                self.worker.send(file.command(token, settings));
            }
            if send_on_stop {
                let reply = match (&state.compose, thread) {
                    (Some(super::Compose::Reply { id, .. }), false) => Some(id.clone()),
                    _ => None,
                };
                if reply.is_some() {
                    state.compose = None;
                }
                state.voice_sends.push(VoiceSend { file, reply });
            } else {
                state.files.push(file);
            }
            state.files_revision += 1;
        }
        self.render();
    }

    /// "Send voice messages as soon as I stop recording", per device.
    pub(super) fn voice_send_on_stop(&self) -> bool {
        self.prefs
            .get::<bool>("voice-send-on-stop-v1")
            .unwrap_or(false)
    }

    pub(super) fn set_voice_send_on_stop(&self, enabled: bool) {
        self.prefs.set("voice-send-on-stop-v1", enabled);
    }

    /// Sends the oldest uploaded voice message once the conversation has no
    /// other send in flight: on its own, so the composer's text and files
    /// stay where they are.
    pub(super) fn send_voice_messages(&self, state: &mut State) {
        if state.pending.is_some() {
            return;
        }
        let (Some(chat_token), Some(channel)) = (
            state.session.as_ref().map(|session| session.token.clone()),
            state.channel.clone(),
        ) else {
            return;
        };
        let Some(index) = state.voice_sends.iter().position(|send| {
            send.file.channel == channel
                && send.file.attachment.is_some()
                && send.file.error.is_none()
        }) else {
            return;
        };
        let send = state.voice_sends.remove(index);
        let Some(id) = send.file.attachment.as_ref().map(|file| file.id.clone()) else {
            return;
        };
        let client_id = uuid::Uuid::new_v4().to_string();
        let thread_root_id = send.file.thread_root_id.clone();
        state.pending = Some(super::Pending {
            client_id: client_id.clone(),
            text: String::new(),
            created_at: chrono::Local::now().to_rfc3339(),
            thread: thread_root_id.clone(),
            files: vec![send.file],
            gif: None,
            share: None,
        });
        state.files_revision += 1;
        self.worker.send(Command::Send {
            generation: state.generation,
            token: state.token.clone(),
            chat_token,
            channel,
            client_id,
            text: String::new(),
            thread_root_id,
            broadcast: false,
            reply_to_id: send.reply,
            attachment_ids: vec![id],
            spoiler_attachment_ids: Vec::new(),
            gif: None,
            suppress_embeds: false,
        });
    }

    /// Drops voice messages sending outside `channel`, the conversation
    /// being opened.
    pub(super) fn discard_voice_sends(&self, state: &mut State, channel: Option<&str>) {
        let (kept, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut state.voice_sends)
            .into_iter()
            .partition(|send| Some(send.file.channel.as_str()) == channel);
        state.voice_sends = kept;
        self.cancel_voice_sends(state, gone);
    }

    /// Stops their uploads and gives back what was reserved.
    fn cancel_voice_sends(&self, state: &State, sends: Vec<VoiceSend>) {
        let ids: Vec<String> = sends
            .iter()
            .inspect(|send| send.file.stop())
            .filter_map(|send| send.file.attachment.as_ref().map(|file| file.id.clone()))
            .collect();
        if let Some(token) = state.token.clone()
            && !ids.is_empty()
        {
            self.worker.send(Command::CancelAssets { token, ids });
        }
    }

    /// Retry (`retry`), or Cancel and Discard, for a voice message's send.
    fn voice_send_action(self: &Rc<Self>, key: u64, retry: bool) {
        {
            let mut state = self.state.borrow_mut();
            let Some(index) = state
                .voice_sends
                .iter()
                .position(|send| send.file.key == key)
            else {
                return;
            };
            if retry {
                let (Some(token), Some(settings)) = (state.token.clone(), state.uploads.clone())
                else {
                    return;
                };
                let file = &mut state.voice_sends[index].file;
                if file.upload.restart().is_some() {
                    self.worker.send(file.command(token, settings));
                }
            } else {
                let send = state.voice_sends.remove(index);
                self.cancel_voice_sends(&state, vec![send]);
            }
            state.files_revision += 1;
        }
        self.render();
    }

    /// Signing out drops what was recording or transcribing.
    pub(super) fn clear_recording(&self) {
        self.recording.take();
        self.dictation.take();
    }

    /// Each composer's microphone, recording bar and dictation row.
    pub(super) fn render_recording(self: &Rc<Self>) {
        let Ok(state) = self.state.try_borrow() else {
            return;
        };
        let recording = self.recording.borrow();
        let dictation = self.dictation.borrow();
        let send_on_stop = self.voice_send_on_stop();
        for thread in [false, true] {
            let composer = if thread {
                &self.ui.thread_composer
            } else {
                &self.ui.composer
            };
            let (enabled, why) = self.mic_state(&state, thread);
            composer.mic.set_sensitive(enabled);
            composer
                .mic
                .set_tooltip_text(Some(if enabled { "Dictate" } else { why }));
            let here = recording
                .as_ref()
                .filter(|recording| state.shows(&recording.origin, thread));
            composer.line.set_visible(here.is_none());
            let bar = &composer.recording;
            bar.root.set_visible(here.is_some());
            if let Some(recording) = here {
                let started = recording.started.is_some();
                let live = started && !recording.sending;
                if live {
                    bar.dot.add_css_class("live");
                } else {
                    bar.dot.remove_css_class("live");
                }
                let timer = voice_note::duration_label(recording.elapsed().as_millis() as u64);
                if bar.timer.label() != timer {
                    bar.timer.set_label(&timer);
                }
                let note = if recording.sending && !recording.as_voice {
                    "Transcribing…"
                } else if recording.sending && send_on_stop {
                    "Sending…"
                } else if recording.sending {
                    "Adding to your message…"
                } else if !started {
                    "Starting microphone…"
                } else {
                    ""
                };
                bar.note.set_label(note);
                bar.note.set_visible(!note.is_empty());
                bar.meter.set_visible(note.is_empty());
                bar.meter.queue_draw();
                let blocked = state.voice_note_blocked(&recording.origin);
                bar.voice_note.set_sensitive(live && blocked.is_none());
                bar.voice_note
                    .set_tooltip_text(Some(blocked.unwrap_or(if send_on_stop {
                        "Stop and send as a voice message"
                    } else {
                        "Stop and keep as a voice message"
                    })));
                bar.done.set_sensitive(live);
            }
            let row = &composer.dictation;
            let shown = dictation
                .as_ref()
                .filter(|dictation| state.shows(&dictation.origin, thread));
            row.root.set_visible(shown.is_some());
            if let Some(dictation) = shown {
                let failed = dictation.error.is_some();
                row.spinner.set_visible(!failed);
                row.spinner.set_spinning(!failed);
                row.text.set_label(&match &dictation.error {
                    Some(error) => format!("Couldn’t insert your words. {error}"),
                    None => "Transcribing…".into(),
                });
                if failed {
                    row.text.add_css_class("error");
                    row.text.remove_css_class("muted");
                } else {
                    row.text.add_css_class("muted");
                    row.text.remove_css_class("error");
                }
                row.retry.set_visible(failed);
                row.keep
                    .set_visible(failed && state.voice_note_blocked(&dictation.origin).is_none());
                row.drop
                    .set_label(if failed { "Discard" } else { "Cancel" });
                row.keep.set_label(if send_on_stop {
                    "Send as voice note"
                } else {
                    "Keep as voice note"
                });
            }
            self.render_voice_sends(&state, thread);
        }
    }
}

impl App {
    /// A composer's voice messages still uploading or failed, above it.
    fn render_voice_sends(self: &Rc<Self>, state: &State, thread: bool) {
        let root = Self::composer_root(state, thread);
        let rows: Vec<_> = state
            .voice_sends
            .iter()
            .filter(|send| {
                state.channel.as_ref() == Some(&send.file.channel)
                    && send.file.thread_root_id == root
                    && thread == root.is_some()
                    && (send.file.attachment.is_none() || send.file.error.is_some())
            })
            .map(|send| {
                (
                    send.file.key,
                    (send.file.progress.clamp(0.0, 1.0) * 100.0).round() as u32,
                    send.file.error.clone(),
                )
            })
            .collect();
        let shown = format!("{rows:?}");
        if self.voice_send_rows.borrow()[usize::from(thread)] == shown {
            return;
        }
        self.voice_send_rows.borrow_mut()[usize::from(thread)] = shown;
        let sends = if thread {
            &self.ui.thread_composer.sends
        } else {
            &self.ui.composer.sends
        };
        while let Some(child) = sends.first_child() {
            sends.remove(&child);
        }
        sends.set_visible(!rows.is_empty());
        for (key, percent, error) in rows {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("dictation-row");
            let color = if error.is_some() {
                "#ff9b82"
            } else {
                "#b9bcbe"
            };
            let icon = self.images.icon_image(crate::images::Icon::Mic, color, 16);
            icon.set_valign(gtk::Align::Center);
            row.append(&icon);
            let text = gtk::Label::new(Some(&match &error {
                Some(error) => format!("Voice message not sent. {error}"),
                None => format!("Sending voice message… {percent}%"),
            }));
            text.add_css_class("small");
            text.add_css_class(if error.is_some() { "error" } else { "muted" });
            text.set_wrap(true);
            text.set_xalign(0.0);
            row.append(&text);
            let buttons: &[(&str, bool)] = if error.is_some() {
                &[("Retry", true), ("Discard", false)]
            } else {
                &[("Cancel", false)]
            };
            for &(name, retry) in buttons {
                let button = gtk::Button::with_label(name);
                button.add_css_class("small-button");
                button.set_valign(gtk::Align::Center);
                let app = Rc::downgrade(self);
                button.connect_clicked(move |_| {
                    if let Some(app) = app.upgrade() {
                        app.voice_send_action(key, retry);
                    }
                });
                row.append(&button);
            }
            sends.append(&row);
        }
    }
}

/// The recording bar's live input meter ("pinch bars"), as on the desktop:
/// the forming bar at the right edge, finished ones to its left, one 3 px
/// pill every 6 px scrolling left as levels arrive, pinching and fading over
/// the left 40 px. Quiet bars are small grey dots and loud ones brighten
/// toward eggshell.
fn draw_levels(
    cr: &cairo::Context,
    width: i32,
    height: i32,
    levels: &[f32],
    pending: f32,
    scroll: f32,
) {
    const BAR: f64 = 3.0;
    const STEP: f64 = 6.0;
    const MIN: f64 = 3.0;
    const QUIET: [f64; 3] = [150.0, 156.0, 158.0];
    const LOUD: [f64; 3] = [241.0, 237.0, 228.0];
    let (width, height) = (f64::from(width), f64::from(height));
    if width <= 0.0 {
        return;
    }
    let smoothstep = |edge0: f64, edge1: f64, x: f64| {
        let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    for (age, level) in std::iter::once(&pending)
        .chain(levels.iter().rev())
        .enumerate()
    {
        let x = width - BAR - f64::from(scroll) * STEP - age as f64 * STEP;
        if x < -STEP {
            break;
        }
        let center = x + BAR / 2.0;
        let scale = 1.0 - 0.70 * (1.0 - smoothstep(0.0, 40.0_f64.min(0.8 * width), center));
        let alpha = smoothstep(0.0, 0.10 * width, center);
        if alpha < 0.02 {
            continue;
        }
        let level = f64::from(voice_note::live_shape(*level));
        let bar_height = (MIN * scale.max(0.6)).max(level * scale * height * 0.94);
        let bar_width = BAR * (scale + 0.4).clamp(0.6, 1.0);
        let tint = (0.25 + 0.9 * level).min(1.0);
        let mix =
            |channel: usize| (QUIET[channel] + (LOUD[channel] - QUIET[channel]) * tint) / 255.0;
        cr.set_source_rgba(mix(0), mix(1), mix(2), alpha);
        let (left, top) = (center - bar_width / 2.0, height / 2.0 - bar_height / 2.0);
        let radius = bar_width.min(bar_height) / 2.0;
        cr.new_sub_path();
        cr.arc(
            left + bar_width - radius,
            top + radius,
            radius,
            -std::f64::consts::FRAC_PI_2,
            0.0,
        );
        cr.arc(
            left + bar_width - radius,
            top + bar_height - radius,
            radius,
            0.0,
            std::f64::consts::FRAC_PI_2,
        );
        cr.arc(
            left + radius,
            top + bar_height - radius,
            radius,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
        );
        cr.arc(
            left + radius,
            top + radius,
            radius,
            std::f64::consts::PI,
            1.5 * std::f64::consts::PI,
        );
        cr.close_path();
        let _ = cr.fill();
    }
}
