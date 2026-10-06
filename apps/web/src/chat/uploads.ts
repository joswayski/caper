import type { ChatAttachment } from "./types.ts";

export const MAX_ATTACHMENTS = 10;
/** Waits between `/complete` attempts while the upload is not visible yet (409). */
const COMPLETE_RETRY_DELAYS_MS = [250, 500, 1_000, 2_000];

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit++; }
  return `${value >= 10 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

export interface UploadTransport {
  fetch: typeof fetch;
  /** PUT with progress; browsers use XMLHttpRequest because fetch cannot report upload progress. */
  put: (url: string, headers: Record<string, string>, body: Blob, progress: (fraction: number) => void, signal: AbortSignal) => Promise<void>;
  /** Pause between `/complete` retries; injectable so tests need not wait. */
  sleep?: (ms: number, signal: AbortSignal) => Promise<void>;
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

function sleep(ms: number, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    if (signal.aborted) { reject(signal.reason); return; }
    const timer = setTimeout(() => { signal.removeEventListener("abort", abort); resolve(); }, ms);
    const abort = () => { clearTimeout(timer); reject(signal.reason); };
    signal.addEventListener("abort", abort, { once: true });
  });
}

export interface UploadLimits {
  /** Largest original the API accepts, when the server reports it. */
  maxUploadBytes?: number;
}

/** Checked before reserving so an oversized file fails at once with a clear message. */
export function uploadSizeError(size: number, limits: UploadLimits): string | undefined {
  if (limits.maxUploadBytes !== undefined && size > limits.maxUploadBytes) {
    return `This file is larger than the ${formatBytes(limits.maxUploadBytes)} upload limit.`;
  }
}

/** Reserve, PUT the original straight to storage with exactly the signed
 * headers, then confirm. Files are never compressed here: the server's media
 * worker processes every upload. Returns the attachment (status "processing")
 * to send with a message. */
export async function uploadFile(channelId: string, file: File, limits: UploadLimits, transport: UploadTransport, progress: (fraction: number) => void, signal: AbortSignal): Promise<ChatAttachment> {
  const tooLarge = uploadSizeError(file.size, limits);
  if (tooLarge) throw new UploadError(tooLarge);
  const created = await transport.fetch("/api/assets", {
    method: "POST",
    headers: { "content-type": "application/json" },
    credentials: "same-origin",
    signal,
    body: JSON.stringify({ channelId, filename: file.name, contentType: file.type || "application/octet-stream", byteSize: file.size }),
  });
  if (!created.ok) throw await failure(created, "This file could not be uploaded.");
  const reservation = await created.json() as { id?: unknown; upload?: Partial<PresignedPut> };
  if (typeof reservation.id !== "string" || typeof reservation.upload?.url !== "string") throw new UploadError("The upload service returned an invalid response.");
  await transport.put(reservation.upload.url, reservation.upload.headers ?? {}, file, progress, signal);
  const path = `/api/assets/${encodeURIComponent(reservation.id)}/complete`;
  for (let attempt = 0; ; attempt++) {
    const completed = await transport.fetch(path, { method: "POST", credentials: "same-origin", signal });
    if (completed.ok) {
      progress(1);
      return await completed.json() as ChatAttachment;
    }
    // 409: storage has not made the object visible yet. Anything else is final.
    if (completed.status !== 409 || attempt >= COMPLETE_RETRY_DELAYS_MS.length) {
      if (completed.status === 422) throw new UploadError("The upload did not arrive intact. Try again.");
      throw await failure(completed, "This file could not be uploaded.");
    }
    await completed.body?.cancel().catch(() => undefined);
    await (transport.sleep ?? sleep)(COMPLETE_RETRY_DELAYS_MS[attempt], signal);
  }
}

export const browserTransport: UploadTransport = {
  fetch: (...args) => fetch(...args),
  put: (url, headers, body, progress, signal) => new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    request.open("PUT", url);
    // Exactly the signed headers; the browser adds the matching content-length.
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
 * Otherwise returns the server's limits from `/api/assets/usage`. */
export async function uploadSettings(fetcher: typeof fetch = fetch): Promise<UploadLimits | undefined> {
  const response = await fetcher("/api/assets/usage", { credentials: "same-origin", cache: "no-store" }).catch(() => undefined);
  if (!response?.ok) return;
  const body = await response.json().catch(() => undefined) as { maxUploadBytes?: unknown } | undefined;
  const max = body?.maxUploadBytes;
  return typeof max === "number" && Number.isSafeInteger(max) && max > 0 ? { maxUploadBytes: max } : {};
}

/** Fresh URLs for attachments whose signed URLs expired in a long-open tab.
 * Only ready attachments have a `url`. */
export async function refreshAttachmentUrls(ids: string[], fetcher: typeof fetch = fetch): Promise<Record<string, { url?: string; previewUrl?: string }>> {
  const response = await fetcher("/api/assets/urls", {
    method: "POST", headers: { "content-type": "application/json" }, credentials: "same-origin", body: JSON.stringify({ ids }),
  });
  if (!response.ok) return {};
  const body = await response.json().catch(() => undefined) as { urls?: Record<string, { url?: unknown; previewUrl?: unknown }> } | undefined;
  const safe = (value: unknown) => typeof value === "string" && /^https?:\/\//.test(value) ? value : undefined;
  const urls: Record<string, { url?: string; previewUrl?: string }> = {};
  for (const [id, entry] of Object.entries(body?.urls && typeof body.urls === "object" ? body.urls : {})) {
    const url = safe(entry?.url), previewUrl = safe(entry?.previewUrl);
    if (url || previewUrl) urls[id] = { ...(url ? { url } : {}), ...(previewUrl ? { previewUrl } : {}) };
  }
  return urls;
}

/** Seconds since epoch at which a signed URL stops working, if present. */
export function urlExpiry(url: string | undefined): number | undefined {
  if (!url) return;
  const expires = Number(new URL(url).searchParams.get("exp"));
  return Number.isSafeInteger(expires) && expires > 0 ? expires : undefined;
}
