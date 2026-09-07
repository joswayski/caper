import { useEffect, useRef } from "react";
import { acquireAudioContext, releaseAudioContext } from "../media/audio-context";

const SAMPLE_INTERVAL_MS = 32;
const ACTIVITY_THRESHOLD = 0.018;
const RELEASE_DELAY_MS = 180;

interface VoiceActivityProps {
  stream?: MediaStream;
  muted: boolean;
  onActivityChange(active: boolean): void;
}

function level(samples: Float32Array) {
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  return Math.sqrt(sum / samples.length);
}

export default function VoiceActivity({ stream, muted, onActivityChange }: VoiceActivityProps) {
  const activityCallback = useRef(onActivityChange);
  activityCallback.current = onActivityChange;

  useEffect(() => {
    let audioContext: AudioContext | undefined;
    let source: MediaStreamAudioSourceNode | undefined;
    let analyser: AnalyserNode | undefined;
    let samples: Float32Array<ArrayBuffer> | undefined;
    let frame = 0;
    let lastSample = 0;
    let lastLoudAt = -Infinity;
    let active = false;

    if (stream && !muted && typeof AudioContext !== "undefined") {
      try {
        audioContext = acquireAudioContext();
        source = audioContext.createMediaStreamSource(stream);
        analyser = audioContext.createAnalyser();
        analyser.fftSize = 256;
        analyser.smoothingTimeConstant = 0.35;
        samples = new Float32Array(analyser.fftSize);
        source.connect(analyser);
        void audioContext.resume().catch(() => undefined);
      } catch {
        if (audioContext) releaseAudioContext(audioContext);
        audioContext = undefined;
      }
    }

    const sample = (now: number) => {
      if (now - lastSample >= SAMPLE_INTERVAL_MS) {
        let rms = 0;
        if (analyser && samples) {
          analyser.getFloatTimeDomainData(samples);
          rms = level(samples);
        }
        lastSample = now;

        if (rms >= ACTIVITY_THRESHOLD) lastLoudAt = now;
        const nextActive = now - lastLoudAt < RELEASE_DELAY_MS;
        if (nextActive !== active) {
          active = nextActive;
          activityCallback.current(active);
        }
      }
      frame = window.requestAnimationFrame(sample);
    };
    frame = window.requestAnimationFrame(sample);

    return () => {
      window.cancelAnimationFrame(frame);
      if (active) activityCallback.current(false);
      source?.disconnect();
      analyser?.disconnect();
      if (audioContext) releaseAudioContext(audioContext);
    };
  }, [muted, stream]);

  return null;
}
