/** Touch drag math shared by Browse, the member drawer and the media viewer. */

/** How far a finger moves before a press becomes a drag, in pixels. */
export const SLOP = 10;
/** Finger speed, in pixels per millisecond, that throws a drag the way it moves. */
export const FLING = 0.3;
/** A fling must also have moved this far, so a twitch on lifting is not one. */
export const FLING_DISTANCE = 40;
/** Only this recent movement counts toward the release speed, in milliseconds. */
const WINDOW = 100;

/** Recent finger positions along one axis, for its speed when it lifts. */
export class Velocity {
  private samples: Array<{ time: number; position: number }> = [];

  add(time: number, position: number) {
    this.samples = this.samples.filter((sample) => time - sample.time <= WINDOW);
    this.samples.push({ time, position });
  }

  /** Pixels per millisecond over the last 100ms before `time`, so a pause before lifting is not a fling. */
  speed(time: number) {
    const recent = this.samples.filter((sample) => time - sample.time <= WINDOW);
    const first = recent[0],
      last = recent[recent.length - 1];
    return first && last && last.time - first.time >= 8
      ? (last.position - first.position) / (last.time - first.time)
      : 0;
  }
}

/** Positions from a move and the moves the browser coalesced into it, which keep their own times. */
export function movesOf(event: PointerEvent) {
  const moves = event.getCoalescedEvents?.() ?? [];
  return moves.length ? moves : [event];
}

/**
 * Whether a released drag settles at its far end (progress 1) rather than its
 * near end (0). A fling goes the way it is thrown, even back against the drag;
 * otherwise the nearer end wins. `speed` and `travelled` are in pixels toward
 * the far end.
 */
export function settlesForward(progress: number, speed: number, travelled: number) {
  if (Math.abs(speed) >= FLING && Math.abs(travelled) >= FLING_DISTANCE) return speed > 0;
  return progress >= 0.5;
}

export const clampProgress = (progress: number) => Math.min(1, Math.max(0, progress));
