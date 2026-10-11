import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { test } from "vitest";
import { AVIF_OPTIONS, encodeAvif, isAvif, tagSrgb } from "../chat/avif.ts";

// The single-threaded libavif build the worker uses, handed its compiled WASM
// (the browser fetches it next to the worker chunk), and the package's decoder.
const root = dirname(createRequire(import.meta.url).resolve("@jsquash/avif/package.json"));
const encoder = await import(join(root, "encode.js"));
const decoder = await import(join(root, "decode.js"));
const codec = await encoder.init(await WebAssembly.compile(await readFile(join(root, "codec/enc/avif_enc.wasm"))));
await decoder.init(await WebAssembly.compile(await readFile(join(root, "codec/dec/avif_dec.wasm"))));

/** A photo-like image: smooth gradients with some fine texture, opaque. */
function photo(width: number, height: number) {
  const rgba = new Uint8ClampedArray(width * height * 4);
  for (let y = 0; y < height; y++)
    for (let x = 0; x < width; x++) {
      const i = (y * width + x) * 4;
      const grain = ((x * 31 + y * 17) % 7) - 3;
      rgba[i] = 40 + (x * 160) / width + grain;
      rgba[i + 1] = 60 + (y * 140) / height + grain;
      rgba[i + 2] = 128 + 80 * Math.sin((x + y) / 23) + grain;
      rgba[i + 3] = 255;
    }
  return rgba;
}

/** The payload of the first ISO BMFF box of `type`, found by its fourcc. */
function box(bytes: Uint8Array, type: string) {
  for (let i = 4; i + 4 <= bytes.length; i++) {
    if (String.fromCharCode(...bytes.subarray(i, i + 4)) !== type) continue;
    const size = new DataView(bytes.buffer, bytes.byteOffset).getUint32(i - 4);
    return bytes.subarray(i + 4, i - 4 + size);
  }
}

test("photos encode as 8-bit 4:2:0 AVIF the API accepts, without metadata, close to the source", async () => {
  const [width, height] = [333, 251];
  const rgba = photo(width, height);
  const raw = codec.encode(rgba, width, height, { ...AVIF_OPTIONS, quality: 85 }) as Uint8Array;
  const cicp = (bytes: Uint8Array) => {
    const nclx = box(bytes, "colr")!;
    assert.equal(String.fromCharCode(...nclx.subarray(0, 4)), "nclx");
    return [0, 2, 4].map((i) => new DataView(nclx.buffer, nclx.byteOffset).getUint16(4 + i));
  };
  assert.deepEqual(cicp(raw), [2, 2, 6], "libavif leaves primaries and transfer unspecified");
  const avif = tagSrgb(raw.slice());
  assert.deepEqual(cicp(avif), [1, 13, 6], "tagged sRGB, BT.601 matrix");
  assert.deepEqual(tagSrgb(avif.slice()), avif, "idempotent");
  assert.equal(avif.length, raw.length);
  assert.ok(isAvif(avif), "ftyp with an AVIF brand");
  assert.equal(String.fromCharCode(...avif.subarray(8, 12)), "avif", "major brand");
  const av1C = box(avif, "av1C");
  assert.ok(av1C, "AV1 codec configuration");
  assert.equal(av1C[2] & 0x60, 0, "8-bit");
  assert.equal(av1C[2] & 0x1c, 0x0c, "4:2:0, not monochrome");
  const ispe = box(avif, "ispe")!;
  assert.deepEqual(
    [new DataView(ispe.buffer, ispe.byteOffset).getUint32(4), new DataView(ispe.buffer, ispe.byteOffset).getUint32(8)],
    [width, height],
  );
  for (const meta of ["Exif", "mime"]) assert.equal(box(avif, meta), undefined, `no ${meta}`);
  assert.equal(box(avif, "auxC"), undefined, "an opaque photo has no alpha plane");

  const decoded = await decoder.default(avif);
  assert.deepEqual(decoded.data, (await decoder.default(raw)).data, "tagging changes no pixels");
  assert.deepEqual([decoded.width, decoded.height], [width, height]);
  let squared = 0;
  for (let i = 0; i < rgba.length; i += 4)
    for (let c = 0; c < 3; c++) squared += (rgba[i + c] - decoded.data[i + c]) ** 2;
  const psnr = 10 * Math.log10(255 ** 2 / (squared / (width * height * 3)));
  assert.ok(psnr > 38, `PSNR ${psnr.toFixed(1)} dB`);
  assert.ok(avif.length < (width * height * 3) / 8, `${avif.length} bytes`);

  // A lower quality is smaller: the server's avifQuality reaches libavif.
  assert.ok((codec.encode(rgba, width, height, { ...AVIF_OPTIONS, quality: 40 }) as Uint8Array).length < avif.length);
});

test("without workers the AVIF encoder is unavailable, so photos fall back to WebP", async () => {
  assert.equal(typeof Worker, "undefined");
  assert.equal(await encodeAvif(photo(8, 8), 8, 8, 85), undefined);
  assert.ok(!isAvif(new TextEncoder().encode("\0\0\0\x18ftypheic")), "other ISO BMFF brands are rejected");
  assert.ok(!isAvif(new TextEncoder().encode("<html>")));
  assert.ok(isAvif(new TextEncoder().encode("\0\0\0\x1cftypmif1")));
});
