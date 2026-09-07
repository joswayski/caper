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
