export type NoiseSuppression = "deepfilter" | "off";
export interface Microphone {
  track: MediaStreamTrack;
  status: string;
  stop(): void;
}

const ASSETS = "/audio/deepfilter-v1";

async function loadAsset(name: string, signal: AbortSignal) {
  const response = await fetch(`${ASSETS}/${name}`, { signal });
  if (!response.ok) throw new Error("Noise suppression download failed.");
  return response.arrayBuffer();
}

/** Owns both hardware capture and the processed track; never sends PCM over IPC. */
export async function captureMicrophone(
  deviceId: string | undefined,
  mode: NoiseSuppression,
  signal: AbortSignal,
  changed: () => void,
): Promise<Microphone> {
  signal.throwIfAborted();
  const stream = await navigator.mediaDevices.getUserMedia({ audio: {
    deviceId: deviceId ? { exact: deviceId } : undefined,
    channelCount: 1,
    echoCancellation: true,
    noiseSuppression: false,
    autoGainControl: true,
  } });
  const raw = stream.getAudioTracks()[0];
  let context: AudioContext | undefined;
  let source: MediaStreamAudioSourceNode | undefined;
  let destination: MediaStreamAudioDestinationNode | undefined;
  let node: AudioWorkletNode | undefined;
  let stopped = false;
  const microphone: Microphone = {
    track: raw,
    status: "Noise suppression off",
    stop() {
      if (stopped) return;
      stopped = true;
      signal.removeEventListener("abort", microphone.stop);
      stream.getTracks().forEach((track) => track.stop());
      destination?.stream.getTracks().forEach((track) => track.stop());
      source?.disconnect();
      node?.port.postMessage("stop");
      node?.disconnect();
      node?.port.close();
      if (context && context.state !== "closed") void context.close().catch(() => undefined);
    },
  };
  signal.addEventListener("abort", microphone.stop, { once: true });
  if (signal.aborted || !raw) {
    microphone.stop();
    signal.throwIfAborted();
    throw new Error("No microphone track was available.");
  }
  if (mode === "off") return microphone;

  const fallback = async () => {
    if (stopped) return;
    if (node) {
      node.onprocessorerror = null;
      node.port.onmessage = null;
      node.port.postMessage("stop");
    }
    node?.disconnect();
    source?.disconnect();
    // Preserve the outgoing track, including its mute state, after a worklet failure.
    if (destination) source?.connect(destination);
    microphone.status = "DeepFilterNet unavailable — noise suppression off";
    try {
      await raw.applyConstraints({ noiseSuppression: true });
      if (raw.getSettings().noiseSuppression) microphone.status = "DeepFilterNet unavailable — browser suppression";
    } catch { /* Keep working audio, without pretending enhanced suppression is active. */ }
    if (!stopped) changed();
  };

  try {
    if (typeof AudioContext === "undefined" || typeof AudioWorkletNode === "undefined") throw new Error("Unsupported browser");
    context = new AudioContext({ sampleRate: 48_000, latencyHint: "interactive" });
    if (context.sampleRate !== 48_000 || !context.audioWorklet) throw new Error("Unsupported audio context");
    // Resume immediately, before downloads, to retain the Join button's user activation.
    void context.resume().catch(() => undefined);
    const loadingSignal = AbortSignal.any([signal, AbortSignal.timeout(30_000)]);
    const [bytes, model] = await Promise.all([
      loadAsset("df_bg.wasm", loadingSignal),
      loadAsset("DeepFilterNet3.bin", loadingSignal),
    ]);
    const module = await WebAssembly.compile(bytes);
    await context.audioWorklet.addModule(`${ASSETS}/worklet.js`);
    signal.throwIfAborted();
    node = new AudioWorkletNode(context, "caper-deepfilter", {
      numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
      channelCount: 1, channelCountMode: "explicit",
      processorOptions: { module, model },
    });
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => finish(new Error("Noise suppression timed out")), 15_000);
      const abort = () => finish(new Error("Microphone setup cancelled"));
      const finish = (error?: Error) => {
        clearTimeout(timer);
        signal.removeEventListener("abort", abort);
        error ? reject(error) : resolve();
      };
      signal.addEventListener("abort", abort, { once: true });
      node!.onprocessorerror = () => finish(new Error("Noise suppression failed"));
      node!.port.onmessage = ({ data }) => finish(data === "ready" ? undefined : new Error("Noise suppression failed"));
    });
    // The context may be blocked by autoplay policy. Do not hang joining forever.
    if (context.state !== "running") throw new Error("Audio context did not start");
    signal.throwIfAborted();
    source = context.createMediaStreamSource(stream);
    destination = context.createMediaStreamDestination();
    destination.channelCount = 1;
    source.connect(node);
    node.connect(destination);
    microphone.track = destination.stream.getAudioTracks()[0];
    microphone.status = "DeepFilterNet active · on-device";
    node.onprocessorerror = () => void fallback();
    node.port.onmessage = ({ data }) => { if (data === "failed") void fallback(); };
    return microphone;
  } catch {
    if (signal.aborted) { microphone.stop(); signal.throwIfAborted(); }
    node?.port.postMessage("stop");
    node?.disconnect();
    node?.port.close();
    if (context && context.state !== "closed") await context.close().catch(() => undefined);
    await fallback();
    return microphone;
  }
}
