import type { ChatAttachment, ChatAttachmentKind } from "./types.ts";
import { encodeIndexedPng } from "./png.ts";

export const MAX_ATTACHMENTS = 10;
const PREVIEW_EDGE = 640;
const PREVIEW_MAX_BYTES = 512 * 1024;
const PREVIEW_QUALITY = 0.8;

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

export const DEFAULT_COMPRESSION: CompressionSettings = {
  imageQuality: 92, imageMaxEdge: 4096, paletteColors: 256, previewEdge: 640,
  videoMaxHeight: 1080, videoBitrateKbps: 4000, audioBitrateKbps: 128,
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
// Decoding enormous images can exhaust memory on phones; upload those as-is.
const MAX_COMPRESS_PIXELS = 50_000_000;

/** Mirrors `assets::kind` on the API: only these render inline. */
export function attachmentKind(contentType: string): ChatAttachmentKind {
  if (["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif"].includes(contentType)) return "image";
  if (["video/mp4", "video/webm", "video/quicktime"].includes(contentType)) return "video";
  if (["audio/mpeg", "audio/mp4", "audio/x-m4a", "audio/aac", "audio/ogg", "audio/wav", "audio/x-wav", "audio/webm", "audio/flac"].includes(contentType)) return "audio";
  return "file";
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
  return `${value >= 10 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

/** Longest edge scaled to `edge`, never enlarged. */
export function fitWithin(width: number, height: number, edge = PREVIEW_EDGE) {
  const scale = Math.min(1, edge / Math.max(width, height));
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

/** Re-encode stills except animated or vector formats. Formats browsers cannot
 * show everywhere (HEIC) are converted whenever the browser can decode them. */
export function compressible(contentType: string) {
  return contentType.startsWith("image/") && !["image/gif", "image/svg+xml", "image/avif"].includes(contentType);
}

/** Keep the re-encoded file only when it is meaningfully smaller, or when the
 * original could not be displayed inline at all. */
export function keepCompressed(original: { type: string; size: number }, compressed: { size: number }) {
  return attachmentKind(original.type) !== "image" || compressed.size < original.size * 0.9;
}

export function renamed(name: string, contentType: string) {
  const extension = ({ "image/webp": "webp", "image/jpeg": "jpg", "image/png": "png", "video/mp4": "mp4" } as Record<string, string>)[contentType];
  if (!extension) return name;
  const dot = name.lastIndexOf(".");
  return `${dot > 0 ? name.slice(0, dot) : name}.${extension}`;
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

function surface(width: number, height: number): Surface {
  if (typeof OffscreenCanvas !== "undefined") return new OffscreenCanvas(width, height);
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  return canvas;
}

async function encode(canvas: Surface, type: string, quality: number): Promise<Blob | undefined> {
  const blob = "convertToBlob" in canvas
    ? await canvas.convertToBlob({ type, quality }).catch(() => undefined)
    : await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, type, quality));
  // Browsers that cannot encode a type silently return PNG instead.
  return blob && blob.type === type ? blob : undefined;
}

type Context2D = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

async function draw(source: CanvasImageSource, width: number, height: number, edge?: number) {
  const size = edge ? fitWithin(width, height, edge) : { width, height };
  const canvas = surface(size.width, size.height);
  const context = canvas.getContext("2d") as Context2D | null;
  if (!context) return;
  context.imageSmoothingQuality = "high";
  context.drawImage(source, 0, 0, size.width, size.height);
  return canvas;
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

async function preview(source: CanvasImageSource, width: number, height: number, edge = PREVIEW_EDGE) {
  const canvas = await draw(source, width, height, edge);
  const blob = canvas && await webpOrJpeg(canvas, PREVIEW_QUALITY);
  return blob && blob.size <= PREVIEW_MAX_BYTES ? blob : undefined;
}

/** Which encoding a still gets, in order of preference. */
export function stillPlan(settings: CompressionSettings, colors: "within-palette" | "too-many") {
  if (colors === "within-palette" && settings.paletteColors > 0) return "indexed-png" as const;
  return settings.imageQuality < 100 ? "lossy" as const : "lossless-png" as const;
}

async function prepareImage(file: File, settings: CompressionSettings): Promise<PreparedFile> {
  const base: PreparedFile = { blob: file, name: file.name, contentType: file.type, kind: attachmentKind(file.type), sourceSize: file.size };
  if (typeof createImageBitmap === "undefined") return base;
  // No colour-space or alpha conversion, so a palette encode is pixel-exact.
  const bitmap = await createImageBitmap(file, { colorSpaceConversion: "none", premultiplyAlpha: "none" }).catch(() => undefined);
  if (!bitmap) return base;
  try {
    const original = { width: bitmap.width, height: bitmap.height };
    const size = settings.imageMaxEdge > 0 ? fitWithin(original.width, original.height, settings.imageMaxEdge) : original;
    let prepared: PreparedFile = { ...base, ...size };
    if (compressible(file.type) && original.width * original.height <= MAX_COMPRESS_PIXELS) {
      // Re-encoding also drops EXIF metadata such as photo GPS coordinates.
      const canvas = await draw(bitmap, size.width, size.height);
      const context = canvas?.getContext("2d") as Context2D | null | undefined;
      let compressed: Blob | undefined;
      if (canvas && context) {
        const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
        const indexed = settings.paletteColors > 0 ? await encodeIndexedPng(pixels, canvas.width, canvas.height, settings.paletteColors) : undefined;
        const plan = stillPlan(settings, indexed ? "within-palette" : "too-many");
        compressed = plan === "indexed-png" ? new Blob([indexed as BlobPart], { type: "image/png" })
          : plan === "lossy" ? await webpOrJpeg(canvas, settings.imageQuality / 100)
          : await encode(canvas, "image/png", 1);
      }
      const resized = size.width !== original.width || size.height !== original.height;
      if (compressed && (keepCompressed(file, compressed) || (resized && compressed.size < file.size))) {
        prepared = { ...prepared, blob: compressed, contentType: compressed.type, kind: "image", name: renamed(file.name, compressed.type) };
      } else {
        prepared = { ...prepared, ...original };
      }
    } else {
      prepared = { ...prepared, ...original };
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

/** Output size for a transcode, or undefined to keep the original size. The
 * limit bounds the short edge ("1080p"), so portrait phone video keeps detail. */
export function videoTargetSize(width: number, height: number, maxShortEdge: number): { width: number } | { height: number } | undefined {
  if (maxShortEdge <= 0 || Math.min(width, height) <= maxShortEdge) return undefined;
  const even = Math.max(2, Math.floor(maxShortEdge / 2) * 2);
  return width < height ? { width: even } : { height: even };
}

/** Re-encode to H.264/AAC MP4 with WebCodecs. Returns undefined to keep the
 * original: unsupported browser or codec, a dropped track, or no saving. */
async function transcodeVideo(file: File, settings: CompressionSettings, progress: (fraction: number) => void): Promise<Blob | undefined> {
  if (settings.videoMaxHeight <= 0 || typeof VideoEncoder === "undefined") return;
  const media = await import("mediabunny");
  const input = new media.Input({ source: new media.BlobSource(file), formats: media.ALL_FORMATS });
  try {
    const track = await input.getPrimaryVideoTrack();
    if (!track) return;
    const size = videoTargetSize(track.displayWidth, track.displayHeight, settings.videoMaxHeight);
    const bitrate = settings.videoBitrateKbps * 1000;
    if (!await media.canEncodeVideo("avc", { bitrate })) return;
    const audioCodec = await media.canEncodeAudio("aac") ? "aac" as const : await media.canEncodeAudio("opus") ? "opus" as const : undefined;
    const hasAudio = !!await input.getPrimaryAudioTrack();
    if (hasAudio && !audioCodec) return;
    const output = new media.Output({ format: new media.Mp4OutputFormat({ fastStart: "in-memory" }), target: new media.BufferTarget() });
    const conversion = await media.Conversion.init({
      input, output,
      video: { ...size, codec: "avc", bitrate, forceTranscode: true },
      ...(hasAudio && audioCodec ? { audio: { codec: audioCodec, bitrate: settings.audioBitrateKbps * 1000 } } : {}),
    });
    // Never trade a smaller file for silently losing the soundtrack.
    if (!conversion.isValid || conversion.discardedTracks.length) return;
    conversion.onProgress = (fraction) => progress(fraction);
    await conversion.execute();
    const buffer = output.target.buffer;
    if (!buffer) return;
    const blob = new Blob([buffer], { type: "video/mp4" });
    return blob.size < file.size || attachmentKind(file.type) !== "video" ? blob : undefined;
  } catch {
    return;
  } finally {
    input.dispose();
  }
}

function mediaElement(file: File, tag: "video" | "audio") {
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

async function prepareTimed(file: File, settings: CompressionSettings, progress: (fraction: number) => void): Promise<PreparedFile> {
  const sourceKind = attachmentKind(file.type);
  const looksLikeVideo = sourceKind === "video" || file.type.startsWith("video/");
  const transcoded = looksLikeVideo ? await transcodeVideo(file, settings, progress) : undefined;
  const media = transcoded ? new File([transcoded], renamed(file.name, "video/mp4"), { type: "video/mp4" }) : file;
  const kind = attachmentKind(media.type);
  const prepared: PreparedFile = { blob: media, name: media.name, contentType: media.type, kind, sourceSize: file.size };
  if (kind === "file") return prepared;
  const element = await mediaElement(media, kind === "video" ? "video" : "audio");
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

/** Compress stills, measure media and draw previews in the browser. The API
 * verifies stored bytes independently; this only saves storage and bandwidth. */
export async function prepareFile(file: File, settings = DEFAULT_COMPRESSION, progress: (fraction: number) => void = () => undefined): Promise<PreparedFile> {
  if (file.type.startsWith("image/")) return prepareImage(file, settings);
  const kind = attachmentKind(file.type);
  if (kind === "video" || kind === "audio" || file.type.startsWith("video/")) return prepareTimed(file, settings, progress);
  return { blob: file, name: file.name, contentType: file.type, kind, sourceSize: file.size };
}

export interface UploadTransport {
  fetch: typeof fetch;
  /** PUT with progress; browsers use XMLHttpRequest because fetch cannot report upload progress. */
  put: (url: string, headers: Record<string, string>, body: Blob, progress: (fraction: number) => void, signal: AbortSignal) => Promise<void>;
}

export class UploadError extends Error {
  readonly storageFull: boolean;
  constructor(message: string, storageFull = false) { super(message); this.storageFull = storageFull; }
}

interface PresignedPut { method: "PUT"; url: string; headers: Record<string, string> }

async function failure(response: Response, fallback: string) {
  const body = await response.json().catch(() => undefined) as { error?: unknown; code?: unknown } | undefined;
  if (body?.code === "storage_full") return new UploadError("You’ve used all of your file storage.", true);
  if (response.status === 413) return new UploadError("This file is too large to upload.");
  return new UploadError(typeof body?.error === "string" ? body.error : fallback);
}

/** Reserve, upload straight to storage, then confirm. Returns the attachment
 * description to send with a message. */
export async function uploadPrepared(channelId: string, file: PreparedFile, transport: UploadTransport, progress: (fraction: number) => void, signal: AbortSignal): Promise<ChatAttachment> {
  const created = await transport.fetch("/api/assets", {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "same-origin",
    signal,
    body: JSON.stringify({
      channelId, filename: file.name, contentType: file.contentType, byteSize: file.blob.size,
      sourceByteSize: file.sourceSize || undefined, width: file.width, height: file.height, durationMs: file.durationMs,
      preview: file.preview ? { contentType: file.preview.type, byteSize: file.preview.size } : undefined,
    }),
  });
  if (!created.ok) throw await failure(created, "This file could not be uploaded.");
  const reservation = await created.json() as { id?: unknown; upload?: PresignedPut; previewUpload?: PresignedPut };
  if (typeof reservation.id !== "string" || !reservation.upload?.url) throw new UploadError("The upload service returned an invalid response.");
  const total = file.blob.size + (file.preview?.size ?? 0);
  if (file.preview && reservation.previewUpload) {
    await transport.put(reservation.previewUpload.url, reservation.previewUpload.headers, file.preview, () => undefined, signal);
  }
  await transport.put(reservation.upload.url, reservation.upload.headers, file.blob,
    (fraction) => progress(((file.preview?.size ?? 0) + fraction * file.blob.size) / total), signal);
  const completed = await transport.fetch(`/api/assets/${encodeURIComponent(reservation.id)}/complete`, { method: "POST", credentials: "same-origin", signal });
  if (!completed.ok) throw await failure(completed, "This file could not be uploaded.");
  progress(1);
  return await completed.json() as ChatAttachment;
}

export const browserTransport: UploadTransport = {
  fetch: (...args) => fetch(...args),
  put: (url, headers, body, progress, signal) => new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open("PUT", url);
    for (const [name, value] of Object.entries(headers)) request.setRequestHeader(name, value);
    request.upload.onprogress = (event) => { if (event.lengthComputable) progress(event.loaded / event.total); };
    request.onload = () => request.status >= 200 && request.status < 300 ? resolve() : reject(new UploadError("Storage refused the upload."));
    request.onerror = () => reject(new UploadError("The upload was interrupted."));
    request.onabort = () => reject(new DOMException("Upload cancelled.", "AbortError"));
    signal.addEventListener("abort", () => request.abort(), { once: true });
    request.send(body);
  }),
};

/** Uploads are optional server configuration: undefined hides the control.
 * Otherwise returns the server's current compression settings. */
export async function uploadSettings(fetcher: typeof fetch = fetch): Promise<CompressionSettings | undefined> {
  const response = await fetcher("/api/assets/usage", { credentials: "same-origin", cache: "no-store" }).catch(() => undefined);
  if (!response?.ok) return;
  const body = await response.json().catch(() => undefined) as { compression?: unknown } | undefined;
  return compressionSettings(body?.compression);
}

/** Fresh URLs for attachments whose signed URLs expired in a long-open tab. */
export async function refreshAttachmentUrls(ids: string[], fetcher: typeof fetch = fetch): Promise<Record<string, { url: string; previewUrl?: string }>> {
  const response = await fetcher("/api/assets/urls", {
    method: "POST", headers: { "content-type": "application/json" }, credentials: "same-origin", body: JSON.stringify({ ids }),
  });
  if (!response.ok) return {};
  const body = await response.json() as { urls?: Record<string, { url: string; previewUrl?: string }> };
  return body.urls ?? {};
}

/** Seconds since epoch at which a signed URL stops working, if present. */
export function urlExpiry(url: string | undefined): number | undefined {
  if (!url) return;
  const expires = Number(new URL(url).searchParams.get("exp"));
  return Number.isSafeInteger(expires) && expires > 0 ? expires : undefined;
}
