import assert from "node:assert/strict";
import { test } from "vitest";
import { deflateSync, inflateSync } from "node:zlib";
import {
  exifOrientation,
  stripJpegMetadata,
  stripMetadata,
  stripMp4Metadata,
  stripPngMetadata,
} from "../chat/metadata.ts";

const ascii = (s: string) => [...s].map((c) => c.charCodeAt(0));
const be16 = (n: number) => [(n >> 8) & 0xff, n & 0xff];
const be32 = (n: number) => [(n >>> 24) & 0xff, (n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
const bytesOf = async (blob: Blob) => new Uint8Array(await blob.arrayBuffer());
const contains = (haystack: Uint8Array, needle: string) =>
  Buffer.from(haystack).includes(Buffer.from(needle, "latin1"));

// ---- MP4 ------------------------------------------------------------------------

const box = (type: string, ...content: number[][]) => {
  const body = content.flat();
  return [...be32(8 + body.length), ...ascii(type), ...body];
};
const LOCATION = "+40.6892-074.0445+010.000/";
const samples = Array.from({ length: 4096 }, (_, i) => (i * 31 + 7) & 0xff);

function iphoneLike(moovFirst: boolean, largeMdat = false) {
  const keys = box("keys", [0, 0, 0, 0, 0, 0, 0, 1], box("mdta", ascii("com.apple.quicktime.location.ISO6709")));
  const ilst = box("ilst", box("\x00\x00\x00\x01", box("data", [0, 0, 0, 1, 0, 0, 0, 0], ascii(LOCATION))));
  const moov = box(
    "moov",
    box(
      "mvhd",
      Array.from({ length: 100 }, () => 0),
    ),
    box(
      "meta",
      [0, 0, 0, 0],
      box(
        "hdlr",
        Array.from({ length: 25 }, () => 0),
      ),
      keys,
      ilst,
    ),
    box(
      "trak",
      box(
        "tkhd",
        Array.from({ length: 84 }, () => 0),
      ),
      box(
        "mdia",
        box(
          "mdhd",
          Array.from({ length: 24 }, () => 0),
        ),
      ),
      box("udta", box("name", ascii("Back Camera"))),
    ),
    box("udta", box("\xa9xyz", [0, 26, 0x15, 0xc7], ascii(LOCATION))),
  );
  const mdat = largeMdat
    ? [...be32(1), ...ascii("mdat"), ...be32(0), ...be32(16 + samples.length), ...samples]
    : box("mdat", samples);
  const ftyp = box("ftyp", ascii("qt  "), [0, 0, 0, 0], ascii("qt  "));
  return new Uint8Array(moovFirst ? [...ftyp, ...moov, ...mdat] : [...ftyp, ...mdat, ...moov]);
}

test("MP4/MOV: location boxes become zero-filled free boxes; size, offsets and samples are unchanged", async () => {
  for (const [moovFirst, large] of [
    [true, false],
    [false, false],
    [false, true],
  ]) {
    const input = iphoneLike(moovFirst, large);
    assert.ok(contains(input, LOCATION));
    const output = await bytesOf(await stripMp4Metadata(new Blob([input], { type: "video/quicktime" })));
    assert.equal(output.length, input.length, "same length");
    assert.ok(!contains(output, LOCATION), "location string gone from every byte");
    assert.ok(!contains(output, "com.apple.quicktime.location"));
    assert.ok(!contains(output, "Back Camera"), "track udta cleared too");
    assert.ok(!contains(output, "udta") && !contains(output, "meta"));
    const at = Buffer.from(input).indexOf(Buffer.from(samples));
    assert.ok(at > 0);
    assert.deepEqual(
      output.subarray(at, at + samples.length),
      new Uint8Array(samples),
      "sample data identical, same offset",
    );
    // Everything outside the cleared boxes is byte-identical (mvhd, tkhd, mdia).
    for (const keep of ["mvhd", "tkhd", "mdhd", "mdia", "trak", "moov", "ftyp"])
      assert.equal(Buffer.from(output).indexOf(keep), Buffer.from(input).indexOf(keep), keep);
  }
});

test("MP4: files without metadata, or that do not parse, are returned untouched", async () => {
  const clean = new Blob([
    new Uint8Array([
      ...box("ftyp", ascii("isom"), [0, 0, 0, 0]),
      ...box("moov", box("mvhd", [0, 0])),
      ...box("mdat", [1, 2, 3]),
    ]),
  ]);
  assert.equal(await stripMp4Metadata(clean), clean);
  const truncated = new Blob([iphoneLike(true).subarray(0, 200)]);
  assert.equal(await stripMp4Metadata(truncated), truncated);
  const notMp4 = new Blob([new Uint8Array(ascii("RIFF....WEBPVP8 "))]);
  assert.equal(await stripMp4Metadata(notMp4), notMp4);
});

// ---- JPEG -----------------------------------------------------------------------

const segment = (marker: number, payload: number[]) => [0xff, marker, ...be16(payload.length + 2), ...payload];
function exif(orientation: number) {
  // Little-endian TIFF with Orientation and a GPS IFD pointer (GPS data inline).
  const entries = [
    [0x0112, 3, 1, orientation],
    [0x8825, 4, 1, 38],
  ];
  const le16 = (n: number) => [n & 0xff, n >> 8];
  const le32 = (n: number) => [n & 0xff, (n >> 8) & 0xff, (n >> 16) & 0xff, n >>> 24];
  const tiff = [
    ...ascii("II"),
    42,
    0,
    ...le32(8),
    ...le16(entries.length),
    ...entries.flatMap(([tag, type, count, value]) => [...le16(tag), ...le16(type), ...le32(count), ...le32(value)]),
    ...le32(0),
    ...ascii("GPS 40N 74W"),
  ];
  return [...ascii("Exif\0\0"), ...tiff];
}
const scan = [0xff, 0xda, 0, 8, 1, 1, 0, 0, 0x3f, 0, 0x12, 0x34, 0xff, 0x00, 0x56, 0xff, 0xd9];

function jpeg(orientation: number) {
  return new Uint8Array([
    0xff,
    0xd8,
    ...segment(0xe0, [...ascii("JFIF\0"), 1, 1, 0, 0, 1, 0, 1, 0, 0]),
    ...segment(0xe1, exif(orientation)),
    ...segment(0xe1, ascii("http://ns.adobe.com/xap/1.0/\0<x:xmpmeta>GPSLatitude 40N</x:xmpmeta>")),
    ...segment(0xe2, [...ascii("ICC_PROFILE\0"), 1, 1, 9, 9, 9]),
    ...segment(0xed, ascii("Photoshop 3.0\0 IPTC city")),
    ...segment(0xee, [...ascii("Adobe"), 0, 100, 0, 0, 0, 0, 1]),
    ...segment(
      0xdb,
      Array.from({ length: 65 }, () => 1),
    ),
    ...segment(0xc0, [8, 0, 1, 0, 1, 1, 1, 0x11, 0]),
    ...scan,
  ]);
}

/** Marker → payload of each segment before the scan. */
const segments = (bytes: Uint8Array) => {
  const found = new Map<number, Uint8Array>();
  for (let offset = 2; bytes[offset + 1] !== 0xda;) {
    const length = (bytes[offset + 2] << 8) | bytes[offset + 3];
    found.set(bytes[offset + 1], bytes.subarray(offset + 4, offset + 2 + length));
    offset += 2 + length;
  }
  return found;
};
const markers = (bytes: Uint8Array) => [...segments(bytes).keys()];

test("JPEG: Exif, XMP, IPTC are removed; JFIF, ICC, Adobe, tables and scan data are kept; orientation survives", async () => {
  const input = jpeg(6);
  const output = await bytesOf(await stripJpegMetadata(new Blob([input], { type: "image/jpeg" })));
  assert.ok(!contains(output, "GPS") && !contains(output, "xmpmeta") && !contains(output, "IPTC"));
  assert.deepEqual(markers(output), [0xe0, 0xe1, 0xe2, 0xee, 0xdb, 0xc0], "a minimal Exif follows JFIF");
  assert.equal(exifOrientation(segments(output).get(0xe1)!), 6);
  assert.deepEqual(output.subarray(output.length - scan.length), new Uint8Array(scan), "scan data byte-identical");

  const upright = await bytesOf(await stripJpegMetadata(new Blob([jpeg(1)])));
  assert.deepEqual(markers(upright), [0xe0, 0xe2, 0xee, 0xdb, 0xc0], "orientation 1 needs no Exif at all");
  assert.equal(exifOrientation(new Uint8Array(exif(8))), 8);
});

test("JPEG: a file with nothing to remove, or not a JPEG, is returned untouched", async () => {
  const clean = new Blob([new Uint8Array([0xff, 0xd8, ...segment(0xe0, ascii("JFIF\0")), ...scan])]);
  assert.equal(await stripJpegMetadata(clean), clean);
  const other = new Blob([new Uint8Array([1, 2, 3, 4, 5])]);
  assert.equal(await stripJpegMetadata(other), other);
});

// ---- PNG ------------------------------------------------------------------------

const CRC = (() => {
  const table = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    table[n] = c >>> 0;
  }
  return (bytes: number[]) => {
    let c = 0xffffffff;
    for (const b of bytes) c = table[(c ^ b) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
})();
const chunk = (type: string, data: number[]) => [
  ...be32(data.length),
  ...ascii(type),
  ...data,
  ...be32(CRC([...ascii(type), ...data])),
];

test("PNG: eXIf and text chunks are dropped; every other chunk, and so every pixel, is byte-identical", async () => {
  const raw = [0, 255, 0, 0, 0, 0, 255, 0]; // one row: filter 0, red, green
  const idat = chunk("IDAT", [...deflateSync(Buffer.from(raw))]);
  const ihdr = chunk("IHDR", [...be32(2), ...be32(1), 8, 2, 0, 0, 0]);
  const input = new Uint8Array([
    0x89,
    ...ascii("PNG\r\n\x1a\n"),
    ...ihdr,
    ...chunk("eXIf", [...ascii("MM"), 0, 42, ...ascii("GPS")]),
    ...chunk("iTXt", [...ascii("XML:com.adobe.xmp\0\0\0\0\0<GPSLatitude/>")]),
    ...idat,
    ...chunk("tEXt", [...ascii("Comment\0taken at home")]),
    ...chunk("zTXt", [...ascii("Raw profile type exif\0"), 0, ...deflateSync(Buffer.from("GPS"))]),
    ...chunk("IEND", []),
  ]);
  const output = await bytesOf(await stripPngMetadata(new Blob([input], { type: "image/png" })));
  assert.deepEqual(output, new Uint8Array([0x89, ...ascii("PNG\r\n\x1a\n"), ...ihdr, ...idat, ...chunk("IEND", [])]));
  const at = Buffer.from(output).indexOf("IDAT") + 4;
  assert.deepEqual([...inflateSync(output.subarray(at, at + idat.length - 12))], raw, "pixels unchanged");
});

test("stripping applies by type and leaves other files alone", async () => {
  const gif = new Blob([new Uint8Array(ascii("GIF89a"))]);
  assert.equal(await stripMetadata(gif, "image/gif"), gif);
  const input = iphoneLike(true);
  assert.ok(!contains(await bytesOf(await stripMetadata(new Blob([input]), "video/mp4")), LOCATION));
});
