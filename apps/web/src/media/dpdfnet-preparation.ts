interface PreparedWorker {
  worker: Worker;
  ready: Promise<void>;
  stop(): void;
}

/** One unused, initialized worker. Taking it transfers exclusive ownership to a capture. */
export class DpdfnetPreparation {
  private prepared?: PreparedWorker;

  async prepare() {
    await this.get().ready;
  }

  take() {
    const prepared = this.get();
    this.prepared = undefined;
    // Transfer synchronously so the capture owns cleanup before awaiting readiness.
    // A used worker is never returned: its recurrent/DSP state belongs to that capture.
    return prepared;
  }

  stop() {
    const prepared = this.prepared;
    this.prepared = undefined;
    prepared?.stop();
  }

  private get(): PreparedWorker {
    if (this.prepared) return this.prepared;
    const worker = new Worker("/audio/dpdfnet8-v2/worker.js", { type: "module", name: "caper-dpdfnet8" });
    let resolve!: () => void;
    let reject!: (error: Error) => void;
    const ready = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
    let stopped = false;
    const prepared: PreparedWorker = {
      worker, ready,
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
      if (data?.type === "ready") { clearTimeout(timer); resolve(); }
      else fail(new Error("Noise suppression failed"));
    };
    worker.onerror = () => fail(new Error("Noise suppression failed"));
    // Background preparation may be discarded before anyone awaits it.
    void ready.catch(() => undefined);
    this.prepared = prepared;
    return prepared;
  }
}
