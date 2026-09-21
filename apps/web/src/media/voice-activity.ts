// Processed speech can be clearly audible below the old 0.018 RMS cutoff.
// -48 dBFS remains above the suppressed noise floor without hiding quiet speech.
export const VOICE_ACTIVITY_THRESHOLD = 0.004;

export function hasVoiceActivity(samples: Float32Array) {
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  return Math.sqrt(sum / samples.length) >= VOICE_ACTIVITY_THRESHOLD;
}
