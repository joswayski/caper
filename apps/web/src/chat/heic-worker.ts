// Module worker for HEIC decoding (see heic.ts). Its own chunk, so libheif's
// WASM (inlined in the bundle) loads only when a HEIC needs it.
import createLibheif from "libheif-js/libheif-wasm/libheif-bundle.mjs";
import { displayP3ToSrgb, heicIsDisplayP3 } from "./heic";

interface HeifImage {
  get_width(): number;
  get_height(): number;
  is_primary(): boolean;
  display(target: ImageData, done: (result: ImageData | null) => void): void;
  free(): void;
}

self.onmessage = async (event: MessageEvent<{ input: ArrayBuffer; maxPixels: number }>) => {
  const reply = (message: object, transfer: Transferable[] = []) =>
    (self as unknown as Worker).postMessage(message, transfer);
  try {
    const bytes = new Uint8Array(event.data.input);
    const libheif = await createLibheif();
    const images = new libheif.HeifDecoder().decode(bytes) as HeifImage[];
    const image = images.find((candidate) => candidate.is_primary()) ?? images[0];
    if (!image) throw new Error("no picture");
    const width = image.get_width();
    const height = image.get_height();
    if (!width || !height || width * height > event.data.maxPixels) throw new Error("too large");
    const target = { data: new Uint8ClampedArray(width * height * 4), width, height } as ImageData;
    const result = await new Promise<ImageData | null>((resolve) => image.display(target, resolve));
    for (const each of images) each.free();
    if (!result) throw new Error("decoding failed");
    if (heicIsDisplayP3(bytes.subarray(0, 256 * 1024))) displayP3ToSrgb(result.data);
    const rgba = result.data.buffer as ArrayBuffer;
    reply({ rgba, width, height }, [rgba]);
  } catch {
    reply({});
  }
};
