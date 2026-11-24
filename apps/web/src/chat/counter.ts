/** Message counters appear at 3,000 characters and warm toward the 4,000 limit. */
export const COUNTER_START = 3000;

/**
 * Colors along the eased warm-up: muted gray at 0, gold, orange, red at 1.
 * Every client uses the same stops and curve.
 */
const RAMP: readonly (readonly [number, string])[] = [
  [0, "var(--text-muted)"],
  [0.35, "var(--pin-gold)"],
  [0.65, "#eda361"],
  [1, "#ff827c"],
];

/** 0 at 3,000, 1 at 3,950. Squared, so it stays near gray, then hurries toward red. */
export function counterWarmth(count: number) {
  const linear = Math.min(1, Math.max(0, (count - COUNTER_START) / 950));
  return linear * linear;
}

export function counterColor(count: number) {
  const warmth = counterWarmth(count);
  for (let index = 1; index < RAMP.length; index++) {
    const [from, start] = RAMP[index - 1];
    const [to, end] = RAMP[index];
    if (warmth <= from) return start;
    if (warmth < to)
      return `color-mix(in oklab, ${start}, ${end} ${(((warmth - from) / (to - from)) * 100).toFixed(1)}%)`;
  }
  return RAMP[RAMP.length - 1][1];
}
