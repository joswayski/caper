import assert from "node:assert/strict";
import { test } from "vitest";
import { decodeGif, shrinkGif } from "../account/gif-shrink.ts";

type Colour = [number, number, number];

/**
 * An independent test writer: a 4-colour GIF whose LZW stream clears every
 * two pixels, so it barely compresses and large fixtures stay cheap to build.
 * `transparent` makes colour 3 transparent.
 */
function fixture(
  width: number,
  height: number,
  colours: Colour[],
  frames: { delay: number; pixel: (x: number, y: number) => number }[],
  transparent = false,
): Uint8Array {
  const bytes: number[] = [];
  const word = (value: number) => bytes.push(value & 0xff, value >> 8);
  bytes.push(...Buffer.from("GIF89a"));
  word(width);
  word(height);
  bytes.push(0xf1, 0, 0);
  for (const colour of colours) bytes.push(...colour);
  bytes.push(0x21, 0xff, 11, ...Buffer.from("NETSCAPE2.0"), 3, 1, 0, 0, 0);
  for (const frame of frames) {
    bytes.push(0x21, 0xf9, 4, transparent ? 0x09 : 0x04, frame.delay & 0xff, frame.delay >> 8, 3, 0, 0x2c);
    word(0);
    word(0);
    word(width);
    word(height);
    bytes.push(0, 2);
    const codes: number[] = [];
    let count = 0;
    for (let y = 0; y < height; y++)
      for (let x = 0; x < width; x++) {
        if (count++ % 2 === 0) codes.push(4);
        codes.push(frame.pixel(x, y));
      }
    codes.push(5);
    const data: number[] = [];
    let accumulator = 0;
    let bits = 0;
    for (const code of codes) {
      accumulator |= code << bits;
      bits += 3;
      while (bits >= 8) {
        data.push(accumulator & 0xff);
        accumulator >>= 8;
        bits -= 8;
      }
    }
    if (bits) data.push(accumulator);
    for (let at = 0; at < data.length; at += 255) {
      const block = data.slice(at, at + 255);
      bytes.push(block.length, ...block);
    }
    bytes.push(0);
  }
  bytes.push(0x3b);
  return new Uint8Array(bytes);
}

const palette: Colour[] = [
  [0x63, 0x7a, 0x43],
  [0xf1, 0xed, 0xe4],
  [0xb6, 0x4d, 0x32],
  [0x0c, 0x0d, 0x0f],
];

/** Quadrants that rotate each frame, so every frame differs. */
const quadrants = (width: number, height: number, count: number, delay = 7) =>
  Array.from({ length: count }, (_, frame) => ({
    delay,
    pixel: (x: number, y: number) => ((x < width / 2 ? 0 : 1) + (y < height / 2 ? 0 : 2) + frame) % 4,
  }));

function frames(bytes: Uint8Array) {
  const width = bytes[6] | (bytes[7] << 8);
  const decoded: { delay: number; rgba: Uint8ClampedArray }[] = [];
  decodeGif(bytes, (rgba, delay) => decoded.push({ delay, rgba: rgba.slice() }));
  const at = (frame: number, x: number, y: number) => [
    ...decoded[frame].rgba.subarray((y * width + x) * 4, (y * width + x) * 4 + 4),
  ];
  return { decoded, at, width };
}

const near = (actual: number[], expected: Colour, tolerance = 10) =>
  actual[3] === 255 && actual.slice(0, 3).every((value, index) => Math.abs(value - expected[index]) <= tolerance);

test("the decoder composites every frame with its delay", () => {
  const { decoded, at } = frames(fixture(8, 6, palette, quadrants(8, 6, 3, 12)));
  assert.equal(decoded.length, 3);
  assert.deepEqual(
    decoded.map((frame) => frame.delay),
    [12, 12, 12],
  );
  assert.deepEqual(at(0, 0, 0), [...palette[0], 255]);
  assert.deepEqual(at(1, 7, 5), [...palette[0], 255]); // (1 + 2 + 1) % 4
  assert.deepEqual(at(2, 7, 0), [...palette[3], 255]);
});

test("a GIF over 1024 px shrinks to a 512 px square with every frame and its timing", () => {
  // 1000 × 800 × 14 frames: about 6 MB in this barely-compressed form.
  const input = fixture(1000, 800, palette, quadrants(1000, 800, 14));
  const { bytes, edge } = shrinkGif(input);
  assert.equal(edge, 512);
  assert.ok(bytes.length < 100_000, `${bytes.length} bytes: flat colour and unchanged areas compress`);
  assert.deepEqual([bytes[6] | (bytes[7] << 8), bytes[8] | (bytes[9] << 8)], [512, 512]);
  const { decoded, at } = frames(bytes);
  assert.equal(decoded.length, 14);
  assert.ok(decoded.every((frame) => frame.delay === 7));
  // Centre-cropped (100 px off each side), then halves stay halves.
  for (let frame = 0; frame < 14; frame++) {
    assert.ok(near(at(frame, 10, 10), palette[frame % 4]), `frame ${frame} top left`);
    assert.ok(near(at(frame, 500, 10), palette[(1 + frame) % 4]), `frame ${frame} top right`);
    assert.ok(near(at(frame, 10, 500), palette[(2 + frame) % 4]), `frame ${frame} bottom left`);
    assert.ok(near(at(frame, 500, 500), palette[(3 + frame) % 4]), `frame ${frame} bottom right`);
  }
});

test("a GIF smaller than 512 px keeps its short edge", () => {
  const { bytes, edge } = shrinkGif(fixture(400, 300, palette, quadrants(400, 300, 3)));
  assert.equal(edge, 300);
  assert.equal(frames(bytes).decoded.length, 3);
});

test("transparent pixels stay transparent in every frame", () => {
  const input = fixture(
    300,
    300,
    palette,
    [
      { delay: 4, pixel: (x) => (x < 150 ? 3 : 1) },
      { delay: 4, pixel: (x) => (x < 150 ? 1 : 3) },
    ],
    true,
  );
  const { bytes, edge } = shrinkGif(input);
  assert.equal(edge, 300);
  const { at } = frames(bytes);
  assert.equal(at(0, 10, 150)[3], 0);
  assert.ok(near(at(0, 290, 150), palette[1]));
  assert.ok(near(at(1, 10, 150), palette[1]));
  assert.equal(at(1, 290, 150)[3], 0, "uncovered in the next frame, not left over");
});

test("truncated and non-GIF bytes fail instead of hanging", () => {
  const input = fixture(40, 40, palette, quadrants(40, 40, 2));
  assert.throws(() => shrinkGif(new Uint8Array([1, 2, 3])));
  const { decoded } = frames(input.subarray(0, input.length - 200));
  assert.ok(decoded.length >= 1);
});

test("GIFs of any byte size upload unchanged within 1024 px; wider ones are shrunk", async () => {
  const { preparePicture } = await import("../account/picture-upload.ts");
  // About 9 MB, 1000 × 1000: no byte limit, so it goes up as it is.
  const heavy = new Blob([fixture(1000, 1000, palette, quadrants(1000, 1000, 18)) as BlobPart], { type: "image/gif" });
  assert.ok(heavy.size > 8 * 1024 * 1024);
  const unchanged = await preparePicture(heavy);
  assert.equal(unchanged.blob, heavy);
  const wide = await preparePicture(new Blob([fixture(1200, 800, palette, quadrants(1200, 800, 2)) as BlobPart]));
  assert.deepEqual([wide.contentType, wide.width, wide.height], ["image/gif", 512, 512]);
});
