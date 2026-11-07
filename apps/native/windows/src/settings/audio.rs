//! The Audio test page (egui's `audio_test` dialog) on the Settings card:
//! the microphone and the speaker, each with its system default and
//! devices and its volume, Test speakers and Refresh devices, then the
//! microphone test (Voice enhancement, Test microphone with the input
//! meter, and the Natural and Enhanced samples). The shared
//! `caper_client::voice::Voice` runs the devices and tests; closing the page
//! ends them, as closing egui's dialog does. The choices persist as egui's
//! "audio-preferences-v1", and only against the production server, as
//! Launch at login.
use super::{Builder, Item, Page, SPart, Settings, Style, launch_allowed};
use crate::gfx::{self, Gfx, fill_round, rect, stroke_round};
use crate::overlay::Rect;
use crate::theme::*;
use crate::{App, EV_FOCUS, EV_SCREEN, text};
use caper_client::voice::{MicrophoneState, Preferences, Voice, VoiceOperation};
use std::time::Instant;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows::core::Result;

/// Steps the recording's time and meter, as egui's 80 ms repaint.
pub const TIMER_AUDIO: usize = 13;
const SLIDER_H: f32 = 50.0;
/// The track's centre below a slider's top, under its label and value.
const TRACK_Y: f32 = 34.0;
const MIC_BUTTON_W: f32 = 160.0;
const MIC_BUTTON_H: f32 = 44.0;
/// "Input level" and the meter's dots.
const METER_H: f32 = 56.0;
const SAMPLE_PAD: f32 = 14.0;
/// The meter's lit dots and the latest sample's border, as egui's.
const LIT: u32 = 0x91AB78;
const LATEST: u32 = 0x53633F;

/// The microphone test's step.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mic {
    #[default]
    Idle,
    Preparing,
    Recording,
    /// A changed volume or enhancement prepares a new comparison.
    Processing,
    Ready,
    /// Playing the natural (false) or enhanced sample.
    Playing(bool),
}

/// What the page shows (`snapshot`).
#[derive(Clone, Default, PartialEq, Debug)]
pub struct AudioSnap {
    /// Each picker's choices, the system default first, and the chosen one
    /// (None: the saved device isn't connected).
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub input: Option<usize>,
    pub output: Option<usize>,
    pub refreshing: bool,
    pub device_error: Option<String>,
    pub input_percent: u16,
    pub master_percent: u16,
    pub strength: u8,
    pub speaker_testing: bool,
    pub speaker_failed: bool,
    pub mic: Mic,
    /// The recording's time so far, as "3.2s".
    pub elapsed: Option<String>,
    /// The recorded sample's length and whether nothing was heard.
    pub recorded: Option<(String, bool)>,
    pub mic_error: Option<String>,
    /// The input meter's bars, oldest first, each 0–8.
    pub meter: Vec<u8>,
}

fn default_label(name: &Option<String>, refreshing: bool) -> String {
    match name {
        Some(name) => format!("System default \u{2014} {name}"),
        None if refreshing => "System default \u{2014} finding device\u{2026}".into(),
        None => "System default \u{2014} name unavailable".into(),
    }
}

pub fn snapshot(v: &Voice) -> AudioSnap {
    let choices = |default: &Option<String>, list: &[(String, String)]| {
        std::iter::once(default_label(default, v.refreshing_devices))
            .chain(list.iter().map(|(_, name)| name.clone()))
            .collect()
    };
    let chosen = |saved: &Option<String>, list: &[(String, String)]| match saved {
        None => Some(0),
        Some(id) => list.iter().position(|(guid, _)| guid == id).map(|k| k + 1),
    };
    let (mic, recorded, elapsed) = match &v.microphone {
        MicrophoneState::Idle => (Mic::Idle, None, None),
        MicrophoneState::Preparing => (Mic::Preparing, None, None),
        MicrophoneState::Recording(started) => (
            Mic::Recording,
            None,
            Some(format!("{:.1}s", started.elapsed().as_secs_f32().min(30.0))),
        ),
        MicrophoneState::Processing(r) => (Mic::Processing, Some(*r), None),
        MicrophoneState::Ready(r) => (Mic::Ready, Some(*r), None),
        MicrophoneState::Playing { recorded, enhanced } => {
            (Mic::Playing(*enhanced), Some(*recorded), None)
        }
    };
    let p = &v.preferences;
    AudioSnap {
        inputs: choices(&v.default_input, &v.inputs),
        outputs: choices(&v.default_output, &v.outputs),
        input: chosen(&p.input, &v.inputs),
        output: chosen(&p.output, &v.outputs),
        refreshing: v.refreshing_devices,
        device_error: v.device_error.clone(),
        input_percent: p.input_percent,
        master_percent: p.master_percent,
        strength: p.processing_strength,
        speaker_testing: v.speaker_testing,
        speaker_failed: v.speaker_failed,
        mic,
        elapsed,
        recorded: recorded.map(|r| (format!("{:.1}s", r.seconds), r.silent)),
        mic_error: v.microphone_error.clone(),
        meter: v
            .input_meter
            .iter()
            .map(|level| (level * 8.0).round().clamp(0.0, 8.0) as u8)
            .collect(),
    }
}

fn preferences_path() -> Option<std::path::PathBuf> {
    if !launch_allowed() {
        return None;
    }
    let base = std::path::PathBuf::from(std::env::var_os("APPDATA")?);
    Some(
        base.join("Caper")
            .join("windows")
            .join("audio-preferences-v1.json"),
    )
}

/// The saved devices and volumes, as egui restores them.
pub fn restore_preferences() -> Option<Preferences> {
    let json = std::fs::read_to_string(preferences_path()?).ok()?;
    Some(Preferences::restore(&json))
}

fn save_preferences(preferences: &Preferences) {
    let Some(path) = preferences_path() else {
        return;
    };
    let saved = serde_json::to_string(preferences).map_err(std::io::Error::other);
    let result = saved.and_then(|json| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, json)
    });
    if let Err(e) = result {
        eprintln!("saving audio preferences failed: {e}");
    }
}

/// A slider's track ends inside its rectangle, clear of the knob.
fn track(x: f32, w: f32) -> (f32, f32) {
    (x + 8.0, x + w - 8.0)
}

impl Builder<'_> {
    /// The page, as egui's `audio_test` stacks it (its two pickers side by
    /// side become two lists here).
    pub(super) fn audio(&mut self, s: &AudioSnap) -> Result<()> {
        self.text("Only you can hear these tests.", Style::Detail)?;
        self.y += 14.0;
        self.device(s, true)?;
        self.y += 18.0;
        self.device(s, false)?;
        if let Some(e) = &s.device_error {
            self.y += 6.0;
            self.text(e, Style::Error)?;
        }
        self.y += 10.0;
        self.button(SPart::RefreshDevices, "Refresh devices")?;
        self.separator();
        self.microphone(s)
    }

    fn device(&mut self, s: &AudioSnap, input: bool) -> Result<()> {
        let (kind, choices, chosen) = if input {
            ("Microphone", &s.inputs, s.input)
        } else {
            ("Speaker", &s.outputs, s.output)
        };
        self.text(kind, Style::Section)?;
        self.y += 4.0;
        if s.mic != Mic::Idle {
            self.text("End the microphone test to change devices.", Style::Detail)?;
        }
        for (k, name) in choices.iter().enumerate() {
            let part = if input {
                SPart::Input(k)
            } else {
                SPart::Output(k)
            };
            self.row(part, name);
        }
        if chosen.is_none() {
            self.text("Saved device (unavailable)", Style::Detail)?;
        }
        if s.refreshing {
            self.text("Finding devices\u{2026}", Style::Detail)?;
        } else if choices.len() <= 1 {
            self.text(
                "No devices found. Check system audio settings.",
                Style::Detail,
            )?;
        }
        self.y += 10.0;
        if input {
            self.slider(SPart::MicVolume, "Microphone volume");
            return Ok(());
        }
        self.slider(SPart::SpeakerVolume, "Speaker volume");
        self.y += 6.0;
        let test = if s.speaker_testing {
            "Stop speaker test"
        } else {
            "Test speakers"
        };
        self.button(SPart::SpeakerTest, test)?;
        if s.speaker_failed {
            self.text(
                "Couldn\u{2019}t play audio. Check your output and try again.",
                Style::Error,
            )?;
        }
        Ok(())
    }

    fn slider(&mut self, part: SPart, label: &str) {
        self.items.push(Item {
            part,
            rect: (0.0, self.y, self.inner, SLIDER_H),
            text: label.into(),
            style: Style::Label,
            initials: String::new(),
        });
        self.y += SLIDER_H;
    }

    /// A line of text at `rect` (body-local), outside the stack.
    fn place(&mut self, text: &str, style: Style, rect: Rect) {
        self.items.push(Item {
            part: SPart::Text(self.texts),
            rect,
            text: text.into(),
            style,
            initials: String::new(),
        });
        self.texts += 1;
    }

    fn microphone(&mut self, s: &AudioSnap) -> Result<()> {
        let top = self.y;
        let recording = s.mic == Mic::Recording;
        let heading = if recording {
            "Recording\u{2026}"
        } else {
            "Try your microphone"
        };
        self.text(heading, Style::Title)?;
        if let Some(t) = &s.elapsed {
            let (w, h) = (
                self.width(t, Style::Title)?,
                self.measure(t, Style::Title, 1e6)?,
            );
            self.place(t, Style::Title, (self.inner - w, top, w, h));
        }
        self.y += 6.0;
        self.text("Less noise. Clearer voice.", Style::Detail)?;
        self.y += 16.0;
        // Voice enhancement in its box, Natural to Enhanced.
        let box_top = self.y;
        self.y += 12.0;
        self.slider(SPart::Enhancement, "Voice enhancement");
        let h = self.measure("Natural", Style::Detail, 1e6)?;
        let w = self.width("Enhanced", Style::Detail)?;
        let natural = self.width("Natural", Style::Detail)?;
        self.place("Natural", Style::Detail, (0.0, self.y, natural, h));
        self.place("Enhanced", Style::Detail, (self.inner - w, self.y, w, h));
        self.y += h + 12.0;
        self.frames.push((
            (-12.0, box_top, self.inner + 24.0, self.y - box_top),
            BORDER,
        ));
        self.y += 18.0;
        // Test microphone, the input level beside it.
        let label = match s.mic {
            Mic::Recording => "Stop recording",
            Mic::Preparing | Mic::Processing => "Preparing\u{2026}",
            _ => "Test microphone",
        };
        let button_y = self.y + (METER_H - MIC_BUTTON_H) / 2.0;
        self.items.push(Item {
            part: SPart::MicTest,
            rect: (0.0, button_y, MIC_BUTTON_W, MIC_BUTTON_H),
            text: label.into(),
            style: Style::Label,
            initials: String::new(),
        });
        let meter = if recording {
            "Received microphone level"
        } else {
            "Microphone level inactive"
        };
        let mx = MIC_BUTTON_W + 16.0;
        self.items.push(Item {
            part: SPart::Meter,
            rect: (mx, self.y, (self.inner - mx).max(40.0), METER_H),
            text: meter.into(),
            style: Style::Label,
            initials: String::new(),
        });
        self.y += METER_H + 8.0;
        if let Some(e) = &s.mic_error {
            self.text(e, Style::Error)?;
        }
        let Some((seconds, silent)) = &s.recorded else {
            return Ok(());
        };
        // The samples sit side by side when they fit, as egui's columns.
        self.y += 8.0;
        let side = self.inner >= 500.0;
        let w = if side {
            (self.inner - 10.0) / 2.0
        } else {
            self.inner
        };
        let top = self.y;
        let mut cards = Vec::new();
        for enhanced in [false, true] {
            let (x, y) = match cards.last() {
                Some(&(_, _, h)) if !side => (0.0, top + h + 10.0),
                _ if side && enhanced => (w + 10.0, top),
                _ => (0.0, top),
            };
            let h = self.sample(s, enhanced, seconds, *silent, (x, y, w))?;
            cards.push((x, y, h));
        }
        let tallest = cards.iter().map(|c| c.2).fold(0.0, f32::max);
        let latest = s.mic != Mic::Processing;
        for (k, (x, y, h)) in cards.into_iter().enumerate() {
            let border = if k == 1 && latest { LATEST } else { BORDER };
            let h = if side { tallest } else { h };
            self.frames.push(((x, y, w, h), border));
            self.y = self.y.max(y + h);
        }
        Ok(())
    }

    /// One recorded sample's card at `x, y`, `w` wide; its height.
    fn sample(
        &mut self,
        s: &AudioSnap,
        enhanced: bool,
        seconds: &str,
        silent: bool,
        (x, y, w): (f32, f32, f32),
    ) -> Result<f32> {
        let (ix, iw) = (x + SAMPLE_PAD, w - 2.0 * SAMPLE_PAD);
        let name = if enhanced { "Enhanced" } else { "Natural" };
        let nh = self.measure(name, Style::Title, iw)?;
        self.place(name, Style::Title, (ix, y + SAMPLE_PAD, iw, nh));
        let mut cy = y + SAMPLE_PAD + nh + 10.0;
        if enhanced && s.mic == Mic::Processing {
            let h = self.measure("Preparing\u{2026}", Style::Detail, iw)?;
            self.place("Preparing\u{2026}", Style::Detail, (ix, cy, iw, h));
            return Ok(cy + h + SAMPLE_PAD - y);
        }
        let this = s.mic == Mic::Playing(enhanced);
        let verb = if this { "Stop" } else { "Play" };
        self.items.push(Item {
            part: SPart::Sample(enhanced),
            rect: (ix, cy, 64.0, 30.0),
            text: format!("{verb} {name} audio sample"),
            style: Style::Label,
            initials: String::new(),
        });
        let (sw, sh) = (
            self.width(seconds, Style::Detail)?,
            self.measure(seconds, Style::Detail, 1e6)?,
        );
        self.place(
            seconds,
            Style::Detail,
            (ix + 76.0, cy + (30.0 - sh) / 2.0, sw, sh),
        );
        cy += 30.0;
        if silent {
            let warning = "No audible signal detected. Check your mic and try again.";
            let h = self.measure(warning, Style::Error, iw)?;
            cy += 8.0;
            self.place(warning, Style::Error, (ix, cy, iw, h));
            cy += h;
        }
        Ok(cy + SAMPLE_PAD - y)
    }
}

impl Settings {
    pub fn audio() -> Self {
        Self {
            page: Page::Audio,
            ..Self::new(false, None)
        }
    }

    /// A slider's value, maximum and default.
    pub fn gauge(&self, part: SPart) -> Option<(u16, u16, u16)> {
        let a = self.snap.audio.as_ref()?;
        Some(match part {
            SPart::MicVolume => (a.input_percent, 200, 100),
            SPart::SpeakerVolume => (a.master_percent, 200, 100),
            SPart::Enhancement => (u16::from(a.strength), 100, 25),
            _ => return None,
        })
    }

    /// The value under `px` on a slider's track, drawn to its default
    /// within 2% of the range as egui's `default_slider` does while dragged.
    pub fn gauge_at(&self, part: SPart, px: f32) -> Option<u16> {
        let (x, _, w, _) = self.part_rect(part)?;
        let (_, max, home) = self.gauge(part)?;
        let (left, right) = track(x, w);
        let max = f32::from(max);
        let v = ((px - left) / (right - left)).clamp(0.0, 1.0) * max;
        Some(if (v - f32::from(home)).abs() <= max * 0.02 {
            home
        } else {
            v.round() as u16
        })
    }

    pub(super) fn draw_slider(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        item: &Item,
        (x, y, w, _): Rect,
    ) -> Result<()> {
        let Some((value, max, home)) = self.gauge(item.part) else {
            return Ok(());
        };
        let enabled = self.enabled(item.part);
        let lit = enabled && (self.hover == Some(item.part) || self.pressed == Some(item.part));
        let label = g.layout(&gfx::wide(&item.text), &g.formats.dialog_note, w * 0.7)?;
        let brush = if enabled {
            &g.brushes.text
        } else {
            &g.brushes.muted
        };
        text::draw(dc, &label, x, y, brush);
        let shown = format!("{value}%");
        let v = g.layout(&gfx::wide(&shown), &g.formats.dialog_note, 1e6)?;
        let vw = gfx::metrics(&v).widthIncludingTrailingWhitespace;
        let accent = if item.part == SPart::Enhancement && enabled {
            g.solid(TERRACOTTA_BRIGHT)
        } else {
            g.brushes.muted.clone()
        };
        text::draw(dc, &v, x + w - vw, y, &accent);
        let (left, right) = track(x, w);
        let cy = y + TRACK_Y;
        let span = right - left;
        fill_round(
            dc,
            rect(left, cy - 2.0, right, cy + 2.0),
            2.0,
            &g.brushes.border,
        );
        let at = left + span * f32::from(value) / f32::from(max);
        let fill = if enabled { TERRACOTTA } else { TEXT_MUTED };
        fill_round(dc, rect(left, cy - 2.0, at, cy + 2.0), 2.0, &g.solid(fill));
        // The default's mark, where the knob is drawn to.
        let hx = left + span * f32::from(home) / f32::from(max);
        unsafe {
            dc.FillRectangle(
                &rect(hx - 0.5, cy - 7.0, hx + 0.5, cy + 7.0),
                &g.brushes.muted,
            );
            let knob = D2D1_ELLIPSE {
                point: windows_numerics::Vector2 { X: at, Y: cy },
                radiusX: 8.0,
                radiusY: 8.0,
            };
            dc.FillEllipse(&knob, &g.solid(if enabled { TEXT } else { TEXT_MUTED }));
            if lit {
                dc.DrawEllipse(&knob, &g.solid(TERRACOTTA_BRIGHT), 2.0, None);
            }
        }
        Ok(())
    }

    /// "Input level" over egui's dotted bars, faded at the oldest end.
    pub(super) fn draw_meter(
        &self,
        g: &Gfx,
        dc: &ID2D1DeviceContext,
        (x, y, w, h): Rect,
    ) -> Result<()> {
        let Some(a) = &self.snap.audio else {
            return Ok(());
        };
        let l = g.layout(&gfx::wide("Input level"), &g.formats.direct_note, w)?;
        text::draw(dc, &l, x, y, &g.brushes.muted);
        let top = y + 18.0;
        let mid = top + (h - 18.0) / 2.0;
        let active = a.mic == Mic::Recording;
        let step = w / a.meter.len().max(1) as f32;
        let brush = unsafe { dc.CreateSolidColorBrush(&gfx::color(BORDER, 1.0), None)? };
        for (k, &level) in a.meter.iter().enumerate() {
            let cx = x + step * (k as f32 + 0.5);
            let fade = ((cx - x) / (w * 0.65)).min(1.0);
            let dots = ((3.0 + f32::from(level) * 4.0) / 4.0).ceil() as usize;
            unsafe { brush.SetColor(&gfx::color(if active { LIT } else { BORDER }, fade)) };
            for d in 0..dots {
                let offset = (d as f32 - (dots as f32 - 1.0) / 2.0) * 4.0;
                let e = D2D1_ELLIPSE {
                    point: windows_numerics::Vector2 {
                        X: cx,
                        Y: mid + offset,
                    },
                    radiusX: 1.4,
                    radiusY: 1.4,
                };
                unsafe { dc.FillEllipse(&e, &brush) };
            }
        }
        Ok(())
    }

    pub(super) fn draw_frames(&self, g: &Gfx, dc: &ID2D1DeviceContext) {
        for &(r, border) in &self.frames {
            let (x, y, w, h) = self.client(r);
            let r = rect(x, y, x + w, y + h);
            fill_round(dc, r, 8.0, &g.solid(BLACKOUT));
            stroke_round(dc, r, 8.0, &g.solid(border));
        }
    }

    /// A shown sample's button reads Play or Stop.
    pub(super) fn sample_label(&self, enhanced: bool) -> &'static str {
        let playing = self
            .snap
            .audio
            .as_ref()
            .is_some_and(|a| a.mic == Mic::Playing(enhanced));
        if playing { "Stop" } else { "Play" }
    }
}

impl App {
    /// Opens the Audio test (egui's account menu item): it finds the
    /// devices again.
    pub fn open_audio(&mut self) {
        self.close_settings();
        self.account_menu = None;
        self.close_dm_menu();
        self.close_menu();
        self.close_dialog();
        self.close_gifs(false);
        self.set_composer_focus(false);
        self.settings = Some(Settings::audio());
        if let Some(live) = &mut self.live {
            live.voice.refresh_devices();
        }
        self.settings_changed();
        self.events |= EV_SCREEN | EV_FOCUS;
    }

    /// The page closed: its tests end and its choices are kept.
    pub(super) fn audio_closed(&mut self) {
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_AUDIO);
        }
        if let Some(live) = &mut self.live {
            if !matches!(live.voice.microphone, MicrophoneState::Idle) {
                live.voice.stop_mic_test();
            }
            if live.voice.speaker_testing {
                live.voice.stop_speaker_test();
            }
            save_preferences(&live.voice.preferences);
        }
    }

    /// While recording, the meter takes a level and the time steps.
    pub fn audio_tick(&mut self) {
        if let Some(live) = &mut self.live {
            live.voice.sample_input_meter(Instant::now());
        }
        self.audio_changed();
    }

    /// Voice changed: the open page follows when what it shows did.
    pub fn audio_changed(&mut self) {
        if !self
            .settings
            .as_ref()
            .is_some_and(|s| s.page == Page::Audio)
        {
            return;
        }
        let snap = self.settings_snapshot();
        if self.settings.as_ref().is_some_and(|s| s.snap != snap) {
            self.settings_changed();
        }
    }

    /// The 80 ms step runs only while recording.
    pub(super) fn arm_audio_timer(&self, recording: bool) {
        unsafe {
            if recording {
                SetTimer(Some(self.hwnd), TIMER_AUDIO, 80, None);
            } else {
                let _ = KillTimer(Some(self.hwnd), TIMER_AUDIO);
            }
        }
    }

    /// Moves a slider (a drag, or UI Automation's RangeValue); `done` keeps
    /// the choice once the drag ends.
    pub fn set_gauge(&mut self, part: SPart, value: u16, done: bool) {
        if !self.settings.as_ref().is_some_and(|s| s.enabled(part)) {
            return;
        }
        let Some(live) = &mut self.live else { return };
        let v = &mut live.voice;
        let (input, strength) = (
            v.preferences.input_percent,
            v.preferences.processing_strength,
        );
        match part {
            SPart::MicVolume if value.min(200) != input => {
                v.set_input_processing(value, strength);
                live.effects.slider(f32::from(value.min(200)) / 200.0);
            }
            SPart::SpeakerVolume if value.min(200) != v.preferences.master_percent => {
                v.set_master_gain(value);
                live.effects.slider(f32::from(value.min(200)) / 200.0);
            }
            SPart::Enhancement if value.min(100) != u16::from(strength) => {
                v.set_input_processing(input, value.min(100) as u8);
                live.effects.slider(f32::from(value.min(100)) / 100.0);
            }
            _ => {}
        }
        if done {
            save_preferences(&live.voice.preferences);
        }
        self.settings_changed();
    }

    /// Clicks, Enter and UIA Invoke on the page's choices and buttons.
    pub(super) fn audio_activate(&mut self, part: SPart) {
        let Some(live) = &mut self.live else { return };
        let v = &mut live.voice;
        match part {
            SPart::Input(0) => v.command(VoiceOperation::DefaultInput),
            SPart::Output(0) => v.command(VoiceOperation::DefaultOutput),
            SPart::Input(k) => {
                if let Some((id, _)) = v.inputs.get(k - 1).cloned() {
                    v.command(VoiceOperation::Input(id));
                }
            }
            SPart::Output(k) => {
                if let Some((id, _)) = v.outputs.get(k - 1).cloned() {
                    v.command(VoiceOperation::Output(id));
                }
            }
            SPart::RefreshDevices => v.refresh_devices(),
            SPart::SpeakerTest if v.speaker_testing => v.stop_speaker_test(),
            SPart::SpeakerTest => v.start_speaker_test(),
            SPart::MicTest if matches!(v.microphone, MicrophoneState::Recording(_)) => {
                v.finish_mic_recording()
            }
            SPart::MicTest => v.start_mic_test(),
            SPart::Sample(enhanced) => match v.microphone {
                MicrophoneState::Playing { enhanced: e, .. } if e == enhanced => {
                    v.stop_mic_playback()
                }
                _ => v.play_mic_sample(enhanced),
            },
            _ => {}
        }
        if matches!(part, SPart::Input(_) | SPart::Output(_)) {
            save_preferences(&live.voice.preferences);
        }
        self.settings_changed();
    }
}
