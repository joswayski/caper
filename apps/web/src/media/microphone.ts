export const NOISE_SUPPRESSION_OPTIONS = [
  { value: "deepfilter", label: "DeepFilterNet · balanced", description: "Recommended starting point. Less aggressive than the original setting; a little background sound may remain." },
  { value: "deepfilter-gentle", label: "DeepFilterNet · gentle", description: "Keeps more of your original voice, along with more room noise. Try this if quiet words or laughter get cut off." },
  { value: "deepfilter-strong", label: "DeepFilterNet · strong", description: "The original suppression strength. A quieter background, with more risk of changing your voice." },
  { value: "rnnoise", label: "RNNoise · lightweight", description: "A different on-device model with lower processing cost. Compare it with DeepFilterNet for fans and AC noise." },
  { value: "browser", label: "Browser suppression", description: "Your browser’s built-in filter. Quality and availability depend on the browser and device." },
  { value: "off", label: "Off", description: "No requested noise suppression. Echo cancellation and automatic microphone level stay on in every mode." },
] as const;
export type NoiseSuppression = typeof NOISE_SUPPRESSION_OPTIONS[number]["value"];
export interface Microphone {
  track: MediaStreamTrack;
  status: string;
  stop(): void;
}

async function loadAsset(url: string, signal: AbortSignal) {
  const response = await fetch(url, { signal });
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
    noiseSuppression: mode === "browser",
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
  if (mode === "browser") {
    microphone.status = raw.getSettings().noiseSuppression
      ? "Browser suppression active"
      : "Browser suppression unavailable — noise suppression off";
    return microphone;
  }

  const engine = mode === "rnnoise" ? "rnnoise" : "deepfilter";
  const engineName = engine === "rnnoise" ? "RNNoise" : "DeepFilterNet";
  const attenuationLimit = mode === "deepfilter-gentle" ? 12 : mode === "deepfilter-strong" ? 40 : 20;
  const presetName = mode === "deepfilter-gentle" ? "gentle" : mode === "deepfilter-strong" ? "strong" : "balanced";

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
    microphone.status = `${engineName} unavailable — noise suppression off`;
    try {
      await raw.applyConstraints({ noiseSuppression: true });
      if (raw.getSettings().noiseSuppression) microphone.status = `${engineName} unavailable — browser suppression`;
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
      loadAsset(engine === "rnnoise" ? "/audio/rnnoise-v1/rnnoise.wasm" : "/audio/deepfilter-v1/df_bg.wasm", loadingSignal),
      engine === "deepfilter" ? loadAsset("/audio/deepfilter-v1/DeepFilterNet3.bin", loadingSignal) : undefined,
    ]);
    const module = await WebAssembly.compile(bytes);
    await context.audioWorklet.addModule("/audio/noise-v1/worklet.js");
    signal.throwIfAborted();
    node = new AudioWorkletNode(context, "caper-noise", {
      numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
      channelCount: 1, channelCountMode: "explicit",
      processorOptions: { engine, module, model, attenuationLimit },
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
    microphone.status = engine === "rnnoise" ? "RNNoise active · on-device" : `DeepFilterNet active · ${presetName} · on-device`;
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
