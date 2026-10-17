// Module worker for shrinking oversized GIFs (see gif-shrink.ts), so decoding
// and re-encoding every frame never blocks the page.
import { shrinkGif } from "./gif-shrink.ts";

self.onmessage = (event: MessageEvent<{ bytes: Uint8Array; budget: number }>) => {
  try {
    const { bytes, edge } = shrinkGif(event.data.bytes, event.data.budget);
    (self as unknown as Worker).postMessage({ bytes, edge }, [bytes.buffer]);
  } catch {
    (self as unknown as Worker).postMessage({});
  }
};
