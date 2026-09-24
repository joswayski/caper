//! PCM input gain and voice contour shared by the live publication and local
//! natural/enhanced microphone comparison. Audio stays in this process.

const RATE: f32 = 48_000.0;
const HOP: usize = 480;
const FALLBACK_DEBT_US: u64 = 60_000;

fn next_lag_debt(previous: u64, elapsed_us: u64) -> u64 {
    previous.saturating_add(elapsed_us).saturating_sub(10_000)
}

enum Denoiser {
    Dpdfnet(Box<super::dpdfnet::Dpdfnet>),
    Rnnoise(Box<nnnoiseless::DenoiseState<'static>>),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AudioProcessingDiagnostics {
    /// Actual active engine, never a requested mode or inferred device state.
    pub engine: &'static str,
    pub processed_hops: u64,
    pub total_processing_us: u64,
    pub max_processing_us: u64,
    pub fallback_count: u32,
}

/// Each microphone has its own recurrent denoiser and contour. Do not share
/// state between a comparison and live capture or across calls.
pub struct ProcessedVoice {
    denoiser: Denoiser,
    contour: VoiceProcessor,
    lag_debt_us: u64,
    diagnostics: AudioProcessingDiagnostics,
}

impl ProcessedVoice {
    pub fn new() -> Self {
        let denoiser = super::dpdfnet::Dpdfnet::new()
            .map(|model| Denoiser::Dpdfnet(Box::new(model)))
            .unwrap_or_else(|_| Denoiser::Rnnoise(nnnoiseless::DenoiseState::new()));
        Self {
            diagnostics: AudioProcessingDiagnostics {
                engine: if matches!(&denoiser, Denoiser::Dpdfnet(_)) {
                    "DPDFNet-8"
                } else {
                    "RNNoise fallback"
                },
                fallback_count: u32::from(matches!(&denoiser, Denoiser::Rnnoise(_))),
                ..AudioProcessingDiagnostics::default()
            },
            denoiser,
            contour: VoiceProcessor::default(),
            lag_debt_us: 0,
        }
    }

    pub fn diagnostics(&self) -> AudioProcessingDiagnostics {
        self.diagnostics
    }

    /// One hop is 10 ms. The native capture queue drops oldest at eight hops;
    /// switch to RNNoise before six hops of sustained wall-clock debt accumulate.
    /// Unlike inference-only timing this includes spawn scheduling delays.
    pub fn observe_wall_time(&mut self, elapsed: std::time::Duration) {
        if !matches!(self.denoiser, Denoiser::Dpdfnet(_)) {
            return;
        }
        let duration = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
        self.lag_debt_us = next_lag_debt(self.lag_debt_us, duration);
        if self.lag_debt_us >= FALLBACK_DEBT_US {
            self.denoiser = Denoiser::Rnnoise(nnnoiseless::DenoiseState::new());
            self.diagnostics.engine = "RNNoise fallback";
            self.diagnostics.fallback_count += 1;
        }
    }

    /// Returns (natural, enhanced). Natural is gain + denoise, just as the
    /// shipped web comparison; enhanced additionally applies the contour.
    pub fn process(
        &mut self,
        input: &[i16],
        settings: InputProcessing,
    ) -> Result<(Vec<i16>, Vec<i16>), String> {
        if input.len() != HOP || settings.gain_percent > 200 || settings.strength > 100 {
            return Err("invalid 48 kHz mono microphone frame or input settings".into());
        }
        let gain = f32::from(settings.gain_percent) / 100.0;
        let pcm: [f32; HOP] = std::array::from_fn(|i| f32::from(input[i]) * gain);
        let started = std::time::Instant::now();
        let denoised = match &mut self.denoiser {
            Denoiser::Dpdfnet(model) => {
                let normalized = std::array::from_fn(|i| pcm[i] / 32768.0);
                model
                    .process(&normalized)
                    .map(|output| output.map(|v| v * 32768.0))
            }
            Denoiser::Rnnoise(model) => {
                let mut output = [0.0; HOP];
                model.process_frame(&mut output, &pcm);
                Ok(output)
            }
        };
        let denoised = match denoised {
            Ok(output) if output.iter().all(|x| x.is_finite()) => output,
            _ => {
                self.denoiser = Denoiser::Rnnoise(nnnoiseless::DenoiseState::new());
                self.diagnostics.engine = "RNNoise fallback";
                self.diagnostics.fallback_count += 1;
                let mut output = [0.0; HOP];
                if let Denoiser::Rnnoise(model) = &mut self.denoiser {
                    model.process_frame(&mut output, &pcm);
                }
                if !output.iter().all(|x| x.is_finite()) {
                    return Err("microphone denoising failed".into());
                }
                output
            }
        };
        let elapsed = started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64;
        self.diagnostics.processed_hops += 1;
        self.diagnostics.total_processing_us =
            self.diagnostics.total_processing_us.saturating_add(elapsed);
        self.diagnostics.max_processing_us = self.diagnostics.max_processing_us.max(elapsed);
        let natural: Vec<i16> = if settings.gain_percent == 0 {
            vec![0; HOP]
        } else {
            denoised
                .iter()
                .map(|sample| sample.clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16)
                .collect()
        };
        let enhanced = if settings.gain_percent == 0 {
            self.contour = VoiceProcessor::default();
            vec![0; HOP]
        } else {
            self.contour.process(
                &natural,
                InputProcessing {
                    gain_percent: 100,
                    strength: settings.strength,
                },
            )
        };
        Ok((natural, enhanced))
    }
}

#[derive(Clone, Copy)]
pub struct InputProcessing {
    pub gain_percent: u16,
    pub strength: u8,
}

impl Default for InputProcessing {
    fn default() -> Self {
        Self {
            gain_percent: 100,
            strength: 25,
        }
    }
}

#[derive(Default)]
pub struct VoiceProcessor {
    prev: f32,
    high: f32,
    low: f32,
    upper: f32,
    envelope: f32,
}

impl VoiceProcessor {
    pub fn process(&mut self, input: &[i16], settings: InputProcessing) -> Vec<i16> {
        if settings.gain_percent == 0 {
            return vec![0; input.len()];
        }
        let gain = f32::from(settings.gain_percent) / 100.0;
        if settings.strength == 0 {
            return input
                .iter()
                .map(|sample| {
                    (f32::from(*sample) * gain).clamp(f32::from(i16::MIN), f32::from(i16::MAX))
                        as i16
                })
                .collect();
        }
        let amount = f32::from(settings.strength) / 100.0;
        let dt = 1.0 / RATE;
        let hp_alpha = 1.0 / (1.0 + 2.0 * std::f32::consts::PI * 75.0 * amount * dt);
        let low_alpha = 2.0 * std::f32::consts::PI * 180.0 * dt;
        let presence_alpha = 2.0 * std::f32::consts::PI * 3_000.0 * dt;
        let warmth = 10.0_f32.powf(2.0 * amount / 20.0) - 1.0;
        let presence = 10.0_f32.powf(1.5 * amount / 20.0) - 1.0;
        let makeup = 1.35_f32.powf(amount);
        let threshold = 10.0_f32.powf(-24.0 / 20.0);
        let limit = 10.0_f32.powf(-2.0 / 20.0);
        let ratio = 1.0 + 2.0 * amount;
        input
            .iter()
            .map(|sample| {
                let value = f32::from(*sample) * gain / 32768.0;
                self.high = hp_alpha * (self.high + value - self.prev);
                self.prev = value;
                self.low += low_alpha * (self.high - self.low);
                self.upper += presence_alpha * (self.high - self.upper);
                let shaped = self.high + self.low * warmth + (self.upper - self.low) * presence;
                let level = shaped.abs();
                let speed = if level > self.envelope { 0.008 } else { 0.18 };
                self.envelope += (1.0 - (-dt / speed).exp()) * (level - self.envelope);
                let compressed = if self.envelope > threshold {
                    shaped * (threshold / self.envelope).powf(1.0 - 1.0 / ratio)
                } else {
                    shaped
                } * makeup;
                (compressed.clamp(-limit, limit) * 32768.0) as i16
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sustained_twenty_ms_hops_trigger_fallback_before_eight_queued_frames() {
        let mut debt = 0;
        for _ in 0..5 {
            debt = next_lag_debt(debt, 20_000);
            assert!(debt < FALLBACK_DEBT_US);
        }
        assert_eq!(next_lag_debt(debt, 20_000), FALLBACK_DEBT_US);
        assert_eq!(next_lag_debt(30_000, 4_000), 24_000);
    }

    #[test]
    fn rnnoise_fallback_outputs_bounded_speech_and_zero_gain_is_silent() {
        let mut engine = ProcessedVoice {
            denoiser: Denoiser::Rnnoise(nnnoiseless::DenoiseState::new()),
            contour: VoiceProcessor::default(),
            lag_debt_us: 0,
            diagnostics: AudioProcessingDiagnostics {
                engine: "RNNoise fallback",
                ..AudioProcessingDiagnostics::default()
            },
        };
        let mut speech_peak = 0;
        for hop in 0..45 {
            let frame: [i16; HOP] = std::array::from_fn(|i| {
                let t = (hop * HOP + i) as f32 / 48_000.0;
                let carrier = (t * std::f32::consts::TAU * 161.0).sin();
                let articulation = (t * std::f32::consts::TAU * 1200.0).sin() * 0.3;
                let noise = ((i * 97 + hop * 31) % 257) as f32 / 257.0 - 0.5;
                ((carrier + articulation + noise * 0.1) * 9000.0) as i16
            });
            let (natural, enhanced) = engine
                .process(
                    &frame,
                    InputProcessing {
                        gain_percent: 160,
                        strength: 60,
                    },
                )
                .unwrap();
            speech_peak = speech_peak.max(natural.iter().map(|x| x.unsigned_abs()).max().unwrap());
            assert!(enhanced.iter().all(|x| x.unsigned_abs() <= 29_220));
        }
        assert!(
            speech_peak > 1000,
            "RNNoise fallback suppressed the speech fixture"
        );
        let status = engine.diagnostics();
        assert_eq!(status.engine, "RNNoise fallback");
        assert_eq!(status.processed_hops, 45);
        assert!(status.total_processing_us >= status.max_processing_us);
        let (natural, enhanced) = engine
            .process(
                &[9000; HOP],
                InputProcessing {
                    gain_percent: 0,
                    strength: 80,
                },
            )
            .unwrap();
        assert_eq!(natural, [0; HOP]);
        assert_eq!(enhanced, [0; HOP]);
    }

    #[test]
    fn asymmetric_gain_zero_and_strength_update() {
        let input = [1200, -4100, 8000, -12000, 2400];
        let mut processor = VoiceProcessor::default();
        assert_eq!(
            processor.process(
                &input,
                InputProcessing {
                    gain_percent: 175,
                    strength: 0
                }
            ),
            [2100, -7175, 14000, -21000, 4200]
        );
        assert_eq!(
            processor.process(
                &input,
                InputProcessing {
                    gain_percent: 0,
                    strength: 100
                }
            ),
            [0; 5]
        );
        let enhanced = processor.process(
            &input,
            InputProcessing {
                gain_percent: 175,
                strength: 80,
            },
        );
        assert_ne!(enhanced, input);
        assert_ne!(enhanced, [2100, -7175, 14000, -21000, 4200]);
        assert!(
            enhanced
                .iter()
                .all(|sample| sample.unsigned_abs() <= 29_220)
        );
    }
}
