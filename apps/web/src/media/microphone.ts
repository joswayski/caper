import { NoiseAssets } from "./noise-assets.ts";
import { DpdfnetPreparation } from "./dpdfnet-preparation.ts";

export type NoiseSuppression = "deepfilter" | "deepfilter-gentle" | "deepfilter-strong" | "rnnoise" | "dpdfnet2" | "dpdfnet8" | "browser" | "off";
export type AudioSetup = "speakers" | "headphones";
export interface Microphone {
  track: MediaStreamTrack;
  status: string;
  stop(): void;
}

/** Owns both hardware capture and the processed track; never sends PCM over IPC. */
export async function captureMicrophone(
  deviceId: string | undefined,
  mode: NoiseSuppression,
  signal: AbortSignal,
  changed: () => void,
  audioSetup: AudioSetup = "speakers",
  assets = new NoiseAssets(),
  dpdfnet = new DpdfnetPreparation(),
): Promise<Microphone> {
  signal.throwIfAborted();
  const stream = await navigator.mediaDevices.getUserMedia({ audio: {
    deviceId: deviceId ? { exact: deviceId } : undefined,
    channelCount: 1,
    echoCancellation: audioSetup === "speakers",
    noiseSuppression: mode === "browser",
    autoGainControl: audioSetup === "speakers",
  } });
  const raw = stream.getAudioTracks()[0];
  let context: AudioContext | undefined;
  let source: MediaStreamAudioSourceNode | undefined;
  let destination: MediaStreamAudioDestinationNode | undefined;
  let node: AudioWorkletNode | undefined;
  let prepared: ReturnType<DpdfnetPreparation["take"]> | undefined;
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
      prepared?.stop();
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

  const engine = mode === "rnnoise" ? "rnnoise" : mode === "dpdfnet2" || mode === "dpdfnet8" ? "dpdfnet2" : "deepfilter";
  const engineName = engine === "rnnoise" ? "RNNoise" : engine === "dpdfnet2" ? (mode === "dpdfnet8" ? "DPDFNet-8 HR" : "DPDFNet-2 HR") : "DeepFilterNet";
  const attenuationLimit = mode === "deepfilter-gentle" ? 12 : mode === "deepfilter-strong" ? 40 : 20;
  const presetName = mode === "deepfilter-gentle" ? "gentle" : mode === "deepfilter-strong" ? "strong" : "balanced";

  const fail = () => {
    if (stopped) return;
    microphone.status = `${engineName} failed — microphone stopped`;
    microphone.stop();
    changed();
  };

  try {
    if (typeof AudioContext === "undefined" || typeof AudioWorkletNode === "undefined") throw new Error("Unsupported browser");
    context = new AudioContext({ sampleRate: 48_000, latencyHint: "interactive" });
    if (context.sampleRate !== 48_000 || !context.audioWorklet) throw new Error("Unsupported audio context");
    // Resume immediately, before downloads, to retain the Join button's user activation.
    void context.resume().catch(() => undefined);
    const { module, model } = engine === "dpdfnet2" ? { module: undefined, model: undefined } : await assets.load(engine, signal);
    await context.audioWorklet.addModule(engine === "dpdfnet2" ? "/audio/dpdfnet2-v1/worklet.js" : "/audio/noise-v1/worklet.js");
    signal.throwIfAborted();
    node = new AudioWorkletNode(context, engine === "dpdfnet2" ? "caper-dpdfnet2" : "caper-noise", {
      numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
      channelCount: 1, channelCountMode: "explicit",
      processorOptions: { engine, module, model, attenuationLimit },
    });
    if (engine === "dpdfnet2") {
      prepared = dpdfnet.take(mode === "dpdfnet8" ? "dpdfnet8" : "dpdfnet2");
    }
    await new Promise<void>((resolve, reject) => {
      // ORT's first model compile is substantially slower than the small WASM engines.
      const timer = setTimeout(() => finish(new Error("Noise suppression timed out")), engine === "dpdfnet2" ? 60_000 : 15_000);
      const abort = () => finish(new Error("Microphone setup cancelled"));
      const finish = (error?: Error) => {
        clearTimeout(timer);
        signal.removeEventListener("abort", abort);
        error ? reject(error) : resolve();
      };
      if (prepared) void prepared.ready.then(() => finish(), finish);
      signal.addEventListener("abort", abort, { once: true });
      node!.onprocessorerror = () => finish(new Error("Noise suppression failed"));
      node!.port.onmessage = ({ data }) => finish(data === "ready" ? undefined : new Error("Noise suppression failed"));
    });
    // The context may be blocked by autoplay policy. Do not hang joining forever.
    if (context.state !== "running") throw new Error("Audio context did not start");
    signal.throwIfAborted();
    if (prepared) {
      const worker = prepared.worker;
      worker.onmessage = ({ data }) => {
        if (data?.type === "output") node?.port.postMessage(data, [data.samples]);
        else node?.port.postMessage({ type: "failed" });
      };
      worker.onerror = () => node?.port.postMessage({ type: "failed" });
      node.port.addEventListener("message", ({ data }) => {
        if (data?.type === "process") worker.postMessage(data, [data.samples]);
      });
      node.port.start();
    }
    source = context.createMediaStreamSource(stream);
    destination = context.createMediaStreamDestination();
    destination.channelCount = 1;
    source.connect(node);
    node.connect(destination);
    microphone.track = destination.stream.getAudioTracks()[0];
    microphone.status = engine === "rnnoise" ? "RNNoise active · on-device" : engine === "dpdfnet2" ? `${engineName} active · experimental · on-device` : `DeepFilterNet active · ${presetName} · on-device`;
    node.onprocessorerror = fail;
    node.port.onmessage = ({ data }) => { if (data === "failed") fail(); };
    return microphone;
  } catch {
    microphone.stop();
    signal.throwIfAborted();
    throw new Error(`${engineName} could not start. New microphone audio was not enabled. Please try again.`);
  }
}
