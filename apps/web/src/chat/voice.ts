import { acquireAudioContext, releaseAudioContext } from "../media/audio-context.ts";
import type { ChatAttachment } from "./types.ts";

/** Voice messages: recording, the uploaded peak waveform, playback and transcripts.
 * The wire contract (64 normalised peaks, base64; `transcript` states) is shared
 * with the API and every native client. */

/** Recording stops and sends itself at ten minutes, the server's limit. */
export const MAX_VOICE_MS = 600_000;
/** Shorter recordings are treated as a tap and never uploaded. */
export const MIN_VOICE_MS = 1_000;
/** A press shorter than this on a touch layout is a tap, which records nothing. */
export const HOLD_TAP_MS = 300;
export const WAVEFORM_SLICES = 64;
/** How often the live input level is sampled while recording. */
export const LEVEL_INTERVAL_MS = 50;

/** MediaRecorder types in order of preference. */
export const VOICE_TYPES = ["audio/webm;codecs=opus", "audio/mp4", "audio/ogg;codecs=opus"] as const;

export const VOICE_LABEL = "Voice message";
export const MICROPHONE_DENIED = "Microphone access is needed to record voice messages";

export interface VoiceRecording {
  blob: Blob;
  /** The recorder's type without codec parameters, as uploaded. */
  contentType: string;
  durationMs: number;
  /** Base64 of the 64 normalised slice peaks. */
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

/** Peak amplitude of each of `slices` equal time slices of `peaks` (sampled at a
 * steady rate), scaled linearly so the loudest slice is 255. Silence stays 0. */
export function waveformFromPeaks(peaks: ArrayLike<number>, slices = WAVEFORM_SLICES): Uint8Array {
  const output = new Uint8Array(slices);
  const count = peaks.length;
  if (!count || slices <= 0) return output;
  const sliced = new Float64Array(slices);
  for (let slice = 0; slice < slices; slice++) {
    const start = Math.floor((slice * count) / slices);
    const end = Math.max(start + 1, Math.floor(((slice + 1) * count) / slices));
    let peak = 0;
    for (let index = start; index < end && index < count; index++) {
      const value = Math.abs(Number(peaks[index]) || 0);
      if (value > peak) peak = value;
    }
    sliced[slice] = peak;
  }
  let loudest = 0;
  for (const value of sliced) if (value > loudest) loudest = value;
  if (loudest <= 0) return output;
  for (let slice = 0; slice < slices; slice++) output[slice] = Math.round((sliced[slice] / loudest) * 255);
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

/** Heights 0–1 for `bars` bars from a waveform of any length: the loudest byte
 * of each bar's span when shrinking, the nearest byte when stretching. */
export function resampleWaveform(bytes: Uint8Array | undefined, bars: number): number[] {
  if (bars <= 0) return [];
  if (!bytes?.length) return Array<number>(bars).fill(0);
  const length = bytes.length;
  return Array.from({ length: bars }, (_, bar) => {
    const start = Math.floor((bar * length) / bars);
    const end = Math.max(start + 1, Math.floor(((bar + 1) * length) / bars));
    let peak = 0;
    for (let index = start; index < end && index < length; index++) peak = Math.max(peak, bytes[index]);
    return peak / 255;
  });
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

/** Live input level 0–1 on a loudness scale (−54 dB to 0 dB), for the recording bar. */
export function levelHeight(peak: number): number {
  if (!(peak > 0)) return 0;
  const decibels = 20 * Math.log10(Math.min(1, peak));
  return Math.max(0, Math.min(1, (decibels + 54) / 54));
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
  onLevel?: (peak: number) => void;
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
  const peaks: number[] = [];
  let context: AudioContext | undefined;
  let source: MediaStreamAudioSourceNode | undefined;
  let analyser: AnalyserNode | undefined;
  try {
    context = acquireAudioContext();
    void context.resume().catch(() => undefined);
    source = context.createMediaStreamSource(stream);
    analyser = context.createAnalyser();
    analyser.fftSize = 2048;
    source.connect(analyser);
  } catch {
    // Without Web Audio the recording still works; it just has a flat waveform.
  }
  const samples = analyser ? new Float32Array(analyser.fftSize) : undefined;
  const startedAt = now();
  let finished = false;
  const sample = () => {
    if (finished) return;
    let peak = 0;
    if (analyser && samples) {
      analyser.getFloatTimeDomainData(samples);
      for (const value of samples) peak = Math.max(peak, Math.abs(value));
    }
    peaks.push(peak);
    onLevel?.(peak);
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
            waveform: encodeWaveform(waveformFromPeaks(peaks)),
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
