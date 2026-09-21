import type { CallSnapshot } from "./types.ts";

const START_TIMEOUT_MS = 10_000;
const HEARTBEAT_TIMEOUT_MS = 25_000;
const MAX_BUFFER = 64 * 1024;

/** One media event stream. Reconnection belongs to its call or spectator lifecycle. */
export class CallEvents {
  connected = false;
  private readonly controller = new AbortController();
  private stopped = false;
  private timer?: ReturnType<typeof setTimeout>;

  private readonly changed: () => void;
  private readonly lost: (error: Error, draining: boolean) => void;
  private readonly snapshot?: (snapshot: CallSnapshot & { revision?: number }) => void;
  private readonly draining?: () => void;
  constructor(changed: () => void, lost: (error: Error, draining: boolean) => void, snapshot?: (snapshot: CallSnapshot & { revision?: number }) => void, draining?: () => void) {
    this.changed = changed;
    this.lost = lost;
    this.snapshot = snapshot;
    this.draining = draining;
  }

  open(token: string, signal: AbortSignal): Promise<void> {
    return this.openStream(token, signal);
  }

  openPresence(signal: AbortSignal): Promise<void> {
    return this.openStream(undefined, signal);
  }

  private openStream(token: string | undefined, signal: AbortSignal): Promise<void> {
    if (signal.aborted) return Promise.reject(signal.reason);
    return new Promise<void>((resolve, reject) => {
      let draining = false;
      const abort = () => this.stop();
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) abort();
      const watchdog = (timeout: number) => {
        clearTimeout(this.timer);
        this.timer = setTimeout(() => this.controller.abort(new Error("Live updates timed out. Please try joining again.")), timeout);
      };
      watchdog(START_TIMEOUT_MS);
      const run = async () => {
        const response = await fetch(token === undefined ? "/api/media/presence/events" : "/api/media/events?snapshots=1", {
          headers: { accept: "text/event-stream", ...(token === undefined ? {} : { "x-caper-media-token": token }) },
          signal: this.controller.signal,
          cache: "no-store",
        });
        if (response.status === 503) {
          // Routing can briefly send a reconnect to a terminating pod. This
          // explicit admission rejection is not an ordinary stream outage.
          const body = await response.json().catch(() => undefined);
          draining = body?.code === "api_draining";
        }
        if (!response.ok || !response.body || !response.headers.get("content-type")?.startsWith("text/event-stream")) {
          throw new Error("Live updates are unavailable. Please try joining again.");
        }
        const reader = response.body.getReader();
        const decoder = new TextDecoder();
        let buffer = "";
        try {
          while (true) {
            const { done, value } = await reader.read();
            if (done) throw new Error("Live updates disconnected. Please try joining again.");
            this.controller.signal.throwIfAborted();
            buffer += decoder.decode(value, { stream: true });
            let boundary: RegExpExecArray | null;
            while ((boundary = /\r?\n\r?\n/.exec(buffer))) {
              // Transport chunks can contain many valid frames after buffering.
              // Limit individual events, not the arbitrary chunk boundary.
              if (boundary.index > MAX_BUFFER) throw new Error("Invalid live update stream.");
              const lines = buffer.slice(0, boundary.index).split(/\r?\n/);
              buffer = buffer.slice(boundary.index + boundary[0].length);
              const event = lines.find((line) => line.startsWith("event:"))?.slice(6).trim();
              const data = lines.filter((line) => line.startsWith("data:")).map((line) => line.slice(5).trim()).join("\n");
              if (!["ready", "changed", "heartbeat", "snapshot", "draining"].includes(event ?? "")) continue;
              if (!this.connected && event !== "ready") throw new Error("Invalid live update handshake.");
              if (event !== "snapshot" && data !== "{}") throw new Error("Invalid live update handshake.");
              watchdog(HEARTBEAT_TIMEOUT_MS);
              if (event === "ready") {
                this.connected = true;
                resolve();
              } else if (event === "changed") this.changed();
              else if (event === "snapshot") {
                let value: unknown;
                try { value = JSON.parse(data); } catch { throw new Error("Invalid live update snapshot."); }
                if (!value || typeof value !== "object" || !Array.isArray((value as CallSnapshot).participants)
                  || !(value as CallSnapshot).participants.every((p) => p && typeof p.id === "string" && typeof p.name === "string"
                    && typeof p.muted === "boolean" && typeof p.deafened === "boolean"
                    && (token === undefined ? !("tracks" in p)
                      : Array.isArray(p.tracks) && p.tracks.every((t) => t && typeof t.id === "string" && t.kind === "microphone")))) {
                  throw new Error("Invalid live update snapshot.");
                }
                const revision = (value as { revision?: unknown }).revision;
                if (revision !== undefined && (!Number.isSafeInteger(revision) || (revision as number) < 0)) {
                  throw new Error("Invalid live update snapshot.");
                }
                const snapshot = value as CallSnapshot & { revision?: number };
                this.snapshot?.(token === undefined
                  ? { ...snapshot, participants: snapshot.participants.map((p) => ({ ...p, tracks: [] })) }
                  : snapshot);
              } else if (event === "draining") {
                // Stop here: the following EOF must not replace the fast planned
                // reconnect with the ordinary connection-failure backoff.
                this.stop();
                this.draining?.();
                return;
              }
            }
            if (buffer.length > MAX_BUFFER) throw new Error("Invalid live update stream.");
          }
        } finally {
          await reader.cancel().catch(() => undefined);
          reader.releaseLock();
        }
      };
      void run().catch((reason: unknown) => {
        this.connected = false;
        const error = reason instanceof Error ? reason : new Error("Live updates disconnected.");
        reject(error);
        if (!signal.aborted && !this.stopped) this.lost(error, draining);
      }).finally(() => {
        clearTimeout(this.timer);
        signal.removeEventListener("abort", abort);
        this.controller.abort();
      });
    });
  }

  stop() {
    this.stopped = true;
    this.connected = false;
    clearTimeout(this.timer);
    this.controller.abort();
  }
}
