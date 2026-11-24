/** Message counters appear at 3,000 characters and warm toward the 4,000 limit. */
export const COUNTER_START = 3000;

/** Muted gray at 3,000 that warms gradually through gold and orange to red near 4,000. */
const RAMP: readonly (readonly [number, string])[] = [
  [COUNTER_START, "var(--text-muted)"],
  [3400, "var(--pin-gold)"],
  [3700, "#eda361"],
  [3950, "#ff827c"],
];

export function counterColor(count: number) {
  for (let index = 1; index < RAMP.length; index++) {
    const [from, start] = RAMP[index - 1];
    const [to, end] = RAMP[index];
    if (count <= from) return start;
    if (count < to)
      return `color-mix(in oklab, ${start}, ${end} ${(((count - from) / (to - from)) * 100).toFixed(1)}%)`;
  }
  return RAMP[RAMP.length - 1][1];
}
