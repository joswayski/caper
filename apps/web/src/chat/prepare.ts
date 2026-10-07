// Client-side compression before upload (docs/media.md, "Client compression
// and previews"). The server never transcodes; it only verifies stored bytes,
// so everything here saves the sender's quota and everyone's bandwidth. The
// rule throughout: smaller without visible loss, else the original file.
import type { ChatAttachmentKind } from "./types.ts";
import { encodeIndexedPng, exactPalette } from "./png.ts";
import { stripMetadata } from "./metadata.ts";
import { encodeLosslessWebp, iccProfile, pngBitDepth, pngChunks, webpWithIcc } from "./webp.ts";
import type { HdrTransfer } from "./hdr.ts";

const PREVIEW_MAX_BYTES = 512 * 1024;
const PREVIEW_QUALITY = 0.8;
/** A re-encode must save at least this fraction to be worth a generation loss. */
const MIN_SAVING = 0.1;
/** Bitrate above `BITRATE_SLACK` × target is "inefficient" and worth re-encoding. */
const BITRATE_SLACK = 1.25;
const MIN_VIDEO_KBPS = 1500;
const FULL_HD_PIXELS = 1920 * 1080;
// Decoding enormous images can exhaust memory on phones; upload those as-is.
const MAX_COMPRESS_PIXELS = 50_000_000;

/** Server-tunable (`ASSET_*` settings), delivered with `/api/assets/usage`. */
export interface CompressionSettings {
  imageQuality: number;
  imageMaxEdge: number;
  paletteColors: number;
  previewEdge: number;
  videoMaxHeight: number;
  videoBitrateKbps: number;
  audioBitrateKbps: number;
}

/** Mirrors `Compression::default()` in `apps/api/src/assets.rs`. */
export const DEFAULT_COMPRESSION: CompressionSettings = {
  imageQuality: 92, imageMaxEdge: 4096, paletteColors: 256, previewEdge: 640,
  videoMaxHeight: 1080, videoBitrateKbps: 6000, audioBitrateKbps: 128,
};

/** Unknown or invalid fields fall back to defaults, so older servers work. */
export function compressionSettings(value: unknown): CompressionSettings {
  const input = (value && typeof value === "object" ? value : {}) as Record<string, unknown>;
  const pick = (key: keyof CompressionSettings, min: number, max: number) => {
    const v = input[key];
    return typeof v === "number" && Number.isInteger(v) && v >= min && v <= max ? v : DEFAULT_COMPRESSION[key];
  };
  return {
    imageQuality: pick("imageQuality", 1, 100), imageMaxEdge: pick("imageMaxEdge", 0, 32_768),
    paletteColors: pick("paletteColors", 0, 256), previewEdge: pick("previewEdge", 64, 2048),
    videoMaxHeight: pick("videoMaxHeight", 0, 4320), videoBitrateKbps: pick("videoBitrateKbps", 250, 50_000),
    audioBitrateKbps: pick("audioBitrateKbps", 32, 320),
  };
}

/** Mirrors `assets::kind` on the API: only these render inline. */
export function attachmentKind(contentType: string): ChatAttachmentKind {
  if (["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif"].includes(contentType)) return "image";
  if (["video/mp4", "video/webm", "video/quicktime"].includes(contentType)) return "video";
  if (["audio/mpeg", "audio/mp4", "audio/x-m4a", "audio/aac", "audio/ogg", "audio/wav", "audio/x-wav", "audio/webm", "audio/flac"].includes(contentType)) return "audio";
  return "file";
}

/** Longest edge scaled to `edge`, never enlarged. */
export function fitWithin(width: number, height: number, edge: number) {
  const scale = Math.min(1, edge / Math.max(width, height));
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

export function renamed(name: string, contentType: string) {
  const extension = ({ "image/webp": "webp", "image/jpeg": "jpg", "image/png": "png", "video/mp4": "mp4" } as Record<string, string>)[contentType];
  if (!extension) return name;
  const dot = name.lastIndexOf(".");
  return `${dot > 0 ? name.slice(0, dot) : name}.${extension}`;
}

/** The browser's type, or one inferred from the name when browsers leave it
 * empty (HEIC on some platforms), or the API's generic fallback. */
export function declaredType(file: { type: string; name: string }) {
  if (file.type) return file.type;
  const extension = file.name.toLowerCase().split(".").pop();
  return ({ heic: "image/heic", heif: "image/heif" } as Record<string, string>)[extension ?? ""] ?? "application/octet-stream";
}

// ---- Stills -----------------------------------------------------------------

/**
 * - `lossless`: PNG, BMP, TIFF, lossless WebP. Never lossy-encoded. (16-bit
 *   PNG is kept: a canvas holds 8 bits per channel.)
 * - `photo`: JPEG, HEIC/HEIF, lossy WebP. Re-encoded at `imageQuality`.
 * - `keep`: everything else, including GIF, SVG, AVIF and animated PNG/WebP
 *   (a canvas would keep only the first frame).
 */
export type StillSource = "lossless" | "photo" | "keep";

const ascii = (bytes: Uint8Array, offset: number, text: string) =>
  offset + text.length <= bytes.length && [...text].every((char, i) => bytes[offset + i] === char.charCodeAt(0));

/** WebP container details from its RIFF chunks (the first bytes are enough
 * unless an ICC profile is huge; then it reads as lossy, the safe side for
 * animation but not for losslessness, so callers pass a generous prefix). */
export function webpInfo(bytes: Uint8Array): { lossless: boolean; animated: boolean } | undefined {
  if (!ascii(bytes, 0, "RIFF") || !ascii(bytes, 8, "WEBP")) return;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let lossless = false, animated = false;
  for (let offset = 12; offset + 8 <= bytes.length;) {
    const size = view.getUint32(offset + 4, true);
    if (ascii(bytes, offset, "VP8L")) lossless = true;
    if (ascii(bytes, offset, "VP8X") && offset + 9 <= bytes.length && bytes[offset + 8] & 0x02) animated = true;
    if (ascii(bytes, offset, "ANIM") || ascii(bytes, offset, "ANMF")) animated = true;
    if (ascii(bytes, offset, "VP8 ") || ascii(bytes, offset, "VP8L")) break;
    offset += 8 + size + (size & 1);
  }
  return { lossless, animated };
}

/** APNG declares `acTL` before the first `IDAT`. */
export function pngIsAnimated(bytes: Uint8Array) {
  if (bytes.length < 8 || bytes[0] !== 0x89 || !ascii(bytes, 1, "PNG")) return false;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  for (let offset = 8; offset + 8 <= bytes.length;) {
    if (ascii(bytes, offset + 4, "acTL")) return true;
    if (ascii(bytes, offset + 4, "IDAT")) return false;
    offset += 12 + view.getUint32(offset);
  }
  return false;
}

export function stillSource(contentType: string, head: Uint8Array): StillSource {
  switch (contentType) {
    case "image/png": return pngIsAnimated(head) || pngBitDepth(head) === 16 ? "keep" : "lossless";
    case "image/bmp": case "image/x-ms-bmp": case "image/tiff": return "lossless";
    case "image/jpeg": case "image/heic": case "image/heif": return "photo";
    case "image/webp": {
      const info = webpInfo(head);
      return !info || info.animated ? "keep" : info.lossless ? "lossless" : "photo";
    }
    default: return "keep";
  }
}

export type StillPlan = "indexed-png" | "lossless-webp" | "lossless-png" | "lossy" | "keep";

export interface StillPixels {
  /** At most `paletteColors` exact colours (and no partial transparency). */
  fitsPalette: boolean;
  /** Every pixel fully opaque. */
  opaque: boolean;
  /** Alpha only 0 or 255: a canvas round-trips such pixels exactly, while
   * partially transparent ones are premultiplied and lose precision. */
  binaryAlpha: boolean;
}

/** Candidate encodings for a decodable still; the smallest one `keepStill`
 * accepts wins, and none means the original. Lossless sources stay lossless:
 * an exact indexed PNG when the colours fit, lossless WebP (libwebp), and a
 * canvas PNG for opaque formats browsers cannot show (BMP, TIFF) in case WebP
 * is unavailable. HEIC is converted even at quality 100. */
export function stillPlans(source: StillSource, contentType: string, settings: CompressionSettings, pixels: StillPixels): StillPlan[] {
  if (source === "lossless") {
    const plans: StillPlan[] = [];
    if (pixels.fitsPalette && settings.paletteColors > 0) plans.push("indexed-png");
    if (pixels.binaryAlpha) plans.push("lossless-webp");
    if (attachmentKind(contentType) === "file" && pixels.opaque) plans.push("lossless-png");
    return plans;
  }
  if (source === "photo") return settings.imageQuality < 100 || isHeic(contentType) ? ["lossy"] : [];
  return [];
}

const isHeic = (contentType: string) => contentType === "image/heic" || contentType === "image/heif";

/** Keep an encode only when it pays: lossless output whenever smaller (or the
 * original cannot be shown inline), photos only when at least 10% smaller,
 * and HEIC always (browsers cannot show it). */
export function keepStill(plan: StillPlan, original: { type: string; size: number }, encodedSize: number) {
  if (plan === "keep") return false;
  if (attachmentKind(original.type) === "file") return true;
  if (plan === "lossy") return encodedSize <= original.size * (1 - MIN_SAVING);
  return encodedSize < original.size;
}

export interface PreparedFile {
  blob: Blob;
  name: string;
  contentType: string;
  kind: ChatAttachmentKind;
  sourceSize: number;
  width?: number;
  height?: number;
  durationMs?: number;
  preview?: Blob;
}

type Surface = OffscreenCanvas | HTMLCanvasElement;
type Context2D = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

function surface(width: number, height: number): Surface {
  if (typeof OffscreenCanvas !== "undefined") return new OffscreenCanvas(width, height);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  return canvas;
}

async function encode(canvas: Surface, type: string, quality?: number): Promise<Blob | undefined> {
  const blob = "convertToBlob" in canvas
    ? await canvas.convertToBlob({ type, quality }).catch(() => undefined)
    : await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, type, quality));
  // Browsers that cannot encode a type silently return PNG instead.
  return blob && blob.type === type ? blob : undefined;
}

function draw(source: CanvasImageSource, width: number, height: number) {
  const canvas = surface(width, height);
  const context = canvas.getContext("2d") as Context2D | null;
  if (!context) return;
  context.imageSmoothingQuality = "high";
  context.drawImage(source, 0, 0, width, height);
  return { canvas, context };
}

async function webpOrJpeg(canvas: Surface, quality: number) {
  const webp = await encode(canvas, "image/webp", quality);
  if (webp) return webp;
  // JPEG has no alpha: flatten onto white rather than letting it turn black.
  const flat = surface(canvas.width, canvas.height);
  const context = flat.getContext("2d") as Context2D | null;
  if (!context) return;
  context.fillStyle = "#fff";
  context.fillRect(0, 0, flat.width, flat.height);
  context.drawImage(canvas, 0, 0);
  return encode(flat, "image/jpeg", quality);
}

async function preview(source: CanvasImageSource, width: number, height: number, edge: number) {
  const size = fitWithin(width, height, edge);
  const drawn = draw(source, size.width, size.height);
  const blob = drawn && await webpOrJpeg(drawn.canvas, PREVIEW_QUALITY);
  return blob && blob.size <= PREVIEW_MAX_BYTES ? blob : undefined;
}

function alphaKinds(rgba: Uint8ClampedArray) {
  let opaque = true;
  for (let i = 3; i < rgba.length; i += 4) {
    if (rgba[i] === 255) continue;
    if (rgba[i] !== 0) return { opaque: false, binaryAlpha: false };
    opaque = false;
  }
  return { opaque, binaryAlpha: true };
}

async function encodeStill(plan: StillPlan, drawn: { canvas: Surface; context: Context2D }, rgba: Uint8ClampedArray | undefined, source: Uint8Array | undefined, settings: CompressionSettings): Promise<Blob | undefined> {
  const { width, height } = drawn.canvas;
  switch (plan) {
    case "indexed-png": {
      const indexed = rgba && await encodeIndexedPng(rgba, width, height, settings.paletteColors, source && pngChunks(source, "iCCP")[0]);
      return indexed && new Blob([indexed as BlobPart], { type: "image/png" });
    }
    case "lossless-webp": {
      const webp = rgba && await encodeLosslessWebp(rgba, width, height);
      if (!webp) return;
      // Pixels were read without colour conversion; keep their profile.
      const icc = source && await iccProfile(source);
      return new Blob([(icc ? webpWithIcc(webp, icc, width, height) : webp) as BlobPart], { type: "image/webp" });
    }
    case "lossless-png": return encode(drawn.canvas, "image/png");
    case "lossy": return webpOrJpeg(drawn.canvas, settings.imageQuality / 100);
    case "keep": return;
  }
}

async function prepareImage(file: File, contentType: string, settings: CompressionSettings): Promise<PreparedFile> {
  const base: PreparedFile = { blob: file, name: file.name, contentType, kind: attachmentKind(contentType), sourceSize: file.size };
  if (typeof createImageBitmap === "undefined") return base;
  const head = new Uint8Array(await file.slice(0, 256 * 1024).arrayBuffer());
  const source = stillSource(contentType, head);
  // Lossless sources decode without colour or alpha conversion so a palette
  // encode is pixel-exact; photos convert to sRGB like the canvas they land on.
  // Both apply EXIF orientation, which re-encoding then bakes in.
  const options: ImageBitmapOptions = source === "lossless"
    ? { colorSpaceConversion: "none", premultiplyAlpha: "none", imageOrientation: "from-image" }
    : { imageOrientation: "from-image" };
  const bitmap = await createImageBitmap(file, options).catch(() => undefined);
  if (!bitmap) return base;
  try {
    const original = { width: bitmap.width, height: bitmap.height };
    let prepared: PreparedFile = { ...base, ...original };
    if (source !== "keep" && original.width * original.height <= MAX_COMPRESS_PIXELS) {
      // Only photos are scaled: resampling a screenshot blurs its text.
      const size = source === "photo" && settings.imageMaxEdge > 0 ? fitWithin(original.width, original.height, settings.imageMaxEdge) : original;
      const drawn = draw(bitmap, size.width, size.height);
      if (drawn) {
        let rgba: Uint8ClampedArray | undefined;
        let bytes: Uint8Array | undefined;
        let pixels: StillPixels = { fitsPalette: false, opaque: false, binaryAlpha: false };
        if (source === "lossless") {
          rgba = drawn.context.getImageData(0, 0, size.width, size.height).data;
          bytes = new Uint8Array(await file.arrayBuffer());
          pixels = { fitsPalette: settings.paletteColors > 0 && !!exactPalette(rgba, settings.paletteColors), ...alphaKinds(rgba) };
        }
        // Re-encoding drops EXIF metadata such as photo GPS coordinates.
        let best: { plan: StillPlan; blob: Blob } | undefined;
        for (const plan of stillPlans(source, contentType, settings, pixels)) {
          if (plan === "lossless-png" && best) continue; // only a fallback when WebP failed
          const encoded = await encodeStill(plan, drawn, rgba, bytes, settings).catch(() => undefined);
          if (encoded && keepStill(plan, { type: contentType, size: file.size }, encoded.size) && (!best || encoded.size < best.blob.size)) best = { plan, blob: encoded };
        }
        if (best) {
          prepared = { ...prepared, ...size, blob: best.blob, contentType: best.blob.type, kind: "image", name: renamed(file.name, best.blob.type) };
        }
      }
    }
    const edge = settings.previewEdge;
    if (prepared.kind === "image" && (Math.max(original.width, original.height) > edge || prepared.blob.size > PREVIEW_MAX_BYTES)) {
      prepared.preview = await preview(bitmap, original.width, original.height, edge);
    }
    return prepared;
  } finally {
    bitmap.close();
  }
}

// ---- Video ------------------------------------------------------------------

/** Output size for a transcode, or undefined to keep the original size. The
 * limit bounds the short edge ("1080p"), so portrait phone video keeps detail. */
export function videoTargetSize(width: number, height: number, maxShortEdge: number): { width: number } | { height: number } | undefined {
  if (maxShortEdge <= 0 || Math.min(width, height) <= maxShortEdge) return undefined;
  const even = Math.max(2, Math.floor(maxShortEdge / 2) * 2);
  return width < height ? { width: even } : { height: even };
}

/** Target video bitrate: `videoBitrateKbps` at 1080p, scaled by pixel count,
 * with a floor so small videos are not starved. */
export function videoTargetKbps(width: number, height: number, settings: CompressionSettings) {
  return Math.max(MIN_VIDEO_KBPS, Math.round(settings.videoBitrateKbps * (width * height) / FULL_HD_PIXELS));
}

export interface VideoProbe {
  /** Display size, after rotation. */
  width: number;
  height: number;
  /** Mediabunny codec id: "avc", "hevc", "vp9", "av1", ... */
  codec: string | null;
  /** Video-only bitrate, when it could be estimated. */
  bitrateKbps?: number;
  hdr: boolean;
  /** The container is one the API serves inline (MP4, WebM, QuickTime). */
  inlineContainer: boolean;
}

export type VideoReason = "hdr" | "resolution" | "codec" | "container" | "bitrate";
export type VideoPlan =
  | { action: "keep"; reason: "disabled" | "efficient" }
  | { action: "transcode"; reasons: VideoReason[]; width: number; height: number; bitrateKbps: number };

/** Transcode only when needed; otherwise upload the original untouched so
 * already-efficient phone video is never re-compressed. HDR is always tone
 * mapped to SDR (`hdr.ts`); the caller keeps the original when it cannot. */
export function videoPlan(probe: VideoProbe, settings: CompressionSettings): VideoPlan {
  if (settings.videoMaxHeight <= 0) return { action: "keep", reason: "disabled" };
  const reasons: VideoReason[] = probe.hdr ? ["hdr"] : [];
  const scaled = videoTargetSize(probe.width, probe.height, settings.videoMaxHeight);
  if (scaled) reasons.push("resolution");
  if (probe.codec !== "avc") reasons.push("codec");
  if (!probe.inlineContainer) reasons.push("container");
  const sourceTarget = videoTargetKbps(probe.width, probe.height, settings);
  if (probe.bitrateKbps !== undefined && probe.bitrateKbps > sourceTarget * BITRATE_SLACK) reasons.push("bitrate");
  if (!reasons.length) return { action: "keep", reason: "efficient" };
  const output = !scaled ? { width: probe.width, height: probe.height }
    : "height" in scaled ? { width: Math.round(probe.width * scaled.height / probe.height / 2) * 2, height: scaled.height }
    : { width: scaled.width, height: Math.round(probe.height * scaled.width / probe.width / 2) * 2 };
  return { action: "transcode", reasons, ...output, bitrateKbps: videoTargetKbps(output.width, output.height, settings) };
}

/** A transcode for playability (codec or container) is always kept; one for
 * HDR or resolution only when smaller; a bitrate-only one only when ≥ 10%
 * smaller. */
export function keepTranscode(reasons: VideoReason[], originalSize: number, outputSize: number) {
  if (reasons.includes("codec") || reasons.includes("container")) return true;
  if (reasons.includes("resolution") || reasons.includes("hdr")) return outputSize < originalSize;
  return outputSize <= originalSize * (1 - MIN_SAVING);
}

/** HDR transfer functions (BT.2100 PQ / HLG), from container colour metadata. */
export function isHdrColorSpace(space: { transfer?: string | null } | undefined) {
  return hdrTransfer(space) !== undefined;
}

export function hdrTransfer(space: { transfer?: string | null } | undefined): HdrTransfer | undefined {
  return space?.transfer === "pq" || space?.transfer === "hlg" ? space.transfer : undefined;
}

/** Audio codecs MP4 carries and browsers play, copied without re-encoding. */
const COPY_AUDIO = new Set(["aac", "opus", "mp3"]);

/** Re-encode to H.264 MP4 with WebCodecs when `videoPlan` asks for it.
 * Returns undefined to keep the original: unsupported browser or codec, HDR,
 * a dropped track, cancellation, or not enough saving. */
async function transcodeVideo(file: File, contentType: string, settings: CompressionSettings, progress: (fraction: number) => void, signal?: AbortSignal): Promise<Blob | undefined> {
  if (settings.videoMaxHeight <= 0 || typeof VideoEncoder === "undefined" || signal?.aborted) return;
  const media = await import("mediabunny");
  const input = new media.Input({ source: new media.BlobSource(file), formats: media.ALL_FORMATS });
  try {
    const track = await input.getPrimaryVideoTrack();
    if (!track || !await track.canDecode()) return;
    const audio = await input.getPrimaryAudioTrack();
    const [width, height, codec, hdr, colorSpace, duration] = await Promise.all([
      track.getDisplayWidth(), track.getDisplayHeight(), track.getCodec(),
      track.hasHighDynamicRange().catch(() => false), track.getColorSpace().catch(() => undefined),
      input.computeDuration().catch(() => 0),
    ]);
    // Video bitrate from the whole file minus a sampled audio bitrate.
    let bitrateKbps: number | undefined;
    if (duration > 0) {
      const audioBps = audio ? (await audio.computePacketStats(500).catch(() => undefined))?.averageBitrate ?? 0 : 0;
      bitrateKbps = Math.max(0, (file.size * 8 / duration - audioBps) / 1000);
    }
    const plan = videoPlan({
      width, height, codec, bitrateKbps, hdr: hdr || isHdrColorSpace(colorSpace),
      inlineContainer: attachmentKind(contentType) === "video",
    }, settings);
    if (plan.action === "keep") return;
    const bitrate = plan.bitrateKbps * 1000;
    const hdrMode = plan.reasons.includes("hdr");
    // H.264 in a container browsers will not play inline is only remuxed.
    const remux = !hdrMode && codec === "avc" && plan.reasons.every((reason) => reason === "container");
    if (!remux && !await media.canEncodeVideo("avc", { width: plan.width, height: plan.height, bitrate })) return;
    const audioCodec = !audio ? undefined
      : COPY_AUDIO.has(await audio.getCodec() ?? "") ? "copy" as const
      : await media.canEncodeAudio("aac") ? "aac" as const
      : await media.canEncodeAudio("opus") ? "opus" as const : undefined;
    // Never trade a smaller file for silently losing the soundtrack.
    if (audio && !audioCodec) return;
    if (hdrMode) {
      // HDR whose transfer is unknown cannot be tone mapped correctly.
      const transfer = hdrTransfer(colorSpace);
      if (!transfer) return;
      const { transcodeHdrVideo } = await import("./hdr-video.ts");
      const even = (n: number) => Math.max(2, Math.floor(n / 2) * 2);
      const buffer = await transcodeHdrVideo(media, input, {
        width: even(plan.width), height: even(plan.height), transfer, codec: "avc", bitrate,
        audioCodec, audioBitrate: settings.audioBitrateKbps * 1000, progress, signal,
      });
      return buffer && keepTranscode(plan.reasons, file.size, buffer.byteLength) ? new Blob([buffer], { type: "video/mp4" }) : undefined;
    }
    const output = new media.Output({ format: new media.Mp4OutputFormat({ fastStart: "in-memory" }), target: new media.BufferTarget() });
    const size = plan.reasons.includes("resolution") ? (width < height ? { width: plan.width } : { height: plan.height }) : {};
    const conversion = await media.Conversion.init({
      input, output,
      video: remux ? { codec: "avc" } : { ...size, codec: "avc", bitrate, forceTranscode: true },
      ...(audioCodec && audioCodec !== "copy" ? { audio: { codec: audioCodec, bitrate: settings.audioBitrateKbps * 1000 } } : {}),
    });
    if (!conversion.isValid || conversion.discardedTracks.length) return;
    const cancel = () => void conversion.cancel();
    signal?.addEventListener("abort", cancel, { once: true });
    conversion.onProgress = (fraction) => progress(fraction);
    try {
      await conversion.execute();
    } finally {
      signal?.removeEventListener("abort", cancel);
    }
    const buffer = output.target.buffer;
    if (!buffer) return;
    return keepTranscode(plan.reasons, file.size, buffer.byteLength) ? new Blob([buffer], { type: "video/mp4" }) : undefined;
  } catch {
    return;
  } finally {
    input.dispose();
  }
}

function mediaElement(file: Blob, tag: "video" | "audio") {
  return new Promise<HTMLVideoElement | HTMLAudioElement | undefined>((resolve) => {
    const element = document.createElement(tag);
    const url = URL.createObjectURL(file);
    const done = (value?: HTMLVideoElement | HTMLAudioElement) => { clearTimeout(timer); resolve(value); };
    const timer = setTimeout(() => done(), 8_000);
    element.preload = "metadata";
    element.muted = true;
    element.onloadeddata = () => done(element);
    element.onerror = () => { URL.revokeObjectURL(url); done(); };
    element.src = url;
    if (tag === "audio") element.onloadedmetadata = () => done(element);
  });
}

async function prepareTimed(file: File, contentType: string, settings: CompressionSettings, progress: (fraction: number) => void, signal?: AbortSignal): Promise<PreparedFile> {
  const looksLikeVideo = attachmentKind(contentType) === "video" || contentType.startsWith("video/");
  const transcoded = looksLikeVideo ? await transcodeVideo(file, contentType, settings, progress, signal) : undefined;
  const blob: Blob = transcoded ?? file;
  const type = transcoded ? "video/mp4" : contentType;
  const kind = attachmentKind(type);
  const prepared: PreparedFile = { blob, name: transcoded ? renamed(file.name, type) : file.name, contentType: type, kind, sourceSize: file.size };
  if (kind === "file") return prepared;
  const element = await mediaElement(blob, kind === "video" ? "video" : "audio");
  if (!element) return prepared;
  try {
    if (Number.isFinite(element.duration)) prepared.durationMs = Math.round(element.duration * 1000);
    if (element instanceof HTMLVideoElement && element.videoWidth && element.videoHeight) {
      prepared.width = element.videoWidth;
      prepared.height = element.videoHeight;
      // Poster frame from just after the start, past common black first frames.
      await new Promise<void>((resolve) => {
        const timer = setTimeout(resolve, 3_000);
        element.onseeked = () => { clearTimeout(timer); resolve(); };
        element.currentTime = Math.min(0.5, (element.duration || 1) / 2);
      });
      prepared.preview = await preview(element, element.videoWidth, element.videoHeight, settings.previewEdge);
    }
  } finally {
    URL.revokeObjectURL(element.src);
    element.removeAttribute("src");
  }
  return prepared;
}

/** Compress stills and video, measure media and draw previews in the browser.
 * Any failure falls back to the original file. */
export async function prepareFile(file: File, settings = DEFAULT_COMPRESSION, progress: (fraction: number) => void = () => undefined, signal?: AbortSignal): Promise<PreparedFile> {
  const contentType = declaredType(file);
  const kind = attachmentKind(contentType);
  const prepared = contentType.startsWith("image/") ? await prepareImage(file, contentType, settings)
    : kind === "video" || kind === "audio" || contentType.startsWith("video/") ? await prepareTimed(file, contentType, settings, progress, signal)
    : { blob: file, name: file.name, contentType, kind, sourceSize: file.size };
  // An original upload still loses its location and other metadata, losslessly.
  if (prepared.blob === file) prepared.blob = await stripMetadata(file, contentType);
  return prepared;
}
