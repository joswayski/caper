import type { ChatAttachment, ChatAttachmentKind } from "./types.ts";

export const MAX_ATTACHMENTS = 10;
const PREVIEW_EDGE = 640;
const PREVIEW_MAX_BYTES = 512 * 1024;
const PHOTO_QUALITY = 0.92;
const PREVIEW_QUALITY = 0.8;
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
  const extension = contentType === "image/webp" ? "webp" : contentType === "image/jpeg" ? "jpg" : undefined;
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

async function draw(source: CanvasImageSource, width: number, height: number, edge?: number) {
  const size = edge ? fitWithin(width, height, edge) : { width, height };
  const canvas = surface(size.width, size.height);
  const context = canvas.getContext("2d") as CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D | null;
  if (!context) return;
  context.imageSmoothingQuality = "high";
  context.drawImage(source, 0, 0, size.width, size.height);
  return canvas;
}

async function webpOrJpeg(canvas: Surface, quality: number) {
  return await encode(canvas, "image/webp", quality) ?? await encode(canvas, "image/jpeg", quality);
}

async function preview(source: CanvasImageSource, width: number, height: number) {
  const canvas = await draw(source, width, height, PREVIEW_EDGE);
  const blob = canvas && await webpOrJpeg(canvas, PREVIEW_QUALITY);
  return blob && blob.size <= PREVIEW_MAX_BYTES ? blob : undefined;
}

async function prepareImage(file: File): Promise<PreparedFile> {
  const base: PreparedFile = { blob: file, name: file.name, contentType: file.type, kind: attachmentKind(file.type), sourceSize: file.size };
  if (typeof createImageBitmap === "undefined") return base;
  const bitmap = await createImageBitmap(file).catch(() => undefined);
  if (!bitmap) return base;
  try {
    const { width, height } = bitmap;
    let prepared: PreparedFile = { ...base, width, height };
    if (compressible(file.type) && width * height <= MAX_COMPRESS_PIXELS) {
      // Re-encoding also drops EXIF metadata such as photo GPS coordinates.
      const canvas = await draw(bitmap, width, height);
      const compressed = canvas && await webpOrJpeg(canvas, PHOTO_QUALITY);
      if (compressed && keepCompressed(file, compressed)) {
        prepared = { ...prepared, blob: compressed, contentType: compressed.type, kind: "image", name: renamed(file.name, compressed.type) };
      }
    }
    if (prepared.kind === "image" && (Math.max(width, height) > PREVIEW_EDGE || prepared.blob.size > PREVIEW_MAX_BYTES)) {
      prepared.preview = await preview(bitmap, width, height);
    }
    return prepared;
  } finally {
    bitmap.close();
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

async function prepareTimed(file: File): Promise<PreparedFile> {
  const kind = attachmentKind(file.type);
  const prepared: PreparedFile = { blob: file, name: file.name, contentType: file.type, kind, sourceSize: file.size };
  const element = await mediaElement(file, kind === "video" ? "video" : "audio");
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
      prepared.preview = await preview(element, element.videoWidth, element.videoHeight);
    }
  } finally {
    URL.revokeObjectURL(element.src);
    element.removeAttribute("src");
  }
  return prepared;
}

/** Compress stills, measure media and draw previews in the browser. The API
 * verifies stored bytes independently; this only saves storage and bandwidth. */
export async function prepareFile(file: File): Promise<PreparedFile> {
  if (file.type.startsWith("image/")) return prepareImage(file);
  const kind = attachmentKind(file.type);
  if (kind === "video" || kind === "audio") return prepareTimed(file);
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
