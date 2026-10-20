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
use std::path::Path;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

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
/// The live meter: one bar every 80 ms, as the Audio test's input meter.
pub const LIVE_INTERVAL: Duration = Duration::from_millis(80);

pub const ACCESS_NEEDED: &str = "Microphone access is needed to record voice messages.";
pub const NO_MICROPHONE: &str =
    "No microphone is available. Connect one or choose another input in Audio settings.";

/// The peak of each of `bars` equal slices of `samples`, scaled linearly so
/// the loudest slice is 255. Silence (or no audio) is all zeros.
pub fn peaks(samples: &[i16], bars: usize) -> Vec<u8> {
    if samples.is_empty() || bars == 0 {
        return vec![0; bars];
    }
    let raw: Vec<u32> = (0..bars)
        .map(|bar| {
            let start = bar * samples.len() / bars;
            let end = ((bar + 1) * samples.len() / bars).max(start + 1);
            samples[start.min(samples.len() - 1)..end.min(samples.len())]
                .iter()
                .map(|sample| u32::from(sample.unsigned_abs()))
                .max()
                .unwrap_or(0)
        })
        .collect();
    let loudest = raw.iter().copied().max().unwrap_or(0);
    if loudest == 0 {
        return vec![0; bars];
    }
    raw.iter()
        .map(|peak| ((peak * 255 + loudest / 2) / loudest) as u8)
        .collect()
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

/// `count` bar heights (0–1) from any number of peaks: neighbouring peaks
/// merge by their maximum, and fewer peaks than bars repeat.
pub fn resample(peaks: &[u8], count: usize) -> Vec<f32> {
    if peaks.is_empty() {
        return vec![0.0; count];
    }
    (0..count)
        .map(|bar| {
            let start = bar * peaks.len() / count;
            let end = ((bar + 1) * peaks.len() / count).max(start + 1);
            f32::from(
                peaks[start..end.min(peaks.len())]
                    .iter()
                    .copied()
                    .max()
                    .unwrap_or(0),
            ) / 255.0
        })
        .collect()
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

/// The live meter's height for one RMS reading (the Audio test's scaling).
pub fn live_level(rms: f32) -> f32 {
    (rms.max(0.0).sqrt() * 2.5).min(1.0)
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
    let waveform = encode_base64(&peaks(samples, WAVEFORM_BARS));
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
    fn peaks_are_slice_maxima_normalised_to_the_loudest() {
        let mut samples = vec![0_i16; 6400];
        samples[50] = 1000;
        samples[150] = -2000;
        samples[6399] = 4000;
        let slices = peaks(&samples, WAVEFORM_BARS);
        assert_eq!(slices.len(), 64);
        assert_eq!(slices[0], 64, "1000 of 4000 is a quarter of 255");
        assert_eq!(slices[1], 128, "negative peaks count by magnitude");
        assert_eq!(slices[63], 255, "the loudest slice is 255");
        assert!(slices[2..63].iter().all(|peak| *peak == 0));
        assert_eq!(peaks(&[], 64), vec![0; 64]);
        assert_eq!(peaks(&[0; 100], 64), vec![0; 64], "silence stays flat");
        // Fewer samples than bars still fill every slice.
        let short = peaks(&[i16::MIN, 10, i16::MAX], 64);
        assert_eq!(short.len(), 64);
        assert_eq!(short[0], 255);
        assert_eq!(short[63], 255);
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
        let peaks = [0_u8, 255, 51, 102];
        assert_eq!(resample(&peaks, 2), vec![1.0, 0.4]);
        assert_eq!(resample(&peaks, 4), vec![0.0, 1.0, 0.2, 0.4]);
        assert_eq!(
            resample(&peaks, 8),
            vec![0.0, 0.0, 1.0, 1.0, 0.2, 0.2, 0.4, 0.4]
        );
        let many: Vec<u8> = (0..=255).collect();
        let rising = resample(&many, 40);
        assert_eq!(rising.len(), 40);
        assert_eq!(rising[39], 1.0);
        assert!(rising.windows(2).all(|pair| pair[0] <= pair[1]));
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
    fn gain_scales_and_clamps_and_live_levels_saturate() {
        let mut samples = [1000_i16, -1000, 30_000, -30_000];
        apply_gain(&mut samples, 100);
        assert_eq!(samples, [1000, -1000, 30_000, -30_000]);
        apply_gain(&mut samples, 200);
        assert_eq!(samples, [2000, -2000, i16::MAX, i16::MIN]);
        apply_gain(&mut samples, 0);
        assert_eq!(samples, [0; 4]);
        assert_eq!(live_level(0.0), 0.0);
        assert_eq!(live_level(1.0), 1.0);
        assert!((live_level(0.04) - 0.5).abs() < 1e-6);
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
