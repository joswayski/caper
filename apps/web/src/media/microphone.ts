import { NoiseAssets } from "./noise-assets.ts";
import { DpdfnetPreparation } from "./dpdfnet-preparation.ts";

export type NoiseSuppression = "deepfilter" | "deepfilter-gentle" | "deepfilter-strong" | "rnnoise" | "dpdfnet8" | "browser" | "off";
export type AudioSetup = "speakers" | "headphones";
export type VoiceEnhancement = "natural" | "enhanced";
export interface Microphone {
  track: MediaStreamTrack;
  status: string;
  setInputVolume(volume: number): void;
  setVoiceEnhancement(mode: VoiceEnhancement): void;
  pause(): Promise<void>;
  resume(): Promise<void>;
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
  inputVolume = 100,
  voiceEnhancement: VoiceEnhancement = "enhanced",
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
  let gain: GainNode | undefined;
  let destination: MediaStreamAudioDestinationNode | undefined;
  let node: AudioWorkletNode | undefined;
  let highpass: BiquadFilterNode | undefined;
  let warmth: BiquadFilterNode | undefined;
  let presence: BiquadFilterNode | undefined;
  let compressor: DynamicsCompressorNode | undefined;
  let makeup: GainNode | undefined;
  let limiter: DynamicsCompressorNode | undefined;
  let prepared: ReturnType<DpdfnetPreparation["take"]> | undefined;
  let stopped = false;
  let bypassing = false;
  const connectVoicePath = (mode: VoiceEnhancement) => {
    if (!node || !destination) return;
    node.disconnect();
    highpass?.disconnect();
    warmth?.disconnect();
    presence?.disconnect();
    compressor?.disconnect();
    makeup?.disconnect();
    limiter?.disconnect();
    if (mode === "natural" || !highpass || !warmth || !presence || !compressor || !makeup || !limiter) {
      node.connect(destination);
      return;
    }
    node.connect(highpass).connect(warmth).connect(presence).connect(compressor).connect(makeup).connect(limiter).connect(destination);
  };
  const microphone: Microphone = {
    track: raw,
    status: "Noise suppression off",
    setInputVolume(volume) {
      const value = Math.max(0, Math.min(volume, 200)) / 100;
      if (gain) gain.gain.setValueAtTime(value, gain.context.currentTime);
    },
    setVoiceEnhancement(mode) {
      voiceEnhancement = mode;
      connectVoicePath(mode);
    },
    async pause() {
      if (!stopped && context?.state === "running") await context.suspend();
    },
    async resume() {
      if (!stopped && context?.state === "suspended") await context.resume();
    },
    stop() {
      if (stopped) return;
      stopped = true;
      signal.removeEventListener("abort", microphone.stop);
      stream.getTracks().forEach((track) => track.stop());
      destination?.stream.getTracks().forEach((track) => track.stop());
      source?.disconnect();
      gain?.disconnect();
      node?.port.postMessage("stop");
      node?.disconnect();
      node?.port.close();
      highpass?.disconnect();
      warmth?.disconnect();
      presence?.disconnect();
      compressor?.disconnect();
      makeup?.disconnect();
      limiter?.disconnect();
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
      : "Browser suppression unavailable - noise suppression off";
    return microphone;
  }
  const engine = mode === "rnnoise" ? "rnnoise" : mode === "dpdfnet8" ? "dpdfnet8" : "deepfilter";
  const engineName = engine === "rnnoise" ? "RNNoise" : engine === "dpdfnet8" ? "DPDFNet-8 HR" : "DeepFilterNet";
  const attenuationLimit = mode === "deepfilter-gentle" ? 12 : mode === "deepfilter-strong" ? 40 : 20;
  const presetName = mode === "deepfilter-gentle" ? "gentle" : mode === "deepfilter-strong" ? "strong" : "balanced";

  const fail = () => {
    if (stopped) return;
    microphone.status = `${engineName} failed - microphone stopped`;
    microphone.stop();
    changed();
  };
  const bypass = () => {
    if (stopped || bypassing) return;
    bypassing = true;
    prepared?.stop();
    prepared = undefined;
    microphone.status = `${engineName} unavailable - switching to browser suppression`;
    changed();
    void raw.applyConstraints({ noiseSuppression: true }).catch(() => undefined).then(() => {
      if (stopped) return;
      microphone.status = raw.getSettings().noiseSuppression
        ? `${engineName} unavailable · browser suppression active`
        : `${engineName} unavailable - noise suppression bypassed`;
      changed();
    });
  };

  try {
    if (typeof AudioContext === "undefined" || typeof AudioWorkletNode === "undefined") throw new Error("Unsupported browser");
    context = new AudioContext({ sampleRate: 48_000, latencyHint: "interactive" });
    if (context.sampleRate !== 48_000 || !context.audioWorklet) throw new Error("Unsupported audio context");
    // Resume immediately, before downloads, to retain the Join button's user activation.
    void context.resume().catch(() => undefined);
    source = context.createMediaStreamSource(stream);
    gain = context.createGain();
    destination = context.createMediaStreamDestination();
    destination.channelCount = 1;
    microphone.track = destination.stream.getAudioTracks()[0];
    microphone.setInputVolume(inputVolume);
    highpass = context.createBiquadFilter();
    highpass.type = "highpass";
    highpass.frequency.value = 75;
    highpass.Q.value = 0.7;
    warmth = context.createBiquadFilter();
    warmth.type = "lowshelf";
    warmth.frequency.value = 180;
    warmth.gain.value = 2;
    presence = context.createBiquadFilter();
    presence.type = "peaking";
    presence.frequency.value = 3_000;
    presence.Q.value = 0.8;
    presence.gain.value = 1.5;
    compressor = context.createDynamicsCompressor();
    compressor.threshold.value = -24;
    compressor.knee.value = 12;
    compressor.ratio.value = 3;
    compressor.attack.value = 0.008;
    compressor.release.value = 0.18;
    makeup = context.createGain();
    makeup.gain.value = 1.35;
    limiter = context.createDynamicsCompressor();
    limiter.threshold.value = -2;
    limiter.knee.value = 0;
    limiter.ratio.value = 20;
    limiter.attack.value = 0.002;
    limiter.release.value = 0.08;
    const { module, model } = engine === "dpdfnet8" ? { module: undefined, model: undefined } : await assets.load(engine, signal);
    await context.audioWorklet.addModule(engine === "dpdfnet8" ? "/audio/dpdfnet8-v2/worklet.js" : "/audio/noise-v1/worklet.js");
    signal.throwIfAborted();
    node = new AudioWorkletNode(context, engine === "dpdfnet8" ? "caper-dpdfnet8" : "caper-noise", {
      numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
      channelCount: 1, channelCountMode: "explicit",
      processorOptions: { engine, module, model, attenuationLimit },
    });
    if (engine === "dpdfnet8") {
      prepared = dpdfnet.take();
    }
    await new Promise<void>((resolve, reject) => {
      // ORT's first model compile is substantially slower than the small WASM engines.
      const timer = setTimeout(() => finish(new Error("Noise suppression timed out")), engine === "dpdfnet8" ? 60_000 : 15_000);
      const abort = () => finish(new Error("Microphone setup cancelled"));
      const finish = (error?: Error) => {
        clearTimeout(timer);
        signal.removeEventListener("abort", abort);
        error ? reject(error) : resolve();
      };
      if (prepared) void prepared.ready.then(() => finish(), finish);
      signal.addEventListener("abort", abort, { once: true });
      node!.onprocessorerror = () => finish(new Error("Noise suppression failed"));
      node!.port.onmessage = ({ data }) => {
        if (data !== "ready") finish(new Error("Noise suppression failed"));
        else if (!prepared) finish(); // DPDFNet must wait for its worker, not the worklet.
      };
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
    source.connect(gain).connect(node);
    connectVoicePath(voiceEnhancement);
    microphone.status = engine === "rnnoise" ? "RNNoise active · on-device" : engine === "dpdfnet8" ? `${engineName} active · on-device` : `DeepFilterNet active · ${presetName} · on-device`;
    node.onprocessorerror = fail;
    node.port.onmessage = ({ data }) => { if (data === "bypassed") bypass(); else if (data === "failed") fail(); };
    return microphone;
  } catch {
    microphone.stop();
    signal.throwIfAborted();
    throw new Error(`${engineName} could not start. New microphone audio was not enabled. Please try again.`);
  }
}
