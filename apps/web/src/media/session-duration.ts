/** Elapsed time from the server's continuous occupied voice session. */
export function sessionDuration(startedAt: number, now: number) {
  const seconds = Math.max(0, Math.floor((now - startedAt) / 1_000));
  const minutes = Math.floor(seconds / 60);
  const tail = `${String(minutes % 60).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
  return seconds < 3_600 ? tail : `${Math.floor(seconds / 3_600)}:${tail}`;
}
