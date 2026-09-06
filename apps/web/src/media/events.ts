const START_TIMEOUT_MS = 10_000;
const HEARTBEAT_TIMEOUT_MS = 25_000;
const MAX_BUFFER = 64 * 1024;

/** One authenticated event stream per call. Reconnection belongs to the call lifecycle. */
export class CallEvents {
  connected = false;
  private readonly controller = new AbortController();
  private stopped = false;
  private timer?: ReturnType<typeof setTimeout>;

  private readonly changed: () => void;
  private readonly lost: (error: Error) => void;
  constructor(changed: () => void, lost: (error: Error) => void) {
    this.changed = changed;
    this.lost = lost;
  }

  open(token: string, signal: AbortSignal): Promise<void> {
    if (signal.aborted) return Promise.reject(signal.reason);
    return new Promise<void>((resolve, reject) => {
      const abort = () => this.stop();
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) abort();
      const watchdog = (timeout: number) => {
        clearTimeout(this.timer);
        this.timer = setTimeout(() => this.controller.abort(new Error("Live updates timed out. Please try joining again.")), timeout);
      };
      watchdog(START_TIMEOUT_MS);
      const run = async () => {
        const response = await fetch("/api/media/events", {
          headers: { accept: "text/event-stream", authorization: `Bearer ${token}` },
          signal: this.controller.signal,
          cache: "no-store",
        });
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
            if (buffer.length > MAX_BUFFER) throw new Error("Invalid live update stream.");
            let boundary: RegExpExecArray | null;
            while ((boundary = /\r?\n\r?\n/.exec(buffer))) {
              const lines = buffer.slice(0, boundary.index).split(/\r?\n/);
              buffer = buffer.slice(boundary.index + boundary[0].length);
              const event = lines.find((line) => line.startsWith("event:"))?.slice(6).trim();
              const data = lines.filter((line) => line.startsWith("data:")).map((line) => line.slice(5).trim()).join("\n");
              if (!["ready", "changed", "heartbeat"].includes(event ?? "")) continue;
              if (data !== "{}" || (!this.connected && event !== "ready")) throw new Error("Invalid live update handshake.");
              watchdog(HEARTBEAT_TIMEOUT_MS);
              if (event === "ready") {
                this.connected = true;
                resolve();
              } else if (event === "changed") this.changed();
            }
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
        if (!signal.aborted && !this.stopped) this.lost(error);
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
