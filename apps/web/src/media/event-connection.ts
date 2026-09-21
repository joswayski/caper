import { CallEvents } from "./events.ts";
import type { CallSnapshot } from "./types.ts";

/** One logical subscription, with at most two streams during planned handoff.
 * Ordinary outage recovery and call readiness remain owned by the caller. */
export class EventConnection {
  private active?: CallEvents;
  private candidate?: CallEvents;
  private owner?: AbortSignal;
  private token?: string;
  private stopped = false;
  private revision?: number;
  private retries = 0;
  private retryTimer?: ReturnType<typeof setTimeout>;
  private snapshotTimer?: ReturnType<typeof setTimeout>;
  private readonly changed: () => void;
  private readonly lost: (error: Error, draining: boolean) => void;
  private readonly snapshot: (value: CallSnapshot & { revision?: number }) => void;
  private readonly draining: () => void;

  constructor(
    changed: () => void,
    lost: (error: Error, draining: boolean) => void,
    snapshot: (value: CallSnapshot & { revision?: number }) => void,
    draining: () => void,
  ) {
    this.changed = changed;
    this.lost = lost;
    this.snapshot = snapshot;
    this.draining = draining;
  }

  get connected() { return this.active?.connected ?? false; }

  open(token: string, signal: AbortSignal) { return this.start(token, signal); }
  openPresence(signal: AbortSignal) { return this.start(undefined, signal); }

  private start(token: string | undefined, signal: AbortSignal) {
    this.owner = signal;
    this.token = token;
    signal.addEventListener("abort", this.stop, { once: true });
    if (signal.aborted) { this.stop(); return Promise.reject(signal.reason); }
    this.active = this.stream();
    return this.openStream(this.active);
  }

  private openStream(stream: CallEvents) {
    return this.token === undefined ? stream.openPresence(this.owner!) : stream.open(this.token, this.owner!);
  }

  private stream(): CallEvents {
    const current = new CallEvents(
      () => { if (!this.stopped && this.active === current) this.changed(); },
      (error, draining) => this.failed(current, error, draining),
      (value) => {
        if (this.stopped || (this.active !== current && this.candidate !== current)) return;
        // The old stream can advance while the candidate snapshot is in flight.
        if (this.revision !== undefined && (value.revision === undefined || value.revision < this.revision)) return;
        if (this.candidate === current) {
          const previous = this.active;
          this.active = current;
          this.candidate = undefined;
          this.clearTimers();
          this.retries = 0;
          previous?.stop();
        }
        this.revision = value.revision;
        this.snapshot(value);
      },
      () => this.failed(current, new Error("Live updates are draining."), true),
      () => {
        if (this.stopped) return;
        if (this.candidate === current) {
          this.failed(current, new Error("Replacement is also draining."), true);
        } else if (this.active === current && !this.candidate && this.retryTimer === undefined) {
          this.replace();
        }
      },
    );
    return current;
  }

  private replace() {
    this.retryTimer = undefined;
    if (this.stopped || this.candidate) return;
    const next = this.candidate = this.stream();
    // A ready frame alone is not a handoff. Bound a candidate with no current
    // snapshot so it cannot consume the old server's entire 10s serving window.
    this.snapshotTimer = setTimeout(() => this.failed(next, new Error("Replacement snapshot timed out."), false), 5_000);
    void this.openStream(next).catch(() => { /* The lost callback handles failures. */ });
  }

  private failed(current: CallEvents, error: Error, draining: boolean) {
    if (this.stopped) return;
    if (this.candidate === current) {
      current.stop();
      this.candidate = undefined;
      clearTimeout(this.snapshotTimer);
      this.snapshotTimer = undefined;
      if (this.active?.connected) {
        const attempt = this.retries++;
        const delay = draining && attempt < 10 ? 50 : Math.min(250 * 2 ** Math.min(attempt, 4), 3_000);
        this.retryTimer = setTimeout(() => this.replace(), delay);
        return;
      }
    } else if (this.active === current) {
      // A replacement already opening may succeed even if the old pod's drain
      // deadline expires. Its snapshot watchdog bounds this wait.
      if (this.candidate) return;
      this.clearTimers();
    } else return;
    if (draining) this.draining();
    else this.lost(error, false);
  }

  private clearTimers() {
    clearTimeout(this.retryTimer);
    clearTimeout(this.snapshotTimer);
    this.retryTimer = undefined;
    this.snapshotTimer = undefined;
  }

  stop = () => {
    this.stopped = true;
    this.clearTimers();
    this.active?.stop();
    this.candidate?.stop();
    this.owner?.removeEventListener("abort", this.stop);
  };
}
