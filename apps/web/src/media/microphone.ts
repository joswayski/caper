import { NoiseAssets } from "./noise-assets.ts";
import { DpdfnetPreparation } from "./dpdfnet-preparation.ts";
import {
  clampVoiceProcessingStrength,
  connectVoiceProcessing,
  createVoiceProcessingNodes,
  DEFAULT_VOICE_PROCESSING_STRENGTH,
  disconnectVoiceProcessing,
  setVoiceProcessingStrength,
  type VoiceProcessingNodes,
} from "./voice-processing.ts";

export type NoiseSuppression = "deepfilter" | "deepfilter-gentle" | "deepfilter-strong" | "rnnoise" | "dpdfnet8" | "browser" | "off";
export type AudioSetup = "speakers" | "headphones";
export interface Microphone {
  track: MediaStreamTrack;
  naturalTrack: MediaStreamTrack;
  status: string;
  diagnostics(): object;
  setInputVolume(volume: number): void;
  setVoiceProcessingStrength(strength: number): void;
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
  voiceProcessingStrength = DEFAULT_VOICE_PROCESSING_STRENGTH,
): Promise<Microphone> {
  signal.throwIfAborted();
  const stream = await navigator.mediaDevices.getUserMedia({ audio: {
    deviceId: deviceId ? { exact: deviceId } : undefined,
    channelCount: 1,
    echoCancellation: audioSetup === "speakers",
    noiseSuppression: mode === "browser",
    autoGainControl: audioSetup === "speakers",
  } });
  if (!stream) throw new Error("Microphone access was not granted.");
  const raw = stream.getAudioTracks()[0];
  let context: AudioContext | undefined;
  let source: MediaStreamAudioSourceNode | undefined;
  let gain: GainNode | undefined;
  let voiceInput: GainNode | undefined;
  let destination: MediaStreamAudioDestinationNode | undefined;
  let naturalDestination: MediaStreamAudioDestinationNode | undefined;
  let node: AudioWorkletNode | undefined;
  let voiceProcessing: VoiceProcessingNodes | undefined;
  let prepared: ReturnType<DpdfnetPreparation["take"]> | undefined;
  const fallbackController = new AbortController();
  let stopped = false;
  let bypassing = false;
  type Profile = 8 | 2 | "rnnoise";
  let activeProfile: Profile = 8;
  // One retry per engine for the entire capture, shared by startup and crashes.
  const retried = new Set<Profile>();
  let processedHops = 0;
  let totalProcessingMs = 0;
  let maxProcessingMs = 0;
  let processingConnected: boolean | undefined;
  const connectVoicePath = (strength: number) => {
    if (!context || !voiceInput || !destination || !voiceProcessing) return;
    const nextStrength = clampVoiceProcessingStrength(strength);
    setVoiceProcessingStrength(voiceProcessing, nextStrength, context.currentTime, true);
    if (processingConnected === (nextStrength > 0)) return;
    voiceInput.disconnect();
    disconnectVoiceProcessing(voiceProcessing);
    processingConnected = nextStrength > 0;
    if (!processingConnected) {
      voiceInput.connect(destination);
      return;
    }
    connectVoiceProcessing(voiceInput, destination, voiceProcessing);
  };
  const microphone: Microphone = {
    track: raw,
    naturalTrack: raw,
    status: "Noise suppression off",
    diagnostics() {
      const { sampleRate, channelCount, echoCancellation, noiseSuppression, autoGainControl } = raw.getSettings();
      return {
        requested: mode, status: microphone.status, stopped,
        context: context?.state, contextSampleRate: context?.sampleRate,
        capture: { sampleRate, channelCount, echoCancellation, noiseSuppression, autoGainControl },
        dpdfnet: { profile: activeProfile, processedHops, meanProcessingMs: processedHops ? totalProcessingMs / processedHops : null, maxProcessingMs, hopBudgetMs: 10 },
      };
    },
    setInputVolume(volume) {
      const value = Math.max(0, Math.min(volume, 200)) / 100;
      if (gain) gain.gain.setValueAtTime(value, gain.context.currentTime);
    },
    setVoiceProcessingStrength(strength) {
      voiceProcessingStrength = clampVoiceProcessingStrength(strength);
      connectVoicePath(voiceProcessingStrength);
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
      fallbackController.abort();
      signal.removeEventListener("abort", microphone.stop);
      stream.getTracks().forEach((track) => track.stop());
      destination?.stream.getTracks().forEach((track) => track.stop());
      naturalDestination?.stream.getTracks().forEach((track) => track.stop());
      source?.disconnect();
      gain?.disconnect();
      voiceInput?.disconnect();
      node?.port.postMessage("stop");
      node?.disconnect();
      node?.port.close();
      if (voiceProcessing) disconnectVoiceProcessing(voiceProcessing);
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
  const bridgeWorker = (target: AudioWorkletNode, worker: Worker) => {
    worker.onmessage = ({ data }) => {
      if (stopped || node !== target) return;
      if (data?.type === "output") {
        processedHops++;
        totalProcessingMs += data.duration;
        maxProcessingMs = Math.max(maxProcessingMs, data.duration);
        target.port.postMessage(data, [data.samples]);
      } else bypass(true);
    };
    worker.onerror = () => { if (!stopped && node === target) bypass(true); };
    target.port.addEventListener("message", ({ data }) => {
      if (!stopped && node === target && data?.type === "process") worker.postMessage(data, [data.samples]);
    });
    target.port.start();
  };
  const nextProfile = (profile: Profile, retry: boolean): Profile | undefined => {
    if (retry && !retried.has(profile)) {
      retried.add(profile);
      return profile;
    }
    return profile === 8 ? 2 : profile === 2 ? "rnnoise" : undefined;
  };
  const useFallback = async (profile: Profile): Promise<boolean> => {
    let replacement: AudioWorkletNode | undefined;
    const fallbackSignal = fallbackController.signal;
    try {
      const { module } = profile !== "rnnoise" ? { module: undefined } : await assets.load("rnnoise", fallbackSignal);
      await context!.audioWorklet.addModule(profile !== "rnnoise" ? "/audio/dpdfnet8-v2/worklet-v3.js" : "/audio/noise-v1/worklet.js");
      fallbackSignal.throwIfAborted();
      replacement = new AudioWorkletNode(context!, profile !== "rnnoise" ? "caper-dpdfnet8" : "caper-noise", {
        numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [1],
        channelCount: 1, channelCountMode: "explicit", processorOptions: { engine: profile === "rnnoise" ? "rnnoise" : "dpdfnet8", module },
      });
      if (profile !== "rnnoise") prepared = new DpdfnetPreparation(profile).take();
      await new Promise<void>((resolve, reject) => {
        const finish = (error?: Error) => {
          clearTimeout(timer);
          fallbackSignal.removeEventListener("abort", abort);
          error ? reject(error) : resolve();
        };
        const abort = () => finish(new Error("Fallback cancelled"));
        const timer = setTimeout(() => finish(new Error("Fallback timed out")), profile !== "rnnoise" ? 60_000 : 15_000);
        fallbackSignal.addEventListener("abort", abort, { once: true });
        if (profile !== "rnnoise") void prepared!.ready.then(() => finish(), finish);
        replacement!.onprocessorerror = () => finish(new Error("Fallback failed"));
        replacement!.port.onmessage = ({ data }) => {
          if (data !== "ready") finish(new Error("Fallback failed"));
          else if (profile === "rnnoise") finish();
        };
      });
      fallbackSignal.throwIfAborted();
      // Complete the constraint change before connecting the new engine, so its
      // failure cannot race a stale constraint/status update from this transition.
      await raw.applyConstraints({ noiseSuppression: false }).catch(() => undefined);
      fallbackSignal.throwIfAborted();
      const previous = node;
      gain!.disconnect();
      previous?.disconnect();
      previous?.port.postMessage("stop");
      previous?.port.close();
      node = replacement;
      activeProfile = profile;
      processedHops = totalProcessingMs = maxProcessingMs = 0;
      if (profile !== "rnnoise") {
        bridgeWorker(node, prepared!.worker);
      }
      gain!.connect(node);
      node.connect(naturalDestination!);
      node.connect(voiceInput!);
      node.onprocessorerror = () => bypass(true);
      node.port.onmessage = ({ data }) => {
        if (data === "failed") bypass(true);
        else if (data === "bypassed") bypass();
      };
      bypassing = false;
      microphone.status = profile === 8 ? "DPDFNet-8 HR active · on-device" : profile === 2 ? "DPDFNet-8 HR unavailable · DPDFNet-2 HR active · on-device" : "DPDFNet unavailable · RNNoise active · on-device";
      changed();
      return true;
    } catch {
      prepared?.stop();
      prepared = undefined;
      if (replacement && replacement !== node) {
        replacement.port.postMessage("stop");
        replacement.disconnect();
        replacement.port.close();
        replacement = undefined;
      }
      if (!stopped) {
        const next = nextProfile(profile, true);
        if (next !== undefined) return await useFallback(next);
      }
      // Keep the existing browser fallback and its honest status if RNNoise fails.
      return false;
    } finally {
      if (replacement && replacement !== node) {
        replacement.port.postMessage("stop");
        replacement.disconnect();
        replacement.port.close();
      }
    }
  };
  const bypass = (retry = false) => {
    if (stopped || bypassing) return;
    bypassing = true;
    const next = engine === "dpdfnet8" ? nextProfile(activeProfile, retry) : undefined;
    if (activeProfile === "rnnoise" && next === undefined) {
      microphone.status = "RNNoise failed - microphone stopped";
      microphone.stop();
      changed();
      return;
    }
    prepared?.stop();
    prepared = undefined;
    // A processorerror silences a worklet permanently; do not leave it in the path.
    node!.onprocessorerror = null;
    node!.port.onmessage = null;
    node!.disconnect();
    gain!.disconnect();
    gain!.connect(naturalDestination!);
    gain!.connect(voiceInput!);
    microphone.status = `${engineName} unavailable - switching to browser suppression`;
    changed();
    void raw.applyConstraints({ noiseSuppression: true }).catch(() => undefined).then(() => {
      if (stopped) return;
      microphone.status = raw.getSettings().noiseSuppression
        ? `${engineName} unavailable · browser suppression active`
        : `${engineName} unavailable - noise suppression bypassed`;
      changed();
      if (next !== undefined) void useFallback(next);
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
    voiceInput = context.createGain();
    destination = context.createMediaStreamDestination();
    destination.channelCount = 1;
    naturalDestination = context.createMediaStreamDestination();
    naturalDestination.channelCount = 1;
    microphone.track = destination.stream.getAudioTracks()[0];
    microphone.naturalTrack = naturalDestination.stream.getAudioTracks()[0];
    microphone.setInputVolume(inputVolume);
    voiceProcessing = createVoiceProcessingNodes(context, voiceProcessingStrength);
    const { module, model } = engine === "dpdfnet8" ? { module: undefined, model: undefined } : await assets.load(engine, signal);
    await context.audioWorklet.addModule(engine === "dpdfnet8" ? "/audio/dpdfnet8-v2/worklet-v3.js" : "/audio/noise-v1/worklet.js");
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
      bridgeWorker(node, prepared.worker);
    }
    source.connect(gain).connect(node);
    node.connect(naturalDestination);
    node.connect(voiceInput);
    connectVoicePath(voiceProcessingStrength);
    microphone.status = engine === "rnnoise" ? "RNNoise active · on-device" : engine === "dpdfnet8" ? `${engineName} active · on-device` : `DeepFilterNet active · ${presetName} · on-device`;
    node.onprocessorerror = engine === "dpdfnet8" ? () => bypass(true) : fail;
    node.port.onmessage = ({ data }) => { if (data === "bypassed") bypass(); else if (data === "failed") { if (engine === "dpdfnet8") bypass(true); else fail(); } };
    return microphone;
  } catch {
    if (engine === "dpdfnet8" && !stopped && context?.state === "running" && gain && naturalDestination && voiceInput) {
      prepared?.stop();
      prepared = undefined;
      // Startup recovery stays disconnected until a local processor is ready.
      retried.add(8);
      if (await useFallback(8)) {
        source!.connect(gain);
        connectVoicePath(voiceProcessingStrength);
        return microphone;
      }
    }
    microphone.stop();
    signal.throwIfAborted();
    throw new Error(`${engineName} could not start. New microphone audio was not enabled. Please try again.`);
  }
}
