import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { deflateSync } from "node:zlib";
import { LOSSLESS_WEBP_OPTIONS, iccProfile, pngBitDepth, webpWithIcc } from "../chat/webp.ts";

// libwebp in Node (the SIMD build, which Node supports): hand the codecs
// their compiled WASM, which the browser fetches next to the worker chunk.
const root = dirname(createRequire(import.meta.url).resolve("@jsquash/webp/package.json"));
const encoder = await import(join(root, "encode.js"));
const decoder = await import(join(root, "decode.js"));
await encoder.init(await WebAssembly.compile(await readFile(join(root, "codec/enc/webp_enc_simd.wasm"))));
await decoder.init(await WebAssembly.compile(await readFile(join(root, "codec/dec/webp_dec.wasm"))));

/** A screenshot-like image: gradients, anti-aliased edges, transparent holes. */
function sample(width: number, height: number) {
  const rgba = new Uint8ClampedArray(width * height * 4);
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const i = (y * width + x) * 4;
    rgba[i] = (x * 7 + y) & 0xff;
    rgba[i + 1] = (y * 3) & 0xff;
    rgba[i + 2] = (x ^ y) & 0xff;
    rgba[i + 3] = (x + y) % 17 === 0 ? 0 : 255;
  }
  return rgba;
}

test("lossless WebP round-trips every pixel, alpha included, with the app's settings", async () => {
  const [width, height] = [301, 157];
  const rgba = sample(width, height);
  const webp = new Uint8Array(await encoder.default({ data: rgba, width, height }, LOSSLESS_WEBP_OPTIONS));
  assert.equal(String.fromCharCode(...webp.subarray(12, 16)), "VP8L");
  const decoded = await decoder.default(webp);
  assert.deepEqual([decoded.width, decoded.height], [width, height]);
  assert.ok(Buffer.from(decoded.data.buffer).equals(Buffer.from(rgba.buffer)), "pixel-exact");

  // With an ICC profile attached the pixels are still identical.
  const icc = new Uint8Array(301).map((_, i) => i * 13);
  const tagged = webpWithIcc(webp, icc, width, height);
  assert.equal(String.fromCharCode(...tagged.subarray(12, 16)), "VP8X");
  assert.equal(tagged[20] & 0x30, 0x30, "ICC and alpha flags");
  assert.equal(new DataView(tagged.buffer).getUint32(4, true), tagged.length - 8, "RIFF size");
  assert.deepEqual(await iccProfile(tagged), icc);
  const again = await decoder.default(tagged);
  assert.ok(Buffer.from(again.data.buffer).equals(Buffer.from(rgba.buffer)));
});

test("ICC profiles are read from PNG iCCP chunks and bit depth from IHDR", async () => {
  const be32 = (n: number) => [(n >>> 24) & 0xff, (n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
  const ascii = (s: string) => [...s].map((c) => c.charCodeAt(0));
  const profile = new Uint8Array(64).map((_, i) => 255 - i);
  const iccp = [...ascii("Display P3"), 0, 0, ...deflateSync(profile)];
  const chunk = (type: string, data: number[]) => [...be32(data.length), ...ascii(type), ...data, 0, 0, 0, 0];
  const png = new Uint8Array([0x89, ...ascii("PNG\r\n\x1a\n"), ...chunk("IHDR", [...be32(1), ...be32(1), 8, 6, 0, 0, 0]), ...chunk("iCCP", iccp), ...chunk("IEND", [])]);
  assert.deepEqual(await iccProfile(png), profile);
  assert.equal(pngBitDepth(png), 8);
});
