//! Voice messages: recording one from the composer and drawing its waveform.
//!
//! Recording opens its own private capture through the same libwebrtc
//! device module the Audio test uses (`media::mic_test`), so an active
//! call's microphone and publication are left exactly as they are. The raw
//! 48 kHz mono capture never leaves memory until it is encoded to AAC in MP4
//! by the bundled FFmpeg (16 kHz WAV without it) and uploaded as a voice
//! attachment with its 64 normalised peaks; the server normalises the file
//! and transcribes it (docs/media.md, "Voice messages").

use crate::media::mic_test::{MicTest, MicTestControl};
use crate::model::AttachmentKind;
use crate::uploads::{Body, Prepared, Scratch, VoiceUpload};
use eframe::egui;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Peaks sent with every voice message.
pub const WAVEFORM_BARS: usize = 64;
/// Recording stops (and sends) here.
pub const MAX_DURATION: Duration = Duration::from_secs(10 * 60);
/// Anything shorter is discarded without uploading, like a tap.
pub const MIN_DURATION: Duration = Duration::from_secs(1);
/// The capture rate (`mic_test`).
pub const SAMPLE_RATE: u32 = 48_000;
/// The WAV fallback's rate: a third of the capture, plenty for speech.
const FALLBACK_RATE: u32 = 16_000;
/// The live meter: a new bar every 25 ms, scrolling left between them.
pub const LIVE_INTERVAL: Duration = Duration::from_millis(25);
/// How far a live bar may fall below the one before it: to 35% of it, so
/// the gaps between words show.
const LIVE_FALL: f32 = 0.65;
/// Live bars kept: far more than the widest meter shows.
const LIVE_BARS: usize = 200;
/// The quietest level drawn above the baseline, in dBFS.
const FLOOR_DB: f32 = -50.0;

pub const ACCESS_NEEDED: &str = "Microphone access is needed to record voice messages.";
pub const NO_MICROPHONE: &str =
    "No microphone is available. Connect one or choose another input in Audio settings.";

/// A level on the perceptual scale the waveforms use: dBFS in [-50, 0]
/// mapped linearly to [0, 1]. `amplitude` is RMS as a fraction of full scale.
pub fn perceptual(amplitude: f32) -> f32 {
    if amplitude <= 0.0 || !amplitude.is_finite() {
        return 0.0;
    }
    ((20.0 * amplitude.log10() - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// The uploaded waveform: the RMS of each of `bars` equal slices of the
/// whole recording on the perceptual scale, smoothed by a 3-tap moving
/// average, then normalised so the loudest slice is 255. Silence (or no
/// audio) is all zeros.
pub fn waveform(samples: &[i16], bars: usize) -> Vec<u8> {
    if samples.is_empty() || bars == 0 {
        return vec![0; bars];
    }
    let levels: Vec<f32> = (0..bars)
        .map(|bar| {
            let start = (bar * samples.len() / bars).min(samples.len() - 1);
            let end = ((bar + 1) * samples.len() / bars).clamp(start + 1, samples.len());
            let slice = &samples[start..end];
            let power = slice
                .iter()
                .map(|sample| (f64::from(*sample) / 32_768.0).powi(2))
                .sum::<f64>()
                / slice.len() as f64;
            perceptual(power.sqrt() as f32)
        })
        .collect();
    let smoothed = smooth(&levels);
    let loudest = smoothed.iter().copied().fold(0.0_f32, f32::max);
    if loudest <= 0.0 {
        return vec![0; bars];
    }
    smoothed
        .iter()
        .map(|level| (level / loudest * 255.0).round().clamp(0.0, 255.0) as u8)
        .collect()
}

/// A 3-tap moving average; the ends average the neighbours they have.
pub fn smooth(levels: &[f32]) -> Vec<f32> {
    (0..levels.len())
        .map(|index| {
            let window = &levels[index.saturating_sub(1)..(index + 2).min(levels.len())];
            window.iter().sum::<f32>() / window.len() as f32
        })
        .collect()
}

/// The live recording meter's bars: each is the loudest level read during
/// its `LIVE_INTERVAL`, never below 35% of the bar before it, so syllables
/// stand out and quiet gaps between words show.
#[derive(Default)]
pub struct LiveMeter {
    /// Perceptual levels (0–1), oldest first.
    pub bars: VecDeque<f32>,
    /// The loudest level since the newest bar.
    peak: f32,
    /// When the newest bar was due; `None` until the first level.
    due: Option<Instant>,
}

impl LiveMeter {
    /// Takes one perceptual input level and adds at most one bar, every
    /// `LIVE_INTERVAL` at a constant cadence. After a stall it restarts from
    /// `now` instead of catching up.
    pub fn sample(&mut self, level: f32, now: Instant) {
        self.peak = self.peak.max(level);
        let Some(due) = self.due else {
            // The first bar is due one interval from the first level.
            self.due = Some(now);
            return;
        };
        let since = now.saturating_duration_since(due);
        if since < LIVE_INTERVAL {
            return;
        }
        let previous = self.bars.back().copied().unwrap_or(0.0);
        let bar = self.peak.max(previous * (1.0 - LIVE_FALL));
        self.bars.push_back(bar);
        while self.bars.len() > LIVE_BARS {
            self.bars.pop_front();
        }
        self.peak = 0.0;
        self.due = Some(if since > 3 * LIVE_INTERVAL {
            now
        } else {
            due + LIVE_INTERVAL
        });
    }

    /// How far (in bar steps) the bars have scrolled since the newest one. It
    /// keeps sliding past one step while a bar is late instead of pausing, so
    /// the late bar lands where the slide has got to, up to the three steps
    /// after which the cadence restarts.
    pub fn scroll(&self, now: Instant) -> f32 {
        self.due.map_or(1.0, |due| {
            (now.saturating_duration_since(due).as_secs_f32() / LIVE_INTERVAL.as_secs_f32())
                .clamp(0.0, 3.0)
        })
    }
}

/// A live bar's drawn height (0–1): speech sits around -35 to -12 dBFS, so
/// that band is stretched to the meter's full height.
pub fn live_shape(level: f32) -> f32 {
    ((level - 0.28) / 0.47).clamp(0.0, 1.0).powf(1.2)
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn encode_base64(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            if index <= chunk.len() {
                text.push(ALPHABET[(value >> (18 - 6 * index) & 63) as usize] as char);
            } else {
                text.push('=');
            }
        }
    }
    text
}

/// Standard base64, padded or not; anything else is `None`.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let text = text.trim_end_matches('=');
    if text.len() % 4 == 1 {
        return None;
    }
    let mut bytes = Vec::with_capacity(text.len() * 3 / 4);
    let mut value = 0_u32;
    let mut bits = 0;
    for character in text.bytes() {
        let digit = ALPHABET.iter().position(|known| *known == character)? as u32;
        value = (value << 6) | digit;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((value >> bits) as u8);
        }
    }
    Some(bytes)
}

/// `count` bar heights (0–1) from any number of values, by linear
/// interpolation across the whole waveform (first bar at the first value,
/// last bar at the last).
pub fn resample(values: &[u8], count: usize) -> Vec<f32> {
    match values {
        [] => vec![0.0; count],
        [only] => vec![f32::from(*only) / 255.0; count],
        _ => (0..count)
            .map(|bar| {
                let at = if count > 1 {
                    bar as f32 * (values.len() - 1) as f32 / (count - 1) as f32
                } else {
                    (values.len() - 1) as f32 / 2.0
                };
                let low = at.floor() as usize;
                let high = (low + 1).min(values.len() - 1);
                let t = at - low as f32;
                (f32::from(values[low]) * (1.0 - t) + f32::from(values[high]) * t) / 255.0
            })
            .collect(),
    }
}

/// A message's bars: its decoded waveform, or flat (all zero, drawn at the
/// minimum height) when it has none or it does not decode.
pub fn bars(waveform: Option<&str>, count: usize) -> Vec<f32> {
    waveform
        .and_then(decode_base64)
        .filter(|peaks| !peaks.is_empty())
        .map_or_else(|| vec![0.0; count], |peaks| resample(&peaks, count))
}

/// "0:07", "10:00": minutes and seconds, as the composer timer and the
/// player read.
pub fn duration_label(ms: u64) -> String {
    crate::player::time_label(ms)
}

/// The input volume preference (0–200%) applied to the capture.
pub fn apply_gain(samples: &mut [i16], percent: u16) {
    if percent == 100 {
        return;
    }
    let gain = f32::from(percent.min(200)) / 100.0;
    for sample in samples {
        *sample = (f32::from(*sample) * gain)
            .round()
            .clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16;
    }
}

/// A mono 16-bit PCM WAV file.
pub fn wav(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

/// 48 kHz to 16 kHz by averaging each three samples (a simple low-pass).
fn downsample_by_three(samples: &[i16]) -> Vec<i16> {
    samples
        .chunks(3)
        .map(|chunk| {
            (chunk.iter().map(|&sample| i32::from(sample)).sum::<i32>() / chunk.len() as i32) as i16
        })
        .collect()
}

fn duration_ms(samples: usize) -> u64 {
    (samples as u64 * 1000 / u64::from(SAMPLE_RATE)).max(1)
}

/// The upload for a recording: AAC in MP4 with FFmpeg (mono, 48 kbps,
/// about 360 KB a minute), otherwise a 16 kHz WAV. Either way the server
/// transcodes voice messages, so every client can play every one.
pub fn prepare(samples: &[i16], ffmpeg: Option<&Path>) -> Result<Prepared, String> {
    if samples.is_empty() {
        return Err("No microphone audio was recorded.".into());
    }
    let waveform = encode_base64(&waveform(samples, WAVEFORM_BARS));
    let duration = duration_ms(samples.len());
    let (name, content_type, bytes) = match ffmpeg.map(|binary| encode_aac(binary, samples)) {
        Some(Ok(bytes)) => ("voice-message.m4a", "audio/mp4", bytes),
        // A broken or missing FFmpeg still sends the message.
        _ => (
            "voice-message.wav",
            "audio/wav",
            wav(&downsample_by_three(samples), FALLBACK_RATE),
        ),
    };
    let size = bytes.len() as u64;
    Ok(Prepared {
        name: name.into(),
        content_type: content_type.into(),
        kind: AttachmentKind::Audio,
        source_size: size,
        size,
        body: Body::Memory(bytes.into()),
        width: None,
        height: None,
        duration_ms: Some(duration),
        preview: None,
        thumbnail: None,
        voice: Some(VoiceUpload {
            waveform: Some(waveform),
        }),
    })
}

/// FFmpeg's MP4 muxer needs a seekable output, so both ends are owner-only
/// scratch files that are removed when this returns.
fn encode_aac(binary: &Path, samples: &[i16]) -> Result<Vec<u8>, String> {
    let input = Scratch::new("wav");
    std::fs::write(input.path(), wav(samples, SAMPLE_RATE)).map_err(|error| error.to_string())?;
    let output = Scratch::new("m4a");
    let status = crate::ffmpeg::command(binary)
        .args(["-loglevel", "error", "-y", "-i"])
        .arg(input.path())
        .args(["-vn", "-ac", "1", "-c:a", "aac", "-b:a", "48k"])
        .args(["-movflags", "+faststart", "-f", "mp4"])
        .arg(output.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| error.to_string())?;
    if !status.success() {
        return Err("FFmpeg could not encode the voice message".into());
    }
    let bytes = std::fs::read(output.path()).map_err(|error| error.to_string())?;
    if bytes.len() < 8 || &bytes[4..8] != b"ftyp" {
        return Err("FFmpeg wrote no MP4".into());
    }
    Ok(bytes)
}

pub enum RecorderEvent {
    /// The microphone is open: the timer starts now.
    Started,
    /// Shorter than [`MIN_DURATION`]: nothing to upload.
    TooShort,
    /// Stopped (by Send or at 10:00) and encoded, ready to upload.
    Ready(Box<Prepared>),
    Failed(String),
}

/// One recording, on its own thread. Dropping it discards the recording.
pub struct Recorder {
    control: MicTestControl,
    events: Receiver<RecorderEvent>,
}

impl Recorder {
    /// Opens `input` (`None`: the system default microphone). The input
    /// volume preference (`gain_percent`) applies to what is sent.
    pub fn start(input: Option<String>, gain_percent: u16, repaint: egui::Context) -> Self {
        let control = MicTestControl::new();
        let (sender, events) = mpsc::channel();
        let worker = control.clone();
        std::thread::spawn(move || {
            let send = |event| {
                let _ = sender.send(event);
                repaint.request_repaint();
            };
            let result = (|| -> Result<Option<Prepared>, String> {
                let runtime = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .map_err(|_| ACCESS_NEEDED.to_owned())?;
                let mut test = runtime
                    .block_on(MicTest::start(worker.clone(), input.as_deref(), None))
                    .map_err(|error| start_failure(&error).to_owned())?;
                send(RecorderEvent::Started);
                let recorded = runtime.block_on(test.record_voice_message(MAX_DURATION));
                test.stop();
                drop(test);
                let mut samples = recorded?;
                if samples.len() < (MIN_DURATION.as_millis() as usize) * SAMPLE_RATE as usize / 1000
                {
                    return Ok(None);
                }
                apply_gain(&mut samples, gain_percent);
                prepare(&samples, crate::ffmpeg::binary()).map(Some)
            })();
            // A discarded recording has dropped the receiver: nothing arrives.
            send(match result {
                Ok(Some(prepared)) => RecorderEvent::Ready(Box::new(prepared)),
                Ok(None) => RecorderEvent::TooShort,
                Err(error) => RecorderEvent::Failed(error),
            });
        });
        Self { control, events }
    }

    /// Stop and send what was recorded.
    pub fn finish(&self) {
        self.control.finish_recording();
    }

    /// RMS (0–1) of the latest captured frame.
    pub fn level(&self) -> f32 {
        self.control.input_level()
    }

    pub fn poll(&self) -> Option<RecorderEvent> {
        match self.events.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(RecorderEvent::Failed(ACCESS_NEEDED.into())),
        }
    }
}

impl Drop for Recorder {
    /// Closes the microphone at once; the thread's late result goes nowhere.
    fn drop(&mut self) {
        self.control.cancel();
    }
}

/// What the composer says when the microphone did not open.
fn start_failure(error: &str) -> &'static str {
    if error.contains("unavailable") {
        NO_MICROPHONE
    } else {
        ACCESS_NEEDED
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perceptual_levels_map_minus_fifty_to_zero_db_onto_zero_to_one() {
        assert_eq!(perceptual(1.0), 1.0);
        assert!((perceptual(0.01) - 0.2).abs() < 1e-5, "-40 dBFS");
        assert!((perceptual(0.5) - 0.8796).abs() < 1e-3, "-6 dBFS");
        assert_eq!(perceptual(0.001), 0.0, "-60 dBFS sits on the floor");
        assert_eq!(perceptual(0.0), 0.0);
        assert_eq!(perceptual(f32::NAN), 0.0);
        assert_eq!(smooth(&[0.0, 3.0, 0.0]), vec![1.5, 1.0, 1.5]);
        assert_eq!(smooth(&[3.0]), vec![3.0]);
        assert!(smooth(&[]).is_empty());
    }

    #[test]
    fn waveforms_are_smoothed_perceptual_rms_normalised_to_the_loudest() {
        // 64 slices of 100 samples: loud, quiet, then silent thirds.
        let samples: Vec<i16> = (0..6400)
            .map(|index| {
                let sign = if index % 2 == 0 { 1 } else { -1 };
                sign * match index / 100 {
                    0..=20 => 16_384,
                    21..=41 => 328,
                    _ => 0,
                }
            })
            .collect();
        let levels = waveform(&samples, WAVEFORM_BARS);
        assert_eq!(levels.len(), 64);
        assert_eq!(levels[0], 255, "the loudest slice is 255");
        assert_eq!(levels[10], 255);
        // -40 dBFS against -6 dBFS: 0.2 of 0.88 on the perceptual scale.
        assert!((55..=60).contains(&levels[31]), "{}", levels[31]);
        assert_eq!(levels[63], 0, "silence sits at the floor");
        // The 3-tap average softens each step instead of a cliff.
        assert!(levels[21] < levels[20] && levels[21] > levels[22]);
        assert!(levels[42] > 0 && levels[42] < levels[41]);
        // A lone click in a silent slice is far quieter than sustained sound.
        let mut click = vec![0_i16; 6400];
        click[50] = i16::MAX;
        click[5000..].fill(8_000);
        let levels = waveform(&click, WAVEFORM_BARS);
        assert!(levels[0] < levels[60] / 2, "{levels:?}");
        assert_eq!(waveform(&[], 64), vec![0; 64]);
        assert_eq!(waveform(&[0; 100], 64), vec![0; 64], "silence stays flat");
        assert_eq!(waveform(&[9_000; 6400], 64), vec![255; 64]);
        // Fewer samples than bars still fill every slice.
        assert_eq!(waveform(&[i16::MIN, 10, i16::MAX], 64).len(), 64);
    }

    #[test]
    fn live_bars_keep_each_peak_and_fall_quickly() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut meter = LiveMeter::default();
        meter.sample(0.2, at(0));
        assert!(
            meter.bars.is_empty(),
            "the first bar is due one interval later"
        );
        // The loudest level inside the interval wins, not the last one.
        meter.sample(0.9, at(10));
        meter.sample(0.1, at(20));
        meter.sample(0.1, at(25));
        assert_eq!(meter.bars, [0.9]);
        // Silence falls to 35% of the bar before, then on toward nothing.
        meter.sample(0.0, at(50));
        meter.sample(0.0, at(75));
        assert!(
            (meter.bars[1] - 0.9 * 0.35).abs() < 1e-6,
            "{:?}",
            meter.bars
        );
        assert!((meter.bars[2] - 0.9 * 0.35 * 0.35).abs() < 1e-6);
        // A louder level than the fall replaces it at once.
        meter.sample(0.5, at(100));
        assert_eq!(meter.bars.back(), Some(&0.5));
    }

    #[test]
    fn live_bars_keep_a_constant_cadence_and_restart_after_a_stall() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut meter = LiveMeter::default();
        meter.sample(0.5, at(0));
        // A late level still schedules the next bar from when this one was due.
        meter.sample(0.5, at(32));
        assert_eq!(meter.bars.len(), 1);
        meter.sample(0.5, at(49));
        assert_eq!(meter.bars.len(), 1);
        meter.sample(0.5, at(50));
        assert_eq!(meter.bars.len(), 2);
        assert!((meter.scroll(at(50)) - 0.0).abs() < 1e-6);
        assert!((meter.scroll(at(60)) - 0.4).abs() < 1e-3);
        assert!(
            (meter.scroll(at(90)) - 1.6).abs() < 1e-3,
            "keeps sliding while a bar is late"
        );
        assert_eq!(meter.scroll(at(500)), 3.0);
        // One bar per level even when several are overdue, then a restart.
        meter.sample(0.5, at(500));
        assert_eq!(meter.bars.len(), 3);
        meter.sample(0.5, at(510));
        assert_eq!(
            meter.bars.len(),
            3,
            "the stall restarted the cadence at 500"
        );
        meter.sample(0.5, at(525));
        assert_eq!(meter.bars.len(), 4);
        for ms in 0..1000 {
            meter.sample(0.5, at(1000 + ms * 25));
        }
        assert_eq!(meter.bars.len(), LIVE_BARS, "the history is capped");
        assert_eq!(LiveMeter::default().scroll(start), 1.0);
    }

    #[test]
    fn live_shape_stretches_speech_to_full_height() {
        assert_eq!(live_shape(0.0), 0.0);
        assert_eq!(live_shape(0.28), 0.0, "-36 dBFS and below rest as dots");
        assert!((live_shape(0.75) - 1.0).abs() < 1e-6);
        assert_eq!(live_shape(1.0), 1.0);
        let middle = live_shape(0.515);
        assert!((middle - 0.5_f32.powf(1.2)).abs() < 1e-5, "{middle}");
    }

    #[test]
    fn base64_is_standard_with_padding_and_round_trips() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(&[0, 1, 2, 3]), "AAECAw==");
        assert_eq!(encode_base64(&[251, 255]), "+/8=");
        let peaks: Vec<u8> = (0..64).map(|value| (value * 4) as u8).collect();
        let encoded = encode_base64(&peaks);
        assert_eq!(encoded.len(), 88, "64 bytes are 88 padded characters");
        assert!(encoded.ends_with('='));
        assert_eq!(decode_base64(&encoded), Some(peaks));
        assert_eq!(
            decode_base64("Zm8"),
            Some(b"fo".to_vec()),
            "padding is optional"
        );
        assert_eq!(decode_base64("Zm9v!"), None);
        assert_eq!(decode_base64("Z"), None);
        assert_eq!(
            decode_base64("Zm-_"),
            None,
            "URL-safe digits are not standard"
        );
    }

    #[test]
    fn waveforms_resample_to_any_bar_count() {
        let close = |a: Vec<f32>, b: &[f32]| {
            assert_eq!(a.len(), b.len());
            assert!(
                a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-5),
                "{a:?} != {b:?}"
            );
        };
        // Linear interpolation, ends pinned to the first and last values.
        close(resample(&[0, 255], 5), &[0.0, 0.25, 0.5, 0.75, 1.0]);
        close(resample(&[0, 255, 51], 3), &[0.0, 1.0, 0.2]);
        close(resample(&[0, 255, 51], 5), &[0.0, 0.5, 1.0, 0.6, 0.2]);
        // Fewer bars than values still follows the line between them.
        close(resample(&[0, 100, 200, 255], 2), &[0.0, 1.0]);
        close(resample(&[102], 3), &[0.4; 3]);
        close(resample(&[0, 255], 1), &[0.5]);
        let many: Vec<u8> = (0..=255).collect();
        let rising = resample(&many, 40);
        assert_eq!(rising.len(), 40);
        assert_eq!(rising[39], 1.0);
        assert!(rising.windows(2).all(|pair| pair[0] <= pair[1]));
        // No steps: neighbouring bars of a 64-value ramp at 150 bars differ
        // by at most one source step.
        let ramp: Vec<u8> = (0..64).map(|value| (value * 4) as u8).collect();
        let fine = resample(&ramp, 150);
        assert!(
            fine.windows(2)
                .all(|pair| pair[1] - pair[0] <= 4.0 / 255.0 + 1e-6)
        );
        assert_eq!(resample(&[], 3), vec![0.0; 3]);
        // A message without (or with a broken) waveform draws flat bars.
        assert_eq!(bars(None, 5), vec![0.0; 5]);
        assert_eq!(bars(Some("%%%"), 5), vec![0.0; 5]);
        assert_eq!(bars(Some("/w=="), 3), vec![1.0; 3]);
    }

    #[test]
    fn durations_read_as_minutes_and_seconds() {
        assert_eq!(duration_label(0), "0:00");
        assert_eq!(duration_label(4_200), "0:04");
        assert_eq!(duration_label(65_999), "1:05");
        assert_eq!(duration_label(MAX_DURATION.as_millis() as u64), "10:00");
        assert_eq!(duration_ms(48_000), 1000);
        assert_eq!(duration_ms(10), 1, "never zero");
    }

    #[test]
    fn gain_scales_and_clamps() {
        let mut samples = [1000_i16, -1000, 30_000, -30_000];
        apply_gain(&mut samples, 100);
        assert_eq!(samples, [1000, -1000, 30_000, -30_000]);
        apply_gain(&mut samples, 200);
        assert_eq!(samples, [2000, -2000, i16::MAX, i16::MIN]);
        apply_gain(&mut samples, 0);
        assert_eq!(samples, [0; 4]);
    }

    #[test]
    fn without_ffmpeg_a_voice_upload_is_a_16_khz_wav_with_its_waveform() {
        let samples: Vec<i16> = (0..96_000)
            .map(|index| ((index as f32 / 20.0).sin() * 8000.0) as i16)
            .collect();
        let prepared = prepare(&samples, None).unwrap();
        assert_eq!(prepared.name, "voice-message.wav");
        assert_eq!(prepared.content_type, "audio/wav");
        assert_eq!(prepared.kind, AttachmentKind::Audio);
        assert_eq!(prepared.duration_ms, Some(2000));
        assert_eq!(prepared.size, 44 + 32_000 * 2);
        let Body::Memory(bytes) = &prepared.body else {
            panic!("voice uploads stay in memory");
        };
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            16_000
        );
        let voice = prepared.voice.clone().unwrap();
        let waveform = decode_base64(voice.waveform.as_deref().unwrap()).unwrap();
        assert_eq!(waveform.len(), 64);
        assert_eq!(waveform.iter().max(), Some(&255));
        // Reserved as a voice message, never compressed or renamed here.
        let request = crate::uploads::reservation("channel", &prepared);
        assert_eq!(request["voice"], true);
        assert_eq!(request["purpose"], "attachment");
        assert_eq!(request["durationMs"], 2000);
        assert_eq!(request["contentType"], "audio/wav");
        assert_eq!(request["byteSize"], prepared.size);
        assert_eq!(request["waveform"], voice.waveform.unwrap());
        assert!(prepare(&[], None).is_err());
    }

    #[test]
    fn ordinary_reservations_carry_no_voice_fields() {
        let prepared = crate::uploads::prepare_bytes(
            "song.wav",
            wav(&[0; 480], SAMPLE_RATE),
            &crate::compress::Compression::default(),
        );
        let request = crate::uploads::reservation("channel", &prepared);
        for absent in ["voice", "waveform", "purpose"] {
            assert!(request.get(absent).is_none(), "{absent}");
        }
    }

    #[test]
    #[ignore = "needs CAPER_FFMPEG (the bundled FFmpeg)"]
    fn ffmpeg_encodes_voice_messages_to_aac_in_mp4() {
        let samples: Vec<i16> = (0..SAMPLE_RATE as usize * 3)
            .map(|index| ((index as f32 / 15.0).sin() * 12_000.0) as i16)
            .collect();
        let binary = crate::ffmpeg::binary().expect("set CAPER_FFMPEG");
        let prepared = prepare(&samples, Some(binary)).unwrap();
        assert_eq!(prepared.name, "voice-message.m4a");
        assert_eq!(prepared.content_type, "audio/mp4");
        assert_eq!(prepared.duration_ms, Some(3000));
        let Body::Memory(bytes) = &prepared.body else {
            panic!("voice uploads stay in memory");
        };
        assert_eq!(&bytes[4..8], b"ftyp");
        assert!(prepared.size < 40_000, "3 s at 48 kbps: {}", prepared.size);
        let file = Scratch::new("m4a");
        std::fs::write(file.path(), &bytes[..]).unwrap();
        // `ffmpeg::probe` wants video; read the same summary for sound alone.
        let summary = crate::ffmpeg::command(binary)
            .args(["-loglevel", "info", "-i"])
            .arg(file.path())
            .output()
            .unwrap();
        let probe = crate::ffmpeg::parse_probe(&String::from_utf8_lossy(&summary.stderr));
        assert!(probe.video.is_none());
        let audio = probe.audio.expect("an audio stream");
        assert_eq!(audio.codec, "aac");
        assert_eq!(audio.channels, 1);
        assert!(
            probe
                .duration_ms
                .is_some_and(|duration| (2900..=3100).contains(&duration))
        );
    }
}
