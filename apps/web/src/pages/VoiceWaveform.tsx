import { useEffect, useRef } from "react";

const SAMPLE_INTERVAL_MS = 32;
const ACTIVITY_THRESHOLD = 0.018;
const RELEASE_DELAY_MS = 180;
let sharedContext: AudioContext | undefined;
let contextUsers = 0;

function acquireAudioContext() {
  sharedContext ??= new AudioContext({ latencyHint: "interactive" });
  contextUsers++;
  return sharedContext;
}

function releaseAudioContext(context: AudioContext) {
  if (--contextUsers > 0 || context !== sharedContext) return;
  sharedContext = undefined;
  void context.close().catch(() => undefined);
}

interface VoiceWaveformProps {
  stream?: MediaStream;
  muted: boolean;
  label: string;
  onActivityChange(active: boolean): void;
}

function level(samples: Float32Array) {
  let sum = 0;
  for (const sample of samples) sum += sample * sample;
  return Math.sqrt(sum / samples.length);
}

export default function VoiceWaveform({ stream, muted, label, onActivityChange }: VoiceWaveformProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const activityCallback = useRef(onActivityChange);
  activityCallback.current = onActivityChange;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const pixelRatio = window.devicePixelRatio || 1;
    const width = canvas.clientWidth;
    const height = canvas.clientHeight;
    canvas.width = Math.round(width * pixelRatio);
    canvas.height = Math.round(height * pixelRatio);
    const drawing = canvas.getContext("2d");
    if (!drawing) return;
    drawing.scale(pixelRatio, pixelRatio);

    const columns = Math.floor(width / 3);
    const history = new Array<number>(columns).fill(0);
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

    const draw = (now: number) => {
      if (now - lastSample >= SAMPLE_INTERVAL_MS) {
        let rms = 0;
        if (analyser && samples) {
          analyser.getFloatTimeDomainData(samples);
          rms = level(samples);
        }
        const decibels = rms > 0 ? 20 * Math.log10(rms) : -100;
        const amplitude = Math.max(0, Math.min(1, (decibels + 55) / 43));
        history.shift();
        history.push(amplitude);
        lastSample = now;

        if (rms >= ACTIVITY_THRESHOLD) lastLoudAt = now;
        const nextActive = now - lastLoudAt < RELEASE_DELAY_MS;
        if (nextActive !== active) {
          active = nextActive;
          activityCallback.current(active);
        }
      }

      drawing.clearRect(0, 0, width, height);
      drawing.lineCap = "round";
      drawing.lineWidth = 2;
      drawing.strokeStyle = active ? "rgb(112 137 78)" : "rgb(99 122 67 / .52)";
      history.forEach((amplitude, index) => {
        const barHeight = 2 + amplitude * (height - 4);
        const x = index * 3 + 1;
        drawing.beginPath();
        drawing.moveTo(x, (height - barHeight) / 2);
        drawing.lineTo(x, (height + barHeight) / 2);
        drawing.stroke();
      });
      frame = window.requestAnimationFrame(draw);
    };
    frame = window.requestAnimationFrame(draw);

    return () => {
      window.cancelAnimationFrame(frame);
      if (active) activityCallback.current(false);
      source?.disconnect();
      analyser?.disconnect();
      if (audioContext) releaseAudioContext(audioContext);
    };
  }, [muted, stream]);

  return <canvas ref={canvasRef} className="voice-waveform" role="img" aria-label={label} />;
}
