//! Dictation and voice messages from the composer, as on the egui desktop.
//! The microphone ("Dictate", beside GIF) replaces the composer with a
//! recording bar: Discard, a live dot and timer, the input meter, "Voice
//! note" and "Done, insert text". Done transcribes the recording (`POST
//! /api/dictation`) and inserts the words at the caret; "Voice note" keeps
//! it as a voice message instead, staged with the composer's files (it
//! uploads at once, so transcription starts, and goes with the next Send).
//! While the words come back the bar says "Transcribing…" with Cancel; a
//! failed dictation offers Retry, Keep as voice note and Discard. Escape
//! discards and Enter finishes; the recorder stops itself at 10:00. A
//! recording belongs to its conversation: leaving it discards it.
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::icons::{self, Icon};
use crate::overlay::Rect;
use crate::theme::*;
use crate::{App, EV_SCREEN};
use caper_client::uploads::{Body, Prepared};
use caper_client::voice_note::{self, LiveMeter, Recorder, RecorderEvent};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Direct2D::Common::D2D_RECT_F;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows::core::Result;

/// Reads the recorder and moves the meter while the microphone is open.
pub const TIMER_RECORDING: usize = 15;
/// The bar's height, in the composer's place.
pub const BAR_H: f32 = 48.0;
const DONE: f32 = 32.0;
const NOTE_W: f32 = 88.0;
const BUTTON_H: f32 = 28.0;

/// The bar's controls (and its status text, for UI Automation).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RPart {
    /// Discard while recording, Cancel while transcribing, Discard after a
    /// failure.
    Discard,
    Status,
    VoiceNote,
    Done,
    Retry,
    Keep,
}

impl RPart {
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn button(self) -> bool {
        self != Self::Status
    }
}

pub struct Recording {
    channel: String,
    recorder: Recorder,
    /// When the microphone opened; the timer counts from here.
    started: Option<Instant>,
    /// Done or Voice note (or 10:00): waiting for the encoded file.
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

/// A finished dictation: transcribing (`error` None), or failed with its
/// audio kept.
pub struct Dictation {
    key: u64,
    channel: String,
    prepared: Arc<Prepared>,
    error: Option<String>,
}

pub enum Rec {
    Recording(Recording),
    Dictation(Dictation),
}

impl Rec {
    fn channel(&self) -> &str {
        match self {
            Rec::Recording(r) => &r.channel,
            Rec::Dictation(d) => &d.channel,
        }
    }
}

/// The meter's bars: the forming one at the right edge, finished ones
/// scrolling left, pinching and fading over the left 40 px; quiet bars are
/// small grey dots and loud ones brighten toward eggshell (egui's
/// `paint_live_levels`).
fn live_levels(
    dc: &ID2D1DeviceContext,
    r: D2D_RECT_F,
    levels: &[f32],
    pending: f32,
    scroll: f32,
) -> Result<()> {
    const BAR: f32 = 3.0;
    const STEP: f32 = 6.0;
    const MIN: f32 = 3.0;
    let quiet = [150.0, 156.0, 158.0];
    let loud = [241.0, 237.0, 228.0];
    let width = r.right - r.left;
    let height = r.bottom - r.top;
    if width <= 0.0 {
        return Ok(());
    }
    let smooth = |e0: f32, e1: f32, v: f32| {
        let t = ((v - e0) / (e1 - e0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let brush = unsafe { dc.CreateSolidColorBrush(&gfx::color(TEXT_MUTED, 1.0), None)? };
    unsafe { dc.PushAxisAlignedClip(&r, D2D1_ANTIALIAS_MODE_ALIASED) };
    let mid = r.top + height / 2.0;
    for (age, level) in std::iter::once(&pending)
        .chain(levels.iter().rev())
        .enumerate()
    {
        let x = width - BAR - scroll * STEP - age as f32 * STEP;
        if x < -STEP {
            break;
        }
        let center = x + BAR / 2.0;
        let scale = 1.0 - 0.70 * (1.0 - smooth(0.0, 40.0_f32.min(0.8 * width), center));
        let alpha = smooth(0.0, 0.10 * width, center);
        if alpha < 0.02 {
            continue;
        }
        let level = voice_note::live_shape(*level);
        let h = (MIN * scale.max(0.6)).max(level * scale * height * 0.94);
        let w = BAR * (scale + 0.4).clamp(0.6, 1.0);
        let tint = (0.25 + 0.9 * level).min(1.0);
        let mix = |k: usize| (quiet[k] + (loud[k] - quiet[k]) * tint) / 255.0;
        unsafe {
            brush.SetColor(&D2D1_COLOR_F {
                r: mix(0),
                g: mix(1),
                b: mix(2),
                a: alpha,
            });
        }
        let cx = r.left + x + BAR / 2.0;
        fill_round(
            dc,
            rect(cx - w / 2.0, mid - h / 2.0, cx + w / 2.0, mid + h / 2.0),
            w.min(h) / 2.0,
            &brush,
        );
    }
    unsafe { dc.PopAxisAlignedClip() };
    Ok(())
}

use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;

impl App {
    /// The bar for the open conversation, while one is shown there.
    pub fn rec(&self) -> Option<&Rec> {
        self.rec
            .as_ref()
            .filter(|r| self.ids.open.as_deref() == Some(r.channel()))
    }

    /// Why the microphone can't record here, or None when it can:
    /// recordings go wherever files can be attached, one at a time.
    pub fn mic_blocked(&self) -> Option<&'static str> {
        let live = self.live.as_ref()?;
        if !live.can_attach() {
            Some(live.attach_unavailable())
        } else if matches!(self.rec, Some(Rec::Recording(_))) {
            Some("Finish the current voice message first")
        } else if matches!(self.rec, Some(Rec::Dictation(_))) {
            Some("Finish the current dictation first")
        } else if self.gifs.is_some() {
            Some("Close the GIF picker first")
        } else {
            let drafts = live.open_drafts();
            if drafts.iter().any(|d| d.upload.voice().is_some()) {
                Some("Remove the voice message to record another")
            } else if drafts.len() >= caper_client::uploads::MAX_ATTACHMENTS {
                Some("Remove a file to add a voice message")
            } else {
                None
            }
        }
    }

    /// The microphone's click: it opens and dictates, stopping any voice
    /// message that plays.
    pub fn start_recording(&mut self) {
        if self.mic_blocked().is_some() {
            return;
        }
        let (Some(channel), Some(live)) = (self.ids.open.clone(), &self.live) else {
            return;
        };
        self.status = None;
        self.playback.stop();
        let p = &live.voice.preferences;
        let recorder = Recorder::start(
            p.input.clone(),
            p.input_percent,
            p.processing_strength,
            || {},
        );
        self.rec = Some(Rec::Recording(Recording {
            channel,
            recorder,
            started: None,
            sending: false,
            as_voice: false,
            meter: LiveMeter::default(),
        }));
        self.rec_changed();
    }

    /// The bar appeared, went or changed: the composer's slot is laid out
    /// and drawn again, and the clock runs while the microphone is open.
    fn rec_changed(&mut self) {
        let recording = matches!(self.rec, Some(Rec::Recording(_)));
        unsafe {
            if recording {
                SetTimer(Some(self.hwnd), TIMER_RECORDING, 33, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_RECORDING);
            }
        }
        if self.rec().is_some() {
            self.set_composer_focus(false);
        }
        self.rec_hover = None;
        self.dirty.layout = true;
        self.dirty.composer = true;
        self.events |= EV_SCREEN;
        self.need = true;
    }

    /// Trash, Cancel, Discard or Escape: dropping the recorder closes the
    /// microphone and nothing is uploaded; a late transcript matches no key.
    pub fn discard_recording(&mut self) {
        if self.rec.take().is_some() {
            self.rec_changed();
        }
    }

    /// Done (`as_voice` false) or "Voice note": stop, then transcribe or
    /// keep once encoded.
    pub fn finish_recording(&mut self, as_voice: bool) {
        if let Some(Rec::Recording(r)) = &mut self.rec
            && r.started.is_some()
            && !r.sending
        {
            r.sending = true;
            r.as_voice |= as_voice;
            r.recorder.finish();
        }
        self.dirty.composer = true;
        self.need = true;
    }

    /// The clock's step: the recorder's news, then the live meter.
    pub fn recording_tick(&mut self) {
        loop {
            let event = match &self.rec {
                Some(Rec::Recording(r)) => r.recorder.poll(),
                _ => None,
            };
            let Some(event) = event else { break };
            self.recorder_event(event);
        }
        if let Some(Rec::Recording(r)) = &mut self.rec
            && r.started.is_some()
            && !r.sending
        {
            let level = voice_note::perceptual(r.recorder.level());
            r.meter.sample(level, Instant::now());
            // The recorder stops itself at 10:00; the message goes on as
            // Done would.
            if r.elapsed() >= voice_note::MAX_DURATION {
                r.sending = true;
            }
        }
        if self.rec().is_some() {
            self.dirty.composer = true;
            self.need = true;
        }
    }

    fn recorder_event(&mut self, event: RecorderEvent) {
        match event {
            RecorderEvent::Started => {
                if let Some(Rec::Recording(r)) = &mut self.rec {
                    r.started = Some(Instant::now());
                }
                self.dirty.composer = true;
                self.need = true;
                return;
            }
            // Under a second is a tap: nothing is uploaded.
            RecorderEvent::TooShort => self.rec = None,
            RecorderEvent::Failed(error) => {
                self.rec = None;
                self.status = Some((error, true));
                self.dirty.header = true;
            }
            RecorderEvent::Ready(prepared) => {
                let Some(Rec::Recording(r)) = self.rec.take() else {
                    return;
                };
                let prepared = Arc::new(*prepared);
                if r.as_voice {
                    self.keep_voice(prepared);
                } else {
                    self.rec = Some(Rec::Dictation(Dictation {
                        key: 0,
                        channel: r.channel,
                        prepared,
                        error: None,
                    }));
                    self.send_dictation();
                }
            }
        }
        self.rec_changed();
    }

    /// A stopped voice message waits in the composer with its files.
    fn keep_voice(&mut self, prepared: Arc<Prepared>) {
        if let Some(live) = &mut self.live
            && let Err(e) = live.stage_voice(prepared)
        {
            self.attach_error = Some(e);
        }
        self.drafts_changed();
    }

    /// Transcribes the dictation (again, for Retry).
    fn send_dictation(&mut self) {
        let Some(Rec::Dictation(d)) = &mut self.rec else {
            return;
        };
        let Body::Memory(audio) = &d.prepared.body else {
            d.error = Some(voice_note::dictation_error(Some(400)).into());
            return;
        };
        match self
            .live
            .as_mut()
            .and_then(|l| l.dictate(audio.clone(), d.prepared.content_type.clone()))
        {
            Some(key) => {
                d.key = key;
                d.error = None;
            }
            None => d.error = Some(voice_note::dictation_error(Some(401)).into()),
        }
    }

    /// The server's answer: the words go in at the composer's caret when
    /// its conversation is open.
    pub fn dictated(&mut self, key: u64, result: std::result::Result<String, Option<u16>>) {
        let Some(Rec::Dictation(d)) = &self.rec else {
            return;
        };
        if d.key != key {
            return;
        }
        let here = self.ids.open.as_deref() == Some(d.channel.as_str());
        let failure = match result {
            Err(status) => Some(voice_note::dictation_error(status).to_owned()),
            Ok(words) if words.trim().is_empty() => Some("No words heard.".to_owned()),
            Ok(_) if !here => Some("Open the conversation again to insert them.".to_owned()),
            Ok(words) => (!self.insert_dictation(&words))
                .then(|| "Messages can be at most 4,000 characters.".to_owned()),
        };
        match failure {
            Some(error) => {
                if let Some(Rec::Dictation(d)) = &mut self.rec {
                    d.error = Some(error);
                }
                self.rec_changed();
            }
            None => {
                self.rec = None;
                self.rec_changed();
                self.set_composer_focus(true);
            }
        }
    }

    /// Inserts dictated words over the composer's selection, spaced from
    /// the words around them. False when the draft would be too long.
    fn insert_dictation(&mut self, words: &str) -> bool {
        let (start, end) = self.composer.selection();
        let text = &self.composer.text;
        let before = String::from_utf16_lossy(&text[..start]);
        let after = String::from_utf16_lossy(&text[end..]);
        let spaced = voice_note::dictation_text(&before, &after, words);
        let length = before.chars().count() + spaced.chars().count() + after.chars().count();
        if length > 4000 {
            return false;
        }
        let spaced: Vec<u16> = spaced.encode_utf16().collect();
        self.composer.insert(&spaced);
        self.composer_changed();
        true
    }

    /// The bar's parts, composer-local, for a composer `bw` wide.
    pub fn rec_layout(&self, bw: f32) -> Vec<(RPart, D2D_RECT_F)> {
        let Some(rec) = self.rec() else {
            return Vec::new();
        };
        let cy = BAR_H / 2.0;
        let mut out = Vec::new();
        let width = |text: &str| {
            self.g
                .layout(&gfx::wide(text), &self.g.formats.button, 1e6)
                .map_or(60.0, |l| gfx::metrics(&l).widthIncludingTrailingWhitespace)
                + 20.0
        };
        // Text buttons from the right edge.
        let mut right = bw - 7.0;
        let mut push_right = |out: &mut Vec<_>, part, text: &str| {
            let w = width(text);
            out.push((
                part,
                rect(right - w, cy - BUTTON_H / 2.0, right, cy + BUTTON_H / 2.0),
            ));
            right -= w + 8.0;
        };
        match rec {
            Rec::Recording(_) => {
                out.push((RPart::Discard, rect(7.0, cy - 14.0, 35.0, cy + 14.0)));
                let done = rect(bw - 7.0 - DONE, cy - DONE / 2.0, bw - 7.0, cy + DONE / 2.0);
                let note = rect(
                    done.left - 8.0 - NOTE_W,
                    cy - DONE / 2.0,
                    done.left - 8.0,
                    cy + DONE / 2.0,
                );
                out.push((RPart::Status, rect(43.0, 8.0, note.left - 8.0, BAR_H - 8.0)));
                out.push((RPart::VoiceNote, note));
                out.push((RPart::Done, done));
            }
            Rec::Dictation(d) => {
                if d.error.is_some() {
                    push_right(&mut out, RPart::Discard, "Discard");
                    if self.voice_note_allowed() {
                        push_right(&mut out, RPart::Keep, "Keep as voice note");
                    }
                    push_right(&mut out, RPart::Retry, "Retry");
                } else {
                    push_right(&mut out, RPart::Discard, "Cancel");
                }
                out.push((RPart::Status, rect(14.0, 8.0, right, BAR_H - 8.0)));
            }
        }
        out
    }

    fn voice_note_allowed(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.voice_note_blocked().is_none())
    }

    /// What a part says to screen readers (and the status line's text).
    pub fn rec_label(&self, part: RPart) -> String {
        let Some(rec) = self.rec() else {
            return String::new();
        };
        match (rec, part) {
            (Rec::Recording(_), RPart::Discard) => "Discard dictation".into(),
            (Rec::Recording(r), RPart::Status) => {
                if r.started.is_none() {
                    "Starting microphone\u{2026}".into()
                } else if r.sending && !r.as_voice {
                    "Transcribing\u{2026}".into()
                } else if r.sending {
                    "Adding to your message\u{2026}".into()
                } else {
                    format!(
                        "Recording voice message, {}",
                        voice_note::duration_label(r.elapsed().as_millis() as u64)
                    )
                }
            }
            (Rec::Recording(_), RPart::VoiceNote) => "Voice note".into(),
            (Rec::Recording(_), RPart::Done) => "Done, insert text".into(),
            (Rec::Dictation(d), RPart::Status) => match &d.error {
                Some(e) => format!("Couldn\u{2019}t insert your words. {e}"),
                None => "Transcribing\u{2026}".into(),
            },
            (Rec::Dictation(d), RPart::Discard) => if d.error.is_some() {
                "Discard"
            } else {
                "Cancel"
            }
            .into(),
            (_, RPart::Retry) => "Retry".into(),
            (_, RPart::Keep) => "Keep as voice note".into(),
            _ => String::new(),
        }
    }

    pub fn rec_enabled(&self, part: RPart) -> bool {
        match (self.rec(), part) {
            (Some(Rec::Recording(r)), RPart::VoiceNote) => {
                r.started.is_some() && !r.sending && self.voice_note_allowed()
            }
            (Some(Rec::Recording(r)), RPart::Done) => r.started.is_some() && !r.sending,
            (Some(_), _) => true,
            (None, _) => false,
        }
    }

    /// The bar's parts in client DIPs.
    pub fn rec_parts(&self) -> Vec<(RPart, Rect)> {
        let (bx, by, bw, _) = self.box_rect();
        self.rec_layout(bw)
            .into_iter()
            .map(|(p, r)| {
                (
                    p,
                    (bx + r.left, by + r.top, r.right - r.left, r.bottom - r.top),
                )
            })
            .collect()
    }

    /// The bar's enabled button under (x, y).
    pub fn rec_at(&self, x: f32, y: f32) -> Option<RPart> {
        self.rec_parts()
            .into_iter()
            .find(|(p, (l, t, w, h))| {
                p.button() && self.rec_enabled(*p) && x >= *l && x < l + w && y >= *t && y < t + h
            })
            .map(|(p, _)| p)
    }

    /// A click (or UIA Invoke) on one of the bar's buttons.
    pub fn rec_activate(&mut self, part: RPart) {
        if !self.rec_enabled(part) {
            return;
        }
        match part {
            RPart::Discard => self.discard_recording(),
            RPart::VoiceNote => self.finish_recording(true),
            RPart::Done => self.finish_recording(false),
            RPart::Retry => {
                self.send_dictation();
                self.rec_changed();
            }
            RPart::Keep => {
                if let Some(Rec::Dictation(d)) = self.rec.take() {
                    self.keep_voice(d.prepared);
                }
                self.rec_changed();
            }
            RPart::Status => {}
        }
    }

    pub fn track_rec_hover(&mut self) {
        let hover = self.pointer.and_then(|(x, y)| self.rec_at(x, y));
        if hover != self.rec_hover {
            self.rec_hover = hover;
            self.dirty.composer = true;
            self.need = true;
        }
    }

    /// Escape discards and Enter finishes while the bar shows; whether the
    /// key was taken.
    pub fn rec_key(&mut self, enter: bool) -> bool {
        match self.rec() {
            Some(Rec::Recording(_)) if enter => self.finish_recording(false),
            Some(_) if !enter => self.discard_recording(),
            _ => return false,
        }
        true
    }

    /// The conversation changed: a recording made elsewhere is discarded;
    /// a dictation waits for its conversation.
    pub fn rec_follow(&mut self) {
        if let Some(Rec::Recording(r)) = &self.rec
            && self.ids.open.as_deref() != Some(r.channel.as_str())
        {
            self.discard_recording();
        } else {
            self.rec_changed();
        }
    }

    /// Draws the bar `bw` wide over the composer's slot.
    pub fn draw_rec(&self, g: &Gfx, dc: &ID2D1DeviceContext, bw: f32) -> Result<()> {
        let Some(rec) = self.rec() else {
            return Ok(());
        };
        unsafe { dc.Clear(Some(&D2D1_COLOR_F::default())) };
        let body = rect(0.5, 0.5, bw - 0.5, BAR_H - 0.5);
        fill_round(dc, body, 8.0, &g.solid(SURFACE_COMPOSER));
        stroke_round(dc, body, 8.0, &g.brushes.border);
        let parts = self.rec_layout(bw);
        let at = |p: RPart| parts.iter().find(|(q, _)| *q == p).map(|(_, r)| *r);
        let cy = BAR_H / 2.0;
        let state = |p: RPart| {
            if !self.rec_enabled(p) {
                0
            } else if self.rec_hover == Some(p) {
                2
            } else {
                1
            }
        };
        match rec {
            Rec::Recording(r) => {
                if let Some(d) = at(RPart::Discard) {
                    if self.rec_hover == Some(RPart::Discard) {
                        fill_round(dc, d, 6.0, &g.brushes.hover);
                    }
                    icons::draw(
                        dc,
                        Icon::Trash,
                        rect(d.left + 5.0, d.top + 5.0, d.right - 5.0, d.bottom - 5.0),
                        if self.rec_hover == Some(RPart::Discard) {
                            TEXT
                        } else {
                            TEXT_MUTED
                        },
                        1.0,
                    );
                }
                let live = r.started.is_some() && !r.sending;
                let dot = D2D1_ELLIPSE {
                    point: windows_numerics::Vector2 { X: 48.0, Y: cy },
                    radiusX: 4.5,
                    radiusY: 4.5,
                };
                let color = if live { TERRACOTTA_BRIGHT } else { TEXT_MUTED };
                unsafe { dc.FillEllipse(&dot, &g.solid(color)) };
                let timer = voice_note::duration_label(r.elapsed().as_millis() as u64);
                let t = g.layout(&gfx::wide(&timer), &g.formats.menu_title, 1e6)?;
                let tm = gfx::metrics(&t);
                crate::text::draw(dc, &t, 59.0, cy - tm.height / 2.0, &g.brushes.text);
                let status = at(RPart::Status).unwrap_or(rect(0.0, 0.0, 0.0, 0.0));
                let meter = rect(
                    59.0 + tm.widthIncludingTrailingWhitespace.max(30.0) + 8.0,
                    8.0,
                    status.right,
                    BAR_H - 8.0,
                );
                if live {
                    let levels: Vec<f32> = r.meter.bars.iter().copied().collect();
                    let scroll = if crate::files::motion() {
                        r.meter.scroll(Instant::now())
                    } else {
                        0.0
                    };
                    live_levels(dc, meter, &levels, r.meter.pending(), scroll)?;
                } else {
                    let note = self.rec_label(RPart::Status);
                    let l = g.layout(&gfx::wide(&note), &g.formats.direct_note, 1e6)?;
                    let m = gfx::metrics(&l);
                    crate::text::draw(dc, &l, meter.left, cy - m.height / 2.0, &g.brushes.muted);
                }
                if let Some(n) = at(RPart::VoiceNote) {
                    let s = state(RPart::VoiceNote);
                    if s == 2 {
                        fill_round(dc, n, 8.0, &g.brushes.hover);
                    }
                    stroke_round(dc, n, 8.0, &g.brushes.border);
                    let l = g.layout(&gfx::wide("Voice note"), &g.formats.menu_title, 1e6)?;
                    let m = gfx::metrics(&l);
                    let brush = if s == 0 {
                        &g.brushes.muted
                    } else {
                        &g.brushes.text
                    };
                    crate::text::draw(
                        dc,
                        &l,
                        n.left + (n.right - n.left - m.widthIncludingTrailingWhitespace) / 2.0,
                        cy - m.height / 2.0,
                        brush,
                    );
                }
                if let Some(d) = at(RPart::Done) {
                    let s = state(RPart::Done);
                    if s > 0 {
                        let fill = if s == 2 {
                            TERRACOTTA
                        } else {
                            TERRACOTTA_BRIGHT
                        };
                        fill_round(dc, d, 8.0, &g.solid(fill));
                    }
                    let k = DONE * 0.25;
                    icons::draw(
                        dc,
                        Icon::Check,
                        rect(d.left + k, d.top + k, d.right - k, d.bottom - k),
                        TEXT,
                        if s > 0 { 1.0 } else { 0.5 },
                    );
                }
            }
            Rec::Dictation(d) => {
                let status = at(RPart::Status).unwrap_or(rect(14.0, 8.0, bw - 14.0, BAR_H - 8.0));
                let text = self.rec_label(RPart::Status);
                let l = g.layout(
                    &gfx::wide(&text),
                    &g.formats.direct_note,
                    (status.right - status.left).max(1.0),
                )?;
                let m = gfx::metrics(&l);
                let brush = if d.error.is_some() {
                    &g.brushes.error
                } else {
                    &g.brushes.muted
                };
                crate::text::draw(dc, &l, status.left, cy - m.height / 2.0, brush);
                for part in [RPart::Retry, RPart::Keep, RPart::Discard] {
                    if let Some(r) = at(part) {
                        let label = self.rec_label(part);
                        crate::overlay::draw_button(
                            g,
                            dc,
                            &label,
                            (r.left, r.top, r.right - r.left, r.bottom - r.top),
                            crate::overlay::Tone::Secondary,
                            state(part),
                        )?;
                    }
                }
            }
        }
        Ok(())
    }
}
