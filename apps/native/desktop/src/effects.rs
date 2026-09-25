use rodio::source::UniformSourceIterator;
use rodio::{Decoder, OutputStreamBuilder, Sink, Source, buffer::SamplesBuffer};
use std::collections::VecDeque;
use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum Effect {
    ToggleOff,
    ToggleOn,
    Slider,
    Leave,
    Warning,
    Join,
    Message,
    Delete,
    Disconnect,
}

const WAVS: [&[u8]; 9] = [
    include_bytes!("../../../web/public/audio/effects/toggle-off.wav"),
    include_bytes!("../../../web/public/audio/effects/toggle-on.wav"),
    include_bytes!("../../../web/public/audio/effects/slider-tick.wav"),
    include_bytes!("../../../web/public/audio/effects/channel-leave.wav"),
    include_bytes!("../../../web/public/audio/effects/warning.wav"),
    include_bytes!("../../../web/public/audio/effects/channel-join.wav"),
    include_bytes!("../../../web/public/audio/effects/new-message.wav"),
    include_bytes!("../../../web/public/audio/effects/delete.wav"),
    include_bytes!("../../../web/public/audio/effects/disconnect.wav"),
];

struct Request {
    effect: Effect,
    requested: Instant,
    volume: f32,
    speed: f32,
}

impl Request {
    fn fresh(&self, now: Instant) -> bool {
        now.duration_since(self.requested) <= Duration::from_millis(120)
    }
}

pub struct Effects {
    sender: Option<SyncSender<Request>>,
    last_slider: Option<Instant>,
    cancelled: Arc<AtomicBool>,
}

impl Effects {
    pub fn new(enabled: bool) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let sender = enabled.then(|| {
            let (sender, receiver) = mpsc::sync_channel::<Request>(16);
            let cancelled = cancelled.clone();
            std::thread::spawn(move || {
                let buffers: Vec<_> = WAVS.iter().map(|wav| decode(wav)).collect();
                let mut stream = None;
                let mut active: VecDeque<Sink> = VecDeque::new();
                while let Ok(request) = receiver.recv() {
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    if !request.fresh(Instant::now()) {
                        continue;
                    }
                    if stream.is_none() {
                        stream = OutputStreamBuilder::open_default_stream().ok();
                    }
                    let Some(stream) = &stream else {
                        continue;
                    };
                    let Some(buffer) = &buffers[request.effect as usize] else {
                        continue;
                    };
                    // Opening a slow output device must not play old clicks later.
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    if !request.fresh(Instant::now()) {
                        continue;
                    }
                    active.retain(|sink| !sink.empty());
                    if active.len() == 4
                        && let Some(oldest) = active.pop_front()
                    {
                        oldest.stop();
                    }
                    let sink = Sink::connect_new(stream.mixer());
                    sink.set_volume(request.volume);
                    sink.set_speed(request.speed);
                    sink.append(buffer.clone());
                    active.push_back(sink);
                }
            });
            sender
        });
        Self {
            sender,
            last_slider: None,
            cancelled,
        }
    }

    pub fn play(&self, effect: Effect) {
        self.send(effect, 0.45, 1.0);
    }

    pub fn toggle(&self, on: bool) {
        self.play(if on {
            Effect::ToggleOn
        } else {
            Effect::ToggleOff
        });
    }

    pub fn slider(&mut self, normalized: f32) {
        let now = Instant::now();
        if self
            .last_slider
            .is_some_and(|last| now.duration_since(last) < Duration::from_millis(40))
        {
            return;
        }
        self.last_slider = Some(now);
        let value = normalized.clamp(0.0, 1.0);
        self.send(Effect::Slider, 0.1 + value * 0.22, 0.75 + value * 0.6);
    }

    fn send(&self, effect: Effect, volume: f32, speed: f32) {
        if let Some(sender) = &self.sender {
            let _ = sender.try_send(Request {
                effect,
                requested: Instant::now(),
                volume,
                speed,
            });
        }
    }
}

impl Drop for Effects {
    fn drop(&mut self) {
        // Dropping the sender wakes the worker; cancellation discards queued
        // clicks rather than playing them while the worker shuts down.
        self.cancelled.store(true, Ordering::Release);
    }
}

fn decode(wav: &'static [u8]) -> Option<SamplesBuffer> {
    let decoder = Decoder::try_from(Cursor::new(wav)).ok()?;
    let channels = decoder.channels();
    let rate = decoder.sample_rate();
    Some(SamplesBuffer::new(
        channels,
        rate,
        decoder.collect::<Vec<f32>>(),
    ))
}

/// Web's speaker test sound as 48 kHz mono PCM for the WebRTC output path.
pub fn speaker_test_pcm() -> Option<Vec<i16>> {
    let wav = WAVS[Effect::Join as usize];
    let decoder = Decoder::try_from(Cursor::new(wav)).ok()?;
    let stereo: Vec<f32> = UniformSourceIterator::new(decoder, 2, 48_000).collect();
    Some(
        stereo
            .chunks_exact(2)
            .map(|pair| ((pair[0] + pair[1]) / 2.0 * 32_767.0).clamp(-32_768.0, 32_767.0) as i16)
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speaker_test_is_the_web_join_sound_at_48_khz_mono() {
        let pcm = speaker_test_pcm().expect("bundled channel-join.wav");
        let source = decode(WAVS[Effect::Join as usize]).unwrap();
        let seconds = source.total_duration().unwrap().as_secs_f64();
        assert!((pcm.len() as f64 / 48_000.0 - seconds).abs() < 0.01);
        assert!(pcm.iter().any(|value| value.unsigned_abs() > 300));
    }

    #[test]
    fn canonical_web_effects_decode_to_nonzero_finite_pcm_without_opening_devices() {
        for wav in WAVS {
            let buffer = decode(wav).expect("bundled WAV");
            let samples: Vec<_> = buffer.collect();
            assert!(samples.len() > 100);
            assert!(
                samples
                    .iter()
                    .all(|value| value.is_finite() && value.abs() <= 1.0)
            );
            assert!(samples.iter().any(|value| value.abs() > 0.01));
        }
    }

    #[test]
    fn slider_feedback_is_throttled_and_late_effects_are_discarded() {
        let (sender, receiver) = mpsc::sync_channel(16);
        let mut effects = Effects {
            sender: Some(sender),
            last_slider: None,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        effects.slider(0.8);
        effects.slider(0.1);
        let request = receiver.try_recv().unwrap();
        assert!((request.volume - 0.276).abs() < 0.001);
        assert!((request.speed - 1.23).abs() < 0.001);
        assert!(receiver.try_recv().is_err());
        assert!(request.fresh(request.requested + Duration::from_millis(120)));
        assert!(!request.fresh(request.requested + Duration::from_millis(121)));
        let cancellation = effects.cancelled.clone();
        assert!(!cancellation.load(Ordering::Acquire));
        drop(effects);
        assert!(cancellation.load(Ordering::Acquire));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Disconnected)
        ));
    }
}
