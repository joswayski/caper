export type SoundEffect =
  | "toggle-off"
  | "toggle-on"
  | "slider-tick"
  | "channel-leave"
  | "warning"
  | "channel-join"
  | "new-message";

const MAX_VOICES = 4;
const active: HTMLAudioElement[] = [];

/** Play a short UI sound without allowing audio policy or decode failures to escape. */
export function playSound(effect: SoundEffect, options: { volume?: number; playbackRate?: number } = {}) {
  if (typeof Audio === "undefined") return;
  try {
    while (active.length >= MAX_VOICES) {
      const oldest = active.shift();
      oldest?.pause();
    }
    const audio = new Audio(`/audio/effects/${effect}.wav`);
    audio.preload = "auto";
    audio.volume = Math.max(0, Math.min(1, options.volume ?? 0.45));
    audio.playbackRate = Math.max(0.5, Math.min(2, options.playbackRate ?? 1));
    audio.preservesPitch = false;
    active.push(audio);
    const release = () => {
      const index = active.indexOf(audio);
      if (index !== -1) active.splice(index, 1);
    };
    audio.addEventListener("ended", release, { once: true });
    audio.addEventListener("error", release, { once: true });
    void audio.play().catch(release);
  } catch {
    // Sound effects are decorative and must never interrupt the interaction.
  }
}

let lastSliderTick = Number.NEGATIVE_INFINITY;

/** Slider feedback rises in both pitch and loudness, capped to avoid noisy drags. */
export function playSliderTick(normalizedValue: number) {
  const now = typeof performance === "undefined" ? Date.now() : performance.now();
  if (now - lastSliderTick < 40) return;
  lastSliderTick = now;
  const normalized = Math.max(0, Math.min(1, normalizedValue));
  playSound("slider-tick", {
    playbackRate: 0.75 + normalized * 0.6,
    volume: 0.1 + normalized * 0.22,
  });
}
