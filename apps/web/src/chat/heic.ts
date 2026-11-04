// HEIC/HEIF stills for browsers that cannot decode them (Chrome, Firefox):
// libheif 1.23 with libde265 (WASM, via libheif-js; LGPL-3.0, loaded as its
// own chunk and only when a HEIC needs it) runs in a module worker. libheif
// joins the tile grids iPhone cameras save photos as and applies their crop
// and rotation; Display P3 photos are converted to sRGB here, as browsers
// that decode HEIC themselves draw them.

const ascii = (bytes: Uint8Array, offset: number, text: string) =>
  offset + text.length <= bytes.length && [...text].every((char, i) => bytes[offset + i] === char.charCodeAt(0));

const utf16 = (text: string) => [...text].flatMap((char) => [0, char.charCodeAt(0)]);

function contains(bytes: Uint8Array, needle: number[]) {
  outer: for (let i = 0; i + needle.length <= bytes.length; i++) {
    for (let j = 0; j < needle.length; j++) if (bytes[i + j] !== needle[j]) continue outer;
    return true;
  }
  return false;
}

/** Whether a HEIF file's first colour box (`colr`) says Display P3: nclx
 * primaries 12 (SMPTE EG 432), or an ICC profile described as Display P3 (as
 * ASCII in ICC v2 `desc`, UTF-16BE in v4 `mluc`). */
export function heicIsDisplayP3(bytes: Uint8Array) {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  for (let i = 4; i + 12 <= bytes.length; i++) {
    if (!ascii(bytes, i, "colr")) continue;
    const size = view.getUint32(i - 4);
    if (size < 12 || i - 4 + size > bytes.length) continue;
    if (ascii(bytes, i + 4, "nclx")) return size >= 18 && view.getUint16(i + 8) === 12;
    if (ascii(bytes, i + 4, "prof") || ascii(bytes, i + 4, "rICC")) {
      const icc = bytes.subarray(i + 8, i - 4 + size);
      const name = "Display P3";
      return (
        contains(
          icc,
          [...name].map((char) => char.charCodeAt(0)),
        ) || contains(icc, utf16(name))
      );
    }
  }
  return false;
}

/** Display P3 to sRGB in place: both use the sRGB transfer curve, so only the
 * primaries change (in linear light); colours outside sRGB are clipped. The
 * same matrix as the desktop client (`ffmpeg::display_p3_to_srgb`). */
export function displayP3ToSrgb(rgba: Uint8ClampedArray) {
  const matrix = [
    [1.22494, -0.22494, 0],
    [-0.042057, 1.042057, 0],
    [-0.019638, -0.078636, 1.098274],
  ];
  const linear = Array.from({ length: 256 }, (_, value) => {
    const v = value / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  const steps = 4096;
  const encoded = Array.from({ length: steps + 1 }, (_, step) => {
    const v = step / steps;
    return Math.round((v <= 0.0031308 ? v * 12.92 : 1.055 * v ** (1 / 2.4) - 0.055) * 255);
  });
  for (let i = 0; i < rgba.length; i += 4) {
    const r = linear[rgba[i]],
      g = linear[rgba[i + 1]],
      b = linear[rgba[i + 2]];
    for (let channel = 0; channel < 3; channel++) {
      const [x, y, z] = matrix[channel];
      const value = Math.min(1, Math.max(0, x * r + y * g + z * b));
      rgba[i + channel] = encoded[Math.round(value * steps)];
    }
  }
}

/** Decoded stills larger than this many pixels are refused (as on desktop). */
const MAX_PIXELS = 128 * 1024 * 1024;

/** The primary picture of a HEIC upright in sRGB, as a bitmap; undefined when
 * no worker or WASM is available or decoding fails. */
export async function decodeHeic(file: Blob): Promise<ImageBitmap | undefined> {
  if (!heicFallback()) return;
  const input = await file.arrayBuffer();
  const decoded = await new Promise<{ rgba?: ArrayBuffer; width?: number; height?: number }>((resolve) => {
    let worker: Worker;
    try {
      worker = new Worker(new URL("./heic-worker.ts", import.meta.url), { type: "module", name: "caper-heic" });
    } catch {
      resolve({});
      return;
    }
    const done = (value: { rgba?: ArrayBuffer; width?: number; height?: number }) => {
      worker.terminate();
      resolve(value);
    };
    worker.onmessage = (event) => done(event.data);
    worker.onerror = () => done({});
    worker.postMessage({ input, maxPixels: MAX_PIXELS }, [input]);
  });
  const { rgba, width, height } = decoded;
  if (!rgba || !width || !height || rgba.byteLength !== width * height * 4) return;
  return createImageBitmap(new ImageData(new Uint8ClampedArray(rgba), width, height)).catch(() => undefined);
}

/** Whether a file starts like a HEIC/HEIF still (its `ftyp` brands), not an
 * AVIF or an image sequence. */
export function isHeicFile(head: Uint8Array) {
  if (!ascii(head, 4, "ftyp")) return false;
  const size = Math.min(new DataView(head.buffer, head.byteOffset, head.byteLength).getUint32(0), head.length);
  const brand = (offset: number) => String.fromCharCode(...head.subarray(offset, offset + 4));
  const compatible = Array.from({ length: Math.max(0, (size - 16) >> 2) }, (_, i) => brand(16 + i * 4));
  const major = brand(8);
  if (["heic", "heix", "heif"].includes(major)) return true;
  return major === "mif1" && !compatible.includes("avif");
}

/** `createImageBitmap`, falling back to libheif for HEIC the browser cannot
 * decode. The fallback is already upright and sRGB, whatever `options` ask. */
export async function decodeBitmap(file: Blob, options?: ImageBitmapOptions): Promise<ImageBitmap | undefined> {
  if (typeof createImageBitmap === "undefined") return;
  const bitmap = await createImageBitmap(file, options).catch(() => undefined);
  if (bitmap) return bitmap;
  if (!isHeicFile(new Uint8Array(await file.slice(0, 64).arrayBuffer()))) return;
  return decodeHeic(file);
}

/** Whether HEIC the browser cannot decode itself can be decoded here. */
export const heicFallback = () =>
  typeof Worker !== "undefined" && typeof WebAssembly !== "undefined" && typeof ImageData !== "undefined";
