// Exact-palette indexed PNG, the lossless path for flat images such as UI
// screenshots. Mirrors Captures' `exact_indexed_rgba` + indexed writer
// (captures-image/src/png.rs): no quantization, so pixels are unchanged.

const CRC_TABLE = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return table;
})();

function crc32(bytes: Uint8Array, crc = 0xffffffff) {
  for (const byte of bytes) crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return crc;
}

function chunk(type: string, data: Uint8Array) {
  const out = new Uint8Array(12 + data.length);
  const view = new DataView(out.buffer);
  view.setUint32(0, data.length);
  for (let i = 0; i < 4; i++) out[4 + i] = type.charCodeAt(i);
  out.set(data, 8);
  view.setUint32(8 + data.length, (crc32(out.subarray(4, 8 + data.length)) ^ 0xffffffff) >>> 0);
  return out;
}

async function zlib(data: Uint8Array) {
  // "deflate" is the zlib container PNG requires (RFC 1950), not raw deflate.
  const stream = new Blob([data as BlobPart]).stream().pipeThrough(new CompressionStream("deflate"));
  return new Uint8Array(await new Response(stream).arrayBuffer());
}

/** Distinct colours as RGBA keys, or undefined once there are more than `max`,
 * or when any pixel is partially transparent (canvas round-trips can alter
 * those, so they never take the lossless path). */
export function exactPalette(rgba: Uint8ClampedArray | Uint8Array, max: number): Map<number, number> | undefined {
  if (max < 1) return;
  const palette = new Map<number, number>();
  for (let i = 0; i < rgba.length; i += 4) {
    const alpha = rgba[i + 3];
    if (alpha !== 0 && alpha !== 255) return;
    const key = alpha === 0 ? 0 : ((rgba[i] << 24) | (rgba[i + 1] << 16) | (rgba[i + 2] << 8) | 255) >>> 0;
    if (!palette.has(key)) {
      if (palette.size >= max) return;
      palette.set(key, palette.size);
    }
  }
  return palette;
}

/** Encodes RGBA as an indexed PNG when it has at most `maxColors` colours. */
export async function encodeIndexedPng(rgba: Uint8ClampedArray | Uint8Array, width: number, height: number, maxColors: number): Promise<Uint8Array | undefined> {
  const palette = exactPalette(rgba, Math.min(256, maxColors));
  if (!palette) return;
  const depth = palette.size <= 2 ? 1 : palette.size <= 4 ? 2 : palette.size <= 16 ? 4 : 8;
  const perByte = 8 / depth;
  const stride = Math.ceil(width / perByte);
  const raw = new Uint8Array((stride + 1) * height);
  for (let y = 0; y < height; y++) {
    const row = y * (stride + 1); // filter byte 0 (None) suits palette indices
    for (let x = 0; x < width; x++) {
      const i = (y * width + x) * 4;
      const key = rgba[i + 3] === 0 ? 0 : ((rgba[i] << 24) | (rgba[i + 1] << 16) | (rgba[i + 2] << 8) | 255) >>> 0;
      const index = palette.get(key)!;
      const shift = 8 - depth * ((x % perByte) + 1);
      raw[row + 1 + Math.floor(x / perByte)] |= index << shift;
    }
  }
  const plte = new Uint8Array(palette.size * 3);
  const trns = new Uint8Array(palette.size);
  let lastTransparent = -1;
  for (const [key, index] of palette) {
    plte[index * 3] = key >>> 24;
    plte[index * 3 + 1] = (key >>> 16) & 0xff;
    plte[index * 3 + 2] = (key >>> 8) & 0xff;
    trns[index] = key & 0xff;
    if (trns[index] !== 255) lastTransparent = index;
  }
  const header = new Uint8Array(13);
  const view = new DataView(header.buffer);
  view.setUint32(0, width);
  view.setUint32(4, height);
  header[8] = depth;
  header[9] = 3; // indexed colour
  const parts = [
    new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", header),
    chunk("sRGB", new Uint8Array([0])),
    chunk("PLTE", plte),
    ...(lastTransparent >= 0 ? [chunk("tRNS", trns.subarray(0, lastTransparent + 1))] : []),
    chunk("IDAT", await zlib(raw)),
    chunk("IEND", new Uint8Array()),
  ];
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) { out.set(part, offset); offset += part.length; }
  return out;
}
