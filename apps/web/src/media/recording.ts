export const RECORDING_SECONDS = 5;

export function encodePcm16Wav(chunks: readonly Float32Array[], sampleRate: number, maximumFrames = sampleRate * RECORDING_SECONDS): Blob {
  const frameCount = Math.min(maximumFrames, chunks.reduce((total, chunk) => total + chunk.length, 0));
  const buffer = new ArrayBuffer(44 + frameCount * 2);
  const view = new DataView(buffer);
  const text = (offset: number, value: string) => {
    for (let i = 0; i < value.length; i++) view.setUint8(offset + i, value.charCodeAt(i));
  };
  text(0, "RIFF");
  view.setUint32(4, 36 + frameCount * 2, true);
  text(8, "WAVE");
  text(12, "fmt ");
  view.setUint32(16, 16, true);
  view.setUint16(20, 1, true);
  view.setUint16(22, 1, true);
  view.setUint32(24, sampleRate, true);
  view.setUint32(28, sampleRate * 2, true);
  view.setUint16(32, 2, true);
  view.setUint16(34, 16, true);
  text(36, "data");
  view.setUint32(40, frameCount * 2, true);
  let frame = 0;
  for (const chunk of chunks) {
    for (let i = 0; i < chunk.length && frame < frameCount; i++, frame++) {
      const sample = Math.max(-1, Math.min(1, chunk[i] ?? 0));
      view.setInt16(44 + frame * 2, sample < 0 ? sample * 0x8000 : sample * 0x7fff, true);
    }
  }
  return new Blob([buffer], { type: "audio/wav" });
}

export interface ReceivedRecording {
  result: Promise<Blob>;
  stop(): void;
}

/** Records received audio in memory. The supplied stream and its tracks remain borrowed. */
export function recordReceivedAudio(stream: MediaStream): ReceivedRecording {
  if (typeof AudioContext === "undefined" || typeof AudioWorkletNode === "undefined") {
    throw new Error("Audio recording is not supported by this browser.");
  }
  const context = new AudioContext({ sampleRate: 48_000 });
  // Chromium may not drain the WebRTC receiver's jitter buffer for Web Audio
  // alone. Keep a muted media element playing while PCM capture is active.
  const receiver = new Audio();
  receiver.muted = true;
  receiver.srcObject = stream;
  let source: MediaStreamAudioSourceNode | undefined;
  let node: AudioWorkletNode | undefined;
  let gain: GainNode | undefined;
  let settled = false;
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const chunks: Float32Array[] = [];
  let resolve!: (blob: Blob) => void;
  let reject!: (error: Error) => void;
  const result = new Promise<Blob>((yes, no) => { resolve = yes; reject = no; });
  const cleanup = () => {
    clearTimeout(timer);
    receiver.pause();
    receiver.srcObject = null;
    source?.disconnect();
    if (node) {
      node.port.onmessage = null;
      node.port.postMessage("stop");
      node.disconnect();
      node.port.close();
    }
    gain?.disconnect();
    if (context.state !== "closed") void context.close().catch(() => undefined);
  };
  const fail = (error: unknown) => {
    if (settled) return;
    settled = true;
    cleanup();
    reject(error instanceof Error ? error : new Error("Audio recording failed."));
  };
  const stop = () => {
    if (stopped) return;
    stopped = true;
    fail(new Error("Audio recording cancelled."));
  };

  // Bound startup too; resume can otherwise wait indefinitely on autoplay policy.
  timer = setTimeout(() => { stopped = true; fail(new Error("Audio recording timed out.")); }, 10_000);
  const resumed = Promise.all([context.resume(), receiver.play()]);
  void resumed.catch(fail);
  void (async () => {
    try {
      if (!stream.getAudioTracks().some((track) => track.readyState === "live")) throw new Error("No received audio is available.");
      await context.audioWorklet.addModule("/audio/recording-v1/worklet.js");
      if (stopped) return;
      await resumed;
      if (stopped || settled) return;
      source = context.createMediaStreamSource(stream);
      node = new AudioWorkletNode(context, "caper-recorder", {
        numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
        channelCount: 1, channelCountMode: "explicit",
        processorOptions: { maximumFrames: context.sampleRate * RECORDING_SECONDS },
      });
      gain = context.createGain();
      gain.gain.value = 0;
      node.port.onmessage = ({ data }) => {
        if (settled || stopped) return;
        if (data?.type === "samples" && data.samples instanceof Float32Array) chunks.push(data.samples);
        if (data?.type === "done") {
          settled = true;
          cleanup();
          resolve(encodePcm16Wav(chunks, context.sampleRate));
        }
      };
      node.onprocessorerror = () => fail(new Error("Audio recording failed."));
      source.connect(node).connect(gain).connect(context.destination);
      clearTimeout(timer);
      timer = setTimeout(() => fail(new Error("Audio recording timed out.")), (RECORDING_SECONDS + 2) * 1_000);
    } catch (error) { fail(error); }
  })();
  return { result, stop };
}
