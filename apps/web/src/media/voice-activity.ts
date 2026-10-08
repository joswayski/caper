import { acquireAudioContext, releaseAudioContext } from "./audio-context.ts";

// Processed speech can be clearly audible below the old 0.018 RMS cutoff.
// -48 dBFS remains above the suppressed noise floor without hiding quiet speech.
export const VOICE_ACTIVITY_THRESHOLD = 0.004;

export function hasVoiceActivity(samples: Float32Array) {
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  return Math.sqrt(sum / samples.length) >= VOICE_ACTIVITY_THRESHOLD;
}

/** Observe only the speaking indicator; capture and playback keep running in hidden tabs. */
export function watchVoiceActivity(
  stream: MediaStream | undefined,
  muted: boolean,
  onActivityChange: (active: boolean) => void,
): () => void {
  if (!stream || muted || typeof AudioContext === "undefined") return () => {};

  let context: AudioContext | undefined;
  let source: MediaStreamAudioSourceNode | undefined;
  let analyser: AnalyserNode;
  try {
    context = acquireAudioContext();
    source = context.createMediaStreamSource(stream);
    analyser = context.createAnalyser();
    analyser.fftSize = 256;
    analyser.smoothingTimeConstant = 0.35;
    source.connect(analyser);
    void context.resume().catch(() => undefined);
  } catch {
    source?.disconnect();
    if (context) releaseAudioContext(context);
    return () => {};
  }

  const samples = new Float32Array(analyser.fftSize);
  let timer: number | undefined;
  let lastLoudAt = -Infinity;
  let active = false;
  const sample = () => {
    const now = performance.now();
    analyser.getFloatTimeDomainData(samples);
    if (hasVoiceActivity(samples)) lastLoudAt = now;
    const nextActive = now - lastLoudAt < 180;
    if (nextActive !== active) {
      active = nextActive;
      onActivityChange(active);
    }
  };
  const pause = () => {
    window.clearInterval(timer);
    timer = undefined;
    lastLoudAt = -Infinity;
    if (active) {
      active = false;
      onActivityChange(false);
    }
  };
  const visibilityChanged = () => {
    if (document.visibilityState === "hidden") pause();
    else if (timer === undefined) {
      sample();
      // One callback per sample rather than a callback every display frame.
      timer = window.setInterval(sample, 32);
    }
  };
  document.addEventListener("visibilitychange", visibilityChanged);
  visibilityChanged();
  return () => {
    document.removeEventListener("visibilitychange", visibilityChanged);
    pause();
    source.disconnect();
    analyser.disconnect();
    releaseAudioContext(context);
  };
}
