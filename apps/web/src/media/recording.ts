import { connectVoiceProcessing, createVoiceProcessingNodes } from "./voice-processing.ts";

export const MAX_RECORDING_SECONDS = 10;

export interface ReceivedRecording {
  result: Promise<Blob>;
  finish(): void;
  cancel(): void;
}

function preferredAudioType() {
  const types = ["audio/webm;codecs=opus", "audio/webm", "audio/ogg;codecs=opus"];
  return types.find((type) => MediaRecorder.isTypeSupported(type));
}

function encodeWave(buffer: AudioBuffer) {
  const channels = buffer.numberOfChannels;
  const bytesPerSample = 2;
  const dataSize = buffer.length * channels * bytesPerSample;
  const output = new ArrayBuffer(44 + dataSize);
  const view = new DataView(output);
  const text = (offset: number, value: string) => {
    for (let index = 0; index < value.length; index++) view.setUint8(offset + index, value.charCodeAt(index));
  };
  text(0, "RIFF");
  view.setUint32(4, 36 + dataSize, true);
  text(8, "WAVE");
  text(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, channels, true);
  view.setUint32(24, buffer.sampleRate, true);
  view.setUint32(28, buffer.sampleRate * channels * bytesPerSample, true);
  view.setUint16(32, channels * bytesPerSample, true);
  view.setUint16(34, bytesPerSample * 8, true);
  text(36, "data");
  view.setUint32(40, dataSize, true);
  const samples = Array.from({ length: channels }, (_, channel) => buffer.getChannelData(channel));
  let offset = 44;
  for (let frame = 0; frame < buffer.length; frame++) {
    for (let channel = 0; channel < channels; channel++) {
      const sample = Math.max(-1, Math.min(1, samples[channel][frame]));
      view.setInt16(offset, sample < 0 ? sample * 0x8000 : sample * 0x7fff, true);
      offset += bytesPerSample;
    }
  }
  return new Blob([output], { type: "audio/wav" });
}

export async function createVoiceComparison(blob: Blob, strength: number) {
  const decoder = new AudioContext();
  let decoded: AudioBuffer;
  try {
    decoded = await decoder.decodeAudioData(await blob.arrayBuffer());
  } finally {
    void decoder.close();
  }
  let hasSignal = false;
  for (let channel = 0; channel < decoded.numberOfChannels; channel++) {
    hasSignal ||= decoded.getChannelData(channel).some((sample) => Math.abs(sample) > 0.001);
  }
  const context = new OfflineAudioContext(decoded.numberOfChannels, decoded.length, decoded.sampleRate);
  const source = context.createBufferSource();
  source.buffer = decoded;
  if (strength <= 0) {
    source.connect(context.destination);
  } else {
    connectVoiceProcessing(source, context.destination, createVoiceProcessingNodes(context, strength));
  }
  source.start();
  return { processed: encodeWave(await context.startRendering()), hasSignal };
}

/** Records the timestamped, received WebRTC audio in memory without owning its tracks. */
export function recordReceivedAudio(stream: MediaStream): ReceivedRecording {
  if (typeof MediaRecorder === "undefined") throw new Error("Audio recording is not supported by this browser.");
  if (!stream.getAudioTracks().some((track) => track.readyState === "live")) throw new Error("No received audio is available.");

  const mimeType = preferredAudioType();
  const recorder = new MediaRecorder(stream, mimeType ? { mimeType } : undefined);
  // Chromium does not reliably drain a remote WebRTC jitter buffer for
  // MediaRecorder alone. A muted sink keeps received packets flowing without
  // playing the delayed microphone return to the user while it is recorded.
  const receiver = new Audio();
  receiver.muted = true;
  receiver.srcObject = stream;
  const chunks: Blob[] = [];
  let settled = false;
  let cancelled = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let resolve!: (blob: Blob) => void;
  let reject!: (error: Error) => void;
  const result = new Promise<Blob>((yes, no) => { resolve = yes; reject = no; });

  const cleanup = () => {
    clearTimeout(timer);
    receiver.pause();
    receiver.srcObject = null;
  };
  const fail = (error: unknown) => {
    if (settled) return;
    settled = true;
    cleanup();
    if (recorder.state === "recording") recorder.stop();
    reject(error instanceof Error ? error : new Error("Audio recording failed."));
  };
  recorder.ondataavailable = ({ data }) => {
    if (!cancelled && data.size > 0) chunks.push(data);
  };
  recorder.onerror = ({ error }) => fail(error);
  recorder.onstop = () => {
    cleanup();
    if (settled || cancelled) return;
    if (chunks.length === 0) return fail(new Error("No audio was recorded. Please try again."));
    settled = true;
    resolve(new Blob(chunks, { type: recorder.mimeType || chunks[0]?.type || "audio/webm" }));
  };

  try {
    recorder.start(250);
  } catch (error) {
    cleanup();
    throw error;
  }
  void receiver.play().catch(fail);
  timer = setTimeout(() => {
    if (recorder.state === "recording") recorder.stop();
  }, MAX_RECORDING_SECONDS * 1_000);

  return {
    result,
    finish() {
      if (!settled && recorder.state === "recording") recorder.stop();
    },
    cancel() {
      if (settled) return;
      cancelled = true;
      if (recorder.state === "recording") recorder.stop();
      fail(new Error("Audio recording cancelled."));
    },
  };
}
