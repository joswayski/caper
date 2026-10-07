import assert from "node:assert/strict";
import { test } from "node:test";
import {
  DEFAULT_COMPRESSION, attachmentKind, compressionSettings, declaredType, fitWithin, isHdrColorSpace, keepStill, keepTranscode,
  hdrTransfer, pngIsAnimated, renamed, stillPlans, stillSource, videoPlan, videoTargetKbps, videoTargetSize, webpInfo, type VideoProbe,
} from "../chat/prepare.ts";

const bytes = (...parts: Array<string | number[]>) => new Uint8Array(parts.flatMap((part) => typeof part === "string" ? [...part].map((c) => c.charCodeAt(0)) : part));
const le32 = (n: number) => [n & 0xff, (n >> 8) & 0xff, (n >> 16) & 0xff, (n >> 24) & 0xff];
const be32 = (n: number) => [(n >>> 24) & 0xff, (n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff];
const webp = (...chunks: Array<[string, number[]]>) => bytes("RIFF", le32(0), "WEBP", ...chunks.flatMap(([type, data]) => [type, le32(data.length), data, data.length & 1 ? [0] : []]));
const png = (...chunks: string[]) => bytes([0x89], "PNG\r\n\x1a\n", ...chunks.flatMap((type) => [be32(1), type, [0], [0, 0, 0, 0]]));

test("inline kinds match the API allowlist and SVG/HEIC never render inline", () => {
  assert.equal(attachmentKind("image/png"), "image");
  assert.equal(attachmentKind("video/quicktime"), "video");
  assert.equal(attachmentKind("audio/mpeg"), "audio");
  assert.equal(attachmentKind("image/svg+xml"), "file");
  assert.equal(attachmentKind("image/heic"), "file");
  assert.equal(declaredType({ type: "", name: "IMG_0001.HEIC" }), "image/heic");
  assert.equal(declaredType({ type: "", name: "notes" }), "application/octet-stream");
  assert.equal(renamed("Screenshot 2026.png", "image/webp"), "Screenshot 2026.webp");
  assert.equal(renamed("noext", "image/jpeg"), "noext.jpg");
  assert.equal(renamed("IMG_0001.HEIC", "image/avif"), "IMG_0001.avif");
  assert.deepEqual(fitWithin(3840, 2160, 640), { width: 640, height: 360 });
  assert.deepEqual(fitWithin(300, 200, 640), { width: 300, height: 200 }, "never upscales");
});

test("server compression settings are validated field by field, defaults match the API", () => {
  assert.deepEqual(compressionSettings({ imageQuality: 0, paletteColors: 999, previewEdge: 320, videoBitrateKbps: "fast" }),
    { ...DEFAULT_COMPRESSION, imageFormat: "webp", previewEdge: 320 });
  assert.equal(DEFAULT_COMPRESSION.videoBitrateKbps, 6000);
  assert.equal(DEFAULT_COMPRESSION.imageFormat, "avif");
  assert.equal(DEFAULT_COMPRESSION.avifQuality, 85);
  assert.equal(DEFAULT_COMPRESSION.imageQuality, 92);
  assert.deepEqual(compressionSettings(DEFAULT_COMPRESSION), DEFAULT_COMPRESSION);
  assert.deepEqual(compressionSettings({ ...DEFAULT_COMPRESSION, imageFormat: "webp", avifQuality: 70 }), { ...DEFAULT_COMPRESSION, imageFormat: "webp", avifQuality: 70 });
  assert.deepEqual(compressionSettings({ ...DEFAULT_COMPRESSION, avifQuality: 101, futureField: true }), DEFAULT_COMPRESSION, "invalid quality, unknown fields ignored");
  assert.equal(compressionSettings({ ...DEFAULT_COMPRESSION, avifQuality: 0 }).avifQuality, 85);
  assert.equal(compressionSettings({ ...DEFAULT_COMPRESSION, avifQuality: 1 }).avifQuality, 1);
  // Servers from before AVIF send no imageFormat (or no settings) and keep WebP.
  const { imageFormat: _, ...older } = DEFAULT_COMPRESSION;
  assert.deepEqual(compressionSettings(older), { ...DEFAULT_COMPRESSION, imageFormat: "webp" });
  assert.deepEqual(compressionSettings(null), { ...DEFAULT_COMPRESSION, imageFormat: "webp" });
  assert.equal(compressionSettings({ imageFormat: "AVIF" }).imageFormat, "webp", "unknown formats get WebP");
  assert.equal(compressionSettings({ imageFormat: "jxl" }).imageFormat, "webp");
});

test("container sniffing tells lossless from lossy WebP and finds animation", () => {
  assert.deepEqual(webpInfo(webp(["VP8L", [1, 2, 3]])), { lossless: true, animated: false });
  assert.deepEqual(webpInfo(webp(["VP8 ", [1, 2]])), { lossless: false, animated: false });
  assert.deepEqual(webpInfo(webp(["VP8X", [0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0]], ["ICCP", [1, 2, 3]], ["VP8L", [1]])), { lossless: true, animated: false });
  assert.deepEqual(webpInfo(webp(["VP8X", [0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0]], ["ANIM", [0, 0, 0, 0, 0, 0]])), { lossless: false, animated: true });
  assert.equal(webpInfo(bytes("GIF89a")), undefined);
  assert.ok(pngIsAnimated(png("IHDR", "acTL", "IDAT")));
  assert.ok(!pngIsAnimated(png("IHDR", "IDAT", "acTL")));
  assert.ok(!pngIsAnimated(bytes("not a png")));
});

test("sources are classified: lossless, photo, or left alone", () => {
  assert.equal(stillSource("image/png", png("IHDR", "IDAT")), "lossless");
  assert.equal(stillSource("image/png", png("IHDR", "acTL", "IDAT")), "keep", "APNG keeps its frames");
  assert.equal(stillSource("image/bmp", new Uint8Array()), "lossless");
  assert.equal(stillSource("image/tiff", new Uint8Array()), "lossless");
  assert.equal(stillSource("image/webp", webp(["VP8L", [1]])), "lossless");
  assert.equal(stillSource("image/webp", webp(["VP8 ", [1]])), "photo");
  assert.equal(stillSource("image/webp", webp(["VP8X", [0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0]], ["ANIM", [0]])), "keep");
  assert.equal(stillSource("image/jpeg", new Uint8Array()), "photo");
  assert.equal(stillSource("image/heic", new Uint8Array()), "photo");
  assert.equal(stillSource("image/heif", new Uint8Array()), "photo");
  for (const type of ["image/gif", "image/svg+xml", "image/avif"]) assert.equal(stillSource(type, new Uint8Array()), "keep", type);
});

const px = (overrides: Partial<{ fitsPalette: boolean; opaque: boolean; binaryAlpha: boolean }> = {}) => ({ fitsPalette: false, opaque: true, binaryAlpha: true, ...overrides });

test("lossless stays lossless: indexed PNG and lossless WebP compete, otherwise the original", () => {
  const s = DEFAULT_COMPRESSION;
  assert.deepEqual(stillPlans("lossless", "image/png", s, px({ fitsPalette: true })), ["indexed-png", "lossless-webp"], "smallest of the two wins");
  assert.deepEqual(stillPlans("lossless", "image/png", s, px()), ["lossless-webp"], "a real screenshot is compressed losslessly");
  assert.deepEqual(stillPlans("lossless", "image/webp", s, px()), ["lossless-webp"]);
  assert.deepEqual(stillPlans("lossless", "image/png", s, px({ opaque: false })), ["lossless-webp"], "on/off transparency round-trips exactly");
  assert.deepEqual(stillPlans("lossless", "image/png", s, px({ opaque: false, binaryAlpha: false })), [], "partial alpha keeps the original");
  assert.deepEqual(stillPlans("lossless", "image/png", { ...s, paletteColors: 0 }, px({ fitsPalette: true })), ["lossless-webp"]);
  assert.deepEqual(stillPlans("lossless", "image/png", { ...s, imageQuality: 50 }, px()), ["lossless-webp"], "quality never applies to lossless sources");
  assert.deepEqual(stillPlans("lossless", "image/bmp", s, px()), ["lossless-webp", "lossless-png"], "BMP becomes a viewable lossless image");
  assert.deepEqual(stillPlans("lossless", "image/bmp", s, px({ opaque: false, binaryAlpha: false })), []);
  assert.ok(keepStill("indexed-png", { type: "image/png", size: 1000 }, 990), "any lossless saving is kept");
  assert.ok(keepStill("lossless-webp", { type: "image/png", size: 1000 }, 999));
  assert.ok(!keepStill("indexed-png", { type: "image/png", size: 1000 }, 1000));
  assert.ok(keepStill("lossless-png", { type: "image/bmp", size: 1000 }, 1500), "non-inline sources convert even when larger");
  const ihdr16 = bytes([0x89], "PNG\r\n\x1a\n", be32(13), "IHDR", be32(1), be32(1), [16, 2, 0, 0, 0], [0, 0, 0, 0]);
  assert.equal(stillSource("image/png", ihdr16), "keep", "16-bit PNG cannot round-trip through a canvas");
});

test("photos re-encode lossily and keep the result only when at least 10% smaller; HEIC always converts", () => {
  const s = { ...DEFAULT_COMPRESSION, imageFormat: "webp" as const };
  assert.deepEqual(stillPlans("photo", "image/jpeg", s, px()), ["lossy"]);
  assert.deepEqual(stillPlans("photo", "image/jpeg", { ...s, imageQuality: 100 }, px()), []);
  assert.deepEqual(stillPlans("photo", "image/heic", { ...s, imageQuality: 100 }, px()), ["lossy"], "HEIC must convert to be viewable");
  assert.deepEqual(stillPlans("keep", "image/gif", s, px()), []);
  assert.ok(keepStill("lossy", { type: "image/jpeg", size: 1000 }, 900));
  assert.ok(!keepStill("lossy", { type: "image/jpeg", size: 1000 }, 901));
  assert.ok(!keepStill("lossy", { type: "image/webp", size: 1000 }, 950));
  assert.ok(keepStill("lossy", { type: "image/heic", size: 1000 }, 1400));
  assert.ok(!keepStill("keep", { type: "image/heic", size: 1000 }, 10));
});

test("photos become AVIF when the server asks (WebP when encoding fails), never screenshots", () => {
  const s = DEFAULT_COMPRESSION;
  for (const type of ["image/jpeg", "image/heic", "image/heif"]) assert.deepEqual(stillPlans("photo", type, s, px()), ["avif"], type);
  assert.deepEqual(stillPlans("photo", "image/webp", s, px()), ["avif"], "lossy WebP");
  assert.deepEqual(stillPlans("photo", "image/jpeg", { ...s, imageQuality: 100 }, px()), [], "quality 100 disables lossy re-encoding in either format");
  assert.deepEqual(stillPlans("photo", "image/heic", { ...s, imageQuality: 100 }, px()), ["avif"], "HEIC still converts");
  assert.deepEqual(stillPlans("photo", "image/jpeg", { ...s, avifQuality: 100 }, px()), ["avif"], "AVIF quality alone never disables");
  assert.deepEqual(stillPlans("lossless", "image/png", s, px({ fitsPalette: true })), ["indexed-png", "lossless-webp"]);
  assert.deepEqual(stillPlans("lossless", "image/bmp", s, px()), ["lossless-webp", "lossless-png"]);
  assert.deepEqual(stillPlans("keep", "image/avif", s, px()), [], "AVIF sources stay as they are");
  assert.ok(keepStill("avif", { type: "image/jpeg", size: 1000 }, 900));
  assert.ok(!keepStill("avif", { type: "image/jpeg", size: 1000 }, 901), "same 10% rule as WebP");
  assert.ok(keepStill("avif", { type: "image/heic", size: 1000 }, 1400));
});

const probe = (overrides: Partial<VideoProbe> = {}): VideoProbe => ({ width: 1920, height: 1080, codec: "avc", bitrateKbps: 6000, hdr: false, inlineContainer: true, ...overrides });

test("videos only shrink so the short edge fits, keeping even dimensions", () => {
  assert.deepEqual(videoTargetSize(3840, 2160, 1080), { height: 1080 });
  assert.deepEqual(videoTargetSize(2160, 3840, 1080), { width: 1080 }, "portrait phone video stays 1080p");
  assert.equal(videoTargetSize(1080, 1920, 1080), undefined);
  assert.equal(videoTargetSize(1280, 720, 1080), undefined);
  assert.equal(videoTargetSize(3840, 2160, 0), undefined);
  assert.deepEqual(videoTargetSize(3840, 2160, 721), { height: 720 });
});

test("target bitrate scales with pixels from the 1080p setting, with a floor", () => {
  assert.equal(videoTargetKbps(1920, 1080, DEFAULT_COMPRESSION), 6000);
  assert.equal(videoTargetKbps(1280, 720, DEFAULT_COMPRESSION), 2667);
  assert.equal(videoTargetKbps(640, 360, DEFAULT_COMPRESSION), 1500, "floor");
});

test("efficient H.264 uploads unchanged; transcodes happen only for size, codec, container or bitrate", () => {
  const s = DEFAULT_COMPRESSION;
  assert.deepEqual(videoPlan(probe(), s), { action: "keep", reason: "efficient" });
  assert.deepEqual(videoPlan(probe({ bitrateKbps: 7500 }), s), { action: "keep", reason: "efficient" }, "within 1.25× of target");
  assert.deepEqual(videoPlan(probe({ width: 1080, height: 1920, bitrateKbps: undefined }), s), { action: "keep", reason: "efficient" });
  assert.deepEqual(videoPlan(probe({ bitrateKbps: 7501 }), s), { action: "transcode", reasons: ["bitrate"], width: 1920, height: 1080, bitrateKbps: 6000 });
  assert.deepEqual(videoPlan(probe({ width: 1280, height: 720, bitrateKbps: 3400 }), s), { action: "transcode", reasons: ["bitrate"], width: 1280, height: 720, bitrateKbps: 2667 });
  assert.deepEqual(videoPlan(probe({ codec: "hevc", bitrateKbps: 4000 }), s), { action: "transcode", reasons: ["codec"], width: 1920, height: 1080, bitrateKbps: 6000 }, "HEVC does not play everywhere");
  assert.deepEqual(videoPlan(probe({ codec: "vp9" }), s).action, "transcode");
  assert.deepEqual(videoPlan(probe({ width: 3840, height: 2160, bitrateKbps: 40000 }), s), { action: "transcode", reasons: ["resolution", "bitrate"], width: 1920, height: 1080, bitrateKbps: 6000 });
  assert.deepEqual(videoPlan(probe({ width: 2160, height: 3840, bitrateKbps: 8000 }), s), { action: "transcode", reasons: ["resolution"], width: 1080, height: 1920, bitrateKbps: 6000 });
  assert.deepEqual(videoPlan(probe({ inlineContainer: false }), s), { action: "transcode", reasons: ["container"], width: 1920, height: 1080, bitrateKbps: 6000 });
  assert.deepEqual(videoPlan(probe({ codec: "hevc" }), { ...s, videoMaxHeight: 0 }), { action: "keep", reason: "disabled" });
});

test("HDR video is always tone mapped to SDR, and kept only when smaller unless needed for playback", () => {
  assert.deepEqual(videoPlan(probe({ hdr: true, codec: "hevc", width: 3840, height: 2160, bitrateKbps: 50000 }), DEFAULT_COMPRESSION),
    { action: "transcode", reasons: ["hdr", "resolution", "codec", "bitrate"], width: 1920, height: 1080, bitrateKbps: 6000 });
  assert.deepEqual(videoPlan(probe({ hdr: true }), DEFAULT_COMPRESSION), { action: "transcode", reasons: ["hdr"], width: 1920, height: 1080, bitrateKbps: 6000 });
  assert.deepEqual(videoPlan(probe({ hdr: true }), { ...DEFAULT_COMPRESSION, videoMaxHeight: 0 }), { action: "keep", reason: "disabled" });
  assert.ok(keepTranscode(["hdr"], 1000, 999));
  assert.ok(!keepTranscode(["hdr"], 1000, 1000));
  assert.equal(hdrTransfer({ transfer: "hlg" }), "hlg");
  assert.equal(hdrTransfer({ transfer: "bt709" }), undefined);
  assert.ok(isHdrColorSpace({ primaries: "bt2020", transfer: "pq" } as { transfer: string }));
  assert.ok(isHdrColorSpace({ transfer: "hlg" }));
  assert.ok(!isHdrColorSpace({ transfer: "bt709" }));
  assert.ok(!isHdrColorSpace(undefined));
});

test("transcodes are kept for playability, for resolution when smaller, for bitrate only when 10% smaller", () => {
  assert.ok(keepTranscode(["codec"], 1000, 1500));
  assert.ok(keepTranscode(["container"], 1000, 1100));
  assert.ok(keepTranscode(["resolution"], 1000, 999));
  assert.ok(!keepTranscode(["resolution", "bitrate"], 1000, 1000));
  assert.ok(keepTranscode(["bitrate"], 1000, 900));
  assert.ok(!keepTranscode(["bitrate"], 1000, 901));
});
