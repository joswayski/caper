/** Composer counters appear at 3,000 characters and warm up toward the 4,000 limit. */
export const COUNTER_START = 3000;

export function counterTone(count: number) {
  return count >= 3900 ? "red" : count >= 3750 ? "orange" : count >= 3500 ? "yellow" : "gray";
}
