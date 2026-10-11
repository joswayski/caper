// Lossy AVIF for photos. Canvas cannot encode AVIF, so libavif 1.0.1 with
// libaom 3.7.0 (WASM, via @jsquash/avif; Apache-2.0, libavif and libaom
// BSD-2-Clause) runs in a module worker, loaded only when a photo needs it.
// Only the single-threaded build is used: the threaded one needs
// SharedArrayBuffer, which requires cross-origin isolation (COOP/COEP).

/**
 * libavif settings apart from `quality`, which is the server's `avifQuality`
 * passed straight through: @jsquash/avif sets libavif's `quality` (as
 * `avifenc -q`) and derives the quantizers from it, so it means the same as on
 * every other client.
 *
 * - `subsample: 1` is 4:2:0 and `bitDepth: 8`, like `avifenc -y 420 -d 8`.
 * - `tune: 1` leaves libaom on its default (PSNR) tuning, as avifenc does;
 *   @jsquash's "auto" would switch to SSIM tuning at quality 50 and above.
 * - `speed: 9`: single-threaded WASM on a 4-core x86 server, per 12 MP, took
 *   11–32 s at speed 6 (the native default), 11–16 s at 7 and 3–12 s at 8,
 *   but 1.7–3.6 s at 9 (Chromium and Node alike). On five 6–14 MP photos at
 *   quality 85, speed 9 was 2.2% larger than speed 6 and 82.6% of WebP q92's
 *   bytes, scoring SSIMULACRA2 82.99 against WebP's 83.03 and
 *   `avifenc -q 85 -s 6 -y 420`'s 83.28. Speed 8 was larger than 9.
 *
 * No Exif, XMP or ICC is written. The pixels are sRGB (photos are drawn on
 * an sRGB canvas, as for WebP, whose encoder tags them with an sRGB ICC
 * profile); `tagSrgb` says so in the AVIF's colour box.
 */
export const AVIF_OPTIONS = {
  qualityAlpha: -1,
  denoiseLevel: 0,
  tileColsLog2: 0,
  tileRowsLog2: 0,
  speed: 9,
  subsample: 1,
  chromaDeltaQ: false,
  sharpness: 0,
  tune: 1,
  enableSharpYUV: false,
  bitDepth: 8,
} as const;

const ascii = (bytes: Uint8Array, offset: number, text: string) =>
  offset + text.length <= bytes.length && [...text].every((char, i) => bytes[offset + i] === char.charCodeAt(0));

/** An ISO BMFF `ftyp` with an AVIF major brand, as the API's upload check
 * (`assets::sniff`) requires. */
export function isAvif(bytes: Uint8Array) {
  return ascii(bytes, 4, "ftyp") && ["avif", "avis", "mif1", "msf1"].some((brand) => ascii(bytes, 8, brand));
}

/** Marks libavif's "unspecified" colour primaries and transfer (CICP 2/2) as
 * sRGB (BT.709 primaries, sRGB transfer: CICP 1/13) in the `colr` nclx box
 * of the `meta` box, in place. Browsers already assume sRGB for unspecified
 * values; this makes it explicit for every decoder. */
export function tagSrgb(avif: Uint8Array) {
  const view = new DataView(avif.buffer, avif.byteOffset, avif.byteLength);
  for (let offset = 0; offset + 8 <= avif.length;) {
    const size = view.getUint32(offset);
    if (size < 8 || offset + size > avif.length) break;
    if (ascii(avif, offset + 4, "meta")) {
      for (let i = offset + 8; i + 14 <= offset + size; i++) {
        if (!ascii(avif, i, "colrnclx")) continue;
        if (view.getUint16(i + 8) === 2 && view.getUint16(i + 10) === 2) {
          view.setUint16(i + 8, 1);
          view.setUint16(i + 10, 13);
        }
        break;
      }
      break;
    }
    offset += size;
  }
  return avif;
}

/** Encodes opaque or transparent RGBA as AVIF in a worker, taking ownership of
 * `rgba`'s buffer; undefined when unavailable or failed. */
export function encodeAvif(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  quality: number,
): Promise<Uint8Array | undefined> {
  if (typeof Worker === "undefined" || typeof WebAssembly === "undefined") return Promise.resolve(undefined);
  return new Promise((resolve) => {
    let worker: Worker;
    try {
      worker = new Worker(new URL("./avif-worker.ts", import.meta.url), { type: "module", name: "caper-avif" });
    } catch {
      resolve(undefined);
      return;
    }
    const done = (value?: Uint8Array) => {
      worker.terminate();
      resolve(value && isAvif(value) ? tagSrgb(value) : undefined);
    };
    worker.onmessage = (event: MessageEvent<{ output?: ArrayBuffer }>) =>
      done(event.data.output ? new Uint8Array(event.data.output) : undefined);
    worker.onerror = () => done();
    worker.postMessage({ rgba, width, height, options: { ...AVIF_OPTIONS, quality } }, [rgba.buffer]);
  });
}
