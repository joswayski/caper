type Engine = "deepfilter" | "rnnoise";
interface Assets {
  module: WebAssembly.Module;
  model?: ArrayBuffer;
}

async function download(url: string, signal: AbortSignal) {
  const response = await fetch(url, { signal });
  if (!response.ok) throw new Error("Noise suppression download failed.");
  return response.arrayBuffer();
}

/** Call-client-owned compiled code and model bytes, never microphone or audio context state. */
export class NoiseAssets {
  private readonly loading = new Map<Engine, Promise<Assets>>();

  async load(engine: Engine, signal?: AbortSignal): Promise<Assets> {
    signal?.throwIfAborted();
    let loading = this.loading.get(engine);
    if (!loading) {
      // Preparation survives a cancelled join so the next attempt can reuse it.
      const timeout = AbortSignal.timeout(30_000);
      loading = Promise.all([
        download(engine === "rnnoise" ? "/audio/rnnoise-v1/rnnoise.wasm" : "/audio/deepfilter-v1/df_bg.wasm", timeout)
          .then((bytes) => WebAssembly.compile(bytes)),
        engine === "deepfilter" ? download("/audio/deepfilter-v1/DeepFilterNet3.bin", timeout) : undefined,
      ]).then(([module, model]) => ({ module, model })).catch((error) => {
        this.loading.delete(engine);
        throw error;
      });
      this.loading.set(engine, loading);
    }
    if (!signal) return loading;
    return new Promise<Assets>((resolve, reject) => {
      const abort = () => { signal.removeEventListener("abort", abort); reject(signal.reason); };
      signal.addEventListener("abort", abort, { once: true });
      if (signal.aborted) abort();
      void loading.then(resolve, reject).finally(() => signal.removeEventListener("abort", abort));
    });
  }
}
