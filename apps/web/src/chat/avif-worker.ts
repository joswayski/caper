// Module worker for AVIF photo encoding (see avif.ts). Its own chunk, so
// libavif's WASM loads only when a photo needs it. The single-threaded codec
// is imported directly: @jsquash/avif's `encode` would pick the threaded
// build whenever SharedArrayBuffer is available.
import createEncoder from "@jsquash/avif/codec/enc/avif_enc.js";

interface Job {
  rgba: Uint8ClampedArray;
  width: number;
  height: number;
  options: Parameters<Awaited<ReturnType<typeof createEncoder>>["encode"]>[3];
}

self.onmessage = async (event: MessageEvent<Job>) => {
  const { rgba, width, height, options } = event.data;
  try {
    const encoder = await createEncoder({ noInitialRun: true });
    const encoded = encoder.encode(rgba, width, height, options);
    if (!encoded) throw new Error("AVIF encoding failed");
    const output = encoded.slice().buffer;
    (self as unknown as Worker).postMessage({ output }, [output]);
  } catch {
    (self as unknown as Worker).postMessage({});
  }
};
