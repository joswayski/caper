// Lossless WebP for screenshots and graphics with too many colours for an
// indexed PNG. Canvas cannot encode lossless WebP, so libwebp (WASM, via
// @jsquash/webp) runs in a module worker, loaded only when needed.

/** libwebp settings. `method: 3` measured ~74% smaller than browser PNGs of
 * 2880×1800 screenshots in 0.3–2 s; method 6 saved only 1–2% more in 9–17 s.
 * `exact` keeps RGB under fully transparent pixels, so decoding returns the
 * encoded pixels bit for bit. */
export const LOSSLESS_WEBP_OPTIONS = { lossless: 1, method: 3, quality: 75, exact: 1 } as const;

const ascii = (bytes: Uint8Array, offset: number, text: string) =>
  offset + text.length <= bytes.length && [...text].every((char, i) => bytes[offset + i] === char.charCodeAt(0));

/** Encodes RGBA losslessly in a worker; undefined when unavailable or failed. */
export function encodeLosslessWebp(rgba: Uint8ClampedArray, width: number, height: number): Promise<Uint8Array | undefined> {
  if (typeof Worker === "undefined" || typeof WebAssembly === "undefined") return Promise.resolve(undefined);
  return new Promise((resolve) => {
    let worker: Worker;
    try {
      worker = new Worker(new URL("./webp-worker.ts", import.meta.url), { type: "module", name: "caper-webp" });
    } catch {
      resolve(undefined);
      return;
    }
    const done = (value?: Uint8Array) => { worker.terminate(); resolve(value); };
    worker.onmessage = (event: MessageEvent<{ output?: ArrayBuffer }>) => done(event.data.output ? new Uint8Array(event.data.output) : undefined);
    worker.onerror = () => done();
    // Copy rather than transfer: the caller still needs the pixels.
    worker.postMessage({ rgba, width, height, options: LOSSLESS_WEBP_OPTIONS });
  });
}

/** The PNG chunks of `type`, as views of their data. */
export function pngChunks(bytes: Uint8Array, type: string): Uint8Array[] {
  const found: Uint8Array[] = [];
  if (bytes.length < 8 || bytes[0] !== 0x89 || !ascii(bytes, 1, "PNG")) return found;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  for (let offset = 8; offset + 12 <= bytes.length;) {
    const length = view.getUint32(offset);
    if (offset + 12 + length > bytes.length) break;
    if (ascii(bytes, offset + 4, type)) found.push(bytes.subarray(offset + 8, offset + 8 + length));
    if (ascii(bytes, offset + 4, "IEND")) break;
    offset += 12 + length;
  }
  return found;
}

/** PNG bit depth from IHDR (16-bit sources would lose precision on a canvas). */
export function pngBitDepth(bytes: Uint8Array) {
  const header = pngChunks(bytes.subarray(0, 64), "IHDR")[0];
  return header && header.length >= 9 ? header[8] : undefined;
}

async function inflate(data: Uint8Array) {
  const stream = new Blob([data as BlobPart]).stream().pipeThrough(new DecompressionStream("deflate"));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

/** The ICC profile a PNG (iCCP, zlib) or WebP (ICCP) carries, uncompressed. */
export async function iccProfile(bytes: Uint8Array): Promise<Uint8Array | undefined> {
  const iccp = pngChunks(bytes, "iCCP")[0];
  if (iccp) {
    const nul = iccp.indexOf(0);
    // name, NUL, compression method 0 (zlib), profile
    if (nul < 1 || nul > 79 || iccp[nul + 1] !== 0) return;
    return inflate(iccp.subarray(nul + 2)).catch(() => undefined);
  }
  if (!ascii(bytes, 0, "RIFF") || !ascii(bytes, 8, "WEBP")) return;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  for (let offset = 12; offset + 8 <= bytes.length;) {
    const size = view.getUint32(offset + 4, true);
    if (ascii(bytes, offset, "ICCP")) return offset + 8 + size <= bytes.length ? bytes.slice(offset + 8, offset + 8 + size) : undefined;
    offset += 8 + size + (size & 1);
  }
}

const le24 = (n: number) => [n & 0xff, (n >> 8) & 0xff, (n >> 16) & 0xff];
const le32 = (n: number) => [n & 0xff, (n >> 8) & 0xff, (n >> 16) & 0xff, (n >>> 24) & 0xff];

/** Wraps a simple-format lossless WebP (RIFF/WEBP/VP8L) in the extended
 * format with an ICCP chunk, so pixels decoded without colour conversion keep
 * their colour space (macOS screenshots are Display P3). */
export function webpWithIcc(webp: Uint8Array, icc: Uint8Array, width: number, height: number): Uint8Array {
  if (!ascii(webp, 0, "RIFF") || !ascii(webp, 8, "WEBP") || !ascii(webp, 12, "VP8L")) return webp;
  const image = webp.subarray(12);
  // VP8L header: signature byte, 14+14 bits of size, then the alpha_is_used bit.
  const alpha = image.length > 12 && (image[8 + 4] & 0x10) !== 0;
  const vp8x = [..."VP8X"].map((c) => c.charCodeAt(0)).concat(le32(10), [0x20 | (alpha ? 0x10 : 0), 0, 0, 0], le24(width - 1), le24(height - 1));
  const iccHeader = [..."ICCP"].map((c) => c.charCodeAt(0)).concat(le32(icc.length));
  const pad = icc.length & 1;
  const body = vp8x.length + iccHeader.length + icc.length + pad + image.length;
  const out = new Uint8Array(12 + body);
  out.set([..."RIFF"].map((c) => c.charCodeAt(0)));
  out.set(le32(4 + body), 4);
  out.set([..."WEBP"].map((c) => c.charCodeAt(0)), 8);
  let offset = 12;
  out.set(vp8x, offset); offset += vp8x.length;
  out.set(iccHeader, offset); offset += iccHeader.length;
  out.set(icc, offset); offset += icc.length + pad;
  out.set(image, offset);
  return out;
}
