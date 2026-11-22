interface PreparedWorker {
  worker: Worker;
  ready: Promise<void>;
  /** True once the model has finished initializing. */
  settled: boolean;
  stop(): void;
}

/** How long an initialized worker nobody took stays alive after the last
 * request for it. It holds about 110 MB of the page's memory, and starting
 * another from the HTTP cache takes a fraction of a second. */
export const SPARE_IDLE_MS = 30_000;

const prefetched = new Set<string>();

/** Downloads the model and runtime (about 27 MB, immutable) into the browser's
 * HTTP cache without compiling or keeping them, once per page, so a later
 * preparation only initializes. Best effort: a failure means that preparation
 * downloads them itself. */
export function prefetchDpdfnet() {
  if (typeof document === "undefined") return;
  for (const url of ["/audio/dpdfnet8-v2/ort-wasm-simd-threaded.wasm", "/audio/dpdfnet8-v2/dpdfnet8_48khz_hr.onnx"]) {
    if (prefetched.has(url)) continue;
    prefetched.add(url);
    const link = document.createElement("link");
    if (link.relList?.supports?.("prefetch")) {
      // The browser fills its cache itself; the body never reaches the page.
      link.rel = "prefetch";
      link.href = url;
      document.head.append(link);
      continue;
    }
    // Without prefetch (Safari), read and drop the body: a response is cached
    // only once it has been read to the end.
    void fetch(url)
      .then(async (response) => {
        if (!response.ok) throw new Error("Noise suppression download failed.");
        const reader = response.body?.getReader();
        while (reader && !(await reader.read()).done);
      })
      .catch(() => prefetched.delete(url));
  }
}

/** One unused, initialized worker, kept while a capture is expected. Taking it
 * transfers exclusive ownership to a capture. */
export class DpdfnetPreparation {
  private prepared?: PreparedWorker;
  private idle?: ReturnType<typeof setTimeout>;
  private readonly profile: 8 | 2;

  constructor(profile: 8 | 2 = 8) {
    this.profile = profile;
  }

  /** Starts (or keeps) the spare for a capture expected soon: call it on
   * intent, such as the pointer reaching Join. Unused, it stops by itself. */
  async prepare() {
    const prepared = this.get();
    if (prepared.settled) this.keep();
    await prepared.ready;
  }

  /** Whether a worker is already initialized, without starting one. */
  isReady() {
    return this.prepared?.settled === true;
  }

  /** Whether a worker is held for the next capture, ready or still initializing. */
  hasSpare() {
    return this.prepared !== undefined;
  }

  take() {
    const prepared = this.get();
    clearTimeout(this.idle);
    this.prepared = undefined;
    // Transfer synchronously so the capture owns cleanup before awaiting readiness.
    // A used worker is never returned: its recurrent/DSP state belongs to that capture.
    return prepared;
  }

  stop() {
    const prepared = this.prepared;
    clearTimeout(this.idle);
    this.prepared = undefined;
    prepared?.stop();
  }

  /** Idle time counts from readiness, so a slow first download is never cut short. */
  private keep() {
    clearTimeout(this.idle);
    this.idle = setTimeout(() => this.stop(), SPARE_IDLE_MS);
  }

  private get(): PreparedWorker {
    if (this.prepared) return this.prepared;
    const worker = new Worker(this.profile === 8 ? "/audio/dpdfnet8-v2/worker.js" : "/audio/dpdfnet2-v1/worker.js", {
      type: "module",
      name: `caper-dpdfnet${this.profile}`,
    });
    let resolve!: () => void;
    let reject!: (error: Error) => void;
    const ready = new Promise<void>((yes, no) => {
      resolve = yes;
      reject = no;
    });
    let stopped = false;
    const prepared: PreparedWorker = {
      worker,
      ready,
      settled: false,
      stop() {
        if (stopped) return;
        stopped = true;
        clearTimeout(timer);
        worker.onmessage = null;
        worker.onerror = null;
        worker.terminate();
        reject(new Error("Noise suppression preparation stopped"));
      },
    };
    const fail = (error: Error) => {
      if (this.prepared === prepared) this.prepared = undefined;
      reject(error);
      prepared.stop();
    };
    const timer = setTimeout(() => fail(new Error("Noise suppression timed out")), 60_000);
    worker.onmessage = ({ data }) => {
      if (data?.type === "ready") {
        clearTimeout(timer);
        prepared.settled = true;
        if (this.prepared === prepared) this.keep();
        resolve();
      } else fail(new Error("Noise suppression failed"));
    };
    worker.onerror = () => fail(new Error("Noise suppression failed"));
    // Background preparation may be discarded before anyone awaits it.
    void ready.catch(() => undefined);
    this.prepared = prepared;
    return prepared;
  }
}
