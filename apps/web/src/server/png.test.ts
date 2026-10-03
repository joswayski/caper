import assert from "node:assert/strict";
import { test } from "node:test";
import { inflateSync } from "node:zlib";
import { encodeIndexedPng, exactPalette } from "../chat/png.ts";

function chunks(png: Uint8Array) {
  assert.deepEqual([...png.subarray(0, 8)], [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
  const view = new DataView(png.buffer, png.byteOffset, png.byteLength);
  const out = new Map<string, Uint8Array[]>();
  for (let offset = 8; offset < png.length;) {
    const length = view.getUint32(offset);
    const type = String.fromCharCode(...png.subarray(offset + 4, offset + 8));
    out.set(type, [...(out.get(type) ?? []), png.subarray(offset + 8, offset + 8 + length)]);
    offset += 12 + length;
  }
  return out;
}

/** Decode an indexed PNG (filter 0 rows) back to RGBA to prove it is lossless. */
function decode(png: Uint8Array) {
  const parts = chunks(png);
  const header = new DataView(parts.get("IHDR")![0].buffer, parts.get("IHDR")![0].byteOffset);
  const [width, height, depth] = [header.getUint32(0), header.getUint32(4), parts.get("IHDR")![0][8]];
  const palette = parts.get("PLTE")![0];
  const alpha = parts.get("tRNS")?.[0] ?? new Uint8Array();
  const raw = inflateSync(Buffer.concat(parts.get("IDAT")!));
  const stride = Math.ceil((width * depth) / 8);
  const rgba = new Uint8Array(width * height * 4);
  for (let y = 0; y < height; y++) {
    assert.equal(raw[y * (stride + 1)], 0);
    for (let x = 0; x < width; x++) {
      const byte = raw[y * (stride + 1) + 1 + Math.floor((x * depth) / 8)];
      const index = (byte >> (8 - depth * ((x % (8 / depth)) + 1))) & ((1 << depth) - 1);
      rgba.set([palette[index * 3], palette[index * 3 + 1], palette[index * 3 + 2], alpha[index] ?? 255], (y * width + x) * 4);
    }
  }
  return { width, height, depth, rgba };
}

function image(width: number, height: number, colors: number[][]) {
  const rgba = new Uint8Array(width * height * 4);
  for (let i = 0; i < width * height; i++) rgba.set(colors[(i * 7 + Math.floor(i / width)) % colors.length], i * 4);
  return rgba;
}

test("flat images round-trip pixel-exactly at the smallest bit depth", async () => {
  for (const [count, depth] of [[2, 1], [4, 2], [9, 4], [200, 8]]) {
    const colors = Array.from({ length: count }, (_, i) => [i * 37 % 256, i * 11 % 256, i * 5 % 256, i === 0 ? 0 : 255]);
    colors[0] = [0, 0, 0, 0];
    const rgba = image(37, 23, colors);
    const png = await encodeIndexedPng(rgba, 37, 23, 256);
    assert.ok(png);
    const decoded = decode(png);
    assert.equal(decoded.depth, depth);
    assert.deepEqual(decoded.rgba, rgba, `${count} colours`);
  }
});

test("too many colours or partial transparency skip the lossless path", async () => {
  const colors = Array.from({ length: 300 }, (_, i) => [i % 256, Math.floor(i / 256), 0, 255]);
  assert.equal(await encodeIndexedPng(image(40, 40, colors), 40, 40, 256), undefined);
  assert.equal(exactPalette(new Uint8Array([1, 2, 3, 128]), 256), undefined);
  assert.equal(exactPalette(new Uint8Array([1, 2, 3, 255, 4, 5, 6, 255]), 1), undefined);
  assert.equal(exactPalette(new Uint8Array([1, 2, 3, 255]), 0), undefined);
});
