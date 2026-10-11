/** How long a row counts as arriving: long enough for its entrance, short enough
 * that a row the list remounts later (scrolled away and back) never replays it. */
export const ARRIVAL_MS = 1000;

type List = { key: string; ids: Set<string>; last?: string; live: boolean };

/**
 * Remembers which messages arrived while the reader watched the live end of a
 * list, so only those animate in. The first render of a channel or thread,
 * older pages, jumps into history and newer pages loaded towards the live end
 * all appear in place. A pending send keeps its client message ID once
 * confirmed, so it animates once.
 */
export class Arrivals {
  private lists = new Map<string, List>();
  private arrived = new Map<string, number>();

  constructor(private now: () => number = () => performance.now()) {}

  /**
   * Call on every render with the list's client message IDs, oldest first.
   * `live` is true when the list ends at the newest message. Both this and the
   * previous render must be live, so a newer page loaded on the way back to the
   * live end does not count.
   */
  observe(scope: string, key: string, ids: readonly string[], live: boolean) {
    const previous = this.lists.get(scope);
    const next: List = { key, ids: new Set(ids), last: ids.at(-1), live };
    this.lists.set(scope, next);
    if (!previous || previous.key !== key || !previous.last || !previous.live || !live) return;
    // Only rows after the previous newest row are arrivals; prepended history is not.
    const tail = ids.lastIndexOf(previous.last);
    if (tail < 0) return;
    const at = this.now();
    for (const id of ids.slice(tail + 1)) if (!previous.ids.has(id) && !this.arrived.has(id)) this.arrived.set(id, at);
    for (const [id, time] of this.arrived) if (at - time > ARRIVAL_MS) this.arrived.delete(id);
  }

  has(id: string) {
    const time = this.arrived.get(id);
    return time !== undefined && this.now() - time <= ARRIVAL_MS;
  }
}
