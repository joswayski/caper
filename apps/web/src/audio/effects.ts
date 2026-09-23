export type SoundEffect =
  | "toggle-off"
  | "toggle-on"
  | "slider-tick"
  | "channel-leave"
  | "warning"
  | "channel-join"
  | "new-message"
  | "delete";

const MAX_VOICES = 4;
const MAX_DEFERRED_PLAY_MS = 120;
const effects: readonly SoundEffect[] = [
  "toggle-off", "toggle-on", "slider-tick", "channel-leave", "warning", "channel-join", "new-message", "delete",
];
const buffers = new Map<SoundEffect, Promise<AudioBuffer>>();
const active: AudioBufferSourceNode[] = [];
let context: AudioContext | undefined;

function now() {
  return typeof performance === "undefined" ? Date.now() : performance.now();
}

function getContext() {
  if (context || typeof AudioContext === "undefined") return context;
  try {
    context = new AudioContext({ latencyHint: "interactive" });
    return context;
  } catch {
    return undefined;
  }
}

function load(effect: SoundEffect, audioContext: AudioContext) {
  const cached = buffers.get(effect);
  if (cached) return cached;
  const loading = fetch(`/audio/effects/${effect}.wav`)
    .then((response) => {
      if (!response.ok) throw new Error(`Sound effect request failed (${response.status})`);
      return response.arrayBuffer();
    })
    .then((data) => audioContext.decodeAudioData(data));
  buffers.set(effect, loading);
  void loading.catch(() => {
    if (buffers.get(effect) === loading) buffers.delete(effect);
  });
  return loading;
}

function start(audioContext: AudioContext, buffer: AudioBuffer, volume: number, playbackRate: number) {
  while (active.length >= MAX_VOICES) active.shift()?.stop();
  const source = audioContext.createBufferSource();
  const gain = audioContext.createGain();
  source.buffer = buffer;
  source.playbackRate.value = Math.max(0.5, Math.min(2, playbackRate));
  gain.gain.value = Math.max(0, Math.min(1, volume));
  source.connect(gain).connect(audioContext.destination);
  active.push(source);
  source.addEventListener("ended", () => {
    const index = active.indexOf(source);
    if (index !== -1) active.splice(index, 1);
    source.disconnect();
    gain.disconnect();
  }, { once: true });
  source.start();
}

/** Fetch and decode all effects ahead of interaction. Safe to call during browser mount. */
export async function preloadSoundEffects() {
  const audioContext = getContext();
  if (!audioContext || typeof fetch === "undefined") return;
  await Promise.allSettled(effects.map((effect) => load(effect, audioContext)));
}

/** Play a short UI sound without allowing audio policy or decode failures to escape. */
export function playSound(effect: SoundEffect, options: { volume?: number; playbackRate?: number } = {}) {
  const audioContext = getContext();
  if (!audioContext || typeof fetch === "undefined") {
    if (typeof Audio === "undefined") return;
    try {
      const fallback = new Audio(`/audio/effects/${effect}.wav`);
      fallback.volume = Math.max(0, Math.min(1, options.volume ?? 0.45));
      fallback.playbackRate = Math.max(0.5, Math.min(2, options.playbackRate ?? 1));
      fallback.preservesPitch = false;
      void fallback.play().catch(() => undefined);
    } catch { /* Decorative fallback. */ }
    return;
  }
  try {
    // Calling resume directly in the input handler preserves browser gesture activation.
    const resumed = audioContext.state === "running" ? Promise.resolve() : audioContext.resume();
    const requestedAt = now();
    void Promise.all([load(effect, audioContext), resumed]).then(([buffer]) => {
      // A slow network/decode must not turn old interactions into a burst of late sounds.
      if (audioContext.state !== "running" || now() - requestedAt > MAX_DEFERRED_PLAY_MS) return;
      start(audioContext, buffer, options.volume ?? 0.45, options.playbackRate ?? 1);
    }).catch(() => undefined);
  } catch {
    // Sound effects are decorative and must never interrupt the interaction.
  }
}

let lastSliderTick = Number.NEGATIVE_INFINITY;

/** Slider feedback rises in both pitch and loudness, capped to avoid noisy drags. */
export function playSliderTick(normalizedValue: number) {
  const timestamp = now();
  if (timestamp - lastSliderTick < 40) return;
  lastSliderTick = timestamp;
  const normalized = Math.max(0, Math.min(1, normalizedValue));
  playSound("slider-tick", {
    playbackRate: 0.75 + normalized * 0.6,
    volume: 0.1 + normalized * 0.22,
  });
}
