import { acquireAudioContext, releaseAudioContext } from "../media/audio-context.ts";
import type { ChatAttachment } from "./types.ts";

/** Voice messages: recording, the uploaded waveform, playback and transcripts.
 * The wire contract (64 normalised loudness bytes, base64; `transcript` states)
 * is shared with the API and every native client. */

/** Recording stops and sends itself at ten minutes, the server's limit. */
export const MAX_VOICE_MS = 600_000;
/** Shorter recordings are treated as a tap and never uploaded. */
export const MIN_VOICE_MS = 1_000;
/** On touch layouts a press held at least this long is press-and-hold (release
 * stops); a shorter tap keeps recording hands-free until Stop. */
export const HOLD_TAP_MS = 300;
export const WAVEFORM_SLICES = 64;
/** How often the input level is sampled while recording (50 Hz). */
export const LEVEL_INTERVAL_MS = 20;
/** The live meter adds one bar this often and scrolls between them. */
export const LIVE_BAR_MS = 70;
/** Waveform bars: width and gap in CSS pixels, shared by the player, chip and meter. */
export const BAR_WIDTH = 3;
export const BAR_GAP = 2;
/** Loudness below this is drawn as silence. */
const FLOOR_DB = -50;

/** Per-device preference: send as soon as recording stops, instead of staging
 * the voice message in the composer. Off by default. */
const SEND_ON_STOP = "caper:voice-send-on-stop";
let sendOnStop = false;
export function getVoiceSendOnStop(): boolean {
  try {
    return localStorage.getItem(SEND_ON_STOP) === "on";
  } catch {
    return sendOnStop;
  }
}
export function setVoiceSendOnStop(enabled: boolean) {
  sendOnStop = enabled;
  try {
    localStorage.setItem(SEND_ON_STOP, enabled ? "on" : "off");
  } catch {
    /* The in-memory preference still works. */
  }
  if (typeof window !== "undefined") window.dispatchEvent(new Event(SEND_ON_STOP));
}
export function subscribeVoiceSendOnStop(changed: () => void) {
  window.addEventListener(SEND_ON_STOP, changed);
  window.addEventListener("storage", changed);
  return () => {
    window.removeEventListener(SEND_ON_STOP, changed);
    window.removeEventListener("storage", changed);
  };
}

/** MediaRecorder types in order of preference. */
export const VOICE_TYPES = ["audio/webm;codecs=opus", "audio/mp4", "audio/ogg;codecs=opus"] as const;

export const VOICE_LABEL = "Voice message";
export const MICROPHONE_DENIED = "Microphone access is needed to record voice messages";

export interface VoiceRecording {
  blob: Blob;
  /** The recorder's type without codec parameters, as uploaded. */
  contentType: string;
  durationMs: number;
  /** Base64 of the 64 normalised slice loudness bytes. */
  waveform: string;
}

export function voiceRecordingType(isSupported: (type: string) => boolean): string | undefined {
  return VOICE_TYPES.find((type) => {
    try {
      return isSupported(type);
    } catch {
      return false;
    }
  });
}

/** `audio/webm;codecs=opus` → `audio/webm`; the upload names the container only. */
export function voiceContentType(mimeType: string | undefined): string {
  const base = mimeType?.split(";")[0]?.trim().toLowerCase();
  return base && base.startsWith("audio/") ? base : "audio/webm";
}

export function voiceFilename(contentType: string): string {
  const extension = contentType === "audio/mp4" ? "m4a" : contentType === "audio/ogg" ? "ogg" : "webm";
  return `voice-message.${extension}`;
}

/** Perceptual loudness 0–1 of an RMS amplitude: −50 dB (and below) to 0 dB. */
export function loudness(rms: number): number {
  if (!(rms > 0)) return 0;
  return Math.max(0, Math.min(1, (20 * Math.log10(Math.min(1, rms)) - FLOOR_DB) / -FLOOR_DB));
}

/** The uploaded waveform: the RMS of each of `slices` equal time slices of the
 * RMS samples (taken at a steady rate), on the perceptual scale, smoothed with a
 * 3-tap moving average, then scaled so the loudest slice is 255. */
export function waveformFromLevels(levels: ArrayLike<number>, slices = WAVEFORM_SLICES): Uint8Array {
  const output = new Uint8Array(slices);
  const count = levels.length;
  if (!count || slices <= 0) return output;
  const scaled = Array.from({ length: slices }, (_, slice) => {
    const start = Math.floor((slice * count) / slices);
    const end = Math.max(start + 1, Math.floor(((slice + 1) * count) / slices));
    let energy = 0;
    let samples = 0;
    for (let index = start; index < end && index < count; index++) {
      const value = Number(levels[index]) || 0;
      energy += value * value;
      samples++;
    }
    return loudness(Math.sqrt(energy / Math.max(1, samples)));
  });
  const smoothed = scaled.map((_, index) => {
    const window = scaled.slice(Math.max(0, index - 1), index + 2);
    return window.reduce((sum, value) => sum + value, 0) / window.length;
  });
  const loudest = Math.max(...smoothed);
  if (loudest <= 0) return output;
  for (let slice = 0; slice < slices; slice++) output[slice] = Math.round((smoothed[slice] / loudest) * 255);
  return output;
}

export function encodeWaveform(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

/** The server accepts 1–256 bytes. Anything else is no waveform. */
export function decodeWaveform(value: unknown): Uint8Array | undefined {
  if (typeof value !== "string" || !value || value.length > 344) return;
  try {
    const binary = atob(value);
    if (binary.length < 1 || binary.length > 256) return;
    return Uint8Array.from(binary, (character) => character.charCodeAt(0));
  } catch {
    return;
  }
}

/** Heights 0–1 for `bars` evenly spaced bars from a waveform of any length,
 * linearly interpolated so stretched and squeezed waveforms stay smooth. */
export function resampleWaveform(bytes: Uint8Array | undefined, bars: number): number[] {
  if (bars <= 0) return [];
  if (!bytes?.length) return Array<number>(bars).fill(0);
  const last = bytes.length - 1;
  return Array.from({ length: bars }, (_, bar) => {
    const position = bars === 1 ? last / 2 : (bar * last) / (bars - 1);
    const below = Math.floor(position);
    const above = Math.min(last, below + 1);
    const mix = position - below;
    return (bytes[below] * (1 - mix) + bytes[above] * mix) / 255;
  });
}

/** How many bars of the shared width and gap fit `width` pixels. */
export function barsFor(width: number, minimum = 8): number {
  return Math.max(minimum, Math.floor((width + BAR_GAP) / (BAR_WIDTH + BAR_GAP)));
}

/** How much of bar `index` of `bars` is played at `progress` (0–1): the
 * boundary bar fills partially. */
export function barFill(index: number, bars: number, progress: number): number {
  return Math.max(0, Math.min(1, progress * bars - index));
}

/** `m:ss` (or `h:mm:ss`). Totals round, the elapsed timer floors and a countdown
 * ceils, so playback never shows 0:00 before it has finished. */
export function formatVoiceDuration(ms: number, rounding: "round" | "floor" | "ceil" = "round"): string {
  const seconds = Number.isFinite(ms) && ms > 0 ? Math[rounding](ms / 1_000) : 0;
  const hours = Math.floor(seconds / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  const rest = String(seconds % 60).padStart(2, "0");
  return hours ? `${hours}:${String(minutes).padStart(2, "0")}:${rest}` : `${minutes}:${rest}`;
}

/** Screen-reader wording, e.g. "1 minute 5 seconds". */
export function spokenDuration(ms: number): string {
  const seconds = Number.isFinite(ms) && ms > 0 ? Math.round(ms / 1_000) : 0;
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  const parts = [];
  if (minutes) parts.push(`${minutes} ${minutes === 1 ? "minute" : "minutes"}`);
  if (rest || !minutes) parts.push(`${rest} ${rest === 1 ? "second" : "seconds"}`);
  return parts.join(" ");
}

export type TranscriptView = { state: "none" } | { state: "pending" } | { state: "ready"; text: string };

/** Failed, absent, unknown or empty transcripts show nothing. */
export function transcriptView(attachment: Pick<ChatAttachment, "transcript">): TranscriptView {
  const transcript = attachment.transcript as unknown;
  if (!transcript || typeof transcript !== "object") return { state: "none" };
  const { status, text } = transcript as { status?: unknown; text?: unknown };
  if (status === "pending") return { state: "pending" };
  if (status === "ready" && typeof text === "string" && text.trim()) return { state: "ready", text: text.trim() };
  return { state: "none" };
}

export function isVoiceAttachment(attachment: Partial<Pick<ChatAttachment, "voice" | "kind">>): boolean {
  return attachment.voice === true && attachment.kind === "audio";
}

/** What to call a file in excerpts: voice notes are "Voice message", not their filename. */
export function attachmentLabel(attachment: Pick<ChatAttachment, "voice" | "kind" | "name">): string {
  return isVoiceAttachment(attachment) ? VOICE_LABEL : attachment.name;
}

/** The fallback excerpt for a message without text. */
export function attachmentsExcerpt(attachments: readonly unknown[] | undefined) {
  return attachments?.length &&
    attachments.every(
      (attachment) =>
        !!attachment && typeof attachment === "object" && isVoiceAttachment(attachment as Partial<ChatAttachment>),
    )
    ? VOICE_LABEL
    : "Attachment";
}

/** The live meter's level: fast attack, slower release, per 20 ms sample. */
export function smoothLevel(previous: number, rms: number): number {
  const next = Math.max(0, rms || 0);
  return previous + (next - previous) * (next > previous ? 0.6 : 0.12);
}

// One voice message plays at a time across the app; recording stops playback.
let playing: HTMLMediaElement | undefined;
export function claimVoicePlayback(element: HTMLMediaElement) {
  if (playing && playing !== element) playing.pause();
  playing = element;
}
export function releaseVoicePlayback(element: HTMLMediaElement) {
  if (playing === element) playing = undefined;
}
export function stopVoicePlayback() {
  playing?.pause();
  playing = undefined;
}

export class VoiceRecordingError extends Error {
  readonly denied: boolean;
  constructor(message: string, denied = false) {
    super(message);
    this.denied = denied;
  }
}

export interface VoiceRecorder {
  readonly startedAt: number;
  /** Stops and returns the recording, or undefined when it was shorter than a second. */
  stop(): Promise<VoiceRecording | undefined>;
  cancel(): void;
}

/** Asks for the default microphone (the browser prompts on first use) and starts
 * recording. The stream is our own: a voice channel's tracks are never touched. */
export async function startVoiceRecording({
  onLevel,
  onLimit,
  now = () => performance.now(),
}: {
  /** RMS input level of each ~20 ms sample. */
  onLevel?: (rms: number) => void;
  /** Ten minutes passed; the caller stops and sends. */
  onLimit?: () => void;
  now?: () => number;
} = {}): Promise<VoiceRecorder> {
  if (typeof MediaRecorder === "undefined" || !navigator.mediaDevices?.getUserMedia)
    throw new VoiceRecordingError("Voice messages can’t be recorded in this browser.");
  let stream: MediaStream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true },
    });
  } catch (error) {
    const name = error instanceof DOMException ? error.name : "";
    if (name === "NotAllowedError" || name === "SecurityError") throw new VoiceRecordingError(MICROPHONE_DENIED, true);
    if (name === "NotFoundError" || name === "OverconstrainedError")
      throw new VoiceRecordingError("No microphone was found.");
    throw new VoiceRecordingError("The microphone couldn’t be started.");
  }
  const mimeType = voiceRecordingType((type) => MediaRecorder.isTypeSupported(type));
  let recorder: MediaRecorder;
  try {
    recorder = new MediaRecorder(stream, { ...(mimeType ? { mimeType } : {}), audioBitsPerSecond: 32_000 });
  } catch {
    for (const track of stream.getTracks()) track.stop();
    throw new VoiceRecordingError("Voice messages can’t be recorded in this browser.");
  }
  const chunks: Blob[] = [];
  const levels: number[] = [];
  let context: AudioContext | undefined;
  let source: MediaStreamAudioSourceNode | undefined;
  let analyser: AnalyserNode | undefined;
  try {
    context = acquireAudioContext();
    void context.resume().catch(() => undefined);
    source = context.createMediaStreamSource(stream);
    analyser = context.createAnalyser();
    analyser.fftSize = 1024;
    source.connect(analyser);
  } catch {
    // Without Web Audio the recording still works; it just has a flat waveform.
  }
  const samples = analyser ? new Float32Array(analyser.fftSize) : undefined;
  const startedAt = now();
  let finished = false;
  const sample = () => {
    if (finished) return;
    let rms = 0;
    if (analyser && samples) {
      analyser.getFloatTimeDomainData(samples);
      let energy = 0;
      for (const value of samples) energy += value * value;
      rms = Math.sqrt(energy / samples.length);
    }
    levels.push(rms);
    onLevel?.(rms);
    if (now() - startedAt >= MAX_VOICE_MS) onLimit?.();
  };
  const timer = setInterval(sample, LEVEL_INTERVAL_MS);
  const cleanup = () => {
    finished = true;
    clearInterval(timer);
    for (const track of stream.getTracks()) track.stop();
    try {
      source?.disconnect();
    } catch {
      /* Already disconnected. */
    }
    if (context) releaseAudioContext(context);
  };
  recorder.ondataavailable = ({ data }) => {
    if (data.size > 0) chunks.push(data);
  };
  try {
    recorder.start(250);
  } catch {
    cleanup();
    throw new VoiceRecordingError("The microphone couldn’t be started.");
  }
  let stopping: Promise<VoiceRecording | undefined> | undefined;
  return {
    startedAt,
    stop() {
      stopping ??= new Promise((resolve, reject) => {
        const durationMs = Math.min(MAX_VOICE_MS, Math.round(now() - startedAt));
        if (finished) return resolve(undefined);
        finished = true;
        const done = () => {
          cleanup();
          if (durationMs < MIN_VOICE_MS) return resolve(undefined);
          if (!chunks.length) return reject(new VoiceRecordingError("No audio was recorded. Try again."));
          const contentType = voiceContentType(recorder.mimeType || mimeType || chunks[0]?.type);
          resolve({
            blob: new Blob(chunks, { type: contentType }),
            contentType,
            durationMs,
            waveform: encodeWaveform(waveformFromLevels(levels)),
          });
        };
        recorder.onstop = done;
        recorder.onerror = () => {
          cleanup();
          reject(new VoiceRecordingError("Recording failed. Try again."));
        };
        if (recorder.state === "inactive") done();
        else recorder.stop();
      });
      return stopping;
    },
    cancel() {
      if (stopping) return;
      stopping = Promise.resolve(undefined);
      recorder.ondataavailable = recorder.onstop = recorder.onerror = null;
      if (recorder.state !== "inactive") recorder.stop();
      cleanup();
    },
  };
}
