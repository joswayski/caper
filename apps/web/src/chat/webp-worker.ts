// Module worker for lossless WebP encoding (see webp.ts). Its own chunk, so
// libwebp's WASM loads only when a screenshot needs it.
import encode from "@jsquash/webp/encode.js";

interface Job {
  rgba: Uint8ClampedArray;
  width: number;
  height: number;
  options: Record<string, number>;
}

self.onmessage = async (event: MessageEvent<Job>) => {
  const { rgba, width, height, options } = event.data;
  try {
    const output = await encode({ data: rgba, width, height, colorSpace: "srgb" } as ImageData, options);
    (self as unknown as Worker).postMessage({ output }, [output]);
  } catch {
    (self as unknown as Worker).postMessage({});
  }
};
